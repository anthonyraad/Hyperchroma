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
const WARN: Color32 = Color32::from_rgb(214, 148, 110);

/// Sound boxes are 15% narrower than the 117.6px boxes the 670px window used.
/// Their height stays on that proportion, so the left column does not reflow
/// when the controls fold out.
const TILE_W_BEFORE: f32 = (636.0 - 12.0 * 4.0) / 5.0;
const TILE_W: f32 = TILE_W_BEFORE * 0.85;
const TILE_H: f32 = TILE_W_BEFORE * 0.82;
const TILE_GAP: f32 = 12.0;
const GRID_W: f32 = TILE_W * 5.0 + TILE_GAP * 4.0;
/// The knob column, 15% narrower than the 522px column of the 1220px window.
const CONTROLS_W: f32 = 522.0 * 0.85;
const COLUMN_GAP: f32 = 28.0;
const MARGIN_L: i8 = 18;
const MARGIN_R: i8 = 16;
const MARGIN_T: i8 = 6;
const MARGIN_B: i8 = 6;

pub const WINDOW_COLLAPSED: (u32, u32) = (582, 510);
const WINDOW_EXPANDED: (u32, u32) = (1054, 510);

pub struct KbState {
    settings_open: bool,
    /// The right-hand controls and the settings gear. The editor opens folded.
    controls_open: bool,
    /// Window size to restore the next time the controls fold out.
    expanded_size: (u32, u32),
    /// Applied on the next frame, before layout, so the host and the grid agree.
    pending_resize: Option<(u32, u32)>,
    /// Index into `SAMPLE_NAMES` whose knobs are open. `None` until a box is clicked.
    /// A click on empty paper clears it.
    selected: Option<usize>,
    /// Keytracking edit the host has not applied yet.
    keytrack_edit: Option<PendingBool>,
    /// Follow-host-tempo edit the host has not applied yet.
    sync_edit: Option<PendingBool>,
}

