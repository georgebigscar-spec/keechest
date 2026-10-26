//! KeeChest, a local password manager.

mod app;
mod clipboard;
mod config;
mod generator_page;
mod harden;
pub mod platform;
mod session;
mod settings_page;
mod start;
mod task;
mod widgets;

/// Runs the desktop app until its window is closed.
#[cfg(not(target_os = "android"))]
pub fn run() -> eframe::Result {
    harden::harden_process();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("KeeChest")
            .with_app_id("keechest")
            .with_inner_size([1000.0, 660.0])
            .with_min_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "KeeChest",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(&cc.egui_ctx)))),
    )
}

/// Runs the app in the activity that `app` belongs to. Called from
/// `android_main` of the `keechest-android` library.
#[cfg(target_os = "android")]
pub fn run(app: winit::platform::android::activity::AndroidApp) -> eframe::Result {
    harden::harden_process();
    platform::android::init(&app);
    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    eframe::run_native(
        "KeeChest",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(&cc.egui_ctx)))),
    )
}
