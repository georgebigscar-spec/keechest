//! The unlocked vault: navigation, the entry list, details and the editor.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use egui::{
    Align, Key, KeyboardShortcut, Layout, Margin, Modifiers, Rect, RichText, ScrollArea, Stroke,
    Ui, UiBuilder, pos2, vec2,
};
use fluent_egui::{
    AutoSuggestBox, Button, Checkbox, CommandBar, ContentDialog, DialogResult, Expander,
    FluentTheme, InfoBar, ListTile, ListTileSelectionMode, NavigationView, PaneItem, PasswordBox,
    PasswordRevealMode, Severity, TextBox,
};
use keechest_core::{
    CustomField, Entry, GeneratorOptions, Strength, Vault, VaultFile, Zeroizing, strength,
};
use zeroize::Zeroize;

use crate::clipboard::SecureClipboard;
use crate::config::Config;
use crate::generator_page::{self, GeneratorAction, GeneratorState};
use crate::settings_page::{self, ImportSource, SettingsAction, SettingsState};
use crate::widgets::{caption, error_text, format_time, strength_meter};

/// What the session shares with the app.
pub struct Env<'a> {
    pub cfg: &'a mut Config,
    pub clipboard: &'a mut SecureClipboard,
    /// Set when a setting changed; the app saves the config.
    pub config_changed: bool,
    /// Set when the theme setting changed.
    pub theme_changed: bool,
}

#[derive(Clone, PartialEq)]
pub enum Page {
    All,
    Favorites,
    Group(String),
    Generator,
    Settings,
}

impl Page {
    fn shows(&self, e: &Entry) -> bool {
        match self {
            Self::All => true,
            Self::Favorites => e.favorite,
            Self::Group(g) => e.group == *g,
            Self::Generator | Self::Settings => false,
        }
    }
}

pub enum Action {
    None,
    Lock,
}

struct Notice {
    title: String,
    message: String,
    severity: Severity,
    expires: Option<Instant>,
}

pub struct Session {
    file: VaultFile,
    vault: Vault,
    page: Page,
    selected: Option<String>,
    search: String,
    editor: Option<Editor>,
    /// Which secrets of the selected entry are shown.
    reveal: Reveal,
    confirm_delete: Option<String>,
    notice: Option<Notice>,
    generator: GeneratorState,
    settings: SettingsState,
}

impl Session {
    pub fn new(file: VaultFile, vault: Vault, cfg: &Config) -> Self {
        let name = vault.name.clone();
        Self {
            file,
            vault,
            page: Page::All,
            selected: None,
            search: String::new(),
            editor: None,
            reveal: Reveal::default(),
            confirm_delete: None,
            notice: None,
            generator: GeneratorState::new(&cfg.generator),
            settings: SettingsState::new(name),
        }
    }

    pub fn file(&self) -> &VaultFile {
        &self.file
    }

    fn notify(&mut self, title: &str, message: impl Into<String>, severity: Severity) {
        let expires =
            (severity != Severity::Error).then(|| Instant::now() + Duration::from_secs(6));
        self.notice = Some(Notice {
            title: title.into(),
            message: message.into(),
            severity,
            expires,
        });
    }

    /// Writes the vault to disk, reporting a failure.
    fn persist(&mut self) -> bool {
        match self.file.save(&self.vault) {
            Ok(()) => true,
            Err(e) => {
                self.notify(
                    "Не удалось сохранить",
                    format!("{} Изменения пока только в памяти.", error_text(&e)),
                    Severity::Error,
                );
                false
            }
        }
    }

    fn copy(&mut self, ctx: &egui::Context, env: &mut Env, what: &str, text: &str) {
        let secs = env.cfg.clipboard_seconds;
        let clear_after = (secs > 0).then(|| Duration::from_secs(u64::from(secs)));
        let message = if env.clipboard.copy(ctx, text, clear_after) {
            format!("Буфер обмена очистится через {secs} с.")
        } else {
            String::new()
        };
        self.notify(what, message, Severity::Success);
    }

