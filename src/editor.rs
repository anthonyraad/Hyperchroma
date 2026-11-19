use std::sync::atomic::Ordering;
use std::sync::Arc;

use nih_plug::params::persist::PersistentField;
use nih_plug::prelude::{BoolParam, FloatParam, Param, ParamSetter};
use nih_plug_egui::egui::{self, Color32, Mesh, RichText, Stroke, StrokeKind, Vec2};
use nih_plug_egui::resizable_window::ResizableWindow;
use nih_plug_egui::widgets;
use nih_plug_egui::EguiState;

use crate::names::SAMPLE_NAMES;
use crate::{BankSnapshot, HyperchromaParams, SharedUi};

const INK: Color32 = Color32::from_rgb(24, 32, 28);
const CREAM: Color32 = Color32::from_rgb(239, 231, 214);
const STONE: Color32 = Color32::from_rgb(120, 112, 100);
const PAPER: Color32 = Color32::from_rgb(245, 240, 230);
const POLLEN: Color32 = Color32::from_rgb(224, 186, 122);
const LAVENDER: Color32 = Color32::from_rgb(186, 168, 214);
const OLIVE: Color32 = Color32::from_rgb(188, 194, 142);
const ASH: Color32 = Color32::from_rgb(174, 172, 166);
const WARN: Color32 = Color32::from_rgb(214, 148, 110);

/// Sound boxes are 15% narrower than the 117.6px boxes the 670px window used.
/// Their height stays on that proportion, so the left column does not reflow
/// when the controls fold out.
const TILE_W_BEFORE: f32 = (636.0 - 12.0 * 4.0) / 5.0;
const TILE_W: f32 = TILE_W_BEFORE * 0.85;
const TILE_H: f32 = TILE_W_BEFORE * 0.82;
const TILE_GAP: f32 = 12.0;
const GRID_W: f32 = TILE_W * 6.0 + TILE_GAP * 5.0;
/// The knob column, 15% narrower than the 522px column of the 1220px window.
const CONTROLS_W: f32 = 522.0 * 0.85;
const COLUMN_GAP: f32 = 28.0;
const MARGIN_L: i8 = 18;
const MARGIN_R: i8 = 16;
const MARGIN_T: i8 = 6;
const MARGIN_B: i8 = 6;

pub const WINDOW_COLLAPSED: (u32, u32) = (694, 510);
const WINDOW_EXPANDED: (u32, u32) = (1166, 510);

pub struct KbState {
    settings_open: bool,
    /// The right-hand controls and the settings mark. The editor opens folded.
    controls_open: bool,
    /// Window size to restore the next time the controls fold out.
    expanded_size: (u32, u32),
    /// Applied on the next frame, before layout, so the host and the grid agree.
    pending_resize: Option<(u32, u32)>,
    /// Host size has been matched to `controls_open` once for this editor session.
    fold_synced: bool,
    /// Index into `SAMPLE_NAMES` whose knobs are open. `None` until a box is clicked.
    /// A click on empty paper clears it.
    selected: Option<usize>,
    /// Keytracking edit the host has not applied yet.
    keytrack_edit: Option<PendingBool>,
    /// Stereoentropic edit the host has not applied yet.
    stereoentropic_edit: Option<PendingBool>,
    /// Follow-host-tempo edit the host has not applied yet.
    sync_edit: Option<PendingBool>,
    /// Duck-rate edits the host has not applied yet.
    duck_quarter_edit: Option<PendingBool>,
    duck_half_edit: Option<PendingBool>,
    duck_whole_edit: Option<PendingBool>,
}

impl KbState {
    pub fn new() -> Self {
        Self {
            settings_open: false,
            controls_open: false,
            expanded_size: WINDOW_EXPANDED,
            pending_resize: None,
            fold_synced: false,
            selected: None,
            keytrack_edit: None,
            stereoentropic_edit: None,
            sync_edit: None,
            duck_quarter_edit: None,
            duck_half_edit: None,
            duck_whole_edit: None,
        }
    }
}

pub fn draw(
    ctx: &egui::Context,
    setter: &ParamSetter,
    kb: &mut KbState,
    params: &HyperchromaParams,
    shared: &SharedUi,
    egui_state: &Arc<EguiState>,
) {
    theme(ctx);
    // The host restores the last window size, but KbState always starts folded.
    // Match once on open so a wide project editor shows its knobs. Do not keep
    // re-reading size after that, or a collapse still looks wide for a frame and
    // the next expand click is eaten as another collapse.
    if !kb.fold_synced {
        kb.fold_synced = true;
        let size = egui_state.size();
        if size.0 > WINDOW_COLLAPSED.0 + 40 {
            kb.controls_open = true;
            kb.expanded_size = (
                size.0.max(WINDOW_EXPANDED.0),
                size.1.max(WINDOW_EXPANDED.1),
            );
        }
    }
    if let Some(size) = kb.pending_resize.take() {
        request_window_size(egui_state, setter, ctx, size);
    }
    let min_size = if kb.controls_open {
        Vec2::new(WINDOW_EXPANDED.0 as f32, WINDOW_EXPANDED.1 as f32)
    } else {
        Vec2::new(WINDOW_COLLAPSED.0 as f32, WINDOW_COLLAPSED.1 as f32)
    };
    ResizableWindow::new("hyperchroma")
        .min_size(min_size)
        .show(ctx, egui_state, |ui| {
            let empty = ui.interact(ui.max_rect(), ui.id().with("empty"), egui::Sense::click());
            let snap = snapshot(shared);
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: MARGIN_L,
                    right: MARGIN_R,
                    top: MARGIN_T,
                    bottom: MARGIN_B,
                })
                .show(ui, |ui| {
                    // Stay on the color grid until the window has actually grown.
                    // Drawing the knobs a frame early squeezes the tiles.
                    let shown = kb.controls_open
                        && ui.available_width()
                            >= GRID_W + COLUMN_GAP + CONTROLS_W - 8.0;
                    header(ui, kb, egui_state, shown);
                    ui.add_space(10.0);
                    recordings(ui, setter, kb, params, shared, &snap, shown);
                });
            if let Some(text) = ui.ctx().data(|data| {
                data.get_temp::<&'static str>(egui::Id::new("hyperchroma-knob-help"))
            }) {
                let bounds = ui.max_rect();
                ui.painter().text(
                    egui::pos2(bounds.left() + MARGIN_L as f32, bounds.bottom() - 4.0),
                    egui::Align2::LEFT_BOTTOM,
                    text,
                    egui::FontId::proportional(12.0),
                    STONE,
                );
            }
            ui.ctx().data_mut(|data| {
                data.remove::<&'static str>(egui::Id::new("hyperchroma-knob-help"));
            });
            if kb.controls_open && kb.settings_open {
                settings_panel(ui.ctx(), setter, kb, params, &snap);
            } else {
                // A click can close the panel before the host has taken the edit.
                settle_bool(setter, &params.keytrack, &mut kb.keytrack_edit, None);
                settle_bool(
                    setter,
                    &params.stereoentropic,
                    &mut kb.stereoentropic_edit,
                    None,
                );
                settle_bool(setter, &params.sync, &mut kb.sync_edit, None);
                settle_bool(setter, &params.duck_quarter, &mut kb.duck_quarter_edit, None);
                settle_bool(setter, &params.duck_half, &mut kb.duck_half_edit, None);
                settle_bool(setter, &params.duck_whole, &mut kb.duck_whole_edit, None);
            }
            if empty.clicked() {
                kb.selected = None;
            }
        });
}

