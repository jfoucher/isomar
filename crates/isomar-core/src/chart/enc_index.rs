//! A remembered summary of each S-57 cell in the folders the user added.
//!
//! Reading a cell takes a while and holds all its features, so a folder of
//! thousands is read once into a [`CellSummary`] (bounds, scale, coverage) and
//! the cell itself is read again only when it is on screen. The summaries are
//! kept in a file and reused while the cell and its updates are unchanged.

use super::s57::{CellSummary, S57Error, summarize, update_files};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

/// Bump when what a summary holds, or how a cell is read, changes.
const VERSION: u32 = 2;

/// Size and modification time of the base file and each update beside it.
type Stamp = Vec<(u64, u64)>;

#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
    stamp: Stamp,
    summary: CellSummary,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct File {
    version: u32,
    cells: HashMap<String, Record>,
}

pub struct EncIndex {
    path: PathBuf,
    file: Mutex<File>,
}

fn stamp(base: &Path) -> Stamp {
    std::iter::once(base.to_path_buf())
        .chain(update_files(base))
        .map(|p| {
            let m = std::fs::metadata(&p).ok();
            let mtime = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
            (m.map(|m| m.len()).unwrap_or(0), mtime)
        })
        .collect()
}

impl EncIndex {
    /// Opens the index kept at `path`; a missing or unreadable file is an empty one.
    pub fn open(path: PathBuf) -> EncIndex {
        let file = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<File>(&b).ok())
            .filter(|f| f.version == VERSION)
            .unwrap_or(File { version: VERSION, cells: HashMap::new() });
        EncIndex { path, file: Mutex::new(file) }
    }

    /// The summary of `base`, from the index if the files are unchanged,
    /// otherwise by reading the cell.
    pub fn summary(&self, base: &Path) -> Result<CellSummary, S57Error> {
        let key = base.to_string_lossy().into_owned();
        let now = stamp(base);
        if let Some(r) = self.file.lock().unwrap().cells.get(&key).filter(|r| r.stamp == now) {
            return Ok(r.summary.clone());
        }
        let summary = summarize(base)?;
        // JSON cannot hold the infinities of an empty bounding box.
        if !summary.bounds.is_empty() {
            self.file.lock().unwrap().cells.insert(key, Record { stamp: now, summary: summary.clone() });
        }
        Ok(summary)
    }

    /// Writes the index back, dropping cells whose files are gone.
    pub fn save(&self) {
        let mut f = self.file.lock().unwrap();
        f.cells.retain(|k, _| Path::new(k).exists());
        let Ok(bytes) = serde_json::to_vec(&*f) else { return };
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Option<PathBuf> {
        let p = PathBuf::from("testdata/noaa-us5ma20m/ENC_ROOT/US5MA20M/US5MA20M.000");
        p.exists().then_some(p)
    }

    #[test]
    fn a_summary_is_remembered_between_runs() {
        let Some(cell) = fixture() else { return };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.json");
        let first = EncIndex::open(path.clone());
        let s = first.summary(&cell).unwrap();
        assert!(s.features > 0 && !s.bounds.is_empty());
        first.save();
        let again = EncIndex::open(path);
        assert_eq!(again.file.lock().unwrap().cells.len(), 1, "read back from the file");
        assert_eq!(again.summary(&cell).unwrap(), s);
    }

    #[test]
    fn a_changed_file_is_read_again() {
        let Some(cell) = fixture() else { return };
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("US5MA20M.000");
        std::fs::copy(&cell, &copy).unwrap();
        let idx = EncIndex::open(dir.path().join("index.json"));
        idx.summary(&copy).unwrap();
        // A stale stamp must not be trusted.
        idx.file.lock().unwrap().cells.values_mut().for_each(|r| r.stamp = vec![(1, 1)]);
        let s = idx.summary(&copy).unwrap();
        let key = copy.to_string_lossy().into_owned();
        assert_eq!(idx.file.lock().unwrap().cells[&key].stamp, stamp(&copy));
        assert!(s.features > 0);
    }

    #[test]
    fn an_unreadable_file_is_an_empty_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(EncIndex::open(path).file.lock().unwrap().cells.is_empty());
    }
}
