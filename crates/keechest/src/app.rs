//! The top level: which screen is shown, auto-lock, theme and clipboard.

use std::time::{Duration, Instant};

use fluent_egui::{Brightness, FluentTheme, Localizations, fonts};

use crate::clipboard::SecureClipboard;
use crate::config::Config;
use crate::platform;
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
        if platform::TOUCH {
            // Room for a fingertip between tappable things, and no
            // tooltips popping up after a long press.
            ctx.all_styles_mut(|style| {
                style.spacing.interact_size.y = style.spacing.interact_size.y.max(32.0);
                style.spacing.item_spacing.y = style.spacing.item_spacing.y.max(6.0);
                style.interaction.tooltip_delay = f32::INFINITY;
            });
        }
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
        platform::begin_frame(&ctx);
        // Locks before anything is drawn when coming back from the
        // background.
        if platform::take_backgrounded() && self.cfg.lock_on_minimize {
            self.lock(&ctx);
        }
        let back = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::BrowserBack));
        let active = ctx.input(|i| !i.events.is_empty() || i.pointer.is_moving());
        if active {
            self.last_activity = Instant::now();
        }
        self.clipboard.tick();

        match &mut self.screen {
            Screen::Start(start) => {
                if back && !start.back() {
                    platform::leave();
                }
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
                if back && !session.back() {
                    platform::leave();
                }
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
        platform::end_frame(&ctx);
        // Keep the auto-lock and clipboard timers running while idle.
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}