impl KbState {
    pub fn new() -> Self {
        Self {
            settings_open: false,
            controls_open: false,
            expanded_size: WINDOW_EXPANDED,
            pending_resize: None,
            selected: None,
            keytrack_edit: None,
            sync_edit: None,
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
                settings_panel(ui.ctx(), setter, kb, params, shared, &snap);
            } else {
                // A click can close the panel before the host has taken the edit.
                settle_bool(setter, &params.keytrack, &mut kb.keytrack_edit, None);
                settle_bool(setter, &params.sync, &mut kb.sync_edit, None);
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
                if gear_button(ui, kb.settings_open).clicked() {
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

/// `>>` folds the controls out, `<<` folds them away. Ink on the paper, no fill.
/// The two carets are drawn separately so they can sit closer than the font's pair.
fn fold_button(ui: &mut egui::Ui, expand: bool) -> egui::Response {
    let mark = if expand { ">" } else { "<" };
    let font = egui::FontId::proportional(20.0);
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(mark.to_owned(), font, INK));
    let advance = galley.size().x;
    let step = advance * 0.64;
    let total = advance + step;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(total + 4.0, 28.0), egui::Sense::click());
    let left = rect.center().x - total * 0.5;
    let top = rect.center().y - galley.size().y * 0.5;
    ui.painter().galley(egui::pos2(left, top), galley.clone(), INK);
    ui.painter().galley(egui::pos2(left + step, top), galley, INK);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn gear_button(ui: &mut egui::Ui, open: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::click());
    let color = if open || response.hovered() { INK } else { STONE };
    if open {
        ui.painter().circle_filled(
            rect.center(),
            13.0,
            Color32::from_rgba_unmultiplied(224, 186, 122, 28),
        );
    }
    paint_gear(ui.painter(), rect.center(), 7.2, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn paint_gear(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: Color32) {
    let stroke = Stroke::new(1.45_f32, color);
    painter.circle_stroke(center, radius * 0.46, stroke);
    painter.circle_filled(center, radius * 0.16, color);
    for tooth in 0..6 {
        let angle =
            tooth as f32 * std::f32::consts::TAU / 6.0 - std::f32::consts::FRAC_PI_2;
        let dir = egui::vec2(angle.cos(), angle.sin());
        painter.line_segment(
            [center + dir * (radius * 0.58), center + dir * radius],
            stroke,
        );
    }
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
    const COLS: usize = 5;
    const ROWS: usize = 4;
    let mask = shared.active_mask.load(Ordering::Relaxed);
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
                let playhead = bank_index.and_then(|slot| playhead_of(shared, slot));
                let index = row * COLS + col;
                let clicked = tile(
                    ui,
                    name,
                    tile_size,
                    playing,
                    info.is_some(),
                    kb.selected == Some(index),
                    info.map(|info| (info.wave_min.as_ref(), info.wave_max.as_ref())),
                    playhead.filter(|_| playing),
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
) -> bool {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let radius = 4.0;
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
                paint_live_wave(&painter, plot, mins, maxs, playhead);
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
    shared: &SharedUi,
    snap: &crate::BankSnapshot,
) {
    let max_h = (ctx.screen_rect().height() - 78.0).max(240.0);
    egui::Window::new("hyperchroma-settings")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-(MARGIN_R as f32), 50.0))
        .default_width(440.0)
        .max_height(max_h)
        .vscroll(true)
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
            ui.set_min_width(400.0);
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
            shaping(ui, setter, kb, params, shared);
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

/// A solid white picture of the recording, with a white line at the playhead.
fn paint_live_wave(
    painter: &egui::Painter,
    rect: egui::Rect,
    mins: &[f32],
    maxs: &[f32],
    playhead: f32,
) {
    let bins = mins.len().min(maxs.len());
    if bins < 4 || rect.width() < 8.0 || rect.height() < 8.0 {
        return;
    }
    let playhead = playhead.rem_euclid(1.0);
    let columns = (rect.width() / 2.2).round().clamp(48.0, 180.0) as usize;
    let mid_y = rect.center().y;
    let amp = rect.height() * 0.48;
    let white = Color32::WHITE;

    let mut mesh = Mesh::default();
    for col in 0..columns {
        let t = col as f32 / (columns - 1) as f32;
        let (min_v, max_v) = sample_peak(mins, maxs, t * bins as f32);
        let x = egui::lerp(rect.left()..=rect.right(), t);
        let mut y_top = mid_y - max_v.clamp(-1.0, 1.0) * amp;
        let mut y_bot = mid_y - min_v.clamp(-1.0, 1.0) * amp;
        if y_bot - y_top < 1.5 {
            let mid = 0.5 * (y_top + y_bot);
            y_top = mid - 0.75;
            y_bot = mid + 0.75;
        }
        mesh.colored_vertex(egui::pos2(x, y_top), white);
        mesh.colored_vertex(egui::pos2(x, y_bot), white);
    }
    for index in 0..columns - 1 {
        let a = (index * 2) as u32;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    painter.add(egui::Shape::mesh(mesh));

    let x = egui::lerp(rect.left()..=rect.right(), playhead);
    painter.vline(x, rect.y_range(), Stroke::new(1.5_f32, white));
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

const DIAL: Color32 = Color32::from_rgb(0xf6, 0xf5, 0xfc);
const DIAL_OUTLINE: Color32 = Color32::from_rgb(0x4a, 0x4a, 0x4a);
const DIAL_TRACK: Color32 = Color32::from_rgb(0xba, 0xb5, 0xd1);
const DIAL_ARC: Color32 = Color32::from_rgb(0x14, 0x14, 0x14);
const DIAL_INK: Color32 = Color32::from_rgb(0x14, 0x14, 0x14);
const ROTARY_START: f32 = std::f32::consts::PI * 1.2;
const ROTARY_END: f32 = std::f32::consts::PI * 2.8;
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

/// Haunter's plain dial: a light disc, a track, an accent arc, and a dark pointer.
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
    let radius = dial * 0.5 - 3.0;
    let angle = ROTARY_START + normalized * (ROTARY_END - ROTARY_START);
    let painter = ui.painter();
    painter.circle_filled(center, radius - 1.0, DIAL);
    painter.circle_stroke(center, radius - 1.0, Stroke::new(1.0_f32, DIAL_OUTLINE));
    painter.line(
        arc_points(center, radius - 5.0, ROTARY_START, ROTARY_END),
        Stroke::new(2.0_f32, DIAL_TRACK),
    );
    if bipolar {
        let mid = ROTARY_START + 0.5 * (ROTARY_END - ROTARY_START);
        if (angle - mid).abs() > 0.02 {
            let (from, to) = if angle < mid { (angle, mid) } else { (mid, angle) };
            painter.line(
                arc_points(center, radius - 5.0, from, to),
                Stroke::new(2.0_f32, DIAL_ARC),
            );
        }
    } else if normalized > 0.004 {
        painter.line(
            arc_points(center, radius - 5.0, ROTARY_START, angle),
            Stroke::new(2.0_f32, DIAL_ARC),
        );
    }
    let inner = egui::pos2(
        center.x + angle.sin() * dial * 0.18,
        center.y - angle.cos() * dial * 0.18,
    );
    let outer = egui::pos2(
        center.x + angle.sin() * dial * 0.36,
        center.y - angle.cos() * dial * 0.36,
    );
    painter.line_segment([inner, outer], Stroke::new(1.6_f32, DIAL_INK));

    if value_visible {
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            dial_readout(&readout),
            egui::FontId::proportional(11.0),
            DIAL_INK,
        );
    }
    painter.text(
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
        "Splatter" => "Chance of being processed into rhythmic, short grains",
        "Wash" => "Chance of getting momentarily drenched in reverb",
        "Drift" => "Chance of a note moving towards or away from the effects' set points",
        "Luck" => "Chance of being played",
        "Reverse" => "Chance of playing backward",
        "Spread" => "Chance of getting panned left or right",
        "Leap" => "Chance of playing an octave higher at 2x speed",
        "Charm" => "Chance of being affected by global effects",
        _ => return None,
    })
}

fn arc_points(center: egui::Pos2, radius: f32, start: f32, end: f32) -> Vec<egui::Pos2> {
    let steps = (((end - start).abs() / 0.09).ceil() as i32).max(2);
    (0..=steps)
        .map(|step| {
            let angle = start + (end - start) * (step as f32 / steps as f32);
            egui::pos2(
                center.x + angle.sin() * radius,
                center.y - angle.cos() * radius,
            )
        })
        .collect()
}

fn shaping(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    kb: &mut KbState,
    params: &HyperchromaParams,
    shared: &SharedUi,
) {
    ui.label(RichText::new("Shape").size(16.0).color(INK).strong());
    ui.label(
        RichText::new(
            "The recordings are in C at 120 BPM. Transpose is where they sit. Sample BPM is the tempo they were played at.",
        )
        .size(12.0)
        .color(STONE),
    );
    egui::Grid::new("shape")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 8.0))
        .show(ui, |ui| {
            knob(ui, setter, "Gain", &params.gain);
            knob(ui, setter, "Attack", &params.attack);
            knob(ui, setter, "Release", &params.release);
            knob(ui, setter, "Transpose", &params.root);
            knob(ui, setter, "Sample BPM", &params.sample_bpm);
            knob(ui, setter, "Tempo", &params.tempo);
        });

    let mut track = kb
        .keytrack_edit
        .map(|edit| edit.target)
        .unwrap_or_else(|| params.keytrack.value());
    let track_clicked = ui.checkbox(&mut track, "Keytracking").changed();
    let track = settle_bool(
        setter,
        &params.keytrack,
        &mut kb.keytrack_edit,
        track_clicked.then_some(track),
    );
    let track_copy = if track {
        "The note you play transposes the recording. Transpose is the note that leaves it as recorded."
    } else {
        "Every note plays at Transpose. C4 leaves the recordings as they are."
    };
    ui.label(RichText::new(track_copy).size(12.0).color(STONE));

    let mut follow = kb
        .sync_edit
        .map(|edit| edit.target)
        .unwrap_or_else(|| params.sync.value());
    let follow_clicked = ui.checkbox(&mut follow, "Follow the host tempo").changed();
    let follow = settle_bool(
        setter,
        &params.sync,
        &mut kb.sync_edit,
        follow_clicked.then_some(follow),
    );
    let host = shared.host_bpm_x100.load(Ordering::Relaxed);
    let follow_copy = if follow && host > 0 {
        "The session tempo is moving the slices. Turn this off to use the Tempo knob."
    } else if follow {
        "Waiting for the DAW. Until a tempo arrives, the Tempo knob is used."
    } else {
        "The Tempo knob is in charge, even when the DAW is playing."
    };
    ui.label(RichText::new(follow_copy).size(12.0).color(STONE));
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
        _ => "",
    }
}

