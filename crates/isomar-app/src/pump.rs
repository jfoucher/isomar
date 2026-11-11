//! A main loop for shells that have none of their own to hand futures to: the
//! shell gives a `wake` callback that arranges, on any thread, a call to
//! `run` on the main thread after a delay, and calls `run` when it fires. The
//! futures and timeouts of `platform` then run inside `run`.

use crate::platform::{self, Hooks};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Wake, Waker};
use std::time::{Duration, Instant};

/// Asks the shell to call `run` on the main thread after the delay. Called
/// from any thread (a worker finishing wakes a task from there).
pub type WakeFn = Arc<dyn Fn(Duration) + Send + Sync>;

/// Ids of tasks that have been woken since `run` last looked.
static READY: Mutex<Vec<u64>> = Mutex::new(Vec::new());

type Task = Pin<Box<dyn Future<Output = ()>>>;

thread_local! {
    static TASKS: RefCell<HashMap<u64, Task>> = RefCell::new(HashMap::new());
    static TIMERS: RefCell<Vec<(Instant, Box<dyn FnOnce()>)>> = const { RefCell::new(Vec::new()) };
    static NEXT: Cell<u64> = const { Cell::new(1) };
    static WAKE: RefCell<Option<WakeFn>> = const { RefCell::new(None) };
}

struct TaskWaker {
    id: u64,
    wake: WakeFn,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        READY.lock().unwrap().push(self.id);
        (self.wake)(Duration::ZERO);
    }
}

fn wake_fn() -> WakeFn {
    WAKE.with(|w| w.borrow().clone()).expect("the pump is not installed")
}

/// Installs the platform hooks on the calling thread, running futures and
/// timeouts through `run`. `set_clipboard` is the shell's.
pub fn install(wake: WakeFn, set_clipboard: Box<dyn Fn(&str)>) {
    WAKE.with(|w| *w.borrow_mut() = Some(wake));
    platform::install(Hooks {
        spawn_local: Box::new(|f| {
            let id = NEXT.with(|n| n.replace(n.get() + 1));
            TASKS.with(|t| t.borrow_mut().insert(id, f));
            READY.lock().unwrap().push(id);
            wake_fn()(Duration::ZERO);
        }),
        timeout_once: Box::new(|delay, f| {
            TIMERS.with(|t| t.borrow_mut().push((Instant::now() + delay, f)));
            wake_fn()(delay);
        }),
        set_clipboard,
    });
}

/// Polls the tasks that were woken and calls the timeouts that are due, until
/// nothing more is ready; then asks for another call when the next timeout is.
pub fn run() {
    let wake = wake_fn();
    loop {
        let mut worked = false;
        let ids: Vec<u64> = std::mem::take(&mut *READY.lock().unwrap());
        for id in ids {
            // Out of the map while it runs, so that it may spawn tasks.
            let Some(mut task) = TASKS.with(|t| t.borrow_mut().remove(&id)) else { continue };
            worked = true;
            let waker = Waker::from(Arc::new(TaskWaker { id, wake: wake.clone() }));
            if task.as_mut().poll(&mut Context::from_waker(&waker)).is_pending() {
                TASKS.with(|t| t.borrow_mut().insert(id, task));
            }
        }
        let now = Instant::now();
        let due: Vec<Box<dyn FnOnce()>> = TIMERS.with(|t| {
            let mut t = t.borrow_mut();
            let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut *t).into_iter().partition(|(at, _)| *at <= now);
            *t = later;
            due.into_iter().map(|(_, f)| f).collect()
        });
        for f in due {
            worked = true;
            f();
        }
        if !worked && READY.lock().unwrap().is_empty() {
            break;
        }
    }
    let next = TIMERS.with(|t| t.borrow().iter().map(|(at, _)| *at).min());
    if let Some(at) = next {
        wake(at.saturating_duration_since(Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn wake_counter() -> (WakeFn, Arc<Mutex<Vec<Duration>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let c = calls.clone();
        (Arc::new(move |d| c.lock().unwrap().push(d)), calls)
    }

    /// The pump's state is per thread and the ready list is shared, so these
    /// run on one thread, one after the other.
    #[test]
    fn futures_and_timeouts_run_inside_run() {
        let (wake, calls) = wake_counter();
        install(wake, Box::new(|_| {}));
        let log = Rc::new(RefCell::new(Vec::new()));

        let l = log.clone();
        platform::spawn(async move {
            // A worker's result wakes the task from another thread.
            let v = platform::bg(|| 21 * 2).await;
            l.borrow_mut().push(format!("bg {v}"));
        });
        let l = log.clone();
        platform::timeout_once(Duration::from_millis(30), move || l.borrow_mut().push("timeout".to_string()));
        assert!(log.borrow().is_empty(), "nothing runs until the shell calls run");
        assert!(!calls.lock().unwrap().is_empty(), "the shell was asked to call run");

        let started = Instant::now();
        while log.borrow().len() < 2 && started.elapsed() < Duration::from_secs(5) {
            run();
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut seen = log.borrow().clone();
        seen.sort();
        assert_eq!(seen, ["bg 42", "timeout"]);
        // A timeout that is not due yet is asked for again.
        let l = log.clone();
        platform::timeout_once(Duration::from_secs(60), move || l.borrow_mut().push("late".to_string()));
        calls.lock().unwrap().clear();
        run();
        let asked = calls.lock().unwrap().last().copied().unwrap();
        assert!(asked > Duration::from_secs(50), "{asked:?}");
        assert_eq!(log.borrow().len(), 2);
    }
}