/// Ask the host for a new editor size. `EguiState::set_requested_size` is private, so the
/// persisted size is updated and the host is asked to resize to whatever `size()` reports.
fn request_window_size(
    egui_state: &Arc<EguiState>,
    setter: &ParamSetter,
    ctx: &egui::Context,
    size: (u32, u32),
) {
    let Ok(owned) = Arc::try_unwrap(EguiState::from_size(size.0, size.1)) else {
        return;
    };
    PersistentField::set(egui_state, owned);
    let _ = setter.raw_context.request_resize();
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
        size.0 as f32,
        size.1 as f32,
    )));
    ctx.request_repaint();
}

pub(crate) fn theme(ctx: &egui::Context) {
    install_manrope(ctx);
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = PAPER;
    visuals.panel_fill = PAPER;
    visuals.extreme_bg_color = Color32::from_rgb(12, 18, 16);
    visuals.faint_bg_color = Color32::from_rgb(32, 46, 40);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, CREAM);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(36, 52, 45);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(52, 74, 62);
    visuals.widgets.active.bg_fill = Color32::from_rgb(78, 104, 84);
    visuals.selection.bg_fill = POLLEN;
    visuals.selection.stroke = Stroke::new(1.0_f32, INK);
    ctx.set_visuals(visuals);
    ctx.options_mut(|options| {
        options.input_options.max_double_click_delay = DOUBLE_PRESS;
    });
}

fn install_manrope(ctx: &egui::Context) {
    let id = egui::Id::new("hyperchroma-manrope");
    if ctx.data(|data| data.get_temp::<bool>(id).unwrap_or(false)) {
        return;
    }
    ctx.data_mut(|data| data.insert_temp(id, true));
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Manrope".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../fonts/Manrope-Medium.ttf"
        ))),
    );
    if let Some(proportional) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        proportional.insert(0, "Manrope".to_owned());
    }
    if let Some(monospace) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        monospace.insert(0, "Manrope".to_owned());
    }
    ctx.set_fonts(fonts);
}

fn header(ui: &mut egui::Ui, kb: &mut KbState, egui_state: &Arc<EguiState>, shown: bool) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 40.0), egui::Sense::hover());
    let title_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * 0.7, rect.height()));
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(title_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.label(
                RichText::new("Hyperchroma")
                    .size(30.0)
                    .color(INK)
                    .strong(),
            );
        },
    );
    // The marks stay on the grid's right edge, which is where `>>` sits while folded.
    let grid_right = (rect.left() + GRID_W).min(rect.right());
    let buttons = egui::Rect::from_min_max(
        egui::pos2(grid_right - 48.0, rect.top()),
        egui::pos2(grid_right, rect.bottom()),
    );
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(buttons)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            if fold_button(ui, !shown).clicked() {
                toggle_controls(kb, egui_state);
            }
        },
    );
    if shown {
        let gear = egui::Rect::from_min_max(
            egui::pos2(rect.right() - 28.0, rect.top()),
            egui::pos2(rect.right(), rect.bottom()),
        );
        ui.allocate_new_ui(
            egui::UiBuilder::new()
                .max_rect(gear)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                if settings_button(ui, kb.settings_open).clicked() {
                    kb.settings_open = !kb.settings_open;
                }
            },
        );
    }
}

fn toggle_controls(kb: &mut KbState, egui_state: &EguiState) {
    if kb.controls_open {
        let current = egui_state.size();
        if current.0 > WINDOW_COLLAPSED.0 + 40 {
            kb.expanded_size = current;
        }
        kb.controls_open = false;
        kb.settings_open = false;
        kb.pending_resize = Some(WINDOW_COLLAPSED);
    } else {
        kb.controls_open = true;
        kb.pending_resize = Some(kb.expanded_size);
    }
}

/// `>>` folds the controls out, `<<` folds them away.
/// The two carets are drawn separately so they can sit closer than the font's pair.
/// Same ink as the settings mark: stone at rest, ink when hovered or while the controls are out.
fn fold_button(ui: &mut egui::Ui, expand: bool) -> egui::Response {
    let mark = if expand { ">" } else { "<" };
    let font = egui::FontId::proportional(20.0);
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(mark.to_owned(), font.clone(), INK));
    let advance = galley.size().x;
    let step = advance * 0.64;
    let total = advance + step;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(total + 4.0, 28.0), egui::Sense::click());
    let color = mark_color(!expand, response.hovered());
    let painted = ui.fonts(|fonts| fonts.layout_no_wrap(mark.to_owned(), font, color));
    let left = rect.center().x - total * 0.5;
    let top = rect.center().y - painted.size().y * 0.5;
    ui.painter().galley(egui::pos2(left, top), painted.clone(), color);
    ui.painter()
        .galley(egui::pos2(left + step, top), painted, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn settings_button(ui: &mut egui::Ui, open: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::click());
    let color = mark_color(open, response.hovered());
    if open {
        paint_open_mark(ui.painter(), rect.center());
    }
    let galley = ui.fonts(|fonts| {
        fonts.layout_no_wrap("&".to_owned(), egui::FontId::proportional(20.0), color)
    });
    let pos = rect.center() - galley.size() * 0.5;
    ui.painter().galley(pos, galley, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn mark_color(open: bool, hovered: bool) -> Color32 {
    if open || hovered { INK } else { STONE }
}

fn paint_open_mark(painter: &egui::Painter, center: egui::Pos2) {
    painter.circle_filled(
        center,
        13.0,
        Color32::from_rgba_unmultiplied(224, 186, 122, 28),
    );
}

fn recordings(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    kb: &mut KbState,
    params: &HyperchromaParams,
    shared: &SharedUi,
    snap: &crate::BankSnapshot,
    shown: bool,
) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = if shown { COLUMN_GAP } else { 0.0 };
        ui.vertical(|ui| color_grid(ui, kb, shared, snap));
        if !shown {
            return;
        }
        ui.vertical(|ui| {
            ui.set_min_width(CONTROLS_W);
            global_knobs(ui, setter, params);
            if let Some(index) = kb.selected {
                let name = SAMPLE_NAMES[index];
                if let Some(slice) = params.slice(name) {
                    ui.add_space(18.0 * 0.85);
                    if sound_knobs(ui, setter, name, slice) {
                        if let Some(slot) = snap.infos.iter().position(|info| info.name == name) {
                            shared.preview_slot.store(slot as u32, Ordering::Release);
                        }
                    }
                }
            }
        });
    });
}