    pub fn ui(&mut self, ui: &mut Ui, env: &mut Env) -> Action {
        let ctx = ui.ctx().clone();
        let theme = FluentTheme::of(&ctx);
        let mut action = Action::None;
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::L)))
        {
            return Action::Lock;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::N)))
        {
            self.new_entry();
        }

        let groups = self.vault.groups();
        if let Page::Group(g) = &self.page
            && !groups.contains(g)
        {
            self.page = Page::All;
        }
        let mut items = vec![
            PaneItem::new(Page::All, "Все записи").icon("🔑"),
            PaneItem::new(Page::Favorites, "Избранное").icon("★"),
        ];
        if !groups.is_empty() {
            items.push(PaneItem::header("Группы"));
            for g in &groups {
                items.push(PaneItem::new(Page::Group(g.clone()), g.as_str()).icon("🗀"));
            }
        }
        let footer = [
            PaneItem::new(Page::Generator, "Генератор").icon("⚄"),
            PaneItem::new(Page::Settings, "Настройки").icon("⚙"),
        ];

        let mut page = self.page.clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme.mica_background_color))
            .show(ui, |ui| {
                NavigationView::new("nav")
                    .title(self.vault.name.clone())
                    .items(items)
                    .footer_items(footer)
                    .show(ui, &mut page, |ui| {
                        action = match self.page.clone() {
                            Page::Generator => {
                                self.generator_ui(ui, env);
                                Action::None
                            }
                            Page::Settings => {
                                self.settings_ui(ui, env);
                                Action::None
                            }
                            list => self.list_page(ui, env, &list, &groups),
                        };
                    });
            });
        if page != self.page {
            self.page = page;
            self.search.clear();
        }
        self.delete_dialog(&ctx);
        self.notice_ui(&ctx);
        action
    }

    fn page_title(&self, page: &Page) -> String {
        match page {
            Page::All => "Все записи".into(),
            Page::Favorites => "Избранное".into(),
            Page::Group(g) => g.clone(),
            Page::Generator => "Генератор паролей".into(),
            Page::Settings => "Настройки".into(),
        }
    }

    /// The latest notice, floating at the bottom so the layout does not jump.
    fn notice_ui(&mut self, ctx: &egui::Context) {
        if self
            .notice
            .as_ref()
            .and_then(|n| n.expires)
            .is_some_and(|t| Instant::now() >= t)
        {
            self.notice = None;
        }
        let Some(n) = &self.notice else { return };
        let mut closed = false;
        egui::Area::new(egui::Id::unique("keechest_notice"))
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -20.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_width(460.0);
                let mut bar = InfoBar::new(n.title.as_str())
                    .severity(n.severity)
                    .closable(true);
                if !n.message.is_empty() {
                    bar = bar.message(n.message.as_str()).long(true);
                }
                closed = bar.show(ui).closed;
            });
        if closed {
            self.notice = None;
        }
    }

    fn list_page(&mut self, ui: &mut Ui, env: &mut Env, page: &Page, groups: &[String]) -> Action {
        let theme = FluentTheme::of(ui.ctx());
        let mut action = Action::None;
        egui::Frame::NONE
            .inner_margin(Margin::symmetric(20, 16))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme.typography.title.rich(self.page_title(page)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        CommandBar::new().show(ui, |bar| {
                            if bar
                                .button("🔒", "Заблокировать")
                                .on_hover_text("Ctrl+L")
                                .clicked()
                            {
                                action = Action::Lock;
                            }
                            bar.separator();
                            if bar
                                .button("+", "Новая запись")
                                .on_hover_text("Ctrl+N")
                                .clicked()
                            {
                                self.new_entry();
                            }
                        });
                    });
                });
                ui.add_space(8.0);

                let rect = ui.available_rect_before_wrap();
                let list_width = (rect.width() * 0.4).clamp(220.0, 340.0);
                let list_rect = Rect::from_min_size(rect.min, vec2(list_width, rect.height()));
                let detail_rect =
                    Rect::from_min_max(pos2(list_rect.right() + 17.0, rect.top()), rect.max);
                let mut list_ui = ui.new_child(
                    UiBuilder::new()
                        .max_rect(list_rect)
                        .layout(Layout::top_down(Align::Min)),
                );
                self.list_ui(&mut list_ui, page);
                ui.painter().vline(
                    list_rect.right() + 8.0,
                    rect.y_range(),
                    Stroke::new(1.0, theme.resources.divider_stroke_color_default),
                );
                let mut detail_ui = ui.new_child(
                    UiBuilder::new()
                        .max_rect(detail_rect)
                        .layout(Layout::top_down(Align::Min)),
                );
                self.detail_area(&mut detail_ui, env, groups);
                ui.advance_cursor_after_rect(rect);
            });
        action
    }

    fn list_ui(&mut self, ui: &mut Ui, page: &Page) {
        ui.add(
            TextBox::new(&mut self.search)
                .placeholder("Поиск")
                .clear_button(true)
                .width(ui.available_width())
                .id_salt("search"),
        );
        ui.add_space(8.0);
        let query = self.search.to_lowercase();
        let mut shown: Vec<&Entry> = self
            .vault
            .entries
            .iter()
            .filter(|e| page.shows(e) && e.matches(&query))
            .collect();
        shown.sort_by_cached_key(|e| e.title.to_lowercase());
        if shown.is_empty() {
            let text = if !query.is_empty() {
                "Ничего не найдено."
            } else if self.vault.entries.is_empty() {
                "Записей пока нет. Нажмите «Новая запись»."
            } else {
                "Здесь пусто."
            };
            ui.label(caption(ui, text));
            return;
        }
        let mut clicked = None;
        ScrollArea::vertical()
            .id_salt("entries")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for e in shown {
                    let title = if e.title.is_empty() {
                        "(без названия)"
                    } else {
                        e.title.as_str()
                    };
                    let mut tile = ListTile::new(title)
                        .selection_mode(ListTileSelectionMode::Single)
                        .selected(self.selected.as_ref() == Some(&e.id));
                    if !e.username.is_empty() {
                        tile = tile.subtitle(e.username.as_str());
                    }
                    if e.favorite {
                        tile = tile.trailing("★");
                    }
                    if ui.add(tile).clicked() {
                        clicked = Some(e.id.clone());
                    }
                }
            });
        if let Some(id) = clicked {
            self.selected = Some(id);
            self.editor = None;
            self.reveal = Reveal::default();
        }
    }

    fn new_entry(&mut self) {
        let mut entry = Entry::new();
        if let Page::Group(g) = &self.page {
            entry.group = g.clone();
        }
        entry.favorite = self.page == Page::Favorites;
        if matches!(self.page, Page::Generator | Page::Settings) {
            self.page = Page::All;
        }
        self.editor = Some(Editor::new(entry, true));
    }

    fn detail_area(&mut self, ui: &mut Ui, env: &mut Env, groups: &[String]) {
        let ctx = ui.ctx().clone();
        if let Some(editor) = &mut self.editor {
            match editor_ui(ui, editor, groups, &env.cfg.generator) {
                EditorAction::None => {}
                EditorAction::Cancel => self.editor = None,
                EditorAction::Save => self.save_editor(),
            }
            return;
        }
        let Some(entry) = self.selected.as_deref().and_then(|id| self.vault.get(id)) else {
            self.selected = None;
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(caption(ui, "Выберите запись слева или создайте новую."));
            });
            return;
        };
        match detail_ui(ui, entry, &mut self.reveal) {
            DetailAction::None => {}
            DetailAction::Copy(what, text) => self.copy(&ctx, env, &what, &text),
            DetailAction::Edit => self.editor = Some(Editor::new(entry.clone(), false)),
            DetailAction::Delete => self.confirm_delete = Some(entry.id.clone()),
            DetailAction::ToggleFavorite => {
                let mut e = entry.clone();
                e.favorite = !e.favorite;
                // Not a password change, so upsert keeps the history as is.
                self.vault.upsert(e);
                self.persist();
            }
            DetailAction::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
        }
    }

    fn save_editor(&mut self) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let trimmed = |s: &mut String| {
            let t = s.trim();
            if t.len() != s.len() {
                *s = t.to_owned();
            }
        };
        trimmed(&mut editor.draft.title);
        trimmed(&mut editor.draft.group);
        trimmed(&mut editor.draft.url);
        editor
            .draft
            .fields
            .retain(|f| !f.name.trim().is_empty() || !f.value.is_empty());
        for f in &mut editor.draft.fields {
            trimmed(&mut f.name);
            if f.name.is_empty() {
                f.name = "Поле".into();
            }
        }
        if editor.draft.title.is_empty() {
            editor.error = Some("Укажите название.".into());
            self.editor = Some(editor);
            return;
        }
        let id = editor.draft.id.clone();
        self.vault.upsert(editor.draft);
        self.selected = Some(id);
        self.reveal = Reveal::default();
        if self.persist() {
            self.notify("Сохранено", "", Severity::Success);
        }
    }

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_delete.clone() else {
            return;
        };
        let title = self
            .vault
            .get(&id)
            .map(|e| e.title.clone())
            .unwrap_or_default();
        let result = ContentDialog::new("delete", "Удалить запись?")
            .body(format!(
                "«{title}» будет удалена из хранилища вместе с историей паролей."
            ))
            .primary_button("Удалить")
            .close_button("Отмена")
            .show(ctx);
        match result {
            Some(DialogResult::Primary) => {
                self.confirm_delete = None;
                self.vault.remove(&id);
                if self.selected.as_deref() == Some(id.as_str()) {
                    self.selected = None;
                }
                if self.persist() {
                    self.notify("Запись удалена", "", Severity::Info);
                }
            }
            Some(_) => self.confirm_delete = None,
            None => {}
        }
    }

    fn generator_ui(&mut self, ui: &mut Ui, env: &mut Env) {
        egui::Frame::NONE
            .inner_margin(Margin::symmetric(20, 16))
            .show(ui, |ui| {
                let theme = FluentTheme::of(ui.ctx());
                ui.label(theme.typography.title.rich("Генератор паролей"));
                ui.add_space(8.0);
                match generator_page::ui(ui, &mut self.generator, &mut env.cfg.generator) {
                    GeneratorAction::None => {}
                    GeneratorAction::OptionsChanged => env.config_changed = true,
                    GeneratorAction::Copy(text) => {
                        let ctx = ui.ctx().clone();
                        self.copy(&ctx, env, "Пароль скопирован", &text);
                    }
                }
            });
    }

    fn settings_ui(&mut self, ui: &mut Ui, env: &mut Env) {
        egui::Frame::NONE
            .inner_margin(Margin::symmetric(20, 16))
            .show(ui, |ui| {
                let theme = FluentTheme::of(ui.ctx());
                ui.label(theme.typography.title.rich("Настройки"));
                ui.add_space(8.0);
                let action = settings_page::ui(ui, &mut self.settings, env, &self.file);
                match action {
                    SettingsAction::None => {}
                    SettingsAction::Error(message) => {
                        self.notify("Ошибка", message, Severity::Error);
                    }
                    SettingsAction::Import(entries, source) => {
                        let found = entries.len();
                        let (added, skipped) = self.vault.merge(entries);
                        if self.persist() {
                            let mut message = format!("Добавлено {added} из {found}");
                            if skipped > 0 {
                                message.push_str(&format!(", дубликатов пропущено: {skipped}"));
                            }
                            message.push('.');
                            if source == ImportSource::Csv {
                                message.push_str(" Удалите исходный CSV: он не зашифрован.");
                            }
                            self.notify("Импорт завершён", message, Severity::Success);
                        }
                    }
                    SettingsAction::ExportCsv(path) => {
                        match keechest_core::interop::export_csv_file(&self.vault, &path) {
                            Ok(()) => self.notify(
                                "Экспорт завершён",
                                format!(
                                    "Записей: {}. Файл не зашифрован, удалите его после переноса.",
                                    self.vault.entries.len()
                                ),
                                Severity::Warning,
                            ),
                            Err(e) => self.notify(
                                "Не удалось экспортировать",
                                error_text(&e),
                                Severity::Error,
                            ),
                        }
                    }
                    SettingsAction::Rename(name) => {
                        self.vault.name = name;
                        if self.persist() {
                            self.notify("Хранилище переименовано", "", Severity::Success);
                        }
                    }
                    SettingsAction::Rekeyed(new) => {
                        match self.file.replace_key(*new, &self.vault) {
                            Ok(()) => self.notify(
                                "Мастер-пароль изменён",
                                "Хранилище перешифровано новым ключом.",
                                Severity::Success,
                            ),
                            Err(e) => self.notify(
                                "Не удалось сменить пароль",
                                format!("{} Старый пароль по-прежнему действует.", error_text(&e)),
                                Severity::Error,
                            ),
                        }
                    }
                }
            });
    }
}

