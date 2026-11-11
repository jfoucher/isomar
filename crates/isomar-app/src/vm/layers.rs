//! The Layers popover: what can be shown, grouped by what it is (chart data, base map, overlays,
//! downloads), and for what is downloaded, how much is kept and how fresh it is.

use crate::host::toggleable_layers;
use crate::chips::{Chip, Icon};
use crate::i18n::{gettext, gettext_f};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Group {
    pub title: String,
    /// The layer ids and their labels.
    pub layers: Vec<(String, String)>,
}

const GROUPS: [(&str, &[&str]); 4] = [
    ("Chart data", &["enc", "bsb", "depth", "soundings", "aids", "graticule"]),
    ("Base map", &["land", crate::places::LAYER]),
    ("Overlays", &["nav", "weather", crate::currents::layer::LAYER]),
    ("Downloads", &[crate::tiles::LAND_LAYER, crate::tiles::MARKS_LAYER, crate::dem::LAYER, crate::emodnet::LAYER]),
];

pub fn groups() -> Vec<Group> {
    let all = toggleable_layers();
    GROUPS
        .iter()
        .map(|(title, ids)| Group {
            title: gettext(*title),
            layers: ids.iter().filter_map(|id| all.iter().find(|(i, _)| i == id).map(|(i, l)| (i.to_string(), l.clone()))).collect(),
        })
        .collect()
}

/// What is kept on disk for downloaded data: its size and the newest file's age, in words.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Download {
    pub id: String,
    pub detail: Vec<Chip>,
}

fn walk(dir: &Path, size: &mut u64, newest: &mut Option<SystemTime>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        if m.is_dir() {
            walk(&e.path(), size, newest);
        } else {
            *size += m.len();
            if let Ok(t) = m.modified() {
                *newest = Some(newest.map_or(t, |n| n.max(t)));
            }
        }
    }
}

fn mib(bytes: u64) -> String {
    let m = bytes as f64 / (1024.0 * 1024.0);
    if m < 10.0 { format!("{m:.1} MB") } else { format!("{m:.0} MB") }
}

fn age(t: SystemTime) -> String {
    let secs = SystemTime::now().duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        s if s < 3600 => gettext("updated just now"),
        s if s < 86_400 => gettext_f("updated {n} h ago", &[("n", &(s / 3600).to_string())]),
        s => gettext_f("updated {n} days ago", &[("n", &(s / 86_400).to_string())]),
    }
}

fn describe(dirs: &[PathBuf]) -> Vec<Chip> {
    let (mut size, mut newest) = (0, None);
    for d in dirs {
        walk(d, &mut size, &mut newest);
    }
    match newest {
        None => vec![Chip::plain(gettext("Nothing saved yet"))],
        Some(t) => {
            let kept = mib(size);
            vec![Chip::new(Icon::Size, kept.clone(), gettext_f("{size} kept", &[("size", &kept)])), Chip::with(Icon::Time, age(t))]
        }
    }
}

/// Reads the download folders; blocking, so run it off the main thread.
pub fn downloads() -> Vec<Download> {
    let tiles = crate::tiles::cache_dir();
    vec![
        Download { id: crate::tiles::LAND_LAYER.into(), detail: describe(std::slice::from_ref(&tiles)) },
        Download { id: crate::tiles::MARKS_LAYER.into(), detail: vec![Chip::plain(gettext("Shares the coastline download"))] },
        Download { id: crate::dem::LAYER.into(), detail: describe(&[crate::dem::cache_dir()]) },
        Download { id: crate::emodnet::LAYER.into(), detail: describe(&[crate::emodnet::dir()]) },
        Download { id: crate::places::LAYER.into(), detail: describe(&[crate::places::cache_dir()]) },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_layer_is_in_one_group() {
        let g = groups();
        let mut ids: Vec<String> = g.iter().flat_map(|g| g.layers.iter().map(|(i, _)| i.clone())).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "no layer twice");
        assert_eq!(n, toggleable_layers().len(), "every layer listed");
    }

    #[test]
    fn a_folder_reads_as_size_and_age() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(describe(&[dir.path().to_path_buf()]), vec![Chip::plain(gettext("Nothing saved yet"))]);
        std::fs::create_dir(dir.path().join("7")).unwrap();
        std::fs::write(dir.path().join("7").join("1.pbf"), vec![0u8; 2 * 1024 * 1024]).unwrap();
        let d = describe(&[dir.path().to_path_buf()]);
        assert_eq!((d[0].icon, d[0].text.as_str()), (Some(Icon::Size), "2.0 MB"));
        assert!(d[1].text.contains("just now"), "{d:?}");
    }
}
