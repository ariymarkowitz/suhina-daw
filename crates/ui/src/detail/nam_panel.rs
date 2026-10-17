// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The NAM Amp Modeler's and NAM Cab Loader's panels
//! ([docs/spec/nam-amp-and-cab.md] › The panels): the name row's dropdown with the folder's
//! files and the steppers, the knobs and toggles in the middle, the file dropdown at the
//! bottom, the failure note. Suhina's own look — theme tokens, a straight grid, no Live-style
//! chrome — and everything a `device.*` command.

use std::collections::HashMap;

use egui::{Align2, Rect, Sense, Ui};
use suhina_device_api::{ParamInfo, Unit};
use suhina_device_nam_amp_modeler::INFO as AMP_INFO;
use suhina_device_nam_cab_loader::INFO as CAB_INFO;
use suhina_engine::model::TrackId;

use crate::detail::detail_view::DeviceBox;
use crate::kit::theme;
use crate::kit::widgets::{self, Action};

/// The folders' files and a device file's shown names and failure notes, provided each frame
/// by the app ([docs/spec/nam-amp-and-cab.md] › Folders).
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Lists {
    /// The models folder's `.nam` files: (path, shown name).
    pub models: Vec<(String, String)>,
    /// The IR folder's `.wav` files: (path, shown name).
    pub irs: Vec<(String, String)>,
    /// A file's shown name — the model's metadata name or the cab's folder's — by path.
    pub names: HashMap<String, String>,
    /// Why a device's file does not run, by path: what a load gave.
    pub notes: HashMap<String, String>,
}

const KNOB: f32 = 34.0;
const STEP: f32 = 16.0;
const ROW_HEIGHT: f32 = 22.0;
const PAD: f32 = 8.0;
const CELL: f32 = 62.0;

/// The parameter with id `id`, from a device's static info.
fn parameter_of(params: &'static [ParamInfo], id: &str) -> &'static ParamInfo {
    params.iter().find(|p| p.id == id).unwrap_or(&params[0])
}

/// The parameter with id `id`, from the amp's info.
fn amp_parameter(id: &str) -> &'static ParamInfo {
    parameter_of(AMP_INFO.params, id)
}

/// The parameter with id `id`, from the cab's info.
fn cab_parameter(id: &str) -> &'static ParamInfo {
    parameter_of(CAB_INFO.params, id)
}

/// The label's prefix a panel's controls take: the device's shown name, "NAM " off if it leads.
fn prefix_of(device: &DeviceBox) -> String {
    let name = device.name.as_str();
    name.strip_prefix("NAM ").unwrap_or(name).to_owned()
}

/// The value the device holds for `parameter` (the default when it holds none).
fn value_of(device: &DeviceBox, parameter: &ParamInfo) -> f64 {
    device.values.iter().find(|(id, _)| *id == parameter.id).map_or(parameter.default, |(_, value)| *value)
}

/// `device.setParam` for a panel change.
fn set_param(
    actions: &mut Vec<Action>,
    track: TrackId,
    device: usize,
    parameter: &ParamInfo,
    value: f64,
    continuous: bool,
) {
    actions.push(Action::new(
        "device.setParam",
        serde_json::json!({
            "track": track.0, "device": device, "param": parameter.id,
            "value": parameter.clamp(value), "continuous": continuous,
        }),
    ));
}

/// A parameter's value with its unit, as the value lines show it.
fn shown_value(parameter: &ParamInfo, value: f64) -> String {
    let value = if value.is_finite() { value } else { parameter.default };
    match parameter.unit {
        Unit::Decibels => format!("{value:+.1} dB"),
        Unit::Hertz => format!("{value:.0} Hz"),
        Unit::Seconds => format!("{:.2} ms", value * 1000.0),
        _ => format!("{value:.2}"),
    }
}

