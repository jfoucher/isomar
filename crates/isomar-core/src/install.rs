//! Downloading and installing a registry entry into the data folder.

use super::archive::{self, ArchiveError};
use super::registry::Entry;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

const DEFAULT_MAX_UNPACKED: u64 = 4 * 1024 * 1024 * 1024;

/// The file in an installed folder that says what was downloaded, to tell later whether the
/// source has changed since.
pub const RECEIPT: &str = ".isomar-source.json";
/// The receipt's name before the app was renamed from Chart to Isomar, still read.
const LEGACY_RECEIPT: &str = ".chart-source.json";

/// What the server said about a download.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Remote {
    #[serde(default)]
    pub etag: Option<String>,
    /// As sent (an HTTP date).
    #[serde(default)]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub length: Option<u64>,
}

impl Remote {
    fn from_headers(h: &ureq::http::HeaderMap) -> Remote {
        let text = |name: &str| h.get(name).and_then(|v| v.to_str().ok()).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        Remote { etag: text("etag"), last_modified: text("last-modified"), length: text("content-length").and_then(|v| v.parse().ok()) }
    }

    fn modified(&self) -> Option<SystemTime> {
        let t = chrono::DateTime::parse_from_rfc2822(self.last_modified.as_deref()?).ok()?;
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(t.timestamp()).ok()?))
    }
}

/// What an installed folder came from.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Receipt {
    pub id: String,
    pub url: String,
    #[serde(flatten)]
    pub remote: Remote,
}

/// The receipt of the install in `folder`, if it has one (installs before receipts have none).
pub fn receipt(folder: &Path) -> Option<Receipt> {
    let bytes = fs::read(folder.join(RECEIPT)).or_else(|_| fs::read(folder.join(LEGACY_RECEIPT))).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Whether what the server offers now (`now`) differs from the install in `folder`.
/// With a receipt, the ETag decides, else the date and size; without one, a source changed
/// after the folder was made.
pub fn is_newer(folder: &Path, url: &str, now: &Remote) -> bool {
    match receipt(folder).filter(|r| r.url == url) {
        Some(r) => {
            if let (Some(a), Some(b)) = (&r.remote.etag, &now.etag) {
                return a != b;
            }
            if r.remote.last_modified.is_some() && now.last_modified.is_some() {
                return r.remote.last_modified != now.last_modified || (r.remote.length.is_some() && now.length.is_some() && r.remote.length != now.length);
            }
            false
        }
        None => {
            let made = fs::metadata(folder).and_then(|m| m.modified()).ok();
            matches!((made, now.modified()), (Some(made), Some(changed)) if changed > made)
        }
    }
}

/// `body` is how long the whole body may take to arrive (ureq has no per-read timeout): a check
/// reads none, a download of hundreds of MB on a slow line needs hours.
fn agent(body: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_body(Some(body))
        .user_agent("isomar/0.1 (open-source chart plotter)")
        .build()
        .into()
}

/// Asks the server about `url` without downloading it.
pub fn check_remote(url: &str) -> Result<Remote, InstallError> {
    let resp = agent(Duration::from_secs(120)).head(url).call().map_err(|e| InstallError::Network(e.to_string()))?;
    Ok(Remote::from_headers(resp.headers()))
}

/// The bytes in the files under `dir`.
pub fn folder_size(dir: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => folder_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

/// Deletes an installed source from `data_dir`. The folder is renamed away first, so a
/// half-deleted folder is never read as charts.
pub fn uninstall(entry: &Entry, data_dir: &Path) -> Result<(), InstallError> {
    let spec = entry.install.as_ref().ok_or(InstallError::NotInstallable)?;
    let target = data_dir.join(&spec.dir);
    if !target.exists() {
        return Ok(());
    }
    let gone = data_dir.join(format!(".removing-{}", entry.id));
    let _ = fs::remove_dir_all(&gone);
    fs::rename(&target, &gone).map_err(io)?;
    fs::remove_dir_all(&gone).map_err(io)
}

#[derive(Debug)]
pub enum InstallError {
    Cancelled,
    NotInstallable,
    Network(String),
    /// What came back is not the kind of file the registry promised.
    WrongFile(String),
    Archive(ArchiveError),
    Io(String),
}

impl From<ArchiveError> for InstallError {
    fn from(e: ArchiveError) -> Self {
        InstallError::Archive(e)
    }
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Cancelled => write!(f, "cancelled"),
            InstallError::NotInstallable => write!(f, "this source cannot be installed automatically"),
            InstallError::Network(s) => write!(f, "network error: {s}"),
            InstallError::WrongFile(s) => write!(f, "the download is not what was expected ({s})"),
            InstallError::Archive(e) => write!(f, "{e}"),
            InstallError::Io(s) => write!(f, "disk error: {s}"),
        }
    }
}