fn tile_color(name: &str) -> Color32 {
    match name {
        "bug" => Color32::from_rgb(0xc8, 0x46, 0x3d),
        "dark" => Color32::from_rgb(0xb5, 0x41, 0x6f),
        "dragon" => Color32::from_rgb(0xd9, 0x8b, 0xa8),
        "electric" => Color32::from_rgb(0xe8, 0x94, 0x74),
        "fairy" => Color32::from_rgb(0xd9, 0x69, 0x2e),
        "fighting" => Color32::from_rgb(0xe3, 0xa7, 0x2f),
        "fire" => Color32::from_rgb(0xe8, 0xd3, 0x5e),
        "flying" => Color32::from_rgb(0xa2, 0xb4, 0x55),
        "ghost" => Color32::from_rgb(0x4f, 0x8f, 0x45),
        "grass" => Color32::from_rgb(0x2e, 0x5e, 0x4e),
        "ground" => Color32::from_rgb(0x5f, 0xa5, 0x93),
        "ice" => Color32::from_rgb(0x3a, 0x7e, 0x9e),
        "normal" => Color32::from_rgb(0x4a, 0x8f, 0xdb),
        "poison" => Color32::from_rgb(0x5b, 0x5b, 0xae),
        "psychic" => Color32::from_rgb(0xb5, 0x9c, 0xdb),
        "rock" => Color32::from_rgb(0xa1, 0x5b, 0xb0),
        "steel" => Color32::from_rgb(0x5a, 0x34, 0x56),
        "water" => Color32::from_rgb(0x8a, 0x4f, 0x2e),
        "sage" => Color32::from_rgb(0xd8, 0xd2, 0xc0),
        "clay" => Color32::from_rgb(0x2b, 0x3f, 0x78),
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