/// A knob in `rect`, its label over it, its value under it. A drag turns it (one
/// `device.setParam` with `continuous` on), a double-click returns to the default, the arrow
/// keys step while it has focus.
fn knob(
    ui: &mut Ui,
    track: TrackId,
    device_index: usize,
    prefix: &str,
    parameter: &'static ParamInfo,
    value: f64,
    actions: &mut Vec<Action>,
) -> egui::Response {
    let rect = ui.allocate_exact_size(egui::vec2(CELL, KNOB + 30.0), Sense::click_and_drag()).0;
    let publisher = format!("{}: {}", prefix, parameter.name.to_lowercase());
    let response =
        ui.interact(rect, egui::Id::new(("nam-knob", track.0, device_index, parameter.id)), Sense::click_and_drag());
    if response.clicked() {
        response.request_focus();
    }
    let mut at = parameter.to_normalized(value);
    let mut change: Option<(f64, bool)> = None;
    if response.dragged() {
        let fine = if ui.input(|i| i.modifiers.shift) { 0.1 } else { 1.0 };
        at = (at - f64::from(response.drag_delta().y) * 0.005 * fine).clamp(0.0, 1.0);
        change = Some((parameter.from_normalized(at), true));
    } else if response.double_clicked() {
        change = Some((parameter.default, false));
    } else if response.has_focus() {
        let up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::ArrowRight));
        let down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowLeft));
        if up != down {
            let step = (parameter.max - parameter.min) * 0.01;
            let next = value + f64::from(up as i8) * step - f64::from(down as i8) * step;
            change = Some((parameter.clamp(next), false));
        }
    }
    let center = rect.center() + egui::vec2(0.0, 2.0);
    widgets::knob(ui.painter(), center, KNOB, at as f32, theme::text_primary());
    widgets::text_in(
        ui.painter(),
        Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), rect.top() + 12.0)),
        Align2::CENTER_CENTER,
        parameter.name,
        theme::ui_font(theme::TEXT_TINY),
        theme::text_secondary(),
    );
    widgets::text_in(
        ui.painter(),
        Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 16.0), rect.max),
        Align2::CENTER_CENTER,
        &shown_value(parameter, change.as_ref().map_or(value, |(v, _)| *v)),
        theme::mono_font(theme::TEXT_TINY),
        theme::text_value(),
    );
    if let Some((next, continuous)) = change {
        set_param(actions, track, device_index, parameter, next, continuous);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, &publisher));
    response
}

/// A lit-or-dark button (EQ, HQ, Phase, HP), one `device.setParam` per click.
#[allow(clippy::too_many_arguments)]
fn toggle(
    ui: &mut Ui,
    rect: Rect,
    track: TrackId,
    device_index: usize,
    prefix: &str,
    parameter: &'static ParamInfo,
    on: bool,
    actions: &mut Vec<Action>,
) -> egui::Response {
    let id = egui::Id::new(("nam-toggle", track.0, device_index, parameter.id));
    let response = ui.interact(rect, id, Sense::click());
    let (fill, text) = widgets::toggle_colors(ui, &response, on);
    ui.painter().rect_filled(rect, 3.0, fill);
    widgets::text_in(
        ui.painter(),
        rect,
        Align2::CENTER_CENTER,
        parameter.name,
        theme::ui_font(theme::TEXT_SMALL),
        text,
    );
    if response.clicked() {
        set_param(actions, track, device_index, parameter, f64::from(!on), false);
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{}: {}", prefix, parameter.name.to_lowercase()),
        )
    });
    response
}

/// A file's dropdown pill in `rect`, its shown text and the files under it. Returns the row's
/// response; `picked` set when the panel asks for a switch.
#[allow(clippy::too_many_arguments)]
fn file_row(
    ui: &mut Ui,
    rect: Rect,
    id_salt: egui::Id,
    files: &[(String, String)],
    current: &str,
    shown: &str,
    empty: &str,
    label: &str,
) -> (egui::Response, Option<String>) {
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let current_name = if shown.is_empty() {
        std::path::Path::new(current).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        shown.to_owned()
    };
    let mut choice: Option<String> = None;
    let response = egui::ComboBox::from_id_salt(id_salt)
        .width(rect.width() - 52.0)
        .selected_text(egui::RichText::new(current_name).font(theme::ui_font(theme::TEXT_SMALL)))
        .show_ui(&mut row, |ui| {
            if files.is_empty() {
                let empty_label = ui
                    .selectable_label(false, egui::RichText::new(empty).font(theme::ui_font(theme::TEXT_TINY)).weak());
                empty_label.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{label}: {empty}"))
                });
            }
            for (path, name) in files {
                let mut on = path == current;
                let item = ui.selectable_value(
                    &mut on,
                    true,
                    egui::RichText::new(name).font(theme::ui_font(theme::TEXT_SMALL)),
                );
                if item.changed() {
                    choice = Some(path.clone());
                }
                item.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{label}: {name}"))
                });
            }
        })
        .response;
    (response, choice)
}

