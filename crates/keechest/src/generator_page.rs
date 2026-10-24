//! The password generator page.

use egui::{RichText, Ui, vec2};
use fluent_egui::{Button, Card, Checkbox, FluentTheme, Slider};
use keechest_core::generator::{MAX_LENGTH, MIN_LENGTH};
use keechest_core::{GeneratorOptions, Strength, Zeroizing};

use crate::widgets::{caption, section, strength_meter};

pub struct GeneratorState {
    password: Zeroizing<String>,
    strength: Strength,
}

pub enum GeneratorAction {
    None,
    OptionsChanged,
    Copy(Zeroizing<String>),
}

impl GeneratorState {
    pub fn new(opts: &GeneratorOptions) -> Self {
        let mut state = Self {
            password: Zeroizing::default(),
            strength: Strength { score: 0, bits: 0 },
        };
        state.regenerate(opts);
        state
    }

    fn regenerate(&mut self, opts: &GeneratorOptions) {
        self.password = opts.generate().unwrap_or_default();
        // zxcvbn can't tell a random password from any other unknown string
        // and underrates long ones, so rate by the generator's entropy.
        let bits = opts.entropy_bits();
        let score = match bits {
            b if b < 28.0 => 0,
            b if b < 40.0 => 1,
            b if b < 60.0 => 2,
            b if b < 80.0 => 3,
            _ => 4,
        };
        self.strength = Strength {
            score,
            bits: bits as u32,
        };
    }
}

pub fn ui(ui: &mut Ui, state: &mut GeneratorState, opts: &mut GeneratorOptions) -> GeneratorAction {
    let theme = FluentTheme::of(ui.ctx());
    let mut action = GeneratorAction::None;
    let width = ui.available_width().min(560.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            Card::new().padding(20).show(ui, |ui| {
                ui.set_width(width - 40.0);
                if state.password.is_empty() {
                    ui.label(caption(ui, "Выберите хотя бы один набор символов."));
                } else {
                    ui.add(
                        egui::Label::new(
                            RichText::new(state.password.as_str())
                                .monospace()
                                .size(20.0)
                                .color(theme.resources.text_fill_color_primary),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(10.0);
                strength_meter(ui, state.strength, width - 40.0);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let copy = Button::filled("Копировать").min_size(vec2(120.0, 32.0));
                    if ui.add_enabled(!state.password.is_empty(), copy).clicked() {
                        action = GeneratorAction::Copy(state.password.clone());
                    }
                    if ui.add(Button::new("Новый пароль")).clicked() {
                        state.regenerate(opts);
                    }
                });
            });

            let before = *opts;
            section(ui, &format!("Длина: {}", opts.length));
            let mut length = opts.length as f32;
            ui.add(
                Slider::new(&mut length, MIN_LENGTH as f32..=MAX_LENGTH as f32)
                    .step(1.0)
                    .width(width.min(400.0)),
            );
            opts.length = length as usize;

            section(ui, "Символы");
            ui.add(Checkbox::new(&mut opts.lowercase, "Строчные буквы (a–z)"));
            ui.add(Checkbox::new(&mut opts.uppercase, "Прописные буквы (A–Z)"));
            ui.add(Checkbox::new(&mut opts.digits, "Цифры (0–9)"));
            ui.add(Checkbox::new(&mut opts.symbols, "Спецсимволы (!#$%…)"));
            ui.add(Checkbox::new(
                &mut opts.exclude_ambiguous,
                "Без похожих символов (I, l, 1, O, 0)",
            ));
            if *opts != before {
                state.regenerate(opts);
                action = GeneratorAction::OptionsChanged;
            }
            ui.add_space(12.0);
            ui.label(caption(
                ui,
                "Эти же настройки использует кнопка «Сгенерировать» в редакторе записи.",
            ));
        });
    action
}
