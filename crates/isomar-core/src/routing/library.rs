//! The boat's polars: six built in (`data/polars`, each the average of published polars for a kind of
//! boat) and the user's own, of which exactly one is active. The active one is what weather routing
//! sails with and what the performance log measures against and learns into.
//!
//! The user's polars are files in `<state dir>/polars`: `custom-N.csv` for their own, whose first line
//! is `# name: …`, and `<built-in id>.csv` for a built-in polar they changed (taking it back to the
//! original deletes the file). `<id>.csv.prev` is the polar before the learnt speeds were applied.

use super::polar::Polar;
use std::path::{Path, PathBuf};

/// A polar shipped with the app.
pub struct Builtin {
    pub id: &'static str,
    /// The English name; shells show `gettext` of it.
    pub name: &'static str,
    text: &'static str,
}

/// In the order they are offered: smallest and slowest first.
pub const BUILTINS: [Builtin; 6] = [
    Builtin { id: "small-cruiser", name: "Small cruiser (8 m)", text: include_str!("../../../../data/polars/small-cruiser.csv") },
    Builtin { id: "medium-cruiser", name: "Medium cruiser (11 m)", text: include_str!("../../../../data/polars/medium-cruiser.csv") },
    Builtin { id: "large-cruiser", name: "Large cruiser (15 m)", text: include_str!("../../../../data/polars/large-cruiser.csv") },
    Builtin { id: "cruiser-racer", name: "Cruiser-racer (12 m)", text: include_str!("../../../../data/polars/cruiser-racer.csv") },
    Builtin { id: "imoca-60", name: "IMOCA 60", text: include_str!("../../../../data/polars/imoca-60.csv") },
    Builtin { id: "ultim", name: "Ultim trimaran", text: include_str!("../../../../data/polars/ultim.csv") },
];

/// The polar that is active until the user picks another.
pub const DEFAULT_ID: &str = "medium-cruiser";

const NAME_PREFIX: &str = "# name:";

/// One polar of the library, as a list shows it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub builtin: bool,
    /// A built-in polar the user changed.
    pub modified: bool,
}

fn builtin(id: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.id == id)
}

/// A built-in polar as shipped.
pub fn original(id: &str) -> Option<Polar> {
    builtin(id).map(|b| Polar::parse(b.text).expect("the built-in polars are valid"))
}

/// The name a built-in polar is shown with, translated.
pub fn builtin_name(id: &str) -> Option<String> {
    builtin(id).map(|b| crate::i18n::gettext(b.name))
}

/// Ids are file names: letters, digits and dashes only.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub struct Library {
    dir: PathBuf,
}

impl Library {
    pub fn new(dir: impl Into<PathBuf>) -> Library {
        Library { dir: dir.into() }
    }

    /// The user's library, in the state folder.
    pub fn user() -> Library {
        Library::new(crate::datadir::polars_dir())
    }

