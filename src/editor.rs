use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use nih_plug::prelude::{Param, ParamSetter};
use nih_plug_egui::egui::{self, Color32, RichText, Stroke, Vec2};
use nih_plug_egui::resizable_window::ResizableWindow;
use nih_plug_egui::widgets;
use nih_plug_egui::EguiState;

use crate::names::{display_name, note_name, SAMPLE_NAMES};
use crate::{BankSnapshot, SharedUi, UnderstoryParams};

const INK: Color32 = Color32::from_rgb(24, 32, 28);
const CREAM: Color32 = Color32::from_rgb(239, 231, 214);
const MUTED: Color32 = Color32::from_rgb(168, 176, 160);
const MOSS: Color32 = Color32::from_rgb(18, 28, 24);
const POLLEN: Color32 = Color32::from_rgb(224, 186, 122);
const WARN: Color32 = Color32::from_rgb(214, 148, 110);

const KEYMAP: &[(egui::Key, u8)] = &[
    (egui::Key::Z, 48),
    (egui::Key::S, 49),
    (egui::Key::X, 50),
    (egui::Key::D, 51),
    (egui::Key::C, 52),
    (egui::Key::V, 53),
    (egui::Key::G, 54),
    (egui::Key::B, 55),
    (egui::Key::H, 56),
    (egui::Key::N, 57),
    (egui::Key::J, 58),
    (egui::Key::M, 59),
    (egui::Key::Q, 60),
    (egui::Key::Num2, 61),
    (egui::Key::W, 62),
    (egui::Key::Num3, 63),
    (egui::Key::E, 64),
    (egui::Key::R, 65),
    (egui::Key::Num5, 66),
    (egui::Key::T, 67),
    (egui::Key::Num6, 68),
    (egui::Key::Y, 69),
    (egui::Key::Num7, 70),
    (egui::Key::U, 71),
    (egui::Key::I, 72),
];

#[derive(Clone, Copy)]
pub enum UiMidi {
    On { note: u8, velocity: f32 },
    Off { note: u8 },
}

pub struct KbState {
    mouse: Option<u8>,
    keys: Vec<u8>,
    tx: Arc<Mutex<rtrb::Producer<UiMidi>>>,
}

impl KbState {
    pub fn new(tx: Arc<Mutex<rtrb::Producer<UiMidi>>>) -> Self {
        Self {
            mouse: None,
            keys: Vec::new(),
            tx,
        }
    }

    fn send(&self, event: UiMidi) {
        if let Ok(mut tx) = self.tx.lock() {
            let _ = tx.push(event);
        }
    }

    fn is_down(&self, note: u8) -> bool {
        self.mouse == Some(note) || self.keys.contains(&note)
    }
}

impl Drop for KbState {
    fn drop(&mut self) {
        let keys = std::mem::take(&mut self.keys);
        let mouse = self.mouse.take();
        for note in keys {
            self.send(UiMidi::Off { note });
        }
        if let Some(note) = mouse {
            self.send(UiMidi::Off { note });
        }
    }
}

pub fn draw(
    ctx: &egui::Context,
    setter: &ParamSetter,
    kb: &mut KbState,
    params: &UnderstoryParams,
    shared: &SharedUi,
    egui_state: &EguiState,
) {
    theme(ctx);
    ResizableWindow::new("understory")
        .min_size(Vec2::new(760.0, 560.0))
        .show(ctx, egui_state, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_min_width(720.0);
                header(ui, params, shared);
                ui.add_space(10.0);
                recordings(ui, setter, params, shared);
                ui.add_space(12.0);
                keyboard(ui, kb, params.root.value());
                ui.add_space(12.0);
                shaping(ui, setter, params, shared);
            });
        });
}

