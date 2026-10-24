//! Work on a background thread, polled from the UI. Used for key
//! derivation, which takes about a second on purpose.

use std::sync::mpsc::{self, Receiver, TryRecvError};

pub struct Task<T>(Receiver<T>);

pub enum Poll<T> {
    Pending,
    Done(T),
    /// The thread panicked.
    Failed,
}

impl<T: Send + 'static> Task<T> {
    pub fn spawn(ctx: &egui::Context, work: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
        Self(rx)
    }

    pub fn poll(&self) -> Poll<T> {
        match self.0.try_recv() {
            Ok(v) => Poll::Done(v),
            Err(TryRecvError::Empty) => Poll::Pending,
            Err(TryRecvError::Disconnected) => Poll::Failed,
        }
    }
}