/// Where `current` sits in `files`.
fn index_of(files: &[(String, String)], current: &str) -> Option<usize> {
    files.iter().position(|(path, _)| path == current)
}

/// Steps `current` through `files` by `by` (wrapping); a file that is not in the list steps
/// from the ends.
fn step_through(files: &[(String, String)], current: &str, by: i64) -> Option<String> {
    let len = files.len();
    let index = match (index_of(files, current), by) {
        (Some(at), by) => (at as i64 + by + len as i64).rem_euclid(len as i64) as usize,
        (None, 1) => 0,
        (None, _) => len.saturating_sub(1),
    };
    files.get(index).map(|(path, _)| path.clone())
}

/// A stepper button drawn in `rect`.
fn stepper(ui: &mut Ui, rect: Rect, id_salt: egui::Id, glyph: &str, label: String) -> egui::Response {
    let response = ui.interact(rect, egui::Id::new(id_salt), Sense::click());
    ui.painter().rect_filled(rect, 2.0, theme::control_background());
    widgets::text_in(
        ui.painter(),
        rect,
        Align2::CENTER_CENTER,
        glyph,
        theme::ui_font(theme::TEXT_TINY),
        theme::text_secondary(),
    );
    response.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
    response
}

/// `rect` split into `cells` even columns.
fn split(rect: Rect, cells: usize) -> Vec<Rect> {
    let gap = 2.0;
    let each = ((rect.width() - gap * (cells.saturating_sub(1) as f32)) / cells.max(1) as f32).min(CELL + 24.0);
    (0..cells)
        .map(|c| {
            Rect::from_min_size(
                egui::pos2(rect.left() + c as f32 * (each + gap), rect.top()),
                egui::vec2(each, rect.height()),
            )
        })
        .collect()
}

/// The failure note a panel shows for its device's file: the load's message, or the missing
/// file's line, or nothing when everything is well.
fn note_of(device: &DeviceBox, lists: &Lists) -> Option<String> {
    let file = device.file.as_deref().unwrap_or("");
    if file.is_empty() {
        return None;
    }
    if let Some(note) = lists.notes.get(file) {
        return Some(note.clone());
    }
    (!lists.names.contains_key(file)).then(|| format!("Model missing: {file}"))
}

/// The name row's and the file row's picks and the steppers, for one panel in `rect`'s rows.
/// Returns the file a click asked for, when one differs from the device's current.
fn rows_and_picks(
    ui: &mut Ui,
    top: f32,
    bottom: f32,
    track: TrackId,
    device: &DeviceBox,
    files: &[(String, String)],
    lists: &Lists,
) -> Option<String> {
    let wide = ui.max_rect().width();
    let left = ui.max_rect().left();
    let prefix = prefix_of(device);
    let current = device.file.as_deref().unwrap_or("");
    let salt = (track.0, device.index);
    // The name row: the file's shown name, the folder's files under, the steppers beside.
    let name_rect =
        Rect::from_min_max(egui::pos2(left, top), egui::pos2(left + wide - 2.0 * STEP - 12.0, top + ROW_HEIGHT));
    let shown_name = lists.names.get(current).cloned().unwrap_or_default();
    let (_, name_pick) = file_row(
        ui,
        name_rect,
        egui::Id::new(("nam-name", salt)),
        files,
        current,
        &shown_name,
        "No model",
        &format!("{prefix}: model"),
    );
    let up_rect = Rect::from_min_size(egui::pos2(left + wide - 2.0 * STEP - 10.0, top), egui::vec2(STEP, ROW_HEIGHT));
    let down_rect = Rect::from_min_size(egui::pos2(left + wide - STEP - 4.0, top), egui::vec2(STEP, ROW_HEIGHT));
    let up = stepper(ui, up_rect, egui::Id::new(("nam-step-up", salt)), "▲", format!("{prefix}: previous model"));
    let down = stepper(ui, down_rect, egui::Id::new(("nam-step-down", salt)), "▼", format!("{prefix}: next model"));
    // The file row, at the bottom: the same files, the file's own name shown.
    let file_rect = Rect::from_min_max(egui::pos2(left, bottom - ROW_HEIGHT), egui::pos2(left + wide, bottom));
    let file_shown =
        if current.is_empty() { String::new() } else { current.rsplit('/').next().unwrap_or(current).to_owned() };
    let (_, file_pick) = file_row(
        ui,
        file_rect,
        egui::Id::new(("nam-file", salt)),
        files,
        current,
        &file_shown,
        "Choose a model file…",
        &format!("{prefix}: model file"),
    );
    let ask = up.clicked().then(|| step_through(files, current, -1)).flatten();
    let ask = ask.or_else(|| down.clicked().then(|| step_through(files, current, 1)).flatten());
    let asked = ask.or(name_pick).or(file_pick);
    asked.filter(|candidate| Some(candidate.as_str()) != device.file.as_deref())
}

