//! A plain log: lines go to stderr and to `isomar.log` in the cache directory
//! (`~/.cache/isomar/isomar.log`), which starts afresh once it passes 1 MB.
//! Not for tests, which print for themselves.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const MAX_BYTES: u64 = 1_000_000;

static FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();

pub fn path() -> PathBuf {
    super::datadir::cache_dir().join("isomar.log")
}

fn open() -> Option<std::fs::File> {
    let p = path();
    std::fs::create_dir_all(p.parent()?).ok()?;
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&p, p.with_extension("log.old"));
    }
    std::fs::OpenOptions::new().create(true).append(true).open(p).ok()
}

pub fn write(msg: &str) {
    let line = format!("{} [{}] {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), std::thread::current().name().unwrap_or("worker"));
    eprintln!("{line}");
    if let Ok(mut f) = FILE.get_or_init(|| Mutex::new(open())).lock()
        && let Some(f) = f.as_mut()
    {
        let _ = writeln!(f, "{line}");
    }
}

/// Makes a panic write its message and place to the log before the usual report: a panic on the
/// main thread ends the app (in a toolkit callback or a C entry point it aborts), and without this
/// nothing in `isomar.log` says why. Called by each shell as it starts.
pub fn install_panic_hook() {
    // Once, though a shell may start the engine again after shutting it down.
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            write(&panic_line(info.payload(), info.location().map(|l| format!("{}:{}", l.file(), l.line())).as_deref()));
            before(info);
        }));
    });
}

fn panic_line(payload: &dyn std::any::Any, at: Option<&str>) -> String {
    let what = payload.downcast_ref::<&str>().copied().or_else(|| payload.downcast_ref::<String>().map(String::as_str)).unwrap_or("no message");
    format!("panic at {}: {what}", at.unwrap_or("an unknown place"))
}

/// `log!("downloading {url}")`: formatted like `format!`.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_is_logged_with_its_message_and_place() {
        assert_eq!(panic_line(&"boom", Some("src/a.rs:3")), "panic at src/a.rs:3: boom");
        assert_eq!(panic_line(&String::from("already borrowed"), None), "panic at an unknown place: already borrowed");
        assert_eq!(panic_line(&42, Some("x:1")), "panic at x:1: no message");
    }
}
