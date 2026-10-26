//! What differs between the desktop and Android: file pickers, where files
//! live, the on-screen keyboard and going to the background.
//!
//! Pickers return a [`Task`] because on Android the answer comes from
//! another activity, after some frames. On the desktop it is ready at once.

use std::fs::File;
use std::io;
use std::path::PathBuf;

use crate::task::Task;

#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_os = "android")]
use android as imp;

#[cfg(not(target_os = "android"))]
mod desktop;
#[cfg(not(target_os = "android"))]
use desktop as imp;

/// What a picked file is for; decides the filter and the MIME type.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Vault,
    KeePass,
    KeyFile,
    Csv,
}

impl FileKind {
    /// Name and extensions for a desktop dialog filter.
    #[cfg_attr(target_os = "android", allow(dead_code))]
    fn filter(self) -> Option<(&'static str, &'static [&'static str])> {
        match self {
            Self::Vault => Some(("KeeChest", &[keechest_core::format::EXTENSION])),
            Self::KeePass => Some(("KeePass", &["kdbx", "kdb"])),
            Self::KeyFile => None,
            Self::Csv => Some(("CSV", &["csv"])),
        }
    }

    /// MIME type to open or create. Few file managers know `.kchest` or
    /// `.kdbx`, so those are opened as anything.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    fn mime(self, create: bool) -> &'static str {
        match self {
            Self::Csv if create => "text/csv",
            Self::Vault if create => "application/octet-stream",
            _ => "*/*",
        }
    }
}

/// A file the user picked, open for reading or (when created) writing.
pub struct PickedFile {
    /// What to show the user.
    pub name: String,
    pub file: File,
    /// The path, where there is one. On Android files come from the
    /// system picker as descriptors without a path.
    pub path: Option<PathBuf>,
}

/// `Ok(None)` when the user cancelled.
pub type Picked = io::Result<Option<PickedFile>>;

/// Lets the user pick an existing file to read.
pub fn open_file(ctx: &egui::Context, kind: FileKind) -> Task<Picked> {
    imp::open_file(ctx, kind)
}

/// Lets the user pick where to write a new file, suggesting `name`. The
/// file is created (or truncated) and open for writing.
pub fn create_file(ctx: &egui::Context, kind: FileKind, name: &str) -> Task<Picked> {
    imp::create_file(ctx, kind, name)
}

/// Asks where to save a new vault, starting from `current`. Only where
/// [`CHOOSE_VAULT_LOCATION`].
pub fn choose_vault_location(current: &std::path::Path) -> Option<PathBuf> {
    imp::choose_vault_location(current)
}

/// Where `config.json` goes.
pub fn config_dir() -> Option<PathBuf> {
    imp::config_dir()
}

/// Where new vaults go by default.
pub fn vault_dir() -> PathBuf {
    imp::vault_dir()
}

/// Whether the user chooses where a new vault is saved. On Android vaults
/// live in the app's private storage; copies go out with a backup.
pub const CHOOSE_VAULT_LOCATION: bool = cfg!(not(target_os = "android"));

/// A phone or tablet: no hover, fingers instead of a mouse.
pub const TOUCH: bool = cfg!(target_os = "android");

/// Call at the start of every frame, before any widget.
pub fn begin_frame(ctx: &egui::Context) {
    imp::begin_frame(ctx);
}

/// Call at the end of every frame.
pub fn end_frame(ctx: &egui::Context) {
    imp::end_frame(ctx);
}

/// Whether the app went to the background since the last call.
pub fn take_backgrounded() -> bool {
    imp::take_backgrounded()
}

/// Called for the system Back button when there is nothing to go back to.
pub fn leave() {
    imp::leave();
}
