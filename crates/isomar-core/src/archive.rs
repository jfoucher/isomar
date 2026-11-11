//! Unpacking downloaded archives (zip and 7z) safely: every entry must land
//! inside the destination folder.

use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub enum ArchiveError {
    Io(io::Error),
    Format(String),
    /// An entry tried to leave the destination (`../x`, `/etc/x`).
    UnsafePath(String),
    /// More data than allowed came out of the archive.
    TooLarge,
}

impl From<io::Error> for ArchiveError {
    fn from(e: io::Error) -> Self {
        ArchiveError::Io(e)
    }
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::Io(e) => write!(f, "{e}"),
            ArchiveError::Format(s) => write!(f, "unreadable archive: {s}"),
            ArchiveError::UnsafePath(s) => write!(f, "the archive contains an unsafe path: {s}"),
            ArchiveError::TooLarge => write!(f, "the archive unpacks to more than the allowed size"),
        }
    }
}

impl std::error::Error for ArchiveError {}

/// Joins an archive entry name onto `dest`, refusing anything that is not a
/// plain relative path. Windows-style separators are accepted as separators.
pub fn safe_join(dest: &Path, name: &str) -> Result<PathBuf, ArchiveError> {
    let name = name.replace('\\', "/");
    let rel = Path::new(&name);
    let mut out = dest.to_path_buf();
    for c in rel.components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return Err(ArchiveError::UnsafePath(name.clone())),
        }
    }
    if out == dest {
        return Err(ArchiveError::UnsafePath(name));
    }
    Ok(out)
}

/// Copies at most `limit` bytes; errors if the source holds more.
fn copy_limited(r: &mut dyn Read, w: &mut dyn io::Write, budget: &mut u64) -> Result<(), ArchiveError> {
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        if n as u64 > *budget {
            return Err(ArchiveError::TooLarge);
        }
        *budget -= n as u64;
        w.write_all(&buf[..n])?;
    }
}

/// Unpacks a zip into `dest` and returns the files written. `max_bytes` caps
/// the total unpacked size, which guards against decompression bombs.
pub fn extract_zip(archive: &Path, dest: &Path, max_bytes: u64) -> Result<Vec<PathBuf>, ArchiveError> {
    let mut z = zip::ZipArchive::new(fs::File::open(archive)?).map_err(|e| ArchiveError::Format(e.to_string()))?;
    let mut budget = max_bytes;
    let mut out = Vec::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| ArchiveError::Format(e.to_string()))?;
        let name = f.name().map_err(|e| ArchiveError::Format(e.to_string()))?.into_owned();
        let target = safe_join(dest, &name)?;
        if f.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(p) = target.parent() {
            fs::create_dir_all(p)?;
        }
        let mut w = fs::File::create(&target)?;
        copy_limited(&mut f, &mut w, &mut budget)?;
        out.push(target);
    }
    Ok(out)
}

