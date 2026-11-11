//! What the application engine needs from the toolkit it runs in: a main loop
//! to put futures and timeouts on, and the clipboard. The shell installs the
//! hooks once at start-up, on the main thread; everything here is for that
//! thread, except `bg`, which runs a closure on a worker pool.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::cell::Cell;
use std::time::{Duration, Instant};

/// The main-loop services a shell provides.
pub struct Hooks {
    /// Runs a future on the main loop.
    pub spawn_local: Box<dyn Fn(Pin<Box<dyn Future<Output = ()>>>)>,
    /// Calls the closure once on the main loop after the delay.
    pub timeout_once: Box<dyn Fn(Duration, Box<dyn FnOnce()>)>,
    /// Puts text on the clipboard.
    pub set_clipboard: Box<dyn Fn(&str)>,
}

thread_local! {
    static HOOKS: RefCell<Option<Hooks>> = const { RefCell::new(None) };
}

/// Installs the shell's hooks for the calling (main) thread.
pub fn install(hooks: Hooks) {
    HOOKS.with(|h| *h.borrow_mut() = Some(hooks));
}

/// Whether a shell has installed its hooks (tests drive the engine without).
pub fn installed() -> bool {
    HOOKS.with(|h| h.borrow().is_some())
}

/// Spawns a future on the main loop.
pub fn spawn<F: Future<Output = ()> + 'static>(f: F) {
    HOOKS.with(|h| match h.borrow().as_ref() {
        Some(h) => (h.spawn_local)(Box::pin(f)),
        None => panic!("platform hooks are not installed"),
    });
}

/// Calls `f` once on the main loop after `delay`.
pub fn timeout_once(delay: Duration, f: impl FnOnce() + 'static) {
    HOOKS.with(|h| match h.borrow().as_ref() {
        Some(h) => (h.timeout_once)(delay, Box::new(f)),
        None => panic!("platform hooks are not installed"),
    });
}

/// Waits for the view to settle before loading what it shows, but not for ever: each ask restarts the wait,
/// and an ask `SETTLE_AT_MOST` after the first unanswered one runs anyway. A view that follows the boat
/// moves at every 4 Hz clock tick, as often as the wait, and a plain debounce never ran: under way in a new
/// area no coastline, seamarks or names loaded at all.
#[derive(Default)]
pub struct Settle {
    ticket: Cell<u64>,
    since: Cell<Option<Instant>>,
}

/// The longest a loader waits for a moving view.
pub const SETTLE_AT_MOST: Duration = Duration::from_secs(1);

impl Settle {
    /// Records an ask: the ticket to hand `due` when its wait is over.
    pub fn ask(&self) -> u64 {
        if self.since.get().is_none() {
            self.since.set(Some(Instant::now()));
        }
        self.ticket.set(self.ticket.get() + 1);
        self.ticket.get()
    }

    /// Whether to run now, at the end of the wait for `ticket`: no ask came since, or the asks have gone
    /// unanswered for `SETTLE_AT_MOST`.
    pub fn due(&self, ticket: u64) -> bool {
        self.due_at(ticket, Instant::now())
    }

    fn due_at(&self, ticket: u64, now: Instant) -> bool {
        let Some(since) = self.since.get() else { return false };
        let due = ticket == self.ticket.get() || now.duration_since(since) >= SETTLE_AT_MOST;
        if due {
            self.since.set(None);
        }
        due
    }
}

pub fn set_clipboard(text: &str) {
    HOOKS.with(|h| {
        if let Some(h) = h.borrow().as_ref() {
            (h.set_clipboard)(text);
        }
    });
}

/// A pool for blocking work (network, disk, long computations), apart from
/// rayon's global one: work run here may itself fan out with `par_iter`, and
/// must not wait for threads that are all busy waiting.
fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(8)
            .thread_name(|i| format!("isomar-bg-{i}"))
            .build()
            .expect("worker pool")
    })
}

/// Runs a blocking closure on a worker thread and awaits its result. A panic
/// in the closure is raised again in the caller.
pub async fn bg<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = async_channel::bounded(1);
    pool().spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = tx.send_blocking(r);
    });
    match rx.recv().await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => std::panic::resume_unwind(e),
        Err(_) => panic!("worker pool dropped a job"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Polls a future that needs no main loop to completion.
    fn block_on<T>(f: impl Future<Output = T>) -> T {
        use std::task::{Context, Poll, Wake, Waker};
        use std::sync::Arc;
        struct Thread(std::thread::Thread);
        impl Wake for Thread {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(Thread(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut f = Box::pin(f);
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::park();
        }
    }

    #[test]
    fn bg_returns_the_result_from_a_worker_thread() {
        let main = std::thread::current().id();
        let (same, n) = block_on(bg(move || (std::thread::current().id() == main, 6 * 7)));
        assert!(!same);
        assert_eq!(n, 42);
    }

    #[test]
    fn bg_raises_a_panic_in_the_caller() {
        let r = std::panic::catch_unwind(|| block_on(bg(|| -> i32 { panic!("boom") })));
        assert!(r.is_err());
    }
}

#[cfg(test)]
mod settle_tests {
    use super::*;

    #[test]
    fn a_settled_view_runs_once_and_a_moving_one_at_least_every_second() {
        let s = Settle::default();
        let t0 = Instant::now();
        // Two asks: only the last one's wait runs.
        let (a, b) = (s.ask(), s.ask());
        assert!(!s.due_at(a, t0));
        assert!(s.due_at(b, t0));
        assert!(!s.due_at(b, t0), "answered");
        // Asks every 250 ms for ever: the one a second after the first runs, though another came since.
        let first = s.ask();
        let since = s.since.get().unwrap();
        let next = s.ask();
        assert!(!s.due_at(first, since + Duration::from_millis(250)));
        s.ask();
        assert!(!s.due_at(next, since + Duration::from_millis(500)));
        assert!(s.due_at(next, since + SETTLE_AT_MOST));
        assert!(!s.due_at(next, since + SETTLE_AT_MOST), "answered");
    }
}