fn color_grid(
    ui: &mut egui::Ui,
    kb: &mut KbState,
    shared: &SharedUi,
    snap: &crate::BankSnapshot,
) {
    const COLS: usize = 6;
    const ROWS: usize = 4;
    let mask = shared.active_mask.load(Ordering::Relaxed);
    let b_mask = shared.b_side_mask.load(Ordering::Relaxed);
    if mask != 0 {
        ui.ctx().request_repaint();
    }
    let time = ui.input(|input| input.time);
    ui.set_width(GRID_W);
    ui.spacing_mut().item_spacing = egui::vec2(TILE_GAP, TILE_GAP);
    let tile_size = egui::vec2(TILE_W, TILE_H);
    for row in 0..ROWS {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = TILE_GAP;
            for col in 0..COLS {
                let name = SAMPLE_NAMES[row * COLS + col];
                let info = snap.infos.iter().find(|info| info.name == name);
                let bank_index = snap.infos.iter().position(|info| info.name == name);
                let playing = bank_index
                    .map(|slot| slot < 32 && mask & (1 << slot) != 0)
                    .unwrap_or(false);
                let side_b = bank_index
                    .map(|slot| slot < 32 && b_mask & (1 << slot) != 0)
                    .unwrap_or(false);
                let playhead = bank_index.and_then(|slot| playhead_of(shared, slot));
                let weather = bank_index
                    .filter(|_| playing)
                    .and_then(|slot| weather_of(shared, slot))
                    .unwrap_or(0.0);
                let wave = info.map(|info| {
                    if side_b {
                        match (info.wave_min_b.as_ref(), info.wave_max_b.as_ref()) {
                            (Some(mins), Some(maxs)) => (mins.as_ref(), maxs.as_ref()),
                            _ => (info.wave_min.as_ref(), info.wave_max.as_ref()),
                        }
                    } else {
                        (info.wave_min.as_ref(), info.wave_max.as_ref())
                    }
                });
                let index = row * COLS + col;
                let clicked = tile(
                    ui,
                    name,
                    tile_size,
                    playing,
                    info.is_some(),
                    kb.selected == Some(index),
                    wave,
                    playhead.filter(|_| playing),
                    weather,
                    time,
                    index as f32,
                    side_b,
                );
                if clicked {
                    kb.selected = if kb.selected == Some(index) {
                        None
                    } else {
                        Some(index)
                    };
                }
            }
        });
    }
}

fn tile(
    ui: &mut egui::Ui,
    name: &str,
    size: Vec2,
    playing: bool,
    loaded: bool,
    selected: bool,
    wave: Option<(&[f32], &[f32])>,
    playhead: Option<f32>,
    weather: f32,
    time: f64,
    seed: f32,
    side_b: bool,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let radius = 1.5;
    let base = tile_color(name);
    let fill = if !loaded {
        Color32::from_rgb(28, 34, 32)
    } else if playing {
        lighten(base, 16)
    } else if response.hovered() {
        lighten(base, 10)
    } else {
        base
    };

    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, radius, fill);
        if playing {
            if let (Some((mins, maxs)), Some(playhead)) = (wave, playhead) {
                let plot = rect.shrink2(egui::vec2(12.0, rect.height() * 0.16));
                let ink = if side_b { Color32::BLACK } else { Color32::WHITE };
                paint_live_wave(&painter, plot, mins, maxs, playhead, weather, time, seed, ink);
            }
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(1.5_f32, Color32::from_rgba_unmultiplied(255, 246, 228, 110)),
                StrokeKind::Inside,
            );
        }
        if selected {
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(2.0_f32, Color32::WHITE),
                StrokeKind::Inside,
            );
        }
    }

    let clicked = loaded && response.clicked();
    let _ = response.on_hover_cursor(if loaded {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::Default
    });
    clicked
}

fn settings_panel(
    ctx: &egui::Context,
    setter: &ParamSetter,
    kb: &mut KbState,
    params: &HyperchromaParams,
    snap: &crate::BankSnapshot,
) {
    egui::Window::new("hyperchroma-settings")
        // New id so the taller pre-trim size is not reused from egui memory.
        .id(egui::Id::new("hyperchroma-settings-shape"))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-(MARGIN_R as f32), 50.0))
        .auto_sized()
        .frame(
            egui::Frame::new()
                .fill(PAPER)
                .stroke(Stroke::new(1.0_f32, STONE))
                .corner_radius(12.0)
                .inner_margin(16)
                .shadow(egui::Shadow {
                    offset: [0, 6],
                    blur: 18,
                    spread: 0,
                    color: Color32::from_black_alpha(96),
                }),
        )
        .show(ctx, |ui| {
            ui.set_min_width(360.0);
            let visuals = ui.visuals_mut();
            visuals.override_text_color = Some(INK);
            for widget in [
                &mut visuals.widgets.noninteractive,
                &mut visuals.widgets.inactive,
                &mut visuals.widgets.hovered,
                &mut visuals.widgets.active,
            ] {
                widget.fg_stroke.color = INK;
            }
            visuals.widgets.inactive.bg_fill = Color32::from_rgb(232, 226, 214);
            visuals.widgets.hovered.bg_fill = Color32::from_rgb(222, 214, 200);
            visuals.widgets.active.bg_fill = Color32::from_rgb(210, 200, 184);
            visuals.extreme_bg_color = Color32::from_rgb(232, 226, 214);
            shaping(ui, setter, kb, params);
            let loaded = SAMPLE_NAMES
                .iter()
                .filter(|name| snap.infos.iter().any(|info| info.name == **name))
                .count();
            if loaded == 0 {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("No recordings are loaded. Notes stay silent.")
                        .color(WARN)
                        .size(12.0),
                );
            } else if loaded < SAMPLE_NAMES.len() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!(
                        "{loaded} of {} recordings are in this build.",
                        SAMPLE_NAMES.len()
                    ))
                    .color(WARN)
                    .size(12.0),
                );
            }
            for error in &snap.errors {
                ui.label(RichText::new(error).color(WARN).size(12.0));
            }
        });
}

fn playhead_of(shared: &SharedUi, slot: usize) -> Option<f32> {
    let ppm = shared.playhead_ppm.get(slot)?.load(Ordering::Relaxed);
    if ppm == u32::MAX {
        None
    } else {
        Some(ppm as f32 / 1_000_000.0)
    }
}

fn weather_of(shared: &SharedUi, slot: usize) -> Option<f32> {
    let ppm = shared.weather_ppm.get(slot)?.load(Ordering::Relaxed);
    Some((ppm as f32 / 1_000_000.0).clamp(0.0, 1.0))
}

