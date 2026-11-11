//! Finding chart data on disk.
//!
//! Data is looked for, in order, in `$ISOMAR_DATA_DIR`, the user data directory
//! (`~/.local/share/isomar/data`) and `./testdata` next to the working directory,
//! which is where `scripts/fetch-testdata.sh` puts it.

use std::path::{Path, PathBuf};

/// The directories searched for data, most specific first.
pub fn roots() -> Vec<PathBuf> {
    let mut r = Vec::new();
    if let Some(d) = std::env::var_os("ISOMAR_DATA_DIR") {
        r.push(PathBuf::from(d));
    }
    if let Some(d) = dirs::data_dir() {
        r.push(d.join("isomar").join("data"));
    }
    r.push(PathBuf::from("testdata"));
    r
}

/// Where downloaded and generated data goes.
pub fn user_data_dir() -> PathBuf {
    std::env::var_os("ISOMAR_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|d| d.join("isomar").join("data")))
        .unwrap_or_else(|| PathBuf::from("testdata"))
}

/// Where the user's own data lives: `ISOMAR_STATE_DIR`, else `isomar` in the
/// platform's data directory.
fn state_dir() -> PathBuf {
    std::env::var_os("ISOMAR_STATE_DIR").map(PathBuf::from).unwrap_or_else(|| dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("isomar"))
}

/// Moves the folders the app used under its old name, Chart, to the new one: `chart` becomes `isomar`
/// in the platform's config, data and cache directories, each only when the new one does not exist yet.
/// Called once at start-up by each shell, before anything reads or writes there. A run with its own
/// folders (`ISOMAR_CONFIG_DIR`, `ISOMAR_STATE_DIR` or `ISOMAR_DATA_DIR`: tests, screenshots) moves nothing.
pub fn migrate_legacy() {
    if ["ISOMAR_CONFIG_DIR", "ISOMAR_STATE_DIR", "ISOMAR_DATA_DIR", "ISOMAR_CACHE_DIR"].iter().any(|v| std::env::var_os(v).is_some()) {
        return;
    }
    // On macOS the config and data directories are the same one; the second move then finds nothing.
    for base in [dirs::config_dir(), dirs::data_dir(), dirs::cache_dir()].into_iter().flatten() {
        migrate_in(&base);
    }
}

fn migrate_in(base: &Path) {
    let old = base.join("chart");
    if old.is_dir() {
        merge(&old, &base.join("isomar"));
    }
}

/// Moves `old` to `new`; where `new` already exists (the tests log into the cache, a screenshot run
/// makes one), moves in what it lacks, folder by folder, keeping its own copy of anything in both,
/// and removes `old` once it is empty.
fn merge(old: &Path, new: &Path) {
    if !new.exists() {
        if let Err(e) = std::fs::rename(old, new) {
            eprintln!("isomar: could not move {} to {}: {e}", old.display(), new.display());
        }
        return;
    }
    if !new.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(old) else { return };
    for entry in entries.flatten() {
        let (from, to) = (entry.path(), new.join(entry.file_name()));
        if from.is_dir() && !from.is_symlink() {
            merge(&from, &to);
        } else if !to.exists()
            && let Err(e) = std::fs::rename(&from, &to)
        {
            eprintln!("isomar: could not move {} to {}: {e}", from.display(), to.display());
        }
    }
    let _ = std::fs::remove_dir(old);
}

/// The single polar the versions before the polar library saved (`routing::library::Library::adopt_legacy`).
pub fn legacy_polar_path() -> PathBuf {
    state_dir().join("polar.csv")
}

/// The user's polars (`routing::library`).
pub fn polars_dir() -> PathBuf {
    state_dir().join("polars")
}

/// The navigation database (waypoints, routes, tracks).
pub fn nav_db_path() -> PathBuf {
    state_dir().join("navobj.sqlite")
}

/// Cache directory for derived data (indexes, downloads in progress).
pub fn cache_dir() -> PathBuf {
    // A phone's shell names its own (Android has no home folder for `dirs` to start from).
    if let Some(d) = std::env::var_os("ISOMAR_CACHE_DIR") {
        return PathBuf::from(d);
    }
    dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".cache")).join("isomar")
}

/// The first existing file `rel` under any of `roots`.
pub fn find_in(roots: &[PathBuf], rel: &str) -> Option<PathBuf> {
    roots.iter().map(|r| r.join(rel)).find(|p| p.is_file())
}

pub fn find(rel: &str) -> Option<PathBuf> {
    find_in(&roots(), rel)
}

/// Files below `dir` (searched to `depth` levels) whose extension matches
/// `ext`, case-insensitively, in a stable order. Hidden files and folders are
/// skipped: an archive made on a Mac holds a `._NAME.KAP` of Finder metadata
/// beside every `NAME.KAP`, and each was reported as a chart that failed to open.
pub fn walk(dir: &Path, ext: &str, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_into(dir, ext, depth, &mut out);
    out.sort();
    out
}

fn walk_into(dir: &Path, ext: &str, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if e.file_name().as_encoded_bytes().starts_with(b".") {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                walk_into(&p, ext, depth - 1, out);
            }
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
            out.push(p);
        }
    }
}