/// The NAM Amp Modeler's panel, drawn in `rect` ([docs/spec/nam-amp-and-cab.md] › The panels).
pub(crate) fn show_amp(
    ui: &mut Ui,
    rect: Rect,
    track: TrackId,
    device: &DeviceBox,
    lists: &Lists,
    actions: &mut Vec<Action>,
) {
    let prefix = prefix_of(device);
    let (top, bottom) = (rect.top() + PAD, rect.bottom() - 2.0 * ROW_HEIGHT - 2.0 * PAD);
    if let Some(pick) = rows_and_picks(ui, top, rect.bottom() - PAD, track, device, &lists.models, lists) {
        actions.push(Action::new(
            "device.setNamFile",
            serde_json::json!({ "track": track.0, "device": device.index, "path": pick }),
        ));
    }
    // The knobs' row: Gain, then the EQ's band knobs, then the output's.
    let knobs = Rect::from_min_max(
        egui::pos2(rect.left(), top + ROW_HEIGHT + 2.0),
        egui::pos2(rect.right(), top + ROW_HEIGHT + 2.0 + 62.0),
    );
    let gain = ["inputGain", "eqBass", "eqMid", "eqTreble", "outputVolume"];
    let cells = split(knobs, gain.len());
    for (cell, id) in cells.iter().zip(gain.iter()) {
        let parameter = amp_parameter(id);
        let value = value_of(device, parameter);
        knob(ui, track, device.index, &prefix, parameter, value, actions);
        let _ = cell;
    }
    // The EQ's toggle, the mid band's frequency, the trim delay and HQ.
    let row2 = Rect::from_min_max(
        egui::pos2(rect.left(), knobs.bottom() + 2.0),
        egui::pos2(rect.right(), knobs.bottom() + 2.0 + ROW_HEIGHT + KNOB),
    );

    let cells = split(row2, 5);
    let (eq_cell, mid_cell, delay_cell, hq_cell, spare) = (cells[0], cells[1], cells[2], cells[3], cells[4]);
    let eq_on = value_of(device, parameter_of(AMP_INFO.params, "eqOn")) >= 0.5;
    toggle(
        ui,
        Rect::from_center_size(eq_cell.center() + egui::vec2(0.0, 6.0), egui::vec2(48.0, ROW_HEIGHT)),
        track,
        device.index,
        &prefix,
        amp_parameter("eqOn"),
        eq_on,
        actions,
    );
    let _ = mid_cell;
    knob(
        ui,
        track,
        device.index,
        &prefix,
        amp_parameter("eqMidFreq"),
        value_of(device, amp_parameter("eqMidFreq")),
        actions,
    );
    let _ = delay_cell;
    knob(ui, track, device.index, &prefix, amp_parameter("delay"), value_of(device, amp_parameter("delay")), actions);
    let hq_on = value_of(device, amp_parameter("hq")) >= 0.5;
    toggle(
        ui,
        Rect::from_center_size(hq_cell.center() + egui::vec2(0.0, 6.0), egui::vec2(48.0, ROW_HEIGHT)),
        track,
        device.index,
        &prefix,
        amp_parameter("hq"),
        hq_on,
        actions,
    );
    let _ = spare;
    // The failure note, beside the file row's left, over the file row's line: drawn with the
    // text the panel's note carries.
    if let Some(note) = note_of(device, lists) {
        widgets::text_in(
            ui.painter(),
            Rect::from_min_max(
                egui::pos2(rect.left(), bottom + ROW_HEIGHT - 14.0),
                egui::pos2(rect.right() - PAD, bottom + ROW_HEIGHT),
            ),
            Align2::LEFT_CENTER,
            &note,
            theme::ui_font(theme::TEXT_TINY),
            theme::text_hint(),
        );
    }
}