/// A solid white picture of the recording, with a white line at the playhead.
/// `weather` stirs the silhouette when a global effect is on the note; with Drift it
/// grows or settles with the note instead of snapping.
fn paint_live_wave(
    painter: &egui::Painter,
    rect: egui::Rect,
    mins: &[f32],
    maxs: &[f32],
    playhead: f32,
    weather: f32,
    time: f64,
    seed: f32,
    ink: Color32,
) {
    let bins = mins.len().min(maxs.len());
    if bins < 4 || rect.width() < 8.0 || rect.height() < 8.0 {
        return;
    }
    let playhead = playhead.rem_euclid(1.0);
    let columns = (rect.width() / 2.2).round().clamp(48.0, 180.0) as usize;
    let mid_y = rect.center().y;
    let amp = rect.height() * 0.48;
    let weather = weather.clamp(0.0, 1.0);
    let weather = weather * weather * (3.0 - 2.0 * weather);
    let phase = time as f32 * (1.55 + weather * 1.35) + seed * 1.73;

    let mut mesh = Mesh::default();
    for col in 0..columns {
        let t = col as f32 / (columns - 1) as f32;
        let (min_v, max_v) = sample_peak(mins, maxs, t * bins as f32);
        let mut x = egui::lerp(rect.left()..=rect.right(), t);
        let mut y_top = mid_y - max_v.clamp(-1.0, 1.0) * amp;
        let mut y_bot = mid_y - min_v.clamp(-1.0, 1.0) * amp;
        if y_bot - y_top < 1.5 {
            let mid = 0.5 * (y_top + y_bot);
            y_top = mid - 0.75;
            y_bot = mid + 0.75;
        }
        if weather > 1.0e-4 {
            let edge = (t * (1.0 - t) * 4.0).clamp(0.0, 1.0);
            let swell = (phase * 0.62 + t * 6.4).sin() * 0.55
                + (phase * 1.18 + t * 14.0 + seed).sin() * 0.30
                + (phase * 2.35 + t * 27.0).sin() * 0.15;
            let shimmer = (phase * 3.4 + t * 39.0 + seed * 0.7).sin();
            let lift = (swell * 0.78 + shimmer * 0.22) * amp * (0.045 + 0.11 * weather) * edge;
            let side = (phase * 0.95 + t * 9.5 + seed * 1.1).sin() * (0.35 + 1.1 * weather) * edge;
            let breathe = 1.0 + (phase * 0.48 + seed).sin() * 0.035 * weather;
            let center = 0.5 * (y_top + y_bot);
            let half = 0.5 * (y_bot - y_top) * breathe;
            y_top = center - half + lift;
            y_bot = center + half + lift;
            x += side;
        }
        mesh.colored_vertex(egui::pos2(x, y_top), ink);
        mesh.colored_vertex(egui::pos2(x, y_bot), ink);
    }
    for index in 0..columns - 1 {
        let a = (index * 2) as u32;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    painter.add(egui::Shape::mesh(mesh));

    let mut head_x = egui::lerp(rect.left()..=rect.right(), playhead);
    if weather > 1.0e-4 {
        head_x += (phase * 1.7 + playhead * 12.0 + seed).sin() * (0.4 + 1.4 * weather);
    }
    painter.vline(head_x, rect.y_range(), Stroke::new(1.5_f32, ink));
}

fn sample_peak(mins: &[f32], maxs: &[f32], index: f32) -> (f32, f32) {
    let n = mins.len();
    if n == 0 {
        return (0.0, 0.0);
    }
    let wrapped = index.rem_euclid(n as f32);
    let i0 = wrapped.floor() as usize % n;
    let i1 = (i0 + 1) % n;
    let frac = wrapped.fract();
    (
        mins[i0] + (mins[i1] - mins[i0]) * frac,
        maxs[i0] + (maxs[i1] - maxs[i0]) * frac,
    )
}

/// Faint full-sweep track. Darkens a little while the pointer is over the knob.
const ARC_TRACK: Color32 = Color32::from_rgb(222, 214, 200);
const ARC_TRACK_HOVER: Color32 = Color32::from_rgb(204, 196, 182);
/// 7:30, then 270 degrees clockwise to 4:30. Angle 0 is 12 o'clock.
const ARC_START: f32 = std::f32::consts::PI * 1.25;
const ARC_SWEEP: f32 = std::f32::consts::PI * 1.5;
const ARC_STROKE: f32 = 2.0;
const ARC_DOT: f32 = 3.0;
const KNOB_SLOT: f32 = 96.0;
const VALUE_HOLD: f64 = 0.9;
const DOUBLE_PRESS: f64 = 0.5;

fn global_knobs(ui: &mut egui::Ui, setter: &ParamSetter, params: &HyperchromaParams) {
    let top: [(&str, &FloatParam, bool); 4] = [
        ("Swell", &params.swell, false),
        ("Crush", &params.crush, false),
        ("Pass", &params.pass, false),
        ("Stutter", &params.stutter, false),
    ];
    let bottom: [(&str, &FloatParam, bool); 4] = [
        ("Smear", &params.smear, false),
        ("Splatter", &params.splatter, false),
        ("Wash", &params.escape, false),
        ("Drift", &params.drift, false),
    ];
    let full = ui.available_width();
    let row_h = KNOB_SLOT;
    let gap = 2.0 * 0.85;
    let pad_x = 10.0;
    let pad_y = 6.0 * 0.85;
    let cluster = (full - pad_x * 2.0).max(88.0);
    let box_w = full;
    let box_h = pad_y * 2.0 + row_h * 2.0 + gap;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(full, box_h), egui::Sense::hover());
    let frame = egui::Rect::from_min_size(rect.min, egui::vec2(box_w, box_h));
    ui.painter().rect_stroke(
        frame,
        7.0,
        Stroke::new(1.0_f32, Color32::from_rgb(148, 138, 124)),
        StrokeKind::Inside,
    );
    place_knobs(
        ui,
        setter,
        "global",
        frame.left() + pad_x,
        frame.top() + pad_y,
        cluster,
        row_h,
        4,
        &top,
    );
    place_knobs(
        ui,
        setter,
        "global-2",
        frame.left() + pad_x,
        frame.top() + pad_y + row_h + gap,
        cluster,
        row_h,
        4,
        &bottom,
    );
}

fn sound_knobs(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    name: &str,
    slice: &crate::SliceParams,
) -> bool {
    let full = ui.available_width();
    let label_h = 26.0;
    let height = KNOB_SLOT;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(full, label_h + height), egui::Sense::hover());
    let font = egui::FontId::proportional(16.0);
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(color_label(name).to_owned(), font, INK));
    let text_pos = egui::pos2(
        rect.left(),
        rect.top() + label_h * 0.5 - galley.size().y * 0.5,
    );
    let text_width = galley.size().x;
    ui.painter().galley(text_pos, galley, INK);
    let play = play_button(
        ui,
        name,
        egui::pos2(rect.left() + text_width + 16.0, rect.top() + label_h * 0.5),
    );
    let knobs: [(&str, &FloatParam, bool); 5] = [
        ("Luck", &slice.probability, false),
        ("Reverse", &slice.reverse, false),
        ("Spread", &slice.pan, true),
        ("Leap", &slice.octave, false),
        ("Charm", &slice.charm, true),
    ];
    place_knobs(
        ui,
        setter,
        name,
        rect.left(),
        rect.top() + label_h,
        full,
        height,
        knobs.len(),
        &knobs,
    );
    play
}

