//! Copying secrets with automatic clearing.

use std::time::{Duration, Instant};

use zeroize::Zeroizing;

pub struct SecureClipboard {
    inner: Option<arboard::Clipboard>,
    /// What we put there and when to clear it.
    pending: Option<(Zeroizing<String>, Instant)>,
}

impl SecureClipboard {
    pub fn new() -> Self {
        Self {
            inner: arboard::Clipboard::new().ok(),
            pending: None,
        }
    }

    /// Copies `text`, asking clipboard managers not to keep it, and clears it
    /// after `clear_after` unless something else was copied since. Returns
    /// whether clearing is scheduled.
    pub fn copy(&mut self, ctx: &egui::Context, text: &str, clear_after: Option<Duration>) -> bool {
        let Some(cb) = self.inner.as_mut() else {
            // No native clipboard; egui's copy works but can't be cleared.
            ctx.copy_text(text.to_owned());
            return false;
        };
        if set_private(cb, text).is_err() {
            ctx.copy_text(text.to_owned());
            return false;
        }
        self.pending = clear_after.map(|d| (Zeroizing::new(text.to_owned()), Instant::now() + d));
        self.pending.is_some()
    }

    /// Clears the clipboard once the timer runs out. Call every frame.
    pub fn tick(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, at)| Instant::now() >= *at)
        {
            self.clear_now();
        }
    }

    /// Clears our secret from the clipboard now, if it is still there.
    pub fn clear_now(&mut self) {
        let (Some((text, _)), Some(cb)) = (self.pending.take(), self.inner.as_mut()) else {
            return;
        };
        let current = cb.get_text().ok().map(Zeroizing::new);
        if current.is_some_and(|c| *c == *text) {
            let _ = cb.clear();
        }
    }
}

impl Drop for SecureClipboard {
    fn drop(&mut self) {
        self.clear_now();
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn set_private(cb: &mut arboard::Clipboard, text: &str) -> Result<(), arboard::Error> {
    use arboard::SetExtLinux;
    cb.set().exclude_from_history().text(text)
}

#[cfg(target_os = "windows")]
fn set_private(cb: &mut arboard::Clipboard, text: &str) -> Result<(), arboard::Error> {
    use arboard::SetExtWindows;
    cb.set()
        .exclude_from_history()
        .exclude_from_cloud()
        .text(text)
}

#[cfg(target_os = "macos")]
fn set_private(cb: &mut arboard::Clipboard, text: &str) -> Result<(), arboard::Error> {
    use arboard::SetExtApple;
    cb.set().exclude_from_history().text(text)
}