/// Every file with extension `ext` under `<root>/<prefix>*` for each root.
pub fn find_all(prefix: &str, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots() {
        let Ok(rd) = std::fs::read_dir(&root) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && e.file_name().to_string_lossy().starts_with(prefix) {
                out.extend(walk(&p, ext, 4));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// True for names like `FR571220`: S-57 cell names have eight characters,
/// two for the producer, one for the usage band and five for the cell.
pub fn is_cell_name(stem: &str) -> bool {
    stem.len() == 8 && stem.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Every S-57 base cell (`*.000`) below `dirs`, each cell once. Dated copies
/// such as `FR571220_20071120.000` are older editions and are left out.
pub fn enc_cells(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        for p in walk(d, "000", 6) {
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if is_cell_name(&stem) {
                out.push(p);
            }
        }
    }
    // Folders given twice, or one inside another, find the same cells.
    out.sort();
    out.dedup();
    out
}

/// BSB/KAP raster charts below `dirs`.
pub fn kap_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = dirs.iter().flat_map(|d| walk(d, "kap", 6)).collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn kap_discovery_is_case_insensitive() {
        use super::*;
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("a/b")).unwrap();
        for n in ["a/one.KAP", "a/b/two.kap", "a/three.txt"] {
            std::fs::write(d.path().join(n), "").unwrap();
        }
        assert_eq!(kap_files(&[d.path().to_path_buf(), d.path().to_path_buf()]).len(), 2);
    }

    #[test]
    fn cell_names_have_eight_characters() {
        assert!(super::is_cell_name("FR571220"));
        assert!(super::is_cell_name("US5MA20M"));
        assert!(!super::is_cell_name("FR571220_20071120"));
        assert!(!super::is_cell_name("catalog"));
        assert!(!super::is_cell_name("FR57-220"));
    }

    #[test]
    fn enc_discovery_skips_dated_editions_and_non_cells() {
        use super::*;
        let d = tempfile::tempdir().unwrap();
        let sub = d.path().join("ENC_ROOT/FR571220");
        std::fs::create_dir_all(&sub).unwrap();
        for n in ["FR571220.000", "FR571220.001", "FR571220_20071120.000", "CATALOG.031", "notes.000"] {
            std::fs::write(sub.join(n), "").unwrap();
        }
        let found = enc_cells(&[d.path().to_path_buf(), d.path().join("ENC_ROOT")]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].ends_with("FR571220.000"));
        assert!(enc_cells(&[d.path().join("missing")]).is_empty());
    }

    use super::*;

    #[test]
    fn finds_files_under_the_first_matching_root() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(b.path().join("x")).unwrap();
        std::fs::write(b.path().join("x/f.txt"), "hi").unwrap();
        let roots = vec![a.path().to_path_buf(), b.path().to_path_buf()];
        assert_eq!(find_in(&roots, "x/f.txt"), Some(b.path().join("x/f.txt")));
        assert_eq!(find_in(&roots, "x/missing.txt"), None);
        // A directory is not a file.
        assert_eq!(find_in(&roots, "x"), None);
    }

    #[test]
    fn walk_matches_extensions_case_insensitively_and_respects_depth() {
        let d = tempfile::tempdir().unwrap();
        let deep = d.path().join("a/b/c");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(d.path().join("one.ASC"), "").unwrap();
        std::fs::write(d.path().join("a/two.asc"), "").unwrap();
        std::fs::write(deep.join("three.asc"), "").unwrap();
        std::fs::write(d.path().join("skip.txt"), "").unwrap();
        let shallow = walk(d.path(), "asc", 1);
        assert_eq!(shallow.len(), 2);
        assert_eq!(walk(d.path(), "asc", 5).len(), 3);
        assert!(walk(&d.path().join("nope"), "asc", 3).is_empty());
    }

    #[test]
    fn walk_skips_hidden_files_and_folders() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".hidden")).unwrap();
        for n in ["IO03.KAP", "._IO03.KAP", ".hidden/IO04.KAP"] {
            std::fs::write(d.path().join(n), "").unwrap();
        }
        assert_eq!(kap_files(&[d.path().to_path_buf()]), vec![d.path().join("IO03.KAP")]);
    }

    #[test]
    fn the_old_folder_moves_or_merges_into_the_new_one() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("chart/tiles/7")).unwrap();
        std::fs::write(d.path().join("chart/navobj.sqlite"), "routes").unwrap();
        migrate_in(d.path());
        assert!(!d.path().join("chart").exists());
        assert_eq!(std::fs::read_to_string(d.path().join("isomar/navobj.sqlite")).unwrap(), "routes");
        assert!(d.path().join("isomar/tiles/7").is_dir());

        // A new folder made first (the tests' log): what it lacks moves in, its own copies stay.
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("chart/tiles/7")).unwrap();
        std::fs::create_dir_all(d.path().join("isomar/tiles/8")).unwrap();
        std::fs::write(d.path().join("chart/coast.sqlite"), "coast").unwrap();
        std::fs::write(d.path().join("chart/tiles/7/1.pbf"), "old").unwrap();
        std::fs::write(d.path().join("chart/tiles/a.pbf"), "old").unwrap();
        std::fs::write(d.path().join("isomar/tiles/a.pbf"), "new").unwrap();
        migrate_in(d.path());
        let read = |p: &str| std::fs::read_to_string(d.path().join(p)).unwrap();
        assert_eq!(read("isomar/coast.sqlite"), "coast");
        assert_eq!(read("isomar/tiles/7/1.pbf"), "old");
        assert_eq!(read("isomar/tiles/a.pbf"), "new");
        assert!(d.path().join("isomar/tiles/8").is_dir());
        // Only the copy the new folder already had is left behind.
        assert_eq!(read("chart/tiles/a.pbf"), "old");
        assert!(!d.path().join("chart/coast.sqlite").exists() && !d.path().join("chart/tiles/7").exists());
    }
}