fn play_button(ui: &mut egui::Ui, name: &str, center: egui::Pos2) -> bool {
    let rect = egui::Rect::from_center_size(center, egui::vec2(18.0, 18.0));
    let response = ui.interact(rect, ui.id().with(("preview", name)), egui::Sense::click());
    let color = if response.hovered() { INK } else { STONE };
    if response.hovered() {
        ui.painter().circle_filled(
            center,
            9.0,
            Color32::from_rgba_unmultiplied(24, 32, 28, 18),
        );
    }
    let tip = center + egui::vec2(4.5, 0.0);
    let top = center + egui::vec2(-3.0, -4.5);
    let bottom = center + egui::vec2(-3.0, 4.5);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![top, tip, bottom],
        color,
        Stroke::NONE,
    ));
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn place_knobs(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    salt: &str,
    left: f32,
    top: f32,
    row: f32,
    height: f32,
    slots: usize,
    knobs: &[(&str, &FloatParam, bool)],
) {
    let slots = slots.max(knobs.len()).max(1);
    let slot_w = row / slots as f32;
    for (index, (label, param, bipolar)) in knobs.iter().enumerate() {
        let center_x = left + slot_w * (index as f32 + 0.5);
        let slot = egui::Rect::from_center_size(
            egui::pos2(center_x, top + height * 0.5),
            egui::vec2(slot_w, height),
        );
        let builder = egui::UiBuilder::new()
            .id_salt((*label, salt))
            .max_rect(slot)
            .layout(egui::Layout::top_down(egui::Align::Center));
        ui.allocate_new_ui(builder, |ui| {
            draw_knob(ui, setter, *param, label, *bipolar);
        });
    }
}

/// A double-click reset that has to stay open across frames.
///
/// Cubase throws away a parameter edit that begins and ends in the same frame,
/// which is what sent the knob straight back to the value it already had.
#[derive(Clone, Copy)]
struct PendingReset {
    frames: u32,
    open: bool,
}

/// Drag up to increase. Shift drags finer. Double-click returns the default.
fn draw_knob(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &impl Param,
    name: &str,
    bipolar: bool,
) {
    let width = ui.available_width().clamp(88.0, 150.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, KNOB_SLOT), egui::Sense::click_and_drag());
    let mut normalized = param.modulated_normalized_value();
    let memory = ui.id().with(name);
    let now = ui.input(|input| input.time);
    let pointer_down = response.is_pointer_button_down_on();
    let was_down = ui
        .memory(|mem| mem.data.get_temp::<bool>(memory.with("down")))
        .unwrap_or(false);
    let last_press = ui
        .memory(|mem| mem.data.get_temp::<f64>(memory.with("press")))
        .unwrap_or(f64::NEG_INFINITY);
    let suppress = ui
        .memory(|mem| mem.data.get_temp::<bool>(memory.with("suppress")))
        .unwrap_or(false);
    let press_count = ui.input(|input| {
        input
            .events
            .iter()
            .filter(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    ..
                } => *button == egui::PointerButton::Primary && rect.contains(*pos),
                _ => false,
            })
            .count()
    });
    let started = press_count > 0 || (pointer_down && !was_down);
    // A second press is a reset even when the pointer moved enough that egui
    // calls the gesture a drag instead of a double-click.
    let double = response.double_clicked()
        || press_count >= 2
        || (started && now - last_press < DOUBLE_PRESS);
    let mut turning = ui
        .memory(|mem| mem.data.get_temp::<bool>(memory.with("turning")))
        .unwrap_or(false);
    let mut reset = ui.memory(|mem| mem.data.get_temp::<PendingReset>(memory.with("reset")));

    if double {
        if !turning && !reset.is_some_and(|edit| edit.open) {
            setter.begin_set_parameter(param);
            ui.memory_mut(|mem| mem.data.insert_temp(memory.with("turning"), true));
        }
        setter.set_parameter(param, param.default_plain_value());
        reset = Some(PendingReset {
            frames: 0,
            open: true,
        });
        ui.memory_mut(|mem| {
            mem.data.insert_temp(memory.with("press"), f64::NEG_INFINITY);
            mem.data.insert_temp(memory.with("shown"), now);
            mem.data.insert_temp(memory.with("suppress"), true);
        });
    } else if reset.is_some_and(|edit| edit.open) {
        setter.set_parameter(param, param.default_plain_value());
        let edit = reset.as_mut().unwrap();
        edit.frames = edit.frames.saturating_add(1);
        if edit.frames >= BOOL_HOLD_FRAMES {
            setter.end_set_parameter(param);
            edit.open = false;
            edit.frames = 0;
            ui.memory_mut(|mem| mem.data.remove::<bool>(memory.with("turning")));
        }
        ui.ctx().request_repaint();
    } else if !suppress {
        if started || response.clicked() {
            ui.memory_mut(|mem| mem.data.insert_temp(memory.with("press"), now));
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            setter.begin_set_parameter(param);
            ui.memory_mut(|mem| {
                mem.data.insert_temp(memory.with("start"), normalized);
                mem.data.insert_temp(memory.with("drag"), 0.0f32);
                mem.data.insert_temp(memory.with("turning"), true);
            });
            turning = true;
        }
        if turning && response.dragged() {
            let (start, mut dragged) = ui.memory(|mem| {
                (
                    mem.data
                        .get_temp::<f32>(memory.with("start"))
                        .unwrap_or(normalized),
                    mem.data
                        .get_temp::<f32>(memory.with("drag"))
                        .unwrap_or(0.0),
                )
            });
            dragged += -response.drag_delta().y;
            ui.memory_mut(|mem| mem.data.insert_temp(memory.with("drag"), dragged));
            let fine = ui.input(|input| input.modifiers.shift);
            let scale = if fine { 0.0015 } else { 1.0 / 140.0 };
            let next = (start + dragged * scale).clamp(0.0, 1.0);
            setter.set_parameter(param, param.preview_plain(next));
        }
        if turning && response.drag_stopped() {
            setter.end_set_parameter(param);
            ui.memory_mut(|mem| mem.data.remove::<bool>(memory.with("turning")));
        }
        if response.dragged() {
            ui.memory_mut(|mem| mem.data.insert_temp(memory.with("shown"), now));
        }
    }
    if !pointer_down {
        ui.memory_mut(|mem| mem.data.remove::<bool>(memory.with("suppress")));
    }
    ui.memory_mut(|mem| mem.data.insert_temp(memory.with("down"), pointer_down));
    let mut show_reset = false;
    let mut clear_reset = false;
    if let Some(edit) = reset.as_mut() {
        if edit.open {
            show_reset = true;
        } else {
            let target = param.default_normalized_value();
            let live = param.modulated_normalized_value();
            if (live - target).abs() < 0.004 || edit.frames >= BOOL_SHOW_FRAMES {
                clear_reset = true;
            } else {
                edit.frames = edit.frames.saturating_add(1);
                show_reset = true;
                ui.ctx().request_repaint();
            }
        }
    }
    if clear_reset {
        reset = None;
    }
    if let Some(edit) = reset {
        ui.memory_mut(|mem| mem.data.insert_temp(memory.with("reset"), edit));
    } else {
        ui.memory_mut(|mem| mem.data.remove::<PendingReset>(memory.with("reset")));
    }
    normalized = if show_reset {
        param.default_normalized_value()
    } else {
        param.modulated_normalized_value()
    };
    let readout = if show_reset {
        param.normalized_value_to_string(param.default_normalized_value(), false)
    } else {
        param.to_string()
    };
    let shown = ui
        .memory(|mem| mem.data.get_temp::<f64>(memory.with("shown")))
        .unwrap_or(f64::NEG_INFINITY);
    let now = ui.input(|input| input.time);
    let value_visible = now - shown < VALUE_HOLD;
    if value_visible {
        let remaining = VALUE_HOLD - (now - shown);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(remaining.max(0.05)));
    }

    let dial = 70.0_f32.min(rect.width() - 8.0);
    let center = egui::pos2(rect.center().x, rect.top() + dial * 0.5 + 2.0);
    // The old disc was stroked on this radius. The arc keeps it so the slot stays put.
    let radius = dial * 0.5 - 4.0;
    draw_arc_knob(
        ui.painter(),
        center,
        radius,
        normalized,
        bipolar,
        response.hovered() || response.dragged(),
        ui.is_enabled(),
    );

    if value_visible {
        ui.painter().text(
            center,
            egui::Align2::CENTER_CENTER,
            dial_readout(&readout),
            egui::FontId::proportional(11.0),
            INK,
        );
    }
    ui.painter().text(
        egui::pos2(rect.center().x, rect.top() + dial + 12.0),
        egui::Align2::CENTER_CENTER,
        name.to_uppercase(),
        egui::FontId::proportional(11.0),
        STONE,
    );
    if response.hovered() || response.dragged() {
        if let Some(text) = knob_help(name) {
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("hyperchroma-knob-help"), text);
            });
        }
    }
    let _ = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
}