#[derive(Default)]
struct Reveal {
    password: bool,
    /// Indices of shown secret custom fields.
    fields: HashSet<usize>,
}

enum DetailAction {
    None,
    Copy(String, Zeroizing<String>),
    Edit,
    Delete,
    ToggleFavorite,
    OpenUrl(String),
}

fn field_label(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(caption(ui, text));
}

fn masked(len: usize) -> String {
    "•".repeat(len.clamp(8, 24))
}

fn detail_ui(ui: &mut Ui, entry: &Entry, reveal: &mut Reveal) -> DetailAction {
    let theme = FluentTheme::of(ui.ctx());
    let mut action = DetailAction::None;
    ScrollArea::vertical()
        .id_salt("detail")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme.typography.subtitle.rich(entry.title.as_str()));
                if entry.favorite {
                    ui.label(RichText::new("★").size(18.0));
                }
            });
            if !entry.group.is_empty() {
                ui.label(caption(ui, format!("Группа: {}", entry.group)));
            }

            if !entry.username.is_empty() {
                field_label(ui, "Логин");
                ui.horizontal(|ui| {
                    ui.label(entry.username.as_str());
                    if ui
                        .add(Button::new("Копировать").min_size(vec2(0.0, 26.0)))
                        .clicked()
                    {
                        action = DetailAction::Copy(
                            "Логин скопирован".into(),
                            Zeroizing::new(entry.username.clone()),
                        );
                    }
                });
            }

            field_label(ui, "Пароль");
            ui.horizontal(|ui| {
                if entry.password.is_empty() {
                    ui.label(caption(ui, "не задан"));
                    return;
                }
                if reveal.password {
                    ui.label(
                        RichText::new(entry.password.as_str())
                            .monospace()
                            .size(15.0),
                    );
                } else {
                    ui.label(RichText::new(masked(entry.password.chars().count())).size(15.0));
                }
                let toggle = if reveal.password {
                    "Скрыть"
                } else {
                    "Показать"
                };
                if ui
                    .add(Button::new(toggle).min_size(vec2(0.0, 26.0)))
                    .clicked()
                {
                    reveal.password = !reveal.password;
                }
                if ui
                    .add(Button::new("Копировать").min_size(vec2(0.0, 26.0)))
                    .clicked()
                {
                    action = DetailAction::Copy(
                        "Пароль скопирован".into(),
                        Zeroizing::new(entry.password.clone()),
                    );
                }
            });

            if !entry.url.is_empty() {
                field_label(ui, "Адрес");
                ui.horizontal(|ui| {
                    let url = entry.url.as_str();
                    let lower = url.to_ascii_lowercase();
                    if lower.starts_with("https://") || lower.starts_with("http://") {
                        if ui.add(Button::hyperlink(url)).clicked() {
                            action = DetailAction::OpenUrl(url.to_owned());
                        }
                    } else {
                        ui.label(url);
                    }
                    if ui
                        .add(Button::new("Копировать").min_size(vec2(0.0, 26.0)))
                        .clicked()
                    {
                        action = DetailAction::Copy(
                            "Адрес скопирован".into(),
                            Zeroizing::new(url.to_owned()),
                        );
                    }
                });
            }

            for (i, f) in entry.fields.iter().enumerate() {
                if f.value.is_empty() {
                    continue;
                }
                field_label(
                    ui,
                    if f.name.is_empty() {
                        "Поле"
                    } else {
                        &f.name
                    },
                );
                ui.push_id(("field", i), |ui| {
                    ui.horizontal(|ui| {
                        let shown = !f.secret || reveal.fields.contains(&i);
                        if shown {
                            let text = RichText::new(f.value.as_str());
                            ui.add(
                                egui::Label::new(if f.secret { text.monospace() } else { text })
                                    .wrap(),
                            );
                        } else {
                            ui.label(RichText::new(masked(f.value.chars().count())).size(15.0));
                        }
                        if f.secret {
                            let toggle = if shown {
                                "Скрыть"
                            } else {
                                "Показать"
                            };
                            if ui
                                .add(Button::new(toggle).min_size(vec2(0.0, 26.0)))
                                .clicked()
                                && !reveal.fields.remove(&i)
                            {
                                reveal.fields.insert(i);
                            }
                        }
                        if ui
                            .add(Button::new("Копировать").min_size(vec2(0.0, 26.0)))
                            .clicked()
                        {
                            action = DetailAction::Copy(
                                format!("«{}» скопировано", f.name),
                                Zeroizing::new(f.value.clone()),
                            );
                        }
                    });
                });
            }

            if !entry.notes.is_empty() {
                field_label(ui, "Заметки");
                ui.add(egui::Label::new(entry.notes.as_str()).wrap());
            }

            if !entry.history.is_empty() {
                ui.add_space(8.0);
                Expander::new(
                    ("history", &entry.id),
                    format!("Предыдущие пароли ({})", entry.history.len()),
                )
                .show(ui, |ui| {
                    for (i, h) in entry.history.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(caption(ui, format_time(h.replaced)));
                            if reveal.password {
                                ui.label(RichText::new(h.password.as_str()).monospace());
                            } else {
                                ui.label(masked(h.password.chars().count()));
                            }
                            if ui
                                .push_id(i, |ui| {
                                    ui.add(Button::new("Копировать").min_size(vec2(0.0, 24.0)))
                                })
                                .inner
                                .clicked()
                            {
                                action = DetailAction::Copy(
                                    "Старый пароль скопирован".into(),
                                    Zeroizing::new(h.password.clone()),
                                );
                            }
                        });
                    }
                });
            }

            ui.add_space(12.0);
            ui.label(caption(
                ui,
                format!(
                    "Создано {} · изменено {}",
                    format_time(entry.created),
                    format_time(entry.modified)
                ),
            ));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(Button::filled("Изменить")).clicked() {
                    action = DetailAction::Edit;
                }
                let fav = if entry.favorite {
                    "Убрать из избранного"
                } else {
                    "В избранное"
                };
                if ui.add(Button::new(fav)).clicked() {
                    action = DetailAction::ToggleFavorite;
                }
                if ui.add(Button::new("Удалить")).clicked() {
                    action = DetailAction::Delete;
                }
            });
        });
    action
}