impl std::error::Error for InstallError {}

fn io(e: std::io::Error) -> InstallError {
    InstallError::Io(e.to_string())
}

/// Streams `url` to `dest`, reporting `(bytes so far, total if known)`, and returns what the
/// server said about it.
pub fn download_to(url: &str, dest: &Path, cancel: &AtomicBool, progress: &dyn Fn(u64, Option<u64>)) -> Result<Remote, InstallError> {
    let mut resp = agent(Duration::from_secs(6 * 3600)).get(url).call().map_err(|e| InstallError::Network(e.to_string()))?;
    let remote = Remote::from_headers(resp.headers());
    let total = remote.length;
    let mut reader = resp.body_mut().as_reader();
    let mut out = fs::File::create(dest).map_err(io)?;
    let mut buf = [0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(out);
            let _ = fs::remove_file(dest);
            return Err(InstallError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| InstallError::Network(e.to_string()))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(io)?;
        done += n as u64;
        progress(done, total);
    }
    if let Some(t) = total
        && done < t
    {
        drop(out);
        let _ = fs::remove_file(dest);
        return Err(InstallError::Network("the download was cut short".into()));
    }
    Ok(remote)
}

/// The kind of archive a file really is, from its first bytes.
pub fn sniff(path: &Path) -> Option<&'static str> {
    let mut head = [0u8; 6];
    fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    if &head[..2] == b"PK" {
        Some("zip")
    } else if head == [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C] {
        Some("7z")
    } else {
        None
    }
}