fn dial_readout(text: &str) -> String {
    if text.eq_ignore_ascii_case("center") {
        "C".to_owned()
    } else {
        text.to_owned()
    }
}

fn knob_help(name: &str) -> Option<&'static str> {
    Some(match name {
        "Swell" => "Chance of swelling in reverse",
        "Crush" => "Chance of getting bitcrushed at various rates and depths",
        "Pass" => "Chance of being sent through a HP or LP filter at various frequencies",
        "Stutter" => "Chance of repeating in place at various rates",
        "Smear" => "Chance of being processed into overlapping, slightly detuned grains",
        "Splatter" => "Chance of being thinned until only the strongest partials remain",
        "Wash" => "Chance of getting momentarily drenched in reverb",
        "Drift" => "Chance of a note moving towards or away from the effects, and of its stereo seat drifting left or right",
        "Luck" => "Chance of being played",
        "Reverse" => "Chance of playing backward",
        "Spread" => "Chance of getting panned left or right",
        "Leap" => "Chance of playing an octave higher at 2x speed",
        "Charm" => "Chance of being affected by global effects",
        _ => return None,
    })
}

/// Track plus the ink arc and position dot. Global and color knobs both call this.
fn draw_arc_knob(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    normalized: f32,
    bipolar: bool,
    hovered: bool,
    enabled: bool,
) {
    let normalized = normalized.clamp(0.0, 1.0);
    let angle = ARC_START + normalized * ARC_SWEEP;
    let track = if hovered { ARC_TRACK_HOVER } else { ARC_TRACK };
    let ink = if enabled {
        INK
    } else {
        Color32::from_rgba_unmultiplied(24, 32, 28, 90)
    };
    painter.line(
        arc_points(center, radius, ARC_START, ARC_START + ARC_SWEEP),
        Stroke::new(ARC_STROKE, track),
    );
    let origin = if bipolar {
        ARC_START + ARC_SWEEP * 0.5
    } else {
        ARC_START
    };
    if (angle - origin).abs() > 0.004 {
        let (from, to) = if angle < origin {
            (angle, origin)
        } else {
            (origin, angle)
        };
        painter.line(
            arc_points(center, radius, from, to),
            Stroke::new(ARC_STROKE, ink),
        );
    }
    painter.circle_filled(arc_point(center, radius, angle), ARC_DOT, ink);
}

fn arc_point(center: egui::Pos2, radius: f32, angle: f32) -> egui::Pos2 {
    egui::pos2(
        center.x + angle.sin() * radius,
        center.y - angle.cos() * radius,
    )
}

fn arc_points(center: egui::Pos2, radius: f32, start: f32, end: f32) -> Vec<egui::Pos2> {
    let span = (end - start).abs();
    let steps = ((span * radius) / 1.25).ceil().max(12.0) as i32;
    (0..=steps)
        .map(|step| {
            let angle = start + (end - start) * (step as f32 / steps as f32);
            arc_point(center, radius, angle)
        })
        .collect()
}

fn shaping(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    kb: &mut KbState,
    params: &HyperchromaParams,
) {
    ui.label(RichText::new("Shape").size(16.0).color(INK).strong());
    egui::Grid::new("shape")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 8.0))
        .show(ui, |ui| {
            knob(ui, setter, "Gain", &params.gain);
            knob(ui, setter, "Attack", &params.attack);
            knob(ui, setter, "Release", &params.release);
            knob(ui, setter, "Transpose", &params.root);
        });

    let mut track = kb
        .keytrack_edit
        .map(|edit| edit.target)
        .unwrap_or_else(|| params.keytrack.value());
    let track_clicked = ui.checkbox(&mut track, "Keytracking").changed();
    settle_bool(
        setter,
        &params.keytrack,
        &mut kb.keytrack_edit,
        track_clicked.then_some(track),
    );

    let mut entropy = kb
        .stereoentropic_edit
        .map(|edit| edit.target)
        .unwrap_or_else(|| params.stereoentropic.value());
    let entropy_clicked = ui.checkbox(&mut entropy, "Stereoentropic").changed();
    settle_bool(
        setter,
        &params.stereoentropic,
        &mut kb.stereoentropic_edit,
        entropy_clicked.then_some(entropy),
    );

    ui.add_space(10.0);
    let caption_w = ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(
                "Reverse floor".to_owned(),
                egui::FontId::proportional(12.0),
                STONE,
            )
            .size()
            .x
    });
    const SLIDER_W: f32 = 220.0;
    ui.horizontal(|ui| {
        slider_caption(ui, "A-side", caption_w);
        ui.visuals_mut().selection.bg_fill = Color32::from_rgb(0xb4, 0xcc, 0xd8);
        ui.add(
            widgets::ParamSlider::for_param(&params.b_side, setter)
                .with_width(SLIDER_W)
                .without_value(),
        );
        ui.label(RichText::new("B-side").size(12.0).color(STONE));
    });

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        slider_caption(ui, "Reverse floor", caption_w);
        ui.visuals_mut().selection.bg_fill = LAVENDER;
        ui.add(
            widgets::ParamSlider::for_param(&params.reverse_floor, setter).with_width(SLIDER_W),
        );
    });

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        slider_caption(ui, "Detune", caption_w);
        ui.visuals_mut().selection.bg_fill = OLIVE;
        ui.add(
            widgets::ParamSlider::for_param(&params.detune, setter)
                .with_width(SLIDER_W)
                .without_value(),
        );
        percent_readout(ui, &params.detune);
    });

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        slider_caption(ui, "Ducking", caption_w);
        ui.visuals_mut().selection.bg_fill = ASH;
        ui.add(
            widgets::ParamSlider::for_param(&params.duck, setter)
                .with_width(SLIDER_W)
                .without_value(),
        );
        percent_readout(ui, &params.duck);
        duck_rates(ui, setter, kb, params);
    });
}