pub(crate) fn theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = MOSS;
    visuals.panel_fill = MOSS;
    visuals.extreme_bg_color = Color32::from_rgb(12, 18, 16);
    visuals.faint_bg_color = Color32::from_rgb(32, 46, 40);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, CREAM);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(36, 52, 45);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(52, 74, 62);
    visuals.widgets.active.bg_fill = Color32::from_rgb(78, 104, 84);
    visuals.selection.bg_fill = POLLEN;
    visuals.selection.stroke = Stroke::new(1.0_f32, INK);
    ctx.set_visuals(visuals);
}

fn header(ui: &mut egui::Ui, params: &UnderstoryParams, shared: &SharedUi) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Understory").size(30.0).color(CREAM).strong());
        ui.add_space(8.0);
        ui.label(
            RichText::new("random slices, held for the length of the note")
                .size(13.0)
                .color(MUTED)
                .italics(),
        );
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "Each note chooses one recording and a moment inside it. Pitch follows the key. Time follows the tempo.",
        )
        .size(13.0)
        .color(CREAM),
    );

    let root = note_name(params.root.value());
    let host = shared.host_bpm_x100.load(Ordering::Relaxed);
    let manual = params.tempo.value();
    let tempo_line = if params.sync.value() {
        if host > 0 {
            format!("{:.1} BPM from the DAW", host as f32 / 100.0)
        } else {
            format!("{manual:.1} BPM · the host has not sent a tempo")
        }
    } else {
        format!("{manual:.1} BPM · host tempo ignored")
    };
    ui.add_space(6.0);
    ui.label(
        RichText::new(format!("{root} is the original pitch  ·  {tempo_line}"))
            .size(13.0)
            .color(POLLEN),
    );

    let last = shared.last_index.load(Ordering::Relaxed);
    let voices = shared.voices.load(Ordering::Relaxed);
    let ppm = shared.last_ppm.load(Ordering::Relaxed);
    let snap = snapshot(shared);
    let status = if last == u32::MAX {
        "Play a note. Understory will pick a slice.".to_string()
    } else {
        let name = snap
            .infos
            .get(last as usize)
            .map(|info| display_name(&info.name))
            .unwrap_or_else(|| "a recording".to_string());
        let notes = if voices == 1 {
            "1 note sounding".to_string()
        } else {
            format!("{voices} notes sounding")
        };
        format!("{name} entered at {}% · {notes}", ppm / 10)
    };
    ui.label(RichText::new(status).size(13.0).color(CREAM));

    let peak = f32::from_bits(shared.peak_bits.load(Ordering::Relaxed)).clamp(0.0, 1.0);
    let meter = if peak < 1.0e-4 {
        "quiet".to_string()
    } else {
        format!("{:.1} dB", 20.0 * peak.max(1.0e-6).log10())
    };
    ui.add(
        egui::ProgressBar::new(peak)
            .desired_width(280.0)
            .text(meter),
    );
}

fn recordings(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &UnderstoryParams,
    shared: &SharedUi,
) {
    ui.label(RichText::new("Recordings").size(16.0).color(CREAM).strong());
    let snap = snapshot(shared);
    let loaded = SAMPLE_NAMES
        .iter()
        .filter(|name| snap.infos.iter().any(|info| info.name == **name))
        .count();
    if loaded == 0 {
        ui.label(
            RichText::new(
                "Nothing is loaded. Put bug, dark, dragon, electric, fairy, fighting, fire, flying, ghost, grass, ground, ice, normal, poison, psychic, rock, steel, and water into the samples folder as wav or mp3, then rebuild. Notes stay silent until those loops are part of the plugin.",
            )
            .color(WARN),
        );
    } else if loaded == SAMPLE_NAMES.len() {
        ui.label(
            RichText::new("All 18 recordings are in the draw. Click one to leave it out.")
                .color(MUTED),
        );
    } else {
        ui.label(
            RichText::new(format!(
                "{loaded} of {} recordings are in this build. The rest stay silent.",
                SAMPLE_NAMES.len()
            ))
            .color(WARN),
        );
    }
    for error in &snap.errors {
        ui.label(RichText::new(error).color(WARN).size(12.0));
    }

    let mask = shared.active_mask.load(Ordering::Relaxed);
    ui.add_space(6.0);
    egui::Grid::new("recordings")
        .num_columns(6)
        .spacing(egui::vec2(8.0, 8.0))
        .show(ui, |ui| {
            for (index, name) in SAMPLE_NAMES.iter().enumerate() {
                let info = snap.infos.iter().find(|info| info.name == *name);
                let bank_index = snap.infos.iter().position(|info| info.name == *name);
                let playing = bank_index
                    .map(|slot| slot < 32 && mask & (1 << slot) != 0)
                    .unwrap_or(false);
                let enabled = params.is_enabled(name);
                tile(
                    ui,
                    setter,
                    params,
                    name,
                    info.map(|info| info.seconds),
                    enabled,
                    playing,
                    info.is_some(),
                );
                if index % 6 == 5 {
                    ui.end_row();
                }
            }
        });
}

