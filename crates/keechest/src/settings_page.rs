//! Settings: appearance, locking, the clipboard and the master password.

use egui::Ui;
use fluent_egui::{
    Button, Expander, InfoBar, NumberBox, PasswordBox, ProgressRing, Severity, TextBox,
    ToggleSwitch,
};
use keechest_core::{Result, Strength, VaultFile, Zeroizing, strength};

use crate::session::Env;
use crate::task::{Poll, Task};
use crate::widgets::{KdfPreset, caption, describe_kdf, error_text, section, strength_meter};

const WIDTH: f32 = 320.0;

pub struct SettingsState {
    name: String,
    current: Zeroizing<String>,
    new: Zeroizing<String>,
    confirm: Zeroizing<String>,
    strength: Strength,
    preset: KdfPreset,
    task: Option<Task<Result<VaultFile>>>,
    error: Option<String>,
}

pub enum SettingsAction {
    None,
    Rename(String),
    /// A key for the new master password; the session switches to it.
    Rekeyed(Box<VaultFile>),
}

impl SettingsState {
    pub fn new(name: String) -> Self {
        Self {
            name,
            current: Zeroizing::default(),
            new: Zeroizing::default(),
            confirm: Zeroizing::default(),
            strength: strength::estimate("", &[]),
            preset: KdfPreset::Standard,
            task: None,
            error: None,
        }
    }

    fn poll(&mut self) -> Option<VaultFile> {
        let result = match self.task.as_ref()?.poll() {
            Poll::Pending => return None,
            Poll::Done(r) => r,
            Poll::Failed => {
                self.task = None;
                self.error = Some("Внутренняя ошибка при вычислении ключа.".into());
                return None;
            }
        };
        self.task = None;
        self.current.clear();
        match result {
            Ok(file) => {
                self.new.clear();
                self.confirm.clear();
                self.strength = strength::estimate("", &[]);
                Some(file)
            }
            Err(e) => {
                self.error = Some(match e {
                    keechest_core::Error::WrongPasswordOrCorrupted => {
                        "Текущий мастер-пароль введён неверно.".into()
                    }
                    e => error_text(&e),
                });
                None
            }
        }
    }
}

pub fn ui(
    ui: &mut Ui,
    state: &mut SettingsState,
    env: &mut Env,
    file: &VaultFile,
) -> SettingsAction {
    let mut action = SettingsAction::None;
    if let Some(new) = state.poll() {
        action = SettingsAction::Rekeyed(Box::new(new));
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            let cfg = &mut *env.cfg;
            section(ui, "Внешний вид");
            let mut dark = cfg.dark.unwrap_or_else(|| ui.visuals().dark_mode);
            if ui
                .add(ToggleSwitch::new(&mut dark, "Тёмная тема"))
                .changed()
            {
                cfg.dark = Some(dark);
                env.theme_changed = true;
                env.config_changed = true;
            }

            section(ui, "Блокировка");
            let mut minutes = f64::from(cfg.autolock_minutes);
            ui.horizontal(|ui| {
                ui.add(
                    NumberBox::new(&mut minutes)
                        .range(0.0..=240.0)
                        .precision(0)
                        .width(120.0)
                        .id_salt("autolock"),
                );
                ui.label("мин. без действий, 0 — не блокировать");
            });
            if minutes as u32 != cfg.autolock_minutes {
                cfg.autolock_minutes = minutes as u32;
                env.config_changed = true;
            }
            if ui
                .add(ToggleSwitch::new(
                    &mut cfg.lock_on_minimize,
                    "Блокировать при сворачивании окна",
                ))
                .changed()
            {
                env.config_changed = true;
            }

            section(ui, "Буфер обмена");
            let mut seconds = f64::from(cfg.clipboard_seconds);
            ui.horizontal(|ui| {
                ui.add(
                    NumberBox::new(&mut seconds)
                        .range(0.0..=600.0)
                        .precision(0)
                        .width(120.0)
                        .id_salt("clipboard"),
                );
                ui.label("с до очистки, 0 — не очищать");
            });
            if seconds as u32 != cfg.clipboard_seconds {
                cfg.clipboard_seconds = seconds as u32;
                env.config_changed = true;
            }

            section(ui, "Хранилище");
            ui.label(file.path().display().to_string());
            ui.label(caption(ui, describe_kdf(file.kdf())));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add(
                    TextBox::new(&mut state.name)
                        .width(WIDTH)
                        .id_salt("vault_name"),
                );
                let name = state.name.trim();
                if ui
                    .add_enabled(!name.is_empty(), Button::new("Переименовать"))
                    .clicked()
                {
                    action = SettingsAction::Rename(name.to_owned());
                }
            });

            ui.add_space(12.0);
            Expander::new("change_master", "Сменить мастер-пароль").show(ui, |ui| {
                change_password_ui(ui, state, file);
            });
        });
    action
}

fn change_password_ui(ui: &mut Ui, state: &mut SettingsState, file: &VaultFile) {
    let busy = state.task.is_some();
    ui.add_enabled_ui(!busy, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.add(
            PasswordBox::new(&mut state.current)
                .placeholder("Текущий мастер-пароль")
                .width(WIDTH)
                .id_salt("cur_master"),
        );
        let new = ui.add(
            PasswordBox::new(&mut state.new)
                .placeholder("Новый мастер-пароль")
                .width(WIDTH)
                .id_salt("new_master2"),
        );
        if new.changed() {
            state.strength = strength::estimate(&state.new, &[]);
        }
        strength_meter(ui, state.strength, WIDTH);
        ui.add(
            PasswordBox::new(&mut state.confirm)
                .placeholder("Повторите новый пароль")
                .width(WIDTH)
                .id_salt("confirm_master2"),
        );
        ui.label(caption(ui, "Защита ключа (Argon2id)"));
        KdfPreset::combo(ui, "kdf_preset2", &mut state.preset);
        if ui.add(Button::filled("Сменить пароль")).clicked() {
            state.error = None;
            if state.new.chars().count() < 8 {
                state.error = Some("Новый пароль должен быть не короче 8 символов.".into());
            } else if *state.new != *state.confirm {
                state.error = Some("Новые пароли не совпадают.".into());
            } else {
                let path = file.path().to_owned();
                let current = state.current.clone();
                let new = state.new.clone();
                let kdf = state.preset.params();
                state.task = Some(Task::spawn(ui.ctx(), move || {
                    // Opening the file checks the current password.
                    VaultFile::open(&path, &current)?;
                    VaultFile::with_password(path, &new, kdf)
                }));
            }
        }
    });
    if busy {
        ui.horizontal(|ui| {
            ui.add(ProgressRing::indeterminate().size(20.0));
            ui.label("Вычисление ключа…");
        });
    }
    if let Some(error) = &state.error {
        InfoBar::new(error.as_str())
            .severity(Severity::Error)
            .show(ui);
    }
}
