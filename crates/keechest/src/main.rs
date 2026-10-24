//! KeeChest, a local password manager.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod clipboard;
mod config;
mod generator_page;
mod harden;
mod session;
mod settings_page;
mod start;
mod task;
mod widgets;

fn main() -> eframe::Result {
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
