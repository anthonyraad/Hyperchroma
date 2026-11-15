use std::sync::atomic::Ordering;

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

pub struct KbState {
    settings_open: bool,
    /// Index into `SAMPLE_NAMES` whose knobs are open. `None` until a box is clicked.
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
    egui_state: &EguiState,
) {
    theme(ctx);
    ResizableWindow::new("hyperchroma")
        .min_size(Vec2::new(880.0, 860.0))
        .show(ctx, egui_state, |ui| {
            let snap = snapshot(shared);
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: 18,
                    right: 16,
                    top: 6,
                    bottom: 8,
                })
                .show(ui, |ui| {
                    header(ui, &mut kb.settings_open);
                    ui.add_space(10.0);
                    recordings(ui, setter, kb, params, shared, &snap);
                });
            if kb.settings_open {
                settings_panel(ui.ctx(), setter, kb, params, shared, &snap);
            } else {
                // A click can close the panel before the host has taken the edit.
                settle_bool(setter, &params.keytrack, &mut kb.keytrack_edit, None);
                settle_bool(setter, &params.sync, &mut kb.sync_edit, None);
            }
        });
}

pub(crate) fn theme(ctx: &egui::Context) {
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
}

fn header(ui: &mut egui::Ui, settings_open: &mut bool) {
    let width = ui.available_width();
    ui.horizontal(|ui| {
        ui.set_min_width(width);
        ui.label(
            RichText::new("Hyperchroma")
                .size(30.0)
                .color(INK)
                .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if gear_button(ui, *settings_open).clicked() {
                *settings_open = !*settings_open;
            }
        });
    });
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
) {
    const COLS: usize = 5;
    const ROWS: usize = 4;
    let gap = 12.0;
    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
    let avail = ui.available_size();
    let tile_w = ((avail.x - gap * (COLS - 1) as f32) / COLS as f32 - 1.0).max(88.0);
    // Wider than tall, and short enough that the grid does not fill the window.
    let tile_h = (tile_w * 0.62).clamp(56.0, 104.0);
    let tile_w = tile_w * 0.85 * 0.9;
    let tile_size = egui::vec2(tile_w, tile_h);
    let mask = shared.active_mask.load(Ordering::Relaxed);

    for row in 0..ROWS {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
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

    if let Some(index) = kb.selected {
        let name = SAMPLE_NAMES[index];
        if let Some(slice) = params.slice(name) {
            ui.add_space(8.0);
            if sound_knobs(ui, setter, name, slice) {
                if let Some(slot) = snap.infos.iter().position(|info| info.name == name) {
                    shared.preview_slot.store(slot as u32, Ordering::Release);
                }
            }
        }
    }

    ui.add_space(10.0);
    global_knobs(ui, setter, params);
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
    let fill = if !loaded {
        Color32::from_rgb(28, 34, 32)
    } else if playing {
        lighten(tile_color(name), 16)
    } else if response.hovered() {
        lighten(tile_color(name), 10)
    } else {
        tile_color(name)
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
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-20.0, 58.0))
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
const DIAL_ARC: Color32 = Color32::from_rgb(0x7c, 0x6e, 0xbb);
const DIAL_INK: Color32 = Color32::from_rgb(0x14, 0x14, 0x14);
const ROTARY_START: f32 = std::f32::consts::PI * 1.2;
const ROTARY_END: f32 = std::f32::consts::PI * 2.8;

fn global_knobs(ui: &mut egui::Ui, setter: &ParamSetter, params: &HyperchromaParams) {
    let knobs: [(&str, &FloatParam, bool); 5] = [
        ("Swell", &params.swell, false),
        ("Crush", &params.crush, false),
        ("Pass", &params.pass, false),
        ("Stutter", &params.stutter, false),
        ("Smear", &params.smear, false),
    ];
    knob_row(ui, setter, "global", &knobs);
}

fn sound_knobs(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    name: &str,
    slice: &crate::SliceParams,
) -> bool {
    let full = ui.available_width();
    let row = full * 0.62;
    let label_h = 26.0;
    let height = 118.0;
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
    let knobs: [(&str, &FloatParam, bool); 4] = [
        ("Luck", &slice.probability, false),
        ("Reverse", &slice.reverse, false),
        ("Spread", &slice.pan, true),
        ("Leap", &slice.octave, false),
    ];
    place_knobs(ui, setter, name, rect.left(), rect.top() + label_h, row, height, &knobs);
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

fn knob_row(ui: &mut egui::Ui, setter: &ParamSetter, salt: &str, knobs: &[(&str, &FloatParam, bool)]) {
    let full = ui.available_width();
    // Keep each dial the same size as the four sound knobs, and let a fifth sit to their right.
    let row = full * 0.62 * knobs.len() as f32 / 4.0;
    let height = 118.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(full, height), egui::Sense::hover());
    place_knobs(ui, setter, salt, rect.left(), rect.top(), row, height, knobs);
}

fn place_knobs(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    salt: &str,
    left: f32,
    top: f32,
    row: f32,
    height: f32,
    knobs: &[(&str, &FloatParam, bool)],
) {
    let slot_w = row / knobs.len() as f32;
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

/// Haunter's plain dial: a light disc, a track, an accent arc, and a dark pointer.
/// Drag up to increase. Shift drags finer. Double-click returns the default.
fn draw_knob(ui: &mut egui::Ui, setter: &ParamSetter, param: &impl Param, name: &str, bipolar: bool) {
    let width = ui.available_width().clamp(88.0, 150.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 112.0), egui::Sense::click_and_drag());
    let normalized = param.modulated_normalized_value();
    let memory = ui.id().with(name);

    if response.drag_started_by(egui::PointerButton::Primary) {
        setter.begin_set_parameter(param);
        ui.memory_mut(|mem| {
            mem.data.insert_temp(memory.with("start"), normalized);
            mem.data.insert_temp(memory.with("drag"), 0.0f32);
            mem.data.insert_temp(memory.with("turning"), true);
        });
    }
    let turning = ui
        .memory(|mem| mem.data.get_temp::<bool>(memory.with("turning")))
        .unwrap_or(false);
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
    if response.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, param.default_plain_value());
        setter.end_set_parameter(param);
    }
    if turning && response.drag_stopped() {
        setter.end_set_parameter(param);
        ui.memory_mut(|mem| mem.data.remove::<bool>(memory.with("turning")));
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

    painter.text(
        egui::pos2(rect.center().x, rect.top() + dial + 14.0),
        egui::Align2::CENTER_CENTER,
        param.to_string(),
        egui::FontId::proportional(13.0),
        INK,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + dial + 30.0),
        egui::Align2::CENTER_CENTER,
        name.to_uppercase(),
        egui::FontId::proportional(11.0),
        STONE,
    );
    let _ = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
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
        "bug" => "Olive",
        "dark" => "Slate",
        "dragon" => "Rust",
        "electric" => "Gold",
        "fairy" => "Mauve",
        "fighting" => "Brick",
        "fire" => "Copper",
        "flying" => "Azure",
        "ghost" => "Purple",
        "grass" => "Fern",
        "ground" => "Ochre",
        "ice" => "Teal",
        "normal" => "Beige",
        "poison" => "Plum",
        "psychic" => "Rose",
        "rock" => "Stone",
        "steel" => "Gray",
        "water" => "Blue",
        "sage" => "Sage",
        "clay" => "Clay",
        _ => "",
    }
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
        "sage" => Color32::from_rgb(134, 146, 114),
        "clay" => Color32::from_rgb(176, 132, 108),
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