/// Percent for a settings slider, in a slot sized for `100%`.
/// The settings window is anchored on the right and shrinks to its contents, so a
/// narrower readout would slide the whole panel under the cursor while dragging.
fn percent_readout(ui: &mut egui::Ui, param: &impl Param) {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let width = ui.fonts(|fonts| {
        fonts
            .layout_no_wrap("100%".to_owned(), font.clone(), INK)
            .size()
            .x
    });
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, ui.spacing().interact_size.y),
        egui::Sense::hover(),
    );
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(param.to_string(), font, INK));
    let pos = egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(pos, galley, INK);
}

/// Left caption for a settings slider. The slot is wide enough for the longest
/// caption and the text sits on its right edge, so the tracks share a start.
fn slider_caption(ui: &mut egui::Ui, text: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, ui.spacing().interact_size.y),
        egui::Sense::hover(),
    );
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            ui.label(RichText::new(text).size(12.0).color(STONE));
        },
    );
}

/// Closed label for the duck-rate menu. An empty selection still reads as a
/// quarter note, which is the rate the engine uses in that case.
fn duck_rate_label(quarter: bool, half: bool, whole: bool) -> String {
    let mut parts = Vec::new();
    if quarter {
        parts.push("1/4");
    }
    if half {
        parts.push("1/2");
    }
    if whole {
        parts.push("1/1");
    }
    if parts.is_empty() {
        "1/4".to_owned()
    } else {
        parts.join(" · ")
    }
}

fn duck_rates(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    kb: &mut KbState,
    params: &HyperchromaParams,
) {
    let show_quarter = kb
        .duck_quarter_edit
        .map(|edit| edit.target)
        .unwrap_or_else(|| params.duck_quarter.value());
    let show_half = kb
        .duck_half_edit
        .map(|edit| edit.target)
        .unwrap_or_else(|| params.duck_half.value());
    let show_whole = kb
        .duck_whole_edit
        .map(|edit| edit.target)
        .unwrap_or_else(|| params.duck_whole.value());
    let popup_id = ui.make_persistent_id("hyperchroma-duck-rates");
    let open = ui.memory(|mem| mem.is_popup_open(popup_id));
    let response = duck_rate_button(
        ui,
        &duck_rate_label(show_quarter, show_half, show_whole),
        open,
    );
    if response.clicked() {
        ui.memory_mut(|mem| mem.toggle_popup(popup_id));
    }

    let mut quarter_clicked = None;
    let mut half_clicked = None;
    let mut whole_clicked = None;
    egui::popup::popup_below_widget(
        ui,
        popup_id,
        &response,
        egui::popup::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.visuals_mut().override_text_color = Some(INK);
            ui.set_min_width(88.0);
            quarter_clicked = duck_rate_check(ui, "1/4", show_quarter, show_half || show_whole);
            half_clicked = duck_rate_check(ui, "1/2", show_half, show_quarter || show_whole);
            whole_clicked = duck_rate_check(ui, "1/1", show_whole, show_quarter || show_half);
        },
    );
    settle_bool(
        setter,
        &params.duck_quarter,
        &mut kb.duck_quarter_edit,
        quarter_clicked,
    );
    settle_bool(
        setter,
        &params.duck_half,
        &mut kb.duck_half_edit,
        half_clicked,
    );
    settle_bool(
        setter,
        &params.duck_whole,
        &mut kb.duck_whole_edit,
        whole_clicked,
    );
}

/// One row of the rate menu. Unchecking the last rate is ignored.
fn duck_rate_check(ui: &mut egui::Ui, label: &str, current: bool, others: bool) -> Option<bool> {
    let mut value = current;
    if ui.checkbox(&mut value, label).changed() && (value || others) {
        Some(value)
    } else {
        None
    }
}