struct Editor {
    draft: Entry,
    is_new: bool,
    show_password: bool,
    strength: Strength,
    error: Option<String>,
    focus: bool,
}

impl Editor {
    fn new(draft: Entry, is_new: bool) -> Self {
        let strength = strength::estimate(&draft.password, &[&draft.title, &draft.username]);
        Self {
            draft,
            is_new,
            show_password: false,
            strength,
            error: None,
            focus: true,
        }
    }

    fn update_strength(&mut self) {
        self.strength = strength::estimate(
            &self.draft.password,
            &[&self.draft.title, &self.draft.username],
        );
    }
}

enum EditorAction {
    None,
    Save,
    Cancel,
}

fn editor_ui(
    ui: &mut Ui,
    ed: &mut Editor,
    groups: &[String],
    generator: &GeneratorOptions,
) -> EditorAction {
    let theme = FluentTheme::of(ui.ctx());
    let mut action = EditorAction::None;
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        return EditorAction::Cancel;
    }
    ScrollArea::vertical()
        .id_salt("editor")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            let width = ui.available_width().min(460.0);
            let heading = if ed.is_new {
                "Новая запись"
            } else {
                "Изменение записи"
            };
            ui.label(theme.typography.subtitle.rich(heading));

            field_label(ui, "Название");
            let title = ui.add(
                TextBox::new(&mut ed.draft.title)
                    .placeholder("Например, Почта")
                    .width(width)
                    .id_salt("ed_title"),
            );
            if std::mem::take(&mut ed.focus) {
                title.request_focus();
            }

            field_label(ui, "Логин");
            let user = ui.add(
                TextBox::new(&mut ed.draft.username)
                    .placeholder("Имя пользователя или почта")
                    .width(width)
                    .id_salt("ed_user"),
            );
            if title.changed() || user.changed() {
                ed.update_strength();
            }

            field_label(ui, "Пароль");
            let mode = if ed.show_password {
                PasswordRevealMode::Visible
            } else {
                PasswordRevealMode::Peek
            };
            let pass = ui.add(
                PasswordBox::new(&mut ed.draft.password)
                    .reveal_mode(mode)
                    .width(width)
                    .id_salt("ed_pass"),
            );
            if pass.changed() {
                ed.update_strength();
            }
            ui.horizontal(|ui| {
                if ui.add(Button::new("Сгенерировать")).clicked()
                    && let Some(p) = generator.generate()
                {
                    ed.draft.password.zeroize();
                    ed.draft.password.push_str(&p);
                    ed.show_password = true;
                    ed.update_strength();
                }
                let toggle = if ed.show_password {
                    "Скрыть"
                } else {
                    "Показать"
                };
                if ui.add(Button::new(toggle)).clicked() {
                    ed.show_password = !ed.show_password;
                }
            });
            ui.add_space(4.0);
            strength_meter(ui, ed.strength, width);

            field_label(ui, "Адрес");
            ui.add(
                TextBox::new(&mut ed.draft.url)
                    .placeholder("https://")
                    .width(width)
                    .id_salt("ed_url"),
            );

            field_label(ui, "Группа");
            AutoSuggestBox::new("ed_group", &mut ed.draft.group, groups)
                .placeholder("Без группы")
                .width(width)
                .show(ui);

            fields_editor(ui, &mut ed.draft.fields, width);

            field_label(ui, "Заметки");
            ui.add(
                TextBox::multiline(&mut ed.draft.notes, 4)
                    .width(width)
                    .id_salt("ed_notes"),
            );
            ui.add_space(6.0);
            ui.add(Checkbox::new(&mut ed.draft.favorite, "Избранное"));

            if let Some(error) = &ed.error {
                ui.add_space(6.0);
                InfoBar::new(error.as_str())
                    .severity(Severity::Error)
                    .show(ui);
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(Button::filled("Сохранить")).clicked() {
                    action = EditorAction::Save;
                }
                if ui.add(Button::new("Отмена")).clicked() {
                    action = EditorAction::Cancel;
                }
            });
            if ui.input_mut(|i| {
                i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S))
            }) {
                action = EditorAction::Save;
            }
        });
    action
}

