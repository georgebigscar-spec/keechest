//! Settings: appearance, locking, the clipboard and the master password.

use std::fs::File;
use std::io::Read;

use egui::Ui;
use fluent_egui::{
    Button, ContentDialog, DialogResult, Expander, InfoBar, NumberBox, PasswordBox, ProgressRing,
    Severity, TextBox, ToggleSwitch,
};
use keechest_core::{Entry, Result, Strength, VaultFile, Zeroizing, interop, strength};

use crate::platform::{self, FileKind, Picked};
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
    kdbx: Option<KdbxImport>,
    confirm_export: bool,
    /// A file picker that is open, and what for.
    picker: Option<(Task<Picked>, Pick)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    KeePass,
    KeyFile,
    Csv,
    ExportCsv,
    Backup,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ImportSource {
    KeePass,
    Csv,
}

/// The KeePass import dialog.
struct KdbxImport {
    name: String,
    data: Zeroizing<Vec<u8>>,
    password: Zeroizing<String>,
    /// Name and contents.
    keyfile: Option<(String, Zeroizing<Vec<u8>>)>,
    task: Option<Task<Result<Vec<Entry>>>>,
    error: Option<String>,
}

pub enum SettingsAction {
    None,
    Rename(String),
    /// Entries read from another manager, and where they came from.
    Import(Vec<Entry>, ImportSource),
    /// Write the vault as CSV to this file.
    ExportCsv(File),
    /// Write a copy of the (encrypted) vault file to this file.
    Backup(File),
    Error(String),
    /// A key for the new master password; the session switches to it.
    Rekeyed(Box<VaultFile>),
}

impl SettingsState {
    fn pick(&mut self, ctx: &egui::Context, what: Pick) {
        let task = match what {
            Pick::KeePass => platform::open_file(ctx, FileKind::KeePass),
            Pick::KeyFile => platform::open_file(ctx, FileKind::KeyFile),
            Pick::Csv => platform::open_file(ctx, FileKind::Csv),
            Pick::ExportCsv => platform::create_file(ctx, FileKind::Csv, "keechest-export.csv"),
            Pick::Backup => {
                let name = self.name.trim();
                let name = if name.is_empty() { "KeeChest" } else { name };
                let name = format!("{name}.{}", keechest_core::format::EXTENSION);
                platform::create_file(ctx, FileKind::Vault, &name)
            }
        };
        self.picker = Some((task, what));
    }

    /// Acts on the file the user picked, once they have.
    fn poll_picker(&mut self) -> Option<SettingsAction> {
        let (task, what) = self.picker.as_ref()?;
        let what = *what;
        let picked = match task.poll() {
            Poll::Pending => return None,
            Poll::Done(p) => p,
            Poll::Failed => Ok(None),
        };
        self.picker = None;
        let picked = match picked {
            Ok(Some(p)) => p,
            Ok(None) => return None,
            Err(e) => return Some(SettingsAction::Error(error_text(&e.into()))),
        };
        let read = |mut f: File| -> Result<Zeroizing<Vec<u8>>> {
            let mut data = Zeroizing::new(Vec::new());
            f.read_to_end(&mut data)?;
            Ok(data)
        };
        let action = match what {
            Pick::ExportCsv => SettingsAction::ExportCsv(picked.file),
            Pick::Backup => SettingsAction::Backup(picked.file),
            Pick::Csv => match read(picked.file).and_then(|data| interop::import_csv(&data)) {
                Ok(entries) => SettingsAction::Import(entries, ImportSource::Csv),
                Err(e) => SettingsAction::Error(error_text(&e)),
            },
            Pick::KeePass => match read(picked.file) {
                Ok(data) => {
                    self.kdbx = Some(KdbxImport {
                        name: picked.name,
                        data,
                        password: Zeroizing::default(),
                        keyfile: None,
                        task: None,
                        error: None,
                    });
                    SettingsAction::None
                }
                Err(e) => SettingsAction::Error(error_text(&e)),
            },
            Pick::KeyFile => {
                match (read(picked.file), self.kdbx.as_mut()) {
                    (Ok(data), Some(import)) => import.keyfile = Some((picked.name, data)),
                    (Err(e), Some(import)) => import.error = Some(error_text(&e)),
                    (_, None) => {}
                }
                SettingsAction::None
            }
        };
        Some(action)
    }

    /// Closes an open dialog. Returns whether there was one.
    pub fn close_dialog(&mut self) -> bool {
        let open = self.kdbx.is_some() || self.confirm_export;
        self.kdbx = None;
        self.confirm_export = false;
        open
    }

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
            kdbx: None,
            confirm_export: false,
            picker: None,
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
            ui.horizontal_wrapped(|ui| {
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
                    if platform::TOUCH {
                        "Блокировать при переходе в фон"
                    } else {
                        "Блокировать при сворачивании окна"
                    },
                ))
                .changed()
            {
                env.config_changed = true;
            }