fn duck_rate_button(ui: &mut egui::Ui, label: &str, open: bool) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let width = ui.fonts(|fonts| {
        fonts
            .layout_no_wrap("1/4 · 1/2 · 1/1".to_owned(), font.clone(), STONE)
            .size()
            .x
    }) + 14.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, ui.spacing().interact_size.y),
        egui::Sense::click(),
    );
    if open || response.hovered() {
        ui.painter().rect_filled(
            rect,
            2.0,
            Color32::from_rgba_unmultiplied(224, 186, 122, 40),
        );
    }
    let color = if open || response.hovered() { INK } else { STONE };
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(label.to_owned(), font, color));
    let pos = egui::pos2(rect.left() + 4.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(pos, galley, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn knob(ui: &mut egui::Ui, setter: &ParamSetter, label: &str, param: &impl Param) {
    ui.label(RichText::new(label).color(INK));
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

/// How many editor frames a checkbox edit stays open so the host can apply it.
const BOOL_HOLD_FRAMES: u32 = 8;
/// Frames to keep showing the clicked value after the edit closes.
const BOOL_SHOW_FRAMES: u32 = 48;

#[derive(Clone, Copy)]
struct PendingBool {
    target: bool,
    frames: u32,
    open: bool,
}

/// Send a checkbox change and keep the edit open across frames.
///
/// Cubase drops a parameter edit that begins and ends in the same frame, which
/// made these switches fall back to off. `clicked` is the new value on the
/// frame the box was toggled.
fn settle_bool(
    setter: &ParamSetter,
    param: &BoolParam,
    pending: &mut Option<PendingBool>,
    clicked: Option<bool>,
) -> bool {
    if let Some(target) = clicked {
        if !pending.is_some_and(|edit| edit.open) {
            setter.begin_set_parameter(param);
        }
        setter.set_parameter(param, target);
        *pending = Some(PendingBool {
            target,
            frames: 0,
            open: true,
        });
        return target;
    }

    let Some(edit) = pending.as_mut() else {
        return param.value();
    };
    if edit.open {
        setter.set_parameter(param, edit.target);
        edit.frames = edit.frames.saturating_add(1);
        if edit.frames >= BOOL_HOLD_FRAMES {
            setter.end_set_parameter(param);
            edit.open = false;
            edit.frames = 0;
        }
        return edit.target;
    }
    if param.value() == edit.target || edit.frames >= BOOL_SHOW_FRAMES {
        let applied = param.value();
        *pending = None;
        return applied;
    }
    edit.frames = edit.frames.saturating_add(1);
    edit.target
}

fn color_label(name: &str) -> &'static str {
    match name {
        "bug" => "Poppy",
        "dark" => "Rosehip",
        "dragon" => "Peony",
        "electric" => "Apricot",
        "fairy" => "Persimmon",
        "fighting" => "Marigold",
        "fire" => "Buttercup",
        "flying" => "Lichen",
        "ghost" => "Fern",
        "grass" => "Pine",
        "ground" => "Eucalyptus",
        "ice" => "Juniper",
        "normal" => "Cornflower",
        "poison" => "Iris",
        "psychic" => "Lavender",
        "rock" => "Orchid",
        "steel" => "Aubergine",
        "water" => "Chestnut",
        "sage" => "Birch",
        "clay" => "Indigo",
        "mulberry" => "Mulberry",
        "olive" => "Olive",
        "aloe" => "Aloe",
        "cinnamon" => "Cinnamon",
        _ => "",
    }
}

fn tile_color(name: &str) -> Color32 {
    match name {
        "bug" => Color32::from_rgb(0xb5, 0x65, 0x5a),
        "dark" => Color32::from_rgb(0xe3, 0xaf, 0xbd),
        "dragon" => Color32::from_rgb(0xd9, 0xb2, 0xa9),
        "electric" => Color32::from_rgb(0xd8, 0x9a, 0x78),
        "fairy" => Color32::from_rgb(0xc4, 0x7b, 0x4e),
        "mulberry" => Color32::from_rgb(0x7e, 0x4e, 0x57),
        "fighting" => Color32::from_rgb(0xc9, 0xa1, 0x5b),
        "fire" => Color32::from_rgb(0xdc, 0xcb, 0x8e),
        "flying" => Color32::from_rgb(0xa8, 0xaf, 0x7c),
        "ghost" => Color32::from_rgb(0x7e, 0x98, 0x73),
        "grass" => Color32::from_rgb(0x4f, 0x6b, 0x5c),
        "olive" => Color32::from_rgb(0x8a, 0x84, 0x60),
        "ground" => Color32::from_rgb(0x7f, 0xa7, 0x9a),
        "ice" => Color32::from_rgb(0x5e, 0x85, 0x95),
        "normal" => Color32::from_rgb(0x8f, 0xa9, 0xc4),
        "poison" => Color32::from_rgb(0x7c, 0x7a, 0xa3),
        "psychic" => Color32::from_rgb(0xb7, 0xa8, 0xc9),
        "aloe" => Color32::from_rgb(0xb8, 0xcc, 0xc2),
        "rock" => Color32::from_rgb(0x9c, 0x7a, 0xa0),
        "steel" => Color32::from_rgb(0x5e, 0x4a, 0x5c),
        "water" => Color32::from_rgb(0x8a, 0x65, 0x50),
        "sage" => Color32::from_rgb(0xc9, 0xbc, 0xa5),
        "clay" => Color32::from_rgb(0x3f, 0x50, 0x68),
        "cinnamon" => Color32::from_rgb(0xb8, 0x8b, 0x6e),
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

#[cfg(test)]
mod tests {
    use super::dial_readout;
    use nih_plug_egui::egui;

    #[test]
    fn center_reads_as_c_inside_the_dial() {
        assert_eq!(dial_readout("Center"), "C");
        assert_eq!(dial_readout("center"), "C");
        assert_eq!(dial_readout("50%"), "50%");
        assert_eq!(dial_readout("L 20%"), "L 20%");
    }

    #[test]
    fn manrope_is_the_ui_font() {
        let ctx = egui::Context::default();
        let raw = egui::RawInput::default();
        let _ = ctx.run(raw, |ctx| {
            super::install_manrope(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("Hyperchroma 100%");
            });
        });
        let raw = egui::RawInput::default();
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("Hyperchroma 100%");
            });
        });
    }

    #[test]
    fn double_click_resets_a_knob_and_keeps_the_edit_open() {
        use nih_plug::prelude::{
            FloatParam, FloatRange, GuiContext, ParamPtr, ParamSetter, PluginApi, PluginState,
        };
        use std::sync::Mutex;

        struct Log {
            events: Mutex<Vec<&'static str>>,
            values: Mutex<Vec<f32>>,
        }

        impl GuiContext for Log {
            fn plugin_api(&self) -> PluginApi {
                PluginApi::Vst3
            }
            fn request_resize(&self) -> bool {
                false
            }
            unsafe fn raw_begin_set_parameter(&self, _: ParamPtr) {
                self.events.lock().unwrap().push("begin");
            }
            unsafe fn raw_set_parameter_normalized(&self, _: ParamPtr, normalized: f32) {
                self.events.lock().unwrap().push("set");
                self.values.lock().unwrap().push(normalized);
            }
            unsafe fn raw_end_set_parameter(&self, _: ParamPtr) {
                self.events.lock().unwrap().push("end");
            }
            fn get_state(&self) -> PluginState {
                PluginState {
                    version: String::new(),
                    params: Default::default(),
                    fields: Default::default(),
                }
            }
            fn set_state(&self, _: PluginState) {}
        }

        let param = FloatParam::new(
            "Swell",
            0.0,
            FloatRange::Linear { min: 0.0, max: 1.0 },
        );
        let log = Log {
            events: Mutex::new(Vec::new()),
            values: Mutex::new(Vec::new()),
        };
        let setter = ParamSetter::new(&log);
        let ctx = egui::Context::default();
        let pos = egui::pos2(200.0, 40.0);
        let click = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        let mut time = 0.0f64;
        let mut frame = |events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            time += 0.05;
            let _ = ctx.run(raw, |ctx| {
                super::theme(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    let slot = egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(400.0, super::KNOB_SLOT),
                    );
                    let builder = egui::UiBuilder::new()
                        .id_salt(("Swell", "global"))
                        .max_rect(slot)
                        .layout(egui::Layout::top_down(egui::Align::Center));
                    ui.allocate_new_ui(builder, |ui| {
                        super::draw_knob(ui, &setter, &param, "Swell", false);
                    });
                });
            });
        };

        frame(vec![egui::Event::PointerMoved(pos)]);
        frame(vec![click(true)]);
        frame(vec![click(false)]);
        assert!(
            log.events.lock().unwrap().is_empty(),
            "one click should leave the knob where it is"
        );
        frame(vec![click(true)]);
        let events = log.events.lock().unwrap().clone();
        let values = log.values.lock().unwrap().clone();
        assert!(
            events.contains(&"begin") && events.contains(&"set") && !events.contains(&"end"),
            "the reset has to stay open past this frame, got {events:?}"
        );
        assert!(
            values.iter().any(|value| *value < 0.001),
            "the reset should send the default, got {values:?}"
        );
        for _ in 0..4 {
            frame(vec![]);
        }
        assert!(
            !log.events.lock().unwrap().contains(&"end"),
            "Cubase drops an edit that ends within a few frames of the begin"
        );
        for _ in 0..8 {
            frame(vec![]);
        }
        let events = log.events.lock().unwrap().clone();
        assert!(
            events.contains(&"end") && events.iter().filter(|event| **event == "begin").count()
                == events.iter().filter(|event| **event == "end").count(),
            "the reset gesture should close once, got {events:?}"
        );
    }
}