    fn file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.csv"))
    }

    fn prev_file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.csv.prev"))
    }

    /// The user's own polars: (id, name), by name.
    fn customs(&self) -> Vec<(String, String)> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut out: Vec<(String, String)> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let id = name.strip_suffix(".csv")?;
                (valid_id(id) && builtin(id).is_none()).then(|| id.to_string())
            })
            .filter_map(|id| Some((id.clone(), self.custom_name(&id)?)))
            .collect();
        out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then(a.0.cmp(&b.0)));
        out
    }

    fn custom_name(&self, id: &str) -> Option<String> {
        let text = std::fs::read_to_string(self.file(id)).ok()?;
        let first = text.lines().next().unwrap_or("");
        Some(first.strip_prefix(NAME_PREFIX).map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| id.to_string()))
    }

    /// Every polar: the built-in ones in their order, then the user's by name.
    pub fn list(&self) -> Vec<Entry> {
        let mut out: Vec<Entry> = BUILTINS
            .iter()
            .map(|b| Entry { id: b.id.to_string(), name: crate::i18n::gettext(b.name), builtin: true, modified: self.file(b.id).is_file() })
            .collect();
        out.extend(self.customs().into_iter().map(|(id, name)| Entry { id, name, builtin: false, modified: false }));
        out
    }

    pub fn exists(&self, id: &str) -> bool {
        builtin(id).is_some() || (valid_id(id) && self.file(id).is_file())
    }

    pub fn name(&self, id: &str) -> Option<String> {
        builtin_name(id).or_else(|| if valid_id(id) { self.custom_name(id) } else { None })
    }

    /// `id` when there is such a polar, else the default.
    pub fn resolve(&self, id: &str) -> String {
        if self.exists(id) { id.to_string() } else { DEFAULT_ID.to_string() }
    }

    /// The polar as it is now: the user's version of a built-in one if they changed it.
    pub fn load(&self, id: &str) -> Option<Polar> {
        if valid_id(id)
            && let Ok(text) = std::fs::read_to_string(self.file(id))
            && let Ok(p) = Polar::parse(&text)
        {
            return Some(p);
        }
        original(id)
    }

    fn write(&self, id: &str, name: Option<&str>, p: &Polar) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let mut text = String::new();
        if let Some(n) = name {
            text.push_str(NAME_PREFIX);
            text.push(' ');
            text.push_str(&n.replace(['\n', '\r'], " "));
            text.push('\n');
        }
        text.push_str(&p.to_text());
        let path = self.file(id);
        let tmp = path.with_extension("csv.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }

    /// Saves over an existing polar; a built-in one becomes the user's version of it.
    pub fn save(&self, id: &str, p: &Polar) -> std::io::Result<()> {
        if builtin(id).is_some() {
            return self.write(id, None, p);
        }
        if !self.exists(id) {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("no polar {id}")));
        }
        let name = self.custom_name(id);
        self.write(id, name.as_deref(), p)
    }

    /// Saves a new polar of the user's under `name`; returns its id.
    pub fn save_as(&self, name: &str, p: &Polar) -> std::io::Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the polar needs a name"));
        }
        let id = (1..).map(|n| format!("custom-{n}")).find(|id| !self.file(id).exists()).expect("a free id");
        self.write(&id, Some(name), p)?;
        Ok(id)
    }

    /// A user's polar is deleted; a built-in one goes back to the original.
    pub fn remove(&self, id: &str) -> std::io::Result<()> {
        if !valid_id(id) {
            return Ok(());
        }
        let _ = std::fs::remove_file(self.prev_file(id));
        match std::fs::remove_file(self.file(id)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }

    /// Keeps the polar as it is now, so that `restore_previous` can bring it back.
    pub fn keep_previous(&self, id: &str) -> std::io::Result<()> {
        let Some(p) = self.load(id) else { return Ok(()) };
        std::fs::create_dir_all(&self.dir)?;
        let name = if builtin(id).is_some() { None } else { self.custom_name(id) };
        let mut text = name.map(|n| format!("{NAME_PREFIX} {n}\n")).unwrap_or_default();
        text.push_str(&p.to_text());
        std::fs::write(self.prev_file(id), text)
    }

    pub fn has_previous(&self, id: &str) -> bool {
        valid_id(id) && self.prev_file(id).is_file()
    }

    /// Puts back the polar `keep_previous` kept.
    pub fn restore_previous(&self, id: &str) -> std::io::Result<()> {
        if !self.has_previous(id) {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no earlier polar"));
        }
        std::fs::rename(self.prev_file(id), self.file(id))
    }

    /// The single polar of the versions before the library (`polar.csv` and its `.prev`) becomes one
    /// of the user's, named `name`. Returns its id, or `None` when there was none to take.
    pub fn adopt_legacy(&self, legacy: &Path, name: &str) -> Option<String> {
        let p = Polar::parse(&std::fs::read_to_string(legacy).ok()?).ok()?;
        let id = self.save_as(name, &p).ok()?;
        let prev = legacy.with_extension("csv.prev");
        if let Some(q) = std::fs::read_to_string(&prev).ok().and_then(|t| Polar::parse(&t).ok()) {
            let _ = std::fs::write(self.prev_file(&id), format!("{NAME_PREFIX} {name}\n{}", q.to_text()));
            let _ = std::fs::remove_file(&prev);
        }
        let _ = std::fs::remove_file(legacy);
        Some(id)
    }
}