/// Installs `entry` under `data_dir/<install.dir>` and returns that folder.
/// The folder appears only when everything succeeded; an earlier copy is
/// replaced at the end, never before.
pub fn install(entry: &Entry, data_dir: &Path, cancel: &AtomicBool, progress: &dyn Fn(u64, Option<u64>)) -> Result<PathBuf, InstallError> {
    let spec = entry.install.as_ref().filter(|_| entry.installable()).ok_or(InstallError::NotInstallable)?;
    let staging = data_dir.join(format!(".installing-{}", entry.id));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(io)?;
    let result = (|| -> Result<(), InstallError> {
        let file_name = entry.url.split('?').next().and_then(|u| u.rsplit('/').next()).filter(|n| !n.is_empty()).unwrap_or("download").to_string();
        let downloaded = staging.join(format!("download-{file_name}"));
        let remote = download_to(&entry.url, &downloaded, cancel, progress)?;
        let unpacked = staging.join("content");
        fs::create_dir_all(&unpacked).map_err(io)?;
        match spec.archive.as_str() {
            "file" => {
                fs::rename(&downloaded, unpacked.join(&file_name)).map_err(io)?;
            }
            kind => {
                let wanted = if kind == "7z" { "7z" } else { "zip" };
                if sniff(&downloaded) != Some(wanted) {
                    return Err(InstallError::WrongFile(format!("expected a {wanted} archive")));
                }
                // Extraction picks the format from the extension, so name the file for it.
                let named = staging.join(format!("archive.{wanted}"));
                fs::rename(&downloaded, &named).map_err(io)?;
                archive::extract(&named, &unpacked, spec.max_unpacked_bytes.unwrap_or(DEFAULT_MAX_UNPACKED))?;
            }
        }
        // A cancel during the unpacking lands here: nothing is swapped in.
        if cancel.load(Ordering::Relaxed) {
            return Err(InstallError::Cancelled);
        }
        let receipt = Receipt { id: entry.id.clone(), url: entry.url.clone(), remote };
        fs::write(unpacked.join(RECEIPT), serde_json::to_vec_pretty(&receipt).map_err(|e| InstallError::Io(e.to_string()))?).map_err(io)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    let target = data_dir.join(&spec.dir);
    let old = data_dir.join(format!(".old-{}", entry.id));
    let _ = fs::remove_dir_all(&old);
    if target.exists() {
        fs::rename(&target, &old).map_err(io)?;
    }
    fs::rename(staging.join("content"), &target).map_err(io)?;
    let _ = fs::remove_dir_all(&old);
    let _ = fs::remove_dir_all(&staging);
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Install, Kind, Rating, Status};
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::Arc;

    fn serve(routes: Vec<(&'static str, u16, Vec<u8>)>) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let routes = Arc::new(routes);
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut s) = c else { continue };
                let routes = routes.clone();
                std::thread::spawn(move || {
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let mut line = String::new();
                    let _ = r.read_line(&mut line);
                    let path = line.split(' ').nth(1).unwrap_or("/").to_string();
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).unwrap_or(0) <= 2 {
                            break;
                        }
                    }
                    let (code, body) = routes.iter().find(|(p, _, _)| path.starts_with(p)).map(|(_, c, b)| (*c, b.clone())).unwrap_or((404, vec![]));
                    let _ = write!(s, "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = s.write_all(&body);
                });
            }
        });
        port
    }

    fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(&mut buf);
        for (n, d) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(d).unwrap();
        }
        z.finish().unwrap();
        buf.into_inner()
    }

    fn entry(port: u16, path: &str, archive: &str) -> Entry {
        Entry {
            id: "t".into(),
            name: "T".into(),
            country: "XX".into(),
            region: String::new(),
            bbox: [0.0, 0.0, 1.0, 1.0],
            areas: Vec::new(),
            rating: Rating::B,
            kind: Kind::Enc,
            format: String::new(),
            url: format!("http://127.0.0.1:{port}{path}"),
            size_bytes: Some(10),
            licence: "test".into(),
            attribution: String::new(),
            nav_grade: false,
            encryption: "none".into(),
            redistribute: true,
            status: Status::Live,
            note: String::new(),
            install: Some(Install { archive: archive.into(), dir: "charts".into(), max_unpacked_bytes: Some(1 << 20) }),
        }
    }

    fn quiet(_: u64, _: Option<u64>) {}

    #[test]
    fn a_zip_is_downloaded_unpacked_and_put_in_place() {
        let port = serve(vec![("/a.zip", 200, zip_bytes(&[("ENC_ROOT/X/X.000", b"cell"), ("readme.txt", b"hi")]))]);
        let d = tempfile::tempdir().unwrap();
        let seen = std::sync::Mutex::new((0u64, None));
        let out = install(&entry(port, "/a.zip", "zip"), d.path(), &AtomicBool::new(false), &|n, t| *seen.lock().unwrap() = (n, t)).unwrap();
        assert_eq!(out, d.path().join("charts"));
        assert_eq!(fs::read(out.join("ENC_ROOT/X/X.000")).unwrap(), b"cell");
        let (n, t) = *seen.lock().unwrap();
        assert!(n > 0 && t == Some(n), "progress reached the total: {n} {t:?}");
        // No staging folders are left behind.
        let leftovers: Vec<_> = fs::read_dir(d.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(leftovers, vec!["charts"]);
    }

    #[test]
    fn installing_again_replaces_the_old_copy() {
        let port = serve(vec![("/a.zip", 200, zip_bytes(&[("new.txt", b"2")]))]);
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("charts")).unwrap();
        fs::write(d.path().join("charts/old.txt"), "1").unwrap();
        install(&entry(port, "/a.zip", "zip"), d.path(), &AtomicBool::new(false), &quiet).unwrap();
        assert!(d.path().join("charts/new.txt").exists() && !d.path().join("charts/old.txt").exists());
    }

    #[test]
    fn a_failed_install_leaves_the_old_copy_alone() {
        let port = serve(vec![("/bad.zip", 200, b"<html>not an archive</html>".to_vec())]);
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("charts")).unwrap();
        fs::write(d.path().join("charts/keep.txt"), "keep").unwrap();
        let e = install(&entry(port, "/bad.zip", "zip"), d.path(), &AtomicBool::new(false), &quiet).unwrap_err();
        assert!(matches!(e, InstallError::WrongFile(_)), "{e}");
        assert_eq!(fs::read(d.path().join("charts/keep.txt")).unwrap(), b"keep");
        assert!(!d.path().join(".installing-t").exists(), "staging is cleaned up");
    }

    #[test]
    fn hostile_archives_are_refused() {
        let port = serve(vec![("/evil.zip", 200, zip_bytes(&[("../../escape.txt", b"x")]))]);
        let d = tempfile::tempdir().unwrap();
        let sub = d.path().join("data");
        fs::create_dir_all(&sub).unwrap();
        let e = install(&entry(port, "/evil.zip", "zip"), &sub, &AtomicBool::new(false), &quiet).unwrap_err();
        assert!(matches!(e, InstallError::Archive(ArchiveError::UnsafePath(_))), "{e}");
        assert!(!d.path().join("escape.txt").exists());
        assert!(!sub.join("charts").exists());
    }

    #[test]
    fn missing_files_and_unsupported_entries_are_reported() {
        let port = serve(vec![]);
        let d = tempfile::tempdir().unwrap();
        assert!(matches!(install(&entry(port, "/nope.zip", "zip"), d.path(), &AtomicBool::new(false), &quiet), Err(InstallError::Network(_))));
        let mut paid = entry(port, "/x", "zip");
        paid.install = None;
        assert!(matches!(install(&paid, d.path(), &AtomicBool::new(false), &quiet), Err(InstallError::NotInstallable)));
        let mut s63 = entry(port, "/x", "zip");
        s63.encryption = "s63".into();
        assert!(matches!(install(&s63, d.path(), &AtomicBool::new(false), &quiet), Err(InstallError::NotInstallable)));
    }

    #[test]
    fn plain_files_are_saved_under_their_own_name() {
        let port = serve(vec![("/dir/chart.kap", 200, b"BSB/data".to_vec())]);
        let d = tempfile::tempdir().unwrap();
        let out = install(&entry(port, "/dir/chart.kap?x=1", "file"), d.path(), &AtomicBool::new(false), &quiet).unwrap();
        assert_eq!(fs::read(out.join("chart.kap")).unwrap(), b"BSB/data");
    }

    #[test]
    fn cancelling_stops_and_cleans_up() {
        let port = serve(vec![("/a.zip", 200, zip_bytes(&[("a.txt", b"x")]))]);
        let d = tempfile::tempdir().unwrap();
        let e = install(&entry(port, "/a.zip", "zip"), d.path(), &AtomicBool::new(true), &quiet).unwrap_err();
        assert!(matches!(e, InstallError::Cancelled), "{e}");
        assert!(fs::read_dir(d.path()).unwrap().flatten().all(|e| e.file_name() != "charts"));
    }

    #[test]
    fn an_install_keeps_a_receipt_and_can_be_removed() {
        let port = serve(vec![("/a.zip", 200, zip_bytes(&[("a.txt", b"x")]))]);
        let d = tempfile::tempdir().unwrap();
        let e = entry(port, "/a.zip", "zip");
        let out = install(&e, d.path(), &AtomicBool::new(false), &quiet).unwrap();
        let r = receipt(&out).expect("a receipt");
        assert_eq!((r.id.as_str(), r.url.as_str()), ("t", e.url.as_str()));
        assert!(r.remote.length.is_some());
        assert!(folder_size(&out) > 1);
        uninstall(&e, d.path()).unwrap();
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 0, "nothing left");
        uninstall(&e, d.path()).unwrap();
    }

    #[test]
    fn a_source_is_newer_when_the_server_says_it_changed() {
        let d = tempfile::tempdir().unwrap();
        let write = |remote: Remote| fs::write(d.path().join(RECEIPT), serde_json::to_vec(&Receipt { id: "t".into(), url: "u".into(), remote }).unwrap()).unwrap();
        let tag = |t: &str| Remote { etag: Some(t.into()), ..Default::default() };
        write(tag("1"));
        assert!(!is_newer(d.path(), "u", &tag("1")));
        assert!(is_newer(d.path(), "u", &tag("2")));
        let dated = |t: &str, n: u64| Remote { etag: None, last_modified: Some(t.into()), length: Some(n) };
        write(dated("Mon, 26 Oct 2026 10:00:00 GMT", 5));
        assert!(!is_newer(d.path(), "u", &dated("Mon, 26 Oct 2026 10:00:00 GMT", 5)));
        assert!(is_newer(d.path(), "u", &dated("Tue, 27 Oct 2026 10:00:00 GMT", 5)));
        assert!(is_newer(d.path(), "u", &dated("Mon, 26 Oct 2026 10:00:00 GMT", 6)));
        // An install from before receipts: compared with when the folder was made.
        fs::remove_file(d.path().join(RECEIPT)).unwrap();
        assert!(!is_newer(d.path(), "u", &dated("Mon, 01 Jan 2024 10:00:00 GMT", 5)));
        assert!(is_newer(d.path(), "u", &dated("Fri, 01 Jan 2100 10:00:00 GMT", 5)));
        assert!(!is_newer(d.path(), "u", &Remote::default()), "no news is no update");
    }

    #[test]
    fn sniffing_recognises_archives_by_content() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("z");
        fs::write(&z, zip_bytes(&[("a", b"b")])).unwrap();
        assert_eq!(sniff(&z), Some("zip"));
        let s = d.path().join("s");
        fs::write(&s, [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C, 0, 4]).unwrap();
        assert_eq!(sniff(&s), Some("7z"));
        let h = d.path().join("h");
        fs::write(&h, b"<html>hello world</html>").unwrap();
        assert_eq!(sniff(&h), None);
        assert_eq!(sniff(&d.path().join("missing")), None);
    }
}
