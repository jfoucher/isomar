//! Long tasks the user may want to watch: a download, a route search. Each registers a job; the
//! window shows them in its activity indicator, with a progress bar and a Cancel button.
//!
//! Each job names the action that started it (`Jobs::start`), so that the button for that action can
//! show it: dimmed, with a spinner, and pressing it shows the activity indicator's list rather than
//! starting the work again. The actions: `route` (a weather route, best departure, or previewing a
//! departure), `weather` (Get weather), `autoroute:<route id>`, `ensemble:<route id>`,
//! `chart:<source id>` (installing charts), `perf_analyse`, `perf_backfill`; empty for work no button
//! starts.
//!
//! A Cancel must stop the task at once. Pressing it takes the job off the list there and then,
//! runs its hook (which tells a worker thread to stop), and wakes the flow that awaits the job's
//! work through `JobHandle::unless_cancelled`, which returns at once without waiting for the
//! worker to notice. Flows drop their futures then, so every future that hands work to a thread
//! cancels that work when dropped (`on_drop`).

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::{Rc, Weak};
use std::task::Poll;

pub struct Job {
    pub id: u64,
    /// What started it (see the module's doc), empty for work no button starts.
    pub action: String,
    pub title: String,
    /// 0..1, or `None` while the amount of work is not known.
    pub fraction: Option<f64>,
    pub detail: String,
    state: Rc<State>,
}

/// What a job and its handle share: whether it was cancelled, and how to tell the work.
struct State {
    cancelled: Cell<bool>,
    hook: Option<Rc<dyn Fn()>>,
    /// Closed when the job is cancelled, which wakes whatever waits on `stopped`.
    stop: async_channel::Sender<()>,
    stopped: async_channel::Receiver<()>,
}

#[derive(Default)]
pub struct Jobs {
    list: RefCell<Vec<Job>>,
    next: Cell<u64>,
    /// Changes whenever a job starts, moves or ends, so that a shell redraws only when it must.
    revision: Cell<u64>,
}

impl Jobs {
    /// Starts a job for `action` (see the module's doc); one with a `cancel` hook has a Cancel button.
    /// The hook should tell the work to stop; the flow is woken through its handle besides.
    pub fn start(self: &Rc<Self>, action: &str, title: &str, cancel: Option<Rc<dyn Fn()>>) -> JobHandle {
        let id = self.next.get() + 1;
        self.next.set(id);
        let (stop, stopped) = async_channel::bounded(1);
        let state = Rc::new(State { cancelled: Cell::new(false), hook: cancel, stop, stopped });
        self.list.borrow_mut().push(Job { id, action: action.to_string(), title: title.to_string(), fraction: None, detail: String::new(), state: state.clone() });
        self.bump();
        JobHandle { jobs: self.clone(), id, state }
    }

    /// Starts a job with a Cancel button whose flow stops through `JobHandle::unless_cancelled`
    /// (and the drop guards of what it awaits) rather than a hook.
    pub fn start_cancellable(self: &Rc<Self>, action: &str, title: &str) -> JobHandle {
        self.start(action, title, Some(Rc::new(|| {})))
    }

    /// Whether a job started by `action` is running.
    pub fn busy(&self, action: &str) -> bool {
        !action.is_empty() && self.list.borrow().iter().any(|j| j.action == action)
    }

    fn bump(&self) {
        self.revision.set(self.revision.get() + 1);
    }

    pub fn revision(&self) -> u64 {
        self.revision.get()
    }

    pub fn count(&self) -> usize {
        self.list.borrow().len()
    }

    /// Each running job, as the shells show it.
    pub fn snapshot(&self) -> Vec<crate::vm::window::JobRow> {
        self.list
            .borrow()
            .iter()
            .map(|j| crate::vm::window::JobRow { id: j.id, action: j.action.clone(), title: j.title.clone(), fraction: j.fraction, detail: j.detail.clone(), cancellable: j.state.hook.is_some() })
            .collect()
    }

    /// Cancels job `id`, if it can be: it leaves the list at once, its hook runs and its flow is
    /// woken.
    pub fn cancel(&self, id: u64) {
        let state = {
            let mut list = self.list.borrow_mut();
            let Some(i) = list.iter().position(|j| j.id == id && j.state.hook.is_some()) else { return };
            list.remove(i).state
        };
        self.bump();
        state.cancelled.set(true);
        state.stop.close();
        if let Some(hook) = &state.hook {
            hook();
        }
    }

    fn update(&self, id: u64, f: impl FnOnce(&mut Job)) {
        if let Some(j) = self.list.borrow_mut().iter_mut().find(|j| j.id == id) {
            f(j);
            self.bump();
        }
    }
}

/// A running job; it ends when this is dropped.
pub struct JobHandle {
    jobs: Rc<Jobs>,
    id: u64,
    state: Rc<State>,
}

impl JobHandle {
    pub fn progress(&self, fraction: f64, detail: &str) {
        let detail = detail.to_string();
        self.jobs.update(self.id, |j| {
            j.fraction = Some(fraction.clamp(0.0, 1.0));
            j.detail = detail;
        });
    }

    pub fn detail(&self, detail: &str) {
        let detail = detail.to_string();
        self.jobs.update(self.id, |j| j.detail = detail);
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.get()
    }

    /// Resolves when the job is cancelled (never, if it is not).
    pub async fn cancelled(&self) {
        // Nothing is ever sent: this returns when `cancel` closes the channel.
        let _ = self.state.stopped.recv().await;
    }