/// The active polar as the saved settings say, outside the app (the routing bench): the polar of the
/// versions before the library while the app has not taken it in yet.
pub fn current() -> Polar {
    let id = crate::config::Settings::load().polar;
    if id.is_empty()
        && let Some(p) = std::fs::read_to_string(crate::datadir::legacy_polar_path()).ok().and_then(|t| Polar::parse(&t).ok())
    {
        return p;
    }
    let lib = Library::user();
    lib.load(&lib.resolve(&id)).unwrap_or_else(|| original(DEFAULT_ID).expect("the default polar is built in"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path().join("polars"));
        (dir, lib)
    }

    #[test]
    fn the_built_in_polars_are_sailboats_in_order_of_speed() {
        let mut last = 0.0;
        for b in &BUILTINS {
            let p = original(b.id).unwrap();
            assert_eq!(p.speed(0.0, 12.0), 0.0, "{}", b.id);
            assert!(p.speed(90.0, 12.0) > p.speed(35.0, 12.0), "{}", b.id);
            assert!(p.speed(110.0, 20.0) > p.speed(110.0, 8.0), "{}", b.id);
            let (up, vmg) = p.best_vmg(12.0, true);
            assert!((35.0..60.0).contains(&up) && vmg > 2.0, "{}: {up} {vmg}", b.id);
            let (down, _) = p.best_vmg(12.0, false);
            assert!(down > 120.0, "{}: {down}", b.id);
            // The cruisers grow with their length, and the racers are faster still.
            if b.id != "cruiser-racer" {
                let v = p.speed(90.0, 12.0);
                assert!(v > last, "{}: {v} after {last}", b.id);
                last = v;
            }
        }
        let fastest = |id| original(id).unwrap().max_speed();
        assert!(fastest("small-cruiser") < 10.0);
        assert!((18.0..30.0).contains(&fastest("imoca-60")), "{}", fastest("imoca-60"));
        assert!((30.0..45.0).contains(&fastest("ultim")), "{}", fastest("ultim"));
    }

    #[test]
    fn a_built_in_polar_can_be_changed_and_taken_back() {
        let (_d, lib) = lib();
        assert!(lib.list().iter().all(|e| e.builtin && !e.modified));
        assert_eq!(lib.load("ultim"), original("ultim"));
        let mut p = original("ultim").unwrap();
        p.set_speed(5, 3, 1.0);
        lib.save("ultim", &p).unwrap();
        assert_eq!(lib.load("ultim").unwrap(), p);
        assert!(lib.list().iter().find(|e| e.id == "ultim").unwrap().modified);
        lib.remove("ultim").unwrap();
        assert_eq!(lib.load("ultim"), original("ultim"));
    }

    #[test]
    fn the_users_polars_have_names_and_ids_of_their_own() {
        let (_d, lib) = lib();
        let p = original("small-cruiser").unwrap();
        let a = lib.save_as("Zephyr", &p).unwrap();
        let b = lib.save_as("  Albatros\n2 ", &p).unwrap();
        assert_ne!(a, b);
        assert!(lib.save_as(" ", &p).is_err());
        let names: Vec<String> = lib.list().into_iter().filter(|e| !e.builtin).map(|e| e.name).collect();
        assert_eq!(names, ["Albatros 2", "Zephyr"]);
        assert_eq!(lib.name(&a).as_deref(), Some("Zephyr"));
        // Saving keeps the name.
        let mut q = p.clone();
        q.set_speed(4, 2, 9.0);
        lib.save(&a, &q).unwrap();
        assert_eq!(lib.load(&a).unwrap(), q);
        assert_eq!(lib.name(&a).as_deref(), Some("Zephyr"));
        assert!(lib.save("custom-99", &q).is_err(), "only an existing polar is saved over");
        lib.remove(&a).unwrap();
        assert!(!lib.exists(&a));
        assert_eq!(lib.resolve(&a), DEFAULT_ID);
        assert_eq!(lib.resolve(&b), b);
        assert!(!lib.exists("../polar"), "an id is never a path");
    }

    #[test]
    fn the_polar_before_the_learnt_speeds_comes_back() {
        let (_d, lib) = lib();
        let id = lib.save_as("Mine", &original("small-cruiser").unwrap()).unwrap();
        for id in [id.as_str(), "imoca-60"] {
            let before = lib.load(id).unwrap();
            assert!(!lib.has_previous(id));
            lib.keep_previous(id).unwrap();
            let mut learnt = before.clone();
            learnt.set_speed(6, 1, 3.3);
            lib.save(id, &learnt).unwrap();
            lib.restore_previous(id).unwrap();
            assert_eq!(lib.load(id).unwrap(), before);
            assert!(!lib.has_previous(id));
            assert!(lib.restore_previous(id).is_err());
        }
        assert_eq!(lib.name(&id).as_deref(), Some("Mine"));
    }

    #[test]
    fn the_old_single_polar_is_taken_in() {
        let (d, lib) = lib();
        let legacy = d.path().join("polar.csv");
        assert_eq!(lib.adopt_legacy(&legacy, "My boat"), None);
        let p = Polar::generic_cruiser();
        std::fs::write(&legacy, p.to_text()).unwrap();
        let mut q = p.clone();
        q.set_speed(3, 3, 1.0);
        std::fs::write(legacy.with_extension("csv.prev"), q.to_text()).unwrap();
        let id = lib.adopt_legacy(&legacy, "My boat").unwrap();
        assert_eq!(lib.load(&id).unwrap(), p);
        assert_eq!(lib.name(&id).as_deref(), Some("My boat"));
        assert!(!legacy.exists() && !legacy.with_extension("csv.prev").exists());
        lib.restore_previous(&id).unwrap();
        assert_eq!(lib.load(&id).unwrap(), q);
        assert_eq!(lib.name(&id).as_deref(), Some("My boat"));
    }
}
