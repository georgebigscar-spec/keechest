//! Native file dialogs and the OS's standard directories.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use super::{FileKind, Picked, PickedFile};
use crate::task::Task;

fn dialog(kind: FileKind) -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new();
    match kind.filter() {
        Some((name, extensions)) => dialog.add_filter(name, extensions),
        None => dialog,
    }
}

fn picked(path: PathBuf, file: File) -> PickedFile {
    PickedFile {
        name: crate::start::file_name(&path),
        file,
        path: Some(path),
    }
}

pub fn open_file(_ctx: &egui::Context, kind: FileKind) -> Task<Picked> {
    let result = dialog(kind)
        .pick_file()
        .map(|path| File::open(&path).map(|f| picked(path, f)))
        .transpose();
    Task::ready(result)
}

pub fn create_file(_ctx: &egui::Context, kind: FileKind, name: &str) -> Task<Picked> {
    let result = dialog(kind)
        .set_file_name(name)
        .save_file()
        .map(|path| create_private(&path).map(|f| picked(path, f)))
        .transpose();
    Task::ready(result)
}

/// Creates or truncates `path`, readable only by the user on Unix.
fn create_private(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "KeeChest").map(|d| d.config_dir().to_owned())
}

pub fn vault_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|u| {
            u.document_dir()
                .map(PathBuf::from)
                .or(Some(u.home_dir().into()))
        })
        .unwrap_or_default()
}

pub fn begin_frame(_ctx: &egui::Context) {}

pub fn end_frame(_ctx: &egui::Context) {}

pub fn take_backgrounded() -> bool {
    false
}

pub fn leave() {}

/// Asks where to save a new vault, starting from `current`.
pub fn choose_vault_location(current: &Path) -> Option<PathBuf> {
    let mut dialog = dialog(FileKind::Vault).set_file_name(crate::start::file_name(current));
    if let Some(dir) = current.parent() {
        dialog = dialog.set_directory(dir);
    }
    let mut chosen = dialog.save_file()?;
    if chosen.extension().is_none() {
        chosen.set_extension(keechest_core::format::EXTENSION);
    }
    Some(chosen)
}
