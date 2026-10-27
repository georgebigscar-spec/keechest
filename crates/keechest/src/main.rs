//! KeeChest, a local password manager.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_os = "android"))]
fn main() -> eframe::Result {
    keechest::run()
}

/// On Android the app starts from the `keechest-android` library.
#[cfg(target_os = "android")]
fn main() {}