fn tile(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &UnderstoryParams,
    name: &str,
    seconds: Option<f32>,
    enabled: bool,
    playing: bool,
    loaded: bool,
) {
    let fill = if !loaded {
        Color32::from_rgb(28, 34, 32)
    } else if playing {
        lighten(tile_color(name), 28)
    } else if enabled {
        tile_color(name)
    } else {
        Color32::from_rgb(32, 40, 36)
    };
    let status = if !loaded {
        "not loaded".to_string()
    } else if playing {
        "sounding".to_string()
    } else if enabled {
        match seconds {
            Some(seconds) => format!("{seconds:.1}s"),
            None => "in the draw".to_string(),
        }
    } else {
        "muted".to_string()
    };
    let text_color = if enabled && loaded { CREAM } else { MUTED };
    let mut button = egui::Button::new(
        RichText::new(format!("{}\n{status}", display_name(name)))
            .size(14.0)
            .color(text_color),
    )
    .fill(fill)
    .min_size(egui::vec2(112.0, 58.0));
    if playing {
        button = button.stroke(Stroke::new(2.0_f32, POLLEN));
    }
    let response = ui.add(button);
    if loaded && response.clicked() {
        if let Some(param) = params.sample_toggle(name) {
            set_bool(setter, param, !enabled);
        }
    }
}

fn keyboard(ui: &mut egui::Ui, kb: &mut KbState, root: i32) {
    ui.label(RichText::new("Play").size(16.0).color(CREAM).strong());
    ui.label(
        RichText::new("Click a key, or type: the Z row starts at C3 and the Q row starts at C4.")
            .size(12.0)
            .color(MUTED),
    );

    let typing = ui.ctx().wants_keyboard_input();
    if typing {
        let held = std::mem::take(&mut kb.keys);
        for note in held {
            kb.send(UiMidi::Off { note });
        }
    } else {
        for (key, note) in KEYMAP {
            let down = ui.input(|input| input.key_down(*key));
            let held = kb.keys.contains(note);
            if down && !held {
                kb.keys.push(*note);
                kb.send(UiMidi::On {
                    note: *note,
                    velocity: 0.86,
                });
            } else if !down && held {
                kb.keys.retain(|held_note| held_note != note);
                kb.send(UiMidi::Off { note: *note });
            }
        }
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for note in 48u8..=72 {
            let held = kb.is_down(note);
            let is_root = note as i32 == root;
            let label = note_name(note as i32);
            let color = if held { INK } else { CREAM };
            let mut button = egui::Button::new(RichText::new(label).size(12.0).color(color))
                .min_size(egui::vec2(36.0, 28.0));
            if held {
                button = button.fill(POLLEN);
            } else if is_root {
                button = button.stroke(Stroke::new(1.5_f32, POLLEN));
            }
            let response = ui.add(button);
            let pressed = response.hovered() && ui.input(|input| input.pointer.primary_pressed());
            if pressed && kb.mouse != Some(note) {
                if let Some(previous) = kb.mouse.take() {
                    kb.send(UiMidi::Off { note: previous });
                }
                kb.mouse = Some(note);
                kb.send(UiMidi::On {
                    note,
                    velocity: 0.9,
                });
            }
        }
    });
    let pointer_held = ui.input(|input| input.pointer.any_down());
    if !pointer_held {
        if let Some(note) = kb.mouse.take() {
            kb.send(UiMidi::Off { note });
        }
    }
}

