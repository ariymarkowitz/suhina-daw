// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The same tiny musical scene in each palette. No theme installation or project loading.

use crate::kit::surface::Backgrounds as _;
use egui::{Painter, Rect, Stroke, pos2, vec2};

use super::theme::Palette;

/// A bounded snapshot of the open project's colours. Missing colours follow each preview's palette.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scene {
    pub tracks: [Option<egui::Color32>; 4],
    pub clips: [Option<egui::Color32>; 4],
    pub audio: [bool; 4],
}

impl Scene {
    pub fn from_project(project: &suhina_engine::model::Project) -> Self {
        let mut scene = Self::default();
        let colour = |c: suhina_engine::model::Rgb| egui::Color32::from_rgb(c.0, c.1, c.2);
        for (index, track) in project.tracks.iter().take(4).enumerate() {
            scene.audio[index] = track.content.audio && !track.content.notes;
            scene.tracks[index] = track.color.map(colour);
            scene.clips[index] = project
                .arrangement
                .lanes
                .iter()
                .find(|lane| lane.track == Some(track.id))
                .and_then(|lane| lane.clips.first())
                .and_then(|clip| clip.color)
                .map(colour);
        }
        scene
    }
}

/// Paint a miniature browser, timeline, MIDI/audio clips and controls from explicit colours.
pub fn paint(painter: &Painter, rect: Rect, palette: &Palette, scene: &Scene) {
    if !painter.clip_rect().intersects(rect) || !rect.is_finite() || rect.width() < 30.0 || rect.height() < 20.0 {
        return;
    }
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let cell = |x: f32, y: f32, w: f32, h: f32| {
        Rect::from_min_size(
            pos2(rect.left() + rect.width() * x, rect.top() + rect.height() * y),
            vec2(rect.width() * w, rect.height() * h),
        )
    };
    painter.fill_background(rect, 2.0, palette.window_background);
    painter.fill_background(cell(0.0, 0.0, 1.0, 0.15), 0.0, palette.panel_background);
    painter.fill_background(cell(0.0, 0.15, 0.2, 0.85), 0.0, palette.panel_background);
    for row in 0..5 {
        painter.rect_filled(cell(0.025, 0.22 + row as f32 * 0.11, 0.13, 0.018), 0.0, palette.text_secondary);
    }
    for button in 0..4 {
        painter.fill_background(
            cell(0.025 + button as f32 * 0.06, 0.04, 0.04, 0.065),
            1.0,
            if button == 0 { palette.accent } else { palette.control_background },
        );
    }
    for bar in 0..8 {
        let grid = cell(0.23 + bar as f32 * 0.09, 0.2, 0.0, 0.75);
        painter.vline(grid.left(), grid.y_range(), Stroke::new(0.6, palette.grid_major));
    }
    for row in 0..4 {
        let fallback = if scene.audio[row] { palette.audio_clip_default } else { palette.midi_clip_default };
        let colour = scene.clips[row].or(scene.tracks[row]).unwrap_or(fallback);
        // Arrangement MIDI clips use 82% of the source over the panel surface.
        let colour = super::motion::mix(palette.panel_background, colour, 0.82);
        let top = 0.23 + row as f32 * 0.18;
        let clip = cell(0.25 + row as f32 * 0.05, top, 0.55 - row as f32 * 0.04, 0.14);
        painter.fill_background(clip, 2.0, colour);
        let ink = if super::theme::contrast(colour, palette.text_primary)
            > super::theme::contrast(colour, palette.window_background)
        {
            palette.text_primary
        } else {
            palette.window_background
        };
        if scene.audio[row] {
            for sample in 0..24 {
                let x = clip.left() + clip.width() * (sample as f32 + 0.5) / 24.0;
                let amplitude = clip.height() * (0.1 + ((sample * 7 % 13) as f32 / 13.0) * 0.3);
                painter.vline(x, (clip.center().y - amplitude)..=(clip.center().y + amplitude), Stroke::new(0.8, ink));
            }
        } else {
            for note in 0..6 {
                let y = clip.top() + clip.height() * (0.25 + (note % 3) as f32 * 0.22);
                let x = clip.left() + clip.width() * (0.06 + note as f32 * 0.145);
                painter.line_segment([pos2(x, y), pos2(x + clip.width() * 0.09, y)], Stroke::new(1.1, ink));
            }
        }
        painter.rect_filled(cell(0.93, top + 0.03, 0.025, 0.12), 0.0, palette.meter_normal);
    }
    let playhead = cell(0.64, 0.18, 0.0, 0.78);
    painter.vline(playhead.left(), playhead.y_range(), Stroke::new(1.0, palette.accent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_keep_project_overrides_and_mix_them_over_each_theme_surface() {
        let mut project = crate::demo::demo_project();
        let track = suhina_engine::model::Rgb(210, 170, 60);
        let clip = suhina_engine::model::Rgb(55, 190, 160);
        project.tracks[0].color = Some(track);
        project.arrangement.lanes[0].clips[0].color = Some(clip);
        let original = project.clone();
        let scene = Scene::from_project(&project);
        assert_eq!(scene.tracks[0], Some(egui::Color32::from_rgb(210, 170, 60)));
        assert_eq!(scene.clips[0], Some(egui::Color32::from_rgb(55, 190, 160)));
        assert_eq!(scene.tracks[1], None, "an unset track follows the candidate theme");
        let light = Palette { panel_background: egui::Color32::from_rgb(245, 243, 238), ..Palette::DARK };
        for palette in [Palette::DARK, light] {
            let ctx = egui::Context::default();
            let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(240.0, 92.0));
            let mut output = ctx.run_ui(Default::default(), |ui| paint(ui.painter(), rect, &palette, &scene));
            for source in [egui::Color32::from_rgb(55, 190, 160), palette.midi_clip_default] {
                let expected = super::super::motion::mix(palette.panel_background, source, 0.82);
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Rect(shape) if shape.fill == expected)));
            }
            output.textures_delta.clear();
        }
        assert_eq!(project, original, "previewing never recolours the project");
    }

    #[test]
    fn preview_uses_its_explicit_palette_without_installing_it() {
        let ctx = egui::Context::default();
        let current = super::super::theme::palette();
        let preview = Palette { window_background: egui::Color32::from_rgb(24, 60, 90), ..current };
        let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(240.0, 92.0));
        let mut output = ctx.run_ui(Default::default(), |ui| paint(ui.painter(), rect, &preview, &Scene::default()));
        assert_eq!(super::super::theme::palette(), current);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(shape) if shape.fill == preview.window_background)));
        assert!(output.shapes.iter().all(|shape| shape.clip_rect == rect));
        output.textures_delta.clear();
    }
}