            section(ui, "Буфер обмена");
            let mut seconds = f64::from(cfg.clipboard_seconds);
            ui.horizontal_wrapped(|ui| {
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
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    TextBox::new(&mut state.name)
                        .width(WIDTH.min(ui.available_width()))
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

            ui.add_space(6.0);
            if ui
                .add_enabled(
                    state.picker.is_none(),
                    Button::new("Сохранить резервную копию…"),
                )
                .clicked()
            {
                state.pick(ui.ctx(), Pick::Backup);
            }
            ui.label(caption(
                ui,
                "Копия зашифрована тем же мастер-паролем, её можно хранить где угодно.",
            ));

            ui.add_space(12.0);
            Expander::new("change_master", "Сменить мастер-пароль").show(ui, |ui| {
                change_password_ui(ui, state, file);
            });

            section(ui, "Импорт и экспорт");
            ui.label(caption(
                ui,
                "Записи добавляются к существующим, точные дубликаты пропускаются.",
            ));
            ui.add_space(4.0);
            let idle = state.picker.is_none();
            ui.horizontal_wrapped(|ui| {
                let ctx = ui.ctx().clone();
                if ui
                    .add_enabled(idle, Button::new("Импорт из KeePass (.kdbx)…"))
                    .clicked()
                {
                    state.pick(&ctx, Pick::KeePass);
                }
                if ui
                    .add_enabled(idle, Button::new("Импорт из CSV…"))
                    .clicked()
                {
                    state.pick(&ctx, Pick::Csv);
                }
                if ui
                    .add_enabled(idle, Button::new("Экспорт в CSV…"))
                    .clicked()
                {
                    state.confirm_export = true;
                }
            });
            ui.label(caption(
                ui,
                "CSV подходит для KeePassXC, Bitwarden, Chrome, Firefox и других менеджеров.",
            ));
        });

    let ctx = ui.ctx().clone();
    if let Some(a) = state.poll_picker() {
        action = a;
    }
    if let Some(a) = kdbx_dialog(&ctx, state) {
        action = a;
    }
    if state.confirm_export {
        let result = ContentDialog::new("confirm_export", "Экспортировать без шифрования?")
            .body(
                "В CSV-файле все пароли будут открытым текстом. Любой, кто получит файл, \
                 прочитает их. Удалите его, как только перенесёте данные.",
            )
            .primary_button("Выбрать файл…")
            .close_button("Отмена")
            .show(&ctx);
        if let Some(result) = result {
            state.confirm_export = false;
            if result == DialogResult::Primary {
                state.pick(&ctx, Pick::ExportCsv);
            }
        }
    }
    action
}

fn kdbx_dialog(ctx: &egui::Context, state: &mut SettingsState) -> Option<SettingsAction> {
    let import = state.kdbx.as_mut()?;
    if let Some(task) = &import.task {
        match task.poll() {
            Poll::Pending => {}
            Poll::Done(Ok(entries)) => {
                state.kdbx = None;
                return Some(SettingsAction::Import(entries, ImportSource::KeePass));
            }
            Poll::Done(Err(e)) => {
                import.task = None;
                import.password.clear();
                import.error = Some(error_text(&e));
            }
            Poll::Failed => {
                import.task = None;
                import.error = Some("Внутренняя ошибка при чтении базы KeePass.".into());
            }
        }
    }
    let busy = import.task.is_some() || state.picker.is_some();
    let mut pick_keyfile = false;
    let (result, enter) = ContentDialog::new("kdbx_import", "Импорт из KeePass")
        .body(format!("Файл: {}", import.name))
        .primary_button("Импортировать")
        .close_button("Отмена")
        .width(420.0)
        .show_with(ctx, |ui| {
            let mut enter = false;
            ui.add_enabled_ui(!busy, |ui| {
                let field = ui.add(
                    PasswordBox::new(&mut import.password)
                        .placeholder("Мастер-пароль базы KeePass")
                        .width(ui.available_width())
                        .id_salt("kdbx_password"),
                );
                enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    let label = match &import.keyfile {
                        Some((name, _)) => format!("Ключевой файл: {name}"),
                        None => "Без ключевого файла".into(),
                    };
                    ui.label(caption(ui, label));
                    if ui.add(Button::hyperlink("Выбрать…")).clicked() {
                        pick_keyfile = true;
                    }
                    if import.keyfile.is_some() && ui.add(Button::hyperlink("Убрать")).clicked()
                    {
                        import.keyfile = None;
                    }
                });
            });
            if busy {
                ui.horizontal(|ui| {
                    ui.add(ProgressRing::indeterminate().size(20.0));
                    ui.label("Расшифровка…");
                });
            }
            if let Some(error) = &import.error {
                InfoBar::new(error.as_str())
                    .severity(Severity::Error)
                    .show(ui);
            }
            enter
        });
    if pick_keyfile {
        state.pick(ctx, Pick::KeyFile);
        return None;
    }
    match result {
        Some(DialogResult::Primary) if !busy => start_kdbx(ctx, import),
        Some(DialogResult::Primary) | None => {
            if enter && !busy {
                start_kdbx(ctx, import);
            }
        }
        Some(_) => state.kdbx = None,
    }
    None
}

fn start_kdbx(ctx: &egui::Context, import: &mut KdbxImport) {
    import.error = None;
    let data = import.data.clone();
    let key = import.keyfile.as_ref().map(|(_, k)| k.clone());
    let password = import.password.clone();
    import.task = Some(Task::spawn(ctx, move || {
        interop::import_kdbx(&data, &password, key.as_deref().map(|k| k.as_slice()))
    }));
}

fn change_password_ui(ui: &mut Ui, state: &mut SettingsState, file: &VaultFile) {
    let busy = state.task.is_some();
    ui.add_enabled_ui(!busy, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.add(
            PasswordBox::new(&mut state.current)
                .placeholder("Текущий мастер-пароль")
                .width(WIDTH.min(ui.available_width()))
                .id_salt("cur_master"),
        );
        let new = ui.add(
            PasswordBox::new(&mut state.new)
                .placeholder("Новый мастер-пароль")
                .width(WIDTH.min(ui.available_width()))
                .id_salt("new_master2"),
        );
        if new.changed() {
            state.strength = strength::estimate(&state.new, &[]);
        }
        strength_meter(ui, state.strength, WIDTH.min(ui.available_width()));
        ui.add(
            PasswordBox::new(&mut state.confirm)
                .placeholder("Повторите новый пароль")
                .width(WIDTH.min(ui.available_width()))
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
