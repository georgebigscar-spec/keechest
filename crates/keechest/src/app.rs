//! The top level: which screen is shown, auto-lock, theme and clipboard.

use std::time::{Duration, Instant};

use fluent_egui::{Brightness, FluentTheme, Localizations, fonts};

use crate::clipboard::SecureClipboard;
use crate::config::Config;
use crate::session::{Action, Env, Session};
use crate::start::StartScreen;

enum Screen {
    Start(StartScreen),
    Unlocked(Box<Session>),
}

pub struct App {
    cfg: Config,
    screen: Screen,
    clipboard: SecureClipboard,
    last_activity: Instant,
}

impl App {
    pub fn new(ctx: &egui::Context) -> Self {
        let mut defs = egui::FontDefinitions::default();
        fonts::add_selawik(&mut defs);
        ctx.set_fonts(defs);
        let cfg = Config::load();
        let first = cfg.recent.first().filter(|p| p.exists()).cloned();
        let app = Self {
            screen: Screen::Start(StartScreen::new(first)),
            cfg,
            clipboard: SecureClipboard::new(),
            last_activity: Instant::now(),
        };
        app.install_theme(ctx);
        app
    }

    fn install_theme(&self, ctx: &egui::Context) {
        let dark = self
            .cfg
            .dark
            .unwrap_or_else(|| ctx.system_theme() == Some(egui::Theme::Dark));
        let brightness = if dark {
            Brightness::Dark
        } else {
            Brightness::Light
        };
        FluentTheme::new(brightness)
            .with_semibold_font()
            .install(ctx);
        Localizations::for_locale("ru-RU").install(ctx);
    }

    /// Drops the decrypted vault and the key (both wipe themselves) and
    /// forgets text field state, which can hold copies of typed secrets.
    fn lock(&mut self, ctx: &egui::Context) {
        let path = match &self.screen {
            Screen::Unlocked(s) => s.file().path().to_owned(),
            Screen::Start(_) => return,
        };
        self.screen = Screen::Start(StartScreen::new(Some(path)));
        self.clipboard.clear_now();
        ctx.data_mut(|d| d.remove_by_type::<egui::text_edit::TextEditState>());
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let active = ctx.input(|i| !i.events.is_empty() || i.pointer.is_moving());
        if active {
            self.last_activity = Instant::now();
        }
        self.clipboard.tick();

        match &mut self.screen {
            Screen::Start(start) => {
                let theme = FluentTheme::of(&ctx);
                let mut opened = None;
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(theme.mica_background_color))
                    .show(ui, |ui| opened = start.ui(ui, &self.cfg));
                if let Some((file, vault)) = opened {
                    self.cfg.add_recent(file.path().to_owned());
                    self.cfg.save();
                    self.last_activity = Instant::now();
                    self.screen = Screen::Unlocked(Box::new(Session::new(file, vault, &self.cfg)));
                    ctx.data_mut(|d| d.remove_by_type::<egui::text_edit::TextEditState>());
                }
            }
            Screen::Unlocked(session) => {
                let mut env = Env {
                    cfg: &mut self.cfg,
                    clipboard: &mut self.clipboard,
                    config_changed: false,
                    theme_changed: false,
                };
                let action = session.ui(ui, &mut env);
                let (config_changed, theme_changed) = (env.config_changed, env.theme_changed);
                if config_changed {
                    self.cfg.save();
                }
                if theme_changed {
                    self.install_theme(&ctx);
                }
                let idle = self.cfg.autolock_minutes > 0
                    && self.last_activity.elapsed()
                        >= Duration::from_secs(u64::from(self.cfg.autolock_minutes) * 60);
                let minimized = self.cfg.lock_on_minimize
                    && ctx.input(|i| i.viewport().minimized == Some(true));
                if matches!(action, Action::Lock) || idle || minimized {
                    self.lock(&ctx);
                }
            }
        }
        // Keep the auto-lock and clipboard timers running while idle.
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}