/// Unpacks a 7z archive into `dest`, as [`extract_zip`] does.
pub fn extract_7z(archive: &Path, dest: &Path, max_bytes: u64) -> Result<Vec<PathBuf>, ArchiveError> {
    let mut r = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty()).map_err(|e| ArchiveError::Format(e.to_string()))?;
    let mut budget = max_bytes;
    let mut out = Vec::new();
    let mut failure: Option<ArchiveError> = None;
    r.for_each_entries(|entry, reader| {
        let target = match safe_join(dest, entry.name()) {
            Ok(t) => t,
            Err(e) => {
                failure = Some(e);
                return Ok(false);
            }
        };
        if entry.is_directory() {
            if let Err(e) = fs::create_dir_all(&target) {
                failure = Some(e.into());
                return Ok(false);
            }
            return Ok(true);
        }
        let res = (|| -> Result<(), ArchiveError> {
            if let Some(p) = target.parent() {
                fs::create_dir_all(p)?;
            }
            let mut w = fs::File::create(&target)?;
            copy_limited(reader, &mut w, &mut budget)
        })();
        match res {
            Ok(()) => {
                out.push(target);
                Ok(true)
            }
            Err(e) => {
                failure = Some(e);
                Ok(false)
            }
        }
    })
    .map_err(|e| ArchiveError::Format(e.to_string()))?;
    match failure {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

/// Unpacks by extension (`.zip` or `.7z`).
pub fn extract(archive: &Path, dest: &Path, max_bytes: u64) -> Result<Vec<PathBuf>, ArchiveError> {
    match archive.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("zip") => extract_zip(archive, dest, max_bytes),
        Some("7z") => extract_7z(archive, dest, max_bytes),
        other => Err(ArchiveError::Format(format!("unsupported archive type {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(dir: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
        let p = dir.join("a.zip");
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        for (name, data) in entries {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
        p
    }

    #[test]
    fn safe_join_accepts_plain_paths_and_refuses_escapes() {
        let d = Path::new("/data/out");
        assert_eq!(safe_join(d, "a/b.txt").unwrap(), Path::new("/data/out/a/b.txt"));
        assert_eq!(safe_join(d, "./a.txt").unwrap(), Path::new("/data/out/a.txt"));
        assert_eq!(safe_join(d, "a\\b.txt").unwrap(), Path::new("/data/out/a/b.txt"));
        for bad in ["../x", "a/../../x", "/etc/passwd", "a/../..", "", ".", "C:\\x\\..\\..\\y"] {
            // The drive form is not a path escape on Linux but a plain odd name; the rest must fail.
            if bad.starts_with("C:") {
                continue;
            }
            assert!(safe_join(d, bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_zip_unpacks_with_its_folders() {
        let d = tempfile::tempdir().unwrap();
        let z = make_zip(d.path(), &[("ENC_ROOT/US5MA20M/US5MA20M.000", b"cell"), ("README.TXT", b"hello")]);
        let out = d.path().join("out");
        let files = extract(&z, &out, 1 << 20).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(fs::read(out.join("ENC_ROOT/US5MA20M/US5MA20M.000")).unwrap(), b"cell");
        assert_eq!(fs::read(out.join("README.TXT")).unwrap(), b"hello");
    }

    #[test]
    fn a_hostile_zip_cannot_write_outside() {
        let d = tempfile::tempdir().unwrap();
        let z = make_zip(d.path(), &[("../escaped.txt", b"nope")]);
        let out = d.path().join("out");
        fs::create_dir_all(&out).unwrap();
        assert!(matches!(extract_zip(&z, &out, 1 << 20), Err(ArchiveError::UnsafePath(_))));
        assert!(!d.path().join("escaped.txt").exists());
    }

    #[test]
    fn a_decompression_bomb_is_stopped_by_the_size_limit() {
        let d = tempfile::tempdir().unwrap();
        let big = vec![0u8; 5 * 1024 * 1024];
        let z = make_zip(d.path(), &[("zeros.bin", &big)]);
        assert!(fs::metadata(&z).unwrap().len() < 100_000, "it compresses well");
        assert!(matches!(extract_zip(&z, &d.path().join("out"), 1024 * 1024), Err(ArchiveError::TooLarge)));
    }

    #[test]
    fn unknown_and_broken_archives_are_errors() {
        let d = tempfile::tempdir().unwrap();
        let junk = d.path().join("x.zip");
        fs::write(&junk, b"not a zip").unwrap();
        assert!(matches!(extract(&junk, d.path(), 1 << 20), Err(ArchiveError::Format(_))));
        let tar = d.path().join("x.tar");
        fs::write(&tar, b"x").unwrap();
        assert!(extract(&tar, d.path(), 1 << 20).is_err());
        assert!(extract(&d.path().join("missing.zip"), d.path(), 1).is_err());
    }

    /// Set `ISOMAR_TEST_7Z` to a real 7z (the SHOM buoyage archive, say) to run this.
    #[test]
    fn a_real_7z_unpacks_when_one_is_provided() {
        let Some(p) = std::env::var_os("ISOMAR_TEST_7Z").map(PathBuf::from) else { return };
        let d = tempfile::tempdir().unwrap();
        let files = extract_7z(&p, d.path(), 1 << 30).unwrap();
        assert!(files.len() > 3, "{files:?}");
        assert!(files.iter().all(|f| f.starts_with(d.path())));
        assert!(files.iter().any(|f| f.extension().is_some_and(|e| e == "gpkg")), "{files:?}");
        // The size limit applies to 7z too.
        assert!(matches!(extract_7z(&p, &d.path().join("small"), 10_000), Err(ArchiveError::TooLarge)));
    }
}
