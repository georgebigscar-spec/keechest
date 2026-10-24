//! Small pieces of UI and text shared by the screens.

use egui::{RichText, Ui};
use fluent_egui::{ComboBox, FluentTheme, ProgressBar, combo_value};
use keechest_core::{Error, KdfParams, Strength};

/// A colored bar and a caption for a password's strength.
pub fn strength_meter(ui: &mut Ui, strength: Strength, width: f32) {
    if strength.bits == 0 {
        // Nothing typed yet: an empty track, and a blank line to keep the
        // layout still.
        ui.add(ProgressBar::new(0.0).width(width));
        ui.label(caption(ui, " "));
        return;
    }
    let theme = FluentTheme::of(ui.ctx());
    let r = &theme.resources;
    let (color, label) = match strength.score {
        0 => (r.system_fill_color_critical, "Очень слабый"),
        1 => (r.system_fill_color_critical, "Слабый"),
        2 => (r.system_fill_color_caution, "Средний"),
        3 => (r.system_fill_color_success, "Хороший"),
        _ => (r.system_fill_color_success, "Надёжный"),
    };
    let value = (f32::from(strength.score) + 1.0) * 20.0;
    ui.add(ProgressBar::new(value).width(width).active_color(color));
    ui.label(caption(
        ui,
        format!("{label} · примерно {} бит", strength.bits),
    ));
}

pub fn caption(ui: &Ui, text: impl Into<String>) -> RichText {
    let theme = FluentTheme::of(ui.ctx());
    theme
        .typography
        .caption
        .rich(text.into())
        .color(theme.resources.text_fill_color_secondary)
}

pub fn section(ui: &mut Ui, text: &str) {
    let theme = FluentTheme::of(ui.ctx());
    ui.add_space(16.0);
    ui.label(theme.typography.body_strong.rich(text));
    ui.add_space(4.0);
}

/// Argon2id cost presets offered when creating a vault or changing its
/// password.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KdfPreset {
    Fast,
    Standard,
    Strong,
}

impl KdfPreset {
    pub const ALL: [Self; 3] = [Self::Fast, Self::Standard, Self::Strong];

    pub fn params(self) -> KdfParams {
        let (mib, iterations) = match self {
            Self::Fast => (64, 3),
            Self::Standard => (256, 3),
            Self::Strong => (1024, 4),
        };
        KdfParams {
            memory_kib: mib * 1024,
            iterations,
            parallelism: 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Быстрая: 64 МиБ, для слабых компьютеров",
            Self::Standard => "Стандартная: 256 МиБ",
            Self::Strong => "Усиленная: 1 ГиБ",
        }
    }

    pub fn combo(ui: &mut Ui, id: &str, current: &mut Self) {
        ComboBox::new(id, current.label())
            .width(320.0)
            .show_ui(ui, |ui| {
                for preset in Self::ALL {
                    combo_value(ui, current, preset, preset.label());
                }
            });
    }
}

pub fn describe_kdf(kdf: KdfParams) -> String {
    format!(
        "Argon2id: {} МиБ, {} прох., {} потока · XChaCha20-Poly1305",
        kdf.memory_kib / 1024,
        kdf.iterations,
        kdf.parallelism
    )
}

pub fn error_text(error: &Error) -> String {
    match error {
        Error::Io(e) if e.kind() == std::io::ErrorKind::NotFound => "Файл не найден.".into(),
        Error::Io(e) => format!("Ошибка ввода-вывода: {e}"),
        Error::NotAVault => "Это не файл хранилища KeeChest.".into(),
        Error::UnsupportedVersion(v) => {
            format!("Хранилище создано более новой версией KeeChest (формат {v}).")
        }
        Error::UnsupportedAlgorithm => "Неизвестный алгоритм шифрования в файле.".into(),
        Error::BadKdfParams => "Недопустимые параметры Argon2 в файле.".into(),
        Error::Kdf(e) => format!("Не удалось вычислить ключ: {e}"),
        Error::WrongPasswordOrCorrupted => "Неверный мастер-пароль или файл повреждён.".into(),
        Error::Corrupted(e) => format!("Содержимое хранилища повреждено: {e}"),
        Error::Encrypt => "Не удалось зашифровать хранилище.".into(),
    }
}

/// A Unix timestamp in local time, `dd.mm.yyyy hh:mm`.
pub fn format_time(secs: u64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(secs as i64, 0) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => {
            t.format("%d.%m.%Y %H:%M").to_string()
        }
        chrono::LocalResult::None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_valid() {
        for p in KdfPreset::ALL {
            p.params().validate().unwrap();
        }
    }
}
