//! The screen before a vault is open: unlock an existing one or create one.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use egui::{Align, Key, Layout, Ui, vec2};
use fluent_egui::{
    Button, Card, FluentTheme, InfoBar, PasswordBox, ProgressRing, Severity, TextBox,
};
use keechest_core::{Result, Strength, Vault, VaultFile, Zeroizing, format, strength};

use crate::config::Config;
use crate::platform::{self, FileKind, Picked, PickedFile};
use crate::task::{Poll, Task};
use crate::widgets::{BUTTON_HEIGHT, KdfPreset, caption, error_text, strength_meter};

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
    /// The file picker for "open existing", while it is shown.
    picker: Option<Task<Picked>>,
    /// Width of the fields: narrower than usual on a phone.
    width: f32,
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
            picker: None,
            width: WIDTH,
            // Don't pop the keyboard up over the screen on a phone.
            focus_password: !platform::TOUCH,
            overwrite_confirmed: false,
        }
    }

    /// The system Back button. Returns false when there is nothing to go
    /// back to.
    pub fn back(&mut self) -> bool {
        if self.mode == Mode::Create {
            self.mode = Mode::Unlock;
            self.path = None;
            self.error = None;
            self.password.clear();
            self.confirm.clear();
            true
        } else {
            false
        }
    }

    /// Shows the screen; returns the vault once it is unlocked or created.
    pub fn ui(&mut self, ui: &mut Ui, cfg: &Config) -> Option<(VaultFile, Vault)> {
        let opened = self.poll();
        self.poll_picker();
        let theme = FluentTheme::of(ui.ctx());
        // The card's padding and a margin around it.
        self.width = WIDTH.min(ui.available_width() - 64.0).max(160.0);
        let width = self.width;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    let top = if platform::TOUCH { 0.05 } else { 0.12 };
                    ui.add_space((ui.available_height() * top).max(24.0));
                    ui.label(theme.typography.title_large.rich("KeeChest"));
                    ui.label(caption(ui, "Локальный менеджер паролей"));
                    ui.add_space(20.0);
                    Card::new().padding(24).show(ui, |ui| {
                        ui.set_width(width);
                        ui.with_layout(Layout::top_down(Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.y = 8.0;
                            let busy = self.task.is_some() || self.picker.is_some();
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
                        .width(self.width)
                        .id_salt("master"),
                );
                if std::mem::take(&mut self.focus_password) {
                    field.request_focus();
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let unlock = ui.add(
                    Button::filled("Разблокировать").min_size(vec2(self.width, BUTTON_HEIGHT)),
                );
                if (enter || unlock.clicked()) && !self.password.is_empty() {
                    self.focus_password = false;
                    self.start_unlock(ui.ctx(), path.clone());
                }
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.add(Button::hyperlink("Другое хранилище…")).clicked() {
                        self.pick_existing(ui.ctx());
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
                ui.label(if platform::TOUCH {
                    "Пароли хранятся в одном зашифрованном файле на этом устройстве. \
                     Без мастер-пароля его не открыть, поэтому не забывайте его."
                } else {
                    "Пароли хранятся в одном зашифрованном файле на этом компьютере. \
                     Без мастер-пароля его не открыть, поэтому не забывайте его."
                });
                ui.add_space(8.0);
                if ui
                    .add(
                        Button::filled("Создать хранилище")
                            .min_size(vec2(self.width, BUTTON_HEIGHT)),
                    )
                    .clicked()
                {
                    self.switch_to_create();
                }
                if ui
                    .add(
                        Button::new("Открыть существующее…")
                            .min_size(vec2(self.width, BUTTON_HEIGHT)),
                    )
                    .clicked()
                {
                    self.pick_existing(ui.ctx());
                }
            }
        }
    }

    fn create_ui(&mut self, ui: &mut Ui) {
        let theme = FluentTheme::of(ui.ctx());
        ui.label(theme.typography.subtitle.rich("Новое хранилище"));
        ui.label(caption(ui, "Название"));
        ui.add(
            TextBox::new(&mut self.name)
                .width(self.width)
                .id_salt("name"),
        );

        if platform::CHOOSE_VAULT_LOCATION {
            ui.label(caption(ui, "Файл"));
            let path = self.path.get_or_insert_with(Config::default_vault_path);
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(path.display().to_string())
                        .truncate()
                        .wrap_mode(egui::TextWrapMode::Truncate),
                );
            });
            if ui.add(Button::new("Выбрать место…")).clicked()
                && let Some(chosen) = platform::choose_vault_location(path)
            {
                *path = chosen;
                self.overwrite_confirmed = true;
            }
        } else {
            ui.label(caption(
                ui,
                "Файл хранится в памяти приложения. Резервную копию можно сохранить \
                 в настройках.",
            ));
        }

        ui.label(caption(ui, "Мастер-пароль"));
        let field = ui.add(
            PasswordBox::new(&mut self.password)
                .placeholder("Не короче 8 символов")
                .width(self.width)
                .id_salt("new_master"),
        );
        if std::mem::take(&mut self.focus_password) {
            field.request_focus();
        }
        if field.changed() {
            self.strength = strength::estimate(&self.password, &[&self.name]);
        }
        strength_meter(ui, self.strength, self.width);
        ui.add(
            PasswordBox::new(&mut self.confirm)
                .placeholder("Повторите мастер-пароль")
                .width(self.width)
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

    fn pick_existing(&mut self, ctx: &egui::Context) {
        self.picker = Some(platform::open_file(ctx, FileKind::Vault));
    }

    fn poll_picker(&mut self) {
        let Some(picker) = &self.picker else { return };
        let picked = match picker.poll() {
            Poll::Pending => return,
            Poll::Done(p) => p,
            Poll::Failed => Ok(None),
        };
        self.picker = None;
        let path = match picked {
            Ok(Some(PickedFile {
                path: Some(path), ..
            })) => Ok(path),
            // From the Android picker: keep a copy in the app's storage.
            Ok(Some(picked)) => keep_copy(picked),
            Ok(None) => return,
            Err(e) => Err(e.into()),
        };
        match path {
            Ok(path) => {
                self.path = Some(path);
                self.mode = Mode::Unlock;
                self.error = None;
                self.focus_password = !platform::TOUCH;
            }
            Err(e) => self.error = Some(error_text(&e)),
        }
    }

    fn start_unlock(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.error = None;
        let password = self.password.clone();
        self.task = Some(Task::spawn(ctx, move || VaultFile::open(path, &password)));
    }

    fn start_create(&mut self, ctx: &egui::Context) {
        self.error = None;
        let name = if self.name.trim().is_empty() {
            "Мои пароли".to_owned()
        } else {
            self.name.trim().to_owned()
        };
        let path = if platform::CHOOSE_VAULT_LOCATION {
            match self.path.clone() {
                Some(path) => path,
                None => return,
            }
        } else {
            Config::new_vault_path(&name)
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

/// Copies a vault the user picked into the vault directory, unless an
/// identical copy is already there, and returns its path.
fn keep_copy(mut picked: PickedFile) -> Result<PathBuf> {
    let mut bytes = Vec::new();
    picked.file.read_to_end(&mut bytes)?;
    // Fail early on anything that isn't a vault.
    format::Header::decode(&bytes)?;
    let dir = platform::vault_dir();
    let stem = Path::new(&picked.name)
        .file_stem()
        .map_or_else(|| "KeeChest".into(), |s| s.to_string_lossy().into_owned());
    let same = dir.join(format!("{stem}.{}", format::EXTENSION));
    if std::fs::read(&same).is_ok_and(|b| b == bytes) {
        return Ok(same);
    }
    let path = Config::new_vault_path(&stem);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(path)
}
