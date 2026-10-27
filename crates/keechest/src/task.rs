//! Work on a background thread, polled from the UI. Used for key
//! derivation, which takes about a second on purpose, and for file
//! pickers, which answer later on Android.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

pub struct Task<T>(Receiver<T>);

pub enum Poll<T> {
    Pending,
    Done(T),
    /// The thread panicked, or the answer will never come.
    Failed,
}

impl<T: Send + 'static> Task<T> {
    pub fn spawn(ctx: &egui::Context, work: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, task) = Self::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
        task
    }

    /// A task that is already done.
    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn ready(value: T) -> Self {
        let (tx, task) = Self::channel();
        let _ = tx.send(value);
        task
    }

    /// A task finished by sending its result to the returned sender.
    pub fn channel() -> (Sender<T>, Self) {
        let (tx, rx) = mpsc::channel();
        (tx, Self(rx))
    }

    pub fn poll(&self) -> Poll<T> {
        match self.0.try_recv() {
            Ok(v) => Poll::Done(v),
            Err(TryRecvError::Empty) => Poll::Pending,
            Err(TryRecvError::Disconnected) => Poll::Failed,
        }
    }
}