    /// Awaits `work` unless the job is cancelled first: then `None` at once, and `work` is
    /// dropped (so whatever it runs on a thread must stop when dropped, see `on_drop`).
    pub async fn unless_cancelled<F: Future>(&self, work: F) -> Option<F::Output> {
        if self.is_cancelled() {
            return None;
        }
        let mut work = Box::pin(work);
        let mut stop = Box::pin(self.cancelled());
        std::future::poll_fn(move |cx| {
            if stop.as_mut().poll(cx).is_ready() {
                return Poll::Ready(None);
            }
            work.as_mut().poll(cx).map(Some)
        })
        .await
    }

    /// Calls `f` every `period` on the main loop while the job runs: until it is cancelled or
    /// every `Rc` of its handle is dropped.
    pub fn every(self: &Rc<Self>, period: std::time::Duration, f: impl Fn(&JobHandle) + 'static) {
        let weak: Weak<JobHandle> = Rc::downgrade(self);
        crate::platform::spawn(async move {
            loop {
                match weak.upgrade() {
                    Some(job) if !job.is_cancelled() => f(&job),
                    _ => return,
                }
                let (tx, rx) = async_channel::bounded::<()>(1);
                crate::platform::timeout_once(period, move || {
                    let _ = tx.try_send(());
                });
                let _ = rx.recv().await;
            }
        });
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        let before = self.jobs.list.borrow().len();
        self.jobs.list.borrow_mut().retain(|j| j.id != self.id);
        if self.jobs.list.borrow().len() != before {
            self.jobs.bump();
        }
    }
}

/// Runs `f` when dropped: a future that hands work to a thread holds one that tells the thread
/// to stop, so the work stops when the future is dropped half-way (a cancelled job).
pub struct OnDrop<F: FnOnce()>(Option<F>);

pub fn on_drop<F: FnOnce()>(f: F) -> OnDrop<F> {
    OnDrop(Some(f))
}

impl<F: FnOnce()> Drop for OnDrop<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{Context, Waker};

    /// Polls `f` once.
    fn poll_once<F: Future>(f: std::pin::Pin<&mut F>) -> Poll<F::Output> {
        f.poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn a_job_is_listed_while_its_handle_lives() {
        let jobs = Rc::new(Jobs::default());
        let r0 = jobs.revision();
        let h = jobs.start("weather", "Downloading weather", None);
        assert_eq!(jobs.count(), 1);
        h.progress(0.4, "12 of 30 files");
        let s = jobs.snapshot();
        assert_eq!((s[0].action.as_str(), s[0].title.as_str(), s[0].fraction, s[0].detail.as_str(), s[0].cancellable), ("weather", "Downloading weather", Some(0.4), "12 of 30 files", false));
        assert!(jobs.revision() > r0);
        drop(h);
        assert_eq!(jobs.count(), 0);
    }

    #[test]
    fn a_job_says_its_action_is_busy_while_it_runs() {
        let jobs = Rc::new(Jobs::default());
        let h = jobs.start_cancellable("chart:fr-brest", "Installing");
        let _status = jobs.start("", "Loading map tiles", None);
        assert!(jobs.busy("chart:fr-brest") && !jobs.busy("chart:fr-iroise"));
        assert!(!jobs.busy(""), "work no button started makes no button busy");
        jobs.cancel(h.id);
        assert!(!jobs.busy("chart:fr-brest"), "a cancelled job frees its button at once");
    }

    #[test]
    fn cancelling_runs_the_hook_and_ends_the_job_at_once() {
        let jobs = Rc::new(Jobs::default());
        let hit = Rc::new(Cell::new(false));
        let h2 = hit.clone();
        let h = jobs.start("route", "Routing", Some(Rc::new(move || h2.set(true))));
        let r0 = jobs.revision();
        jobs.cancel(h.id);
        assert!(hit.get() && h.is_cancelled());
        assert_eq!(jobs.count(), 0, "gone from the list before its flow has ended");
        assert!(jobs.revision() > r0);
        jobs.cancel(999);
        drop(h);
    }

    #[test]
    fn a_job_without_cancel_cannot_be_cancelled() {
        let jobs = Rc::new(Jobs::default());
        let h = jobs.start("perf_analyse", "Analysing", None);
        jobs.cancel(h.id);
        assert!(!h.is_cancelled());
        assert_eq!(jobs.count(), 1);
    }

    #[test]
    fn cancelling_wakes_the_flow_without_waiting_for_the_work() {
        let jobs = Rc::new(Jobs::default());
        let h = jobs.start_cancellable("route", "Finding the best departure");
        let stopped = Rc::new(Cell::new(false));
        let s2 = stopped.clone();
        // Work that never ends by itself, and stops its thread when dropped.
        let work = async move {
            let _stop = on_drop(move || s2.set(true));
            std::future::pending::<()>().await
        };
        let mut run = Box::pin(h.unless_cancelled(work));
        assert!(poll_once(run.as_mut()).is_pending());
        jobs.cancel(h.id);
        assert_eq!(poll_once(run.as_mut()), Poll::Ready(None));
        assert!(stopped.get(), "the work was dropped, which stops its thread");
        // Work finished before any cancel comes back as it is.
        let h = jobs.start_cancellable("route", "Computing the route");
        let mut run = Box::pin(h.unless_cancelled(async { 7 }));
        assert_eq!(poll_once(run.as_mut()), Poll::Ready(Some(7)));
    }
}