fn shaping(ui: &mut egui::Ui, setter: &ParamSetter, params: &UnderstoryParams, shared: &SharedUi) {
    ui.label(RichText::new("Shape").size(16.0).color(CREAM).strong());
    ui.label(
        RichText::new(
            "The recordings are in C at 120 BPM. Original pitch is the note that leaves them there. Sample BPM is the tempo they were played at.",
        )
        .size(12.0)
        .color(MUTED),
    );
    egui::Grid::new("shape")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 8.0))
        .show(ui, |ui| {
            knob(ui, setter, "Gain", &params.gain);
            knob(ui, setter, "Attack", &params.attack);
            knob(ui, setter, "Release", &params.release);
            knob(ui, setter, "Original pitch", &params.root);
            knob(ui, setter, "Sample BPM", &params.sample_bpm);
            knob(ui, setter, "Tempo", &params.tempo);
        });

    let mut follow = params.sync.value();
    if ui.checkbox(&mut follow, "Follow the host tempo").changed() {
        set_bool(setter, &params.sync, follow);
    }
    let host = shared.host_bpm_x100.load(Ordering::Relaxed);
    let follow_copy = if follow && host > 0 {
        "The session tempo is moving the slices. Turn this off to use the Tempo knob."
    } else if follow {
        "Waiting for the DAW. Until a tempo arrives, the Tempo knob is used."
    } else {
        "The Tempo knob is in charge, even when the DAW is playing."
    };
    ui.label(RichText::new(follow_copy).size(12.0).color(MUTED));
}

fn knob(ui: &mut egui::Ui, setter: &ParamSetter, label: &str, param: &impl Param) {
    ui.label(RichText::new(label).color(CREAM));
    ui.add(widgets::ParamSlider::for_param(param, setter));
    ui.end_row();
}

fn snapshot(shared: &SharedUi) -> BankSnapshot {
    shared
        .snapshot
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

fn set_bool(setter: &ParamSetter, param: &impl Param<Plain = bool>, value: bool) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

fn tile_color(name: &str) -> Color32 {
    match name {
        "bug" => Color32::from_rgb(92, 122, 64),
        "dark" => Color32::from_rgb(62, 66, 88),
        "dragon" => Color32::from_rgb(140, 84, 62),
        "electric" => Color32::from_rgb(168, 146, 64),
        "fairy" => Color32::from_rgb(156, 112, 138),
        "fighting" => Color32::from_rgb(148, 78, 66),
        "fire" => Color32::from_rgb(164, 96, 52),
        "flying" => Color32::from_rgb(96, 132, 142),
        "ghost" => Color32::from_rgb(112, 102, 146),
        "grass" => Color32::from_rgb(70, 122, 78),
        "ground" => Color32::from_rgb(132, 104, 68),
        "ice" => Color32::from_rgb(92, 142, 148),
        "normal" => Color32::from_rgb(128, 118, 96),
        "poison" => Color32::from_rgb(118, 86, 140),
        "psychic" => Color32::from_rgb(160, 96, 124),
        "rock" => Color32::from_rgb(120, 110, 90),
        "steel" => Color32::from_rgb(108, 124, 128),
        "water" => Color32::from_rgb(64, 112, 140),
        _ => Color32::from_rgb(70, 96, 84),
    }
}

fn lighten(color: Color32, amount: u8) -> Color32 {
    Color32::from_rgb(
        color.r().saturating_add(amount),
        color.g().saturating_add(amount),
        color.b().saturating_add(amount),
    )
}
