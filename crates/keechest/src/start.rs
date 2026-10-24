//! The screen before a vault is open: unlock an existing one or create one.

use std::path::{Path, PathBuf};

use egui::{Align, Key, Layout, Ui, vec2};
use fluent_egui::{
    Button, Card, FluentTheme, InfoBar, PasswordBox, ProgressRing, Severity, TextBox,
};
use keechest_core::{Result, Strength, Vault, VaultFile, Zeroizing, format, strength};

use crate::config::Config;
use crate::task::{Poll, Task};
use crate::widgets::{KdfPreset, caption, error_text, strength_meter};

const MIN_MASTER_LEN: usize = 8;
const WIDTH: f32 = 380.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Unlock,
    Create,
}

pub struct StartScreen {
    mode: Mode,
    path: Option<PathBuf>,
    name: String,
    password: Zeroizing<String>,
    confirm: Zeroizing<String>,
    strength: Strength,
    preset: KdfPreset,
    error: Option<String>,
    task: Option<Task<Result<(VaultFile, Vault)>>>,
    focus_password: bool,
    /// The save dialog already asked about replacing the chosen file.
    overwrite_confirmed: bool,
}

impl StartScreen {
    /// Unlocking `path`, or the choice between opening and creating if none.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            mode: Mode::Unlock,
            path,
            name: "Мои пароли".into(),
            password: Zeroizing::default(),
            confirm: Zeroizing::default(),
            strength: strength::estimate("", &[]),
            preset: KdfPreset::Standard,
            error: None,
            task: None,
            focus_password: true,
            overwrite_confirmed: false,
        }
    }

    /// Shows the screen; returns the vault once it is unlocked or created.
    pub fn ui(&mut self, ui: &mut Ui, cfg: &Config) -> Option<(VaultFile, Vault)> {
        let opened = self.poll();
        let theme = FluentTheme::of(ui.ctx());
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    ui.add_space((ui.available_height() * 0.12).max(24.0));
                    ui.label(theme.typography.title_large.rich("KeeChest"));
                    ui.label(caption(ui, "Локальный менеджер паролей"));
                    ui.add_space(20.0);
                    Card::new().padding(24).show(ui, |ui| {
                        ui.set_width(WIDTH);
                        ui.with_layout(Layout::top_down(Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.y = 8.0;
                            let busy = self.task.is_some();
                            ui.add_enabled_ui(!busy, |ui| match self.mode {
                                Mode::Unlock => self.unlock_ui(ui, cfg),
                                Mode::Create => self.create_ui(ui),
                            });
                            if busy {
                                ui.horizontal(|ui| {
                                    ui.add(ProgressRing::indeterminate().size(20.0));
                                    ui.label("Вычисление ключа…");
                                });
                            }
                            if let Some(error) = &self.error {
                                InfoBar::new("Ошибка")
                                    .message(error.as_str())
                                    .severity(Severity::Error)
                                    .long(true)
                                    .show(ui);
                            }
                        });
                    });
                });
            });
        opened
    }

    fn poll(&mut self) -> Option<(VaultFile, Vault)> {
        let task = self.task.as_ref()?;
        let result = match task.poll() {
            Poll::Pending => return None,
            Poll::Done(r) => r,
            Poll::Failed => {
                self.task = None;
                self.error = Some("Внутренняя ошибка при вычислении ключа.".into());
                return None;
            }
        };
        self.task = None;
        self.password.clear();
        self.confirm.clear();
        match result {
            Ok(opened) => Some(opened),
            Err(e) => {
                self.error = Some(error_text(&e));
                self.focus_password = true;
                None
            }
        }
    }

    fn unlock_ui(&mut self, ui: &mut Ui, cfg: &Config) {
        let theme = FluentTheme::of(ui.ctx());
        match self.path.clone() {
            Some(path) => {
                ui.label(theme.typography.subtitle.rich(file_name(&path)));
                ui.label(caption(ui, path.display().to_string()));
                ui.add_space(4.0);
                let field = ui.add(
                    PasswordBox::new(&mut self.password)
                        .placeholder("Мастер-пароль")
                        .width(WIDTH)
                        .id_salt("master"),
                );
                if std::mem::take(&mut self.focus_password) {
                    field.request_focus();
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let unlock = ui.add(Button::filled("Разблокировать").min_size(vec2(WIDTH, 32.0)));
                if (enter || unlock.clicked()) && !self.password.is_empty() {
                    self.start_unlock(ui.ctx(), path.clone());
                }
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.add(Button::hyperlink("Другое хранилище…")).clicked() {
                        self.pick_existing();
                    }
                    if ui.add(Button::hyperlink("Создать новое")).clicked() {
                        self.switch_to_create();
                    }
                });
                let others: Vec<&PathBuf> = cfg.recent.iter().filter(|p| **p != path).collect();
                if !others.is_empty() {
                    ui.add_space(4.0);
                    ui.label(caption(ui, "Недавние"));
                    for p in others {
                        if ui
                            .add(Button::hyperlink(file_name(p)))
                            .on_hover_text(p.display().to_string())
                            .clicked()
                        {
                            self.path = Some(p.clone());
                            self.error = None;
                            self.focus_password = true;
                        }
                    }
                }
            }
            None => {
                ui.label(theme.typography.subtitle.rich("Добро пожаловать"));
                ui.label(
                    "Пароли хранятся в одном зашифрованном файле на этом компьютере. \
                     Без мастер-пароля его не открыть, поэтому не забывайте его.",
                );
                ui.add_space(8.0);
                if ui
                    .add(Button::filled("Создать хранилище").min_size(vec2(WIDTH, 32.0)))
                    .clicked()
                {
                    self.switch_to_create();
                }
                if ui
                    .add(Button::new("Открыть существующее…").min_size(vec2(WIDTH, 32.0)))
                    .clicked()
                {
                    self.pick_existing();
                }
            }
        }
    }

    fn create_ui(&mut self, ui: &mut Ui) {
        let theme = FluentTheme::of(ui.ctx());
        ui.label(theme.typography.subtitle.rich("Новое хранилище"));
        ui.label(caption(ui, "Название"));
        ui.add(TextBox::new(&mut self.name).width(WIDTH).id_salt("name"));

        ui.label(caption(ui, "Файл"));
        let path = self.path.get_or_insert_with(Config::default_vault_path);
        ui.horizontal(|ui| {
            ui.add(
                egui::Label::new(path.display().to_string())
                    .truncate()
                    .wrap_mode(egui::TextWrapMode::Truncate),
            );
        });
        if ui.add(Button::new("Выбрать место…")).clicked() {
            let mut dialog = rfd::FileDialog::new()
                .add_filter("KeeChest", &[format::EXTENSION])
                .set_file_name(file_name(path));
            if let Some(dir) = path.parent() {
                dialog = dialog.set_directory(dir);
            }
            if let Some(mut chosen) = dialog.save_file() {
                if chosen.extension().is_none() {
                    chosen.set_extension(format::EXTENSION);
                }
                *path = chosen;
                self.overwrite_confirmed = true;
            }
        }

        ui.label(caption(ui, "Мастер-пароль"));
        let field = ui.add(
            PasswordBox::new(&mut self.password)
                .placeholder("Не короче 8 символов")
                .width(WIDTH)
                .id_salt("new_master"),
        );
        if std::mem::take(&mut self.focus_password) {
            field.request_focus();
        }
        if field.changed() {
            self.strength = strength::estimate(&self.password, &[&self.name]);
        }
        strength_meter(ui, self.strength, WIDTH);
        ui.add(
            PasswordBox::new(&mut self.confirm)
                .placeholder("Повторите мастер-пароль")
                .width(WIDTH)
                .id_salt("confirm_master"),
        );

        ui.label(caption(ui, "Защита ключа (Argon2id)"));
        KdfPreset::combo(ui, "kdf_preset", &mut self.preset);

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(Button::filled("Создать")).clicked() {
                self.start_create(ui.ctx());
            }
            if ui.add(Button::new("Назад")).clicked() {
                self.mode = Mode::Unlock;
                self.path = None;
                self.error = None;
                self.password.clear();
                self.confirm.clear();
            }
        });
    }

    fn switch_to_create(&mut self) {
        self.mode = Mode::Create;
        self.overwrite_confirmed = false;
        self.path = None;
        self.error = None;
        self.password.clear();
        self.confirm.clear();
        self.strength = strength::estimate("", &[]);
        self.focus_password = true;
    }

    fn pick_existing(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("KeeChest", &[format::EXTENSION])
            .pick_file()
        {
            self.path = Some(path);
            self.mode = Mode::Unlock;
            self.error = None;
            self.focus_password = true;
        }
    }

    fn start_unlock(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.error = None;
        let password = self.password.clone();
        self.task = Some(Task::spawn(ctx, move || VaultFile::open(path, &password)));
    }

    fn start_create(&mut self, ctx: &egui::Context) {
        self.error = None;
        let Some(path) = self.path.clone() else {
            return;
        };
        if path.exists() && !self.overwrite_confirmed {
            self.error = Some(
                "Такой файл уже есть. Выберите другое место, чтобы не перезаписать его.".into(),
            );
            return;
        }
        if self.password.chars().count() < MIN_MASTER_LEN {
            self.error = Some(format!(
                "Мастер-пароль должен быть не короче {MIN_MASTER_LEN} символов."
            ));
            return;
        }
        if *self.password != *self.confirm {
            self.error = Some("Пароли не совпадают.".into());
            return;
        }
        let password = self.password.clone();
        let kdf = self.preset.params();
        let name = if self.name.trim().is_empty() {
            "Мои пароли".to_owned()
        } else {
            self.name.trim().to_owned()
        };
        self.task = Some(Task::spawn(ctx, move || {
            let vault = Vault::new(name);
            let file = VaultFile::create(path, &password, kdf, &vault)?;
            Ok((file, vault))
        }));
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}