/// Rows of name, value and a "secret" switch for the custom fields.
fn fields_editor(ui: &mut Ui, fields: &mut Vec<CustomField>, width: f32) {
    field_label(ui, "Дополнительные поля");
    let mut remove = None;
    // Wider than the other fields: the row also holds the switch and button.
    let width = ui.available_width().min(width + 180.0);
    let name_width = (width * 0.3).round();
    let controls = 130.0 + 3.0 * ui.spacing().item_spacing.x;
    for (i, f) in fields.iter_mut().enumerate() {
        ui.push_id(("ed_field", i), |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    TextBox::new(&mut f.name)
                        .placeholder("Название")
                        .width(name_width)
                        .id_salt("name"),
                );
                let value_width = (width - name_width - controls).max(80.0);
                if f.secret {
                    ui.add(
                        PasswordBox::new(&mut f.value)
                            .placeholder("Значение")
                            .width(value_width)
                            .id_salt("value"),
                    );
                } else {
                    ui.add(
                        TextBox::new(&mut f.value)
                            .placeholder("Значение")
                            .width(value_width)
                            .id_salt("value"),
                    );
                }
                ui.add(Checkbox::new(&mut f.secret, "Скрытое"));
                if ui
                    .add(Button::new("✕").min_size(vec2(32.0, 30.0)))
                    .on_hover_text("Удалить поле")
                    .clicked()
                {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        fields.remove(i);
    }
    if ui.add(Button::new("+ Добавить поле")).clicked() {
        fields.push(CustomField::default());
    }
}