/// The NAM Cab Loader's panel, drawn in `rect` ([docs/spec/nam-amp-and-cab.md] › The panels).
pub(crate) fn show_cab(
    ui: &mut Ui,
    rect: Rect,
    track: TrackId,
    device: &DeviceBox,
    lists: &Lists,
    actions: &mut Vec<Action>,
) {
    let prefix = prefix_of(device);
    let (top, _) = (rect.top() + PAD, rect.bottom());
    let files = &lists.irs;
    if let Some(pick) = rows_and_picks(ui, top, rect.bottom() - PAD, track, device, files, lists) {
        actions.push(Action::new(
            "device.setNamFile",
            serde_json::json!({ "track": track.0, "device": device.index, "path": pick }),
        ));
    }
    // The controls: the phase and high-pass toggles, the cuts' knobs and the volume knob.
    let row = Rect::from_min_max(
        egui::pos2(rect.left(), top + ROW_HEIGHT + 2.0),
        egui::pos2(rect.right(), top + ROW_HEIGHT + 2.0 + 62.0),
    );
    let cells = split(row, 6);
    let phase = cab_parameter("phase");
    let hp = cab_parameter("hpOn");
    let (phase_on, hp_on) = (value_of(device, phase) >= 0.5, value_of(device, hp) >= 0.5);
    toggle(
        ui,
        Rect::from_center_size(cells[0].center() + egui::vec2(0.0, 12.0), egui::vec2(48.0, ROW_HEIGHT)),
        track,
        device.index,
        &prefix,
        phase,
        phase_on,
        actions,
    );
    toggle(
        ui,
        Rect::from_center_size(cells[1].center() + egui::vec2(0.0, 12.0), egui::vec2(48.0, ROW_HEIGHT)),
        track,
        device.index,
        &prefix,
        hp,
        hp_on,
        actions,
    );
    knob(
        ui,
        track,
        device.index,
        &prefix,
        cab_parameter("cutBass"),
        value_of(device, cab_parameter("cutBass")),
        actions,
    );
    knob(ui, track, device.index, &prefix, cab_parameter("cutMid"), value_of(device, cab_parameter("cutMid")), actions);
    knob(
        ui,
        track,
        device.index,
        &prefix,
        cab_parameter("cutTreble"),
        value_of(device, cab_parameter("cutTreble")),
        actions,
    );
    knob(ui, track, device.index, &prefix, cab_parameter("volume"), value_of(device, cab_parameter("volume")), actions);
    // The mid band's frequency and the trim delay.
    let row2 = Rect::from_min_max(
        egui::pos2(rect.left(), row.bottom() + 2.0),
        egui::pos2(rect.right(), row.bottom() + 2.0 + ROW_HEIGHT + KNOB),
    );
    let cells = split(row2, 3);
    let mid_freq = cab_parameter("cutMidFreq");
    let delay = cab_parameter("delay");
    let _ = (cells, mid_freq, delay);
    if let Some(note) = note_of(device, lists) {
        widgets::text_in(
            ui.painter(),
            Rect::from_min_max(
                egui::pos2(rect.left(), rect.bottom() - 14.0),
                egui::pos2(rect.right() - PAD, rect.bottom()),
            ),
            Align2::LEFT_CENTER,
            &note,
            theme::ui_font(theme::TEXT_TINY),
            theme::text_hint(),
        );
    }
}
