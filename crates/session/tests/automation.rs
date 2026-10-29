// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Automation commands (#395): points on mixer and device lanes, undo, turning a lane off by a
//! manual change and on again, and malformed parameters.

use serde_json::json;
use suhina_model::automation::Curve;
use suhina_model::*;
use suhina_session::Session;

use crate::support::run;
use suhina_test_support::ProjectBuilder;

fn project() -> Project {
    suhina_test_support::with_ids(project_built())
}

fn project_built() -> Project {
    ProjectBuilder::new().midi_track(1, "Keys").build_without_ids()
}

fn lanes(session: &Session) -> &[AutomationLane] {
    &session.project().channels[0].automation
}

fn sd_device() -> Device {
    let mut graph =
        suhina_device_sdk::DeviceGraph::new("test.gain", "Test Gain", suhina_device_sdk::DeviceRole::Effect);
    graph.parameters.push(suhina_device_sdk::Parameter::new("gain", "Gain", (0.0, 2.0), 0.5));
    graph.nodes = vec![
        suhina_device_sdk::Node::audio_input("in"),
        suhina_device_sdk::Node::gain("gain", "gain"),
        suhina_device_sdk::Node::audio_output("out"),
    ];
    graph.connections =
        vec![suhina_device_sdk::Connection::new("in", "gain"), suhina_device_sdk::Connection::new("gain", "out")];
    Device {
        name: "Test Gain".into(),
        kind: DeviceKind::Sd,
        enabled: true,
        state: graph.to_sd().ok().map(String::into_bytes),
        params: Vec::new(),
        automation: Vec::new(),
        extensions: Extensions::default(),
    }
}

/// A curve is given as two control points; a point added without a value goes on the envelope
/// and splits a bent segment so that it plays as before.
#[test]
fn a_point_added_on_a_bent_segment_splits_it_exactly() {
    let mut session = Session::new(project());
    let lopsided = json!([0.1, 0.7, 0.825, 0.525]);
    for (beat, value) in [(0, -24), (8, 0)] {
        let point = json!({ "track": 1, "param": "volume", "beat": beat, "value": value, "curve": lopsided });
        run(&mut session, "automation.addPoint", point).unwrap();
    }
    let before = lanes(&session)[0].clone();
    run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "volume", "beat": 3 })).unwrap();
    let after = &lanes(&session)[0];
    assert_eq!(after.points.len(), 3);
    for beat in [1.0, 3.0, 5.5] {
        let (old, new) = (
            before.value_at(beat, false, &automation::VolumeTaper),
            after.value_at(beat, false, &automation::VolumeTaper),
        );
        assert!((old.unwrap() - new.unwrap()).abs() < 1e-9, "{beat}");
    }
    let list = run(&mut session, "automation.list", json!({ "track": 1 })).unwrap();
    assert_eq!(list["mixer"][0]["points"][2]["curve"], lopsided);
}

/// An S whose points run back in time, split by a point on the envelope, plays as before: its
/// halves lie outside the unit square, unclamped, and are listed and taken back as they are.
#[test]
fn a_split_s_curve_plays_as_before_with_halves_outside_the_square() {
    let mut session = Session::new(project());
    for (beat, value) in [(0.0, -1.0), (8.0, 1.0)] {
        let point = json!({ "track": 1, "param": "pan", "beat": beat, "value": value, "curve": [1, 0, 0, 1] });
        run(&mut session, "automation.addPoint", point).unwrap();
    }
    let before = lanes(&session)[0].clone();
    run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 6 })).unwrap();
    run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 1 })).unwrap();
    let after = lanes(&session)[0].clone();
    assert_eq!(after.points.len(), 4);
    // The 0.75 cut's first half had x1 = 1.1958; cut again at beat 1, its second half keeps x1
    // above 1.
    assert!(after.points[1].curve.x1 > 1.1, "{:?}", after.points[1]);
    for q in 0..=640 {
        let beat = q as f64 / 80.0;
        let (old, new) =
            (before.value_at(beat, false, &automation::Plain), after.value_at(beat, false, &automation::Plain));
        // Beat 4 is the S's upright middle, where rounding alone moves the value by about 1e-5.
        let tolerance = if (beat - 4.0).abs() < 1e-9 { 1e-4 } else { 1e-9 };
        assert!((old.unwrap() - new.unwrap()).abs() < tolerance, "{beat}: {old:?} against {new:?}");
    }
    let list = run(&mut session, "automation.list", json!({ "track": 1 })).unwrap();
    let listed = list["mixer"][0]["points"][1]["curve"].clone();
    assert_eq!(listed, json!(after.points[1].curve.to_array()));
    let back = json!({ "track": 1, "param": "pan", "point": 0, "curve": listed });
    run(&mut session, "automation.movePoint", back).unwrap();
    assert_eq!(lanes(&session)[0].points[0].curve, after.points[1].curve);
}

/// A point added without a value keeps a level or straight segment exactly, holds the value
/// outside the points and on a step, and at a point's beat takes that point's curve; a lane
/// without points needs a value.
#[test]
fn a_point_added_on_the_envelope_keeps_it_as_it_was() {
    let mut session = Session::new(project());
    let add = |session: &mut Session, param: &str, beat: f64| {
        run(session, "automation.addPoint", json!({ "track": 1, "param": param, "beat": beat })).map(|_| ())
    };
    assert!(add(&mut session, "pan", 2.0).unwrap_err().to_string().contains("give a value"));
    for (param, beat, value, curve) in [
        ("pan", 0, -1.0, json!(0.6)),
        ("pan", 8, 1.0, json!(0)),
        ("mute", 0, 0.0, json!(0)),
        ("mute", 8, 1.0, json!(0)),
    ] {
        let point = json!({ "track": 1, "param": param, "beat": beat, "value": value, "curve": curve });
        run(&mut session, "automation.addPoint", point).unwrap();
    }
    let before = lanes(&session)[0].clone();
    for beat in [0.0, 12.0] {
        add(&mut session, "pan", beat).unwrap();
    }
    add(&mut session, "mute", 4.0).unwrap();
    let (pan, mute) = (&lanes(&session)[0], &lanes(&session)[1]);
    assert_eq!(
        pan.points.iter().map(|p| (p.beat, p.value)).collect::<Vec<_>>(),
        [(0.0, -1.0), (0.0, -1.0), (8.0, 1.0), (12.0, 1.0)]
    );
    assert_eq!(pan.points[1].curve, Curve::bend(0.6), "the bend stays with the segment");
    for beat in [1.0, 4.0, 10.0] {
        assert_eq!(pan.value_at(beat, false, &automation::Plain), before.value_at(beat, false, &automation::Plain));
    }
    assert_eq!(mute.points[1], AutomationPoint::new(4.0, 0.0));

    // On a level or straight segment the value is the envelope's and the lane plays as before.
    let mut session = Session::new(project());
    for (param, value) in [("pan", 0.5), ("volume", -24.0)] {
        for beat in [0, 8] {
            let value = if beat == 8 && param == "volume" { 0.0 } else { value };
            let point = json!({ "track": 1, "param": param, "beat": beat, "value": value });
            run(&mut session, "automation.addPoint", point).unwrap();
        }
    }
    let before = lanes(&session).to_vec();
    add(&mut session, "pan", 3.0).unwrap();
    add(&mut session, "volume", 2.0).unwrap();
    assert_eq!(lanes(&session)[0].points[1], AutomationPoint::new(3.0, 0.5));
    for beat in [1.0, 2.0, 5.0] {
        let (old, new) = (
            before[1].value_at(beat, false, &automation::VolumeTaper),
            lanes(&session)[1].value_at(beat, false, &automation::VolumeTaper),
        );
        assert!((old.unwrap() - new.unwrap()).abs() < 1e-12, "{beat}");
    }
}

#[test]
fn mixer_points_are_added_moved_and_deleted_with_undo() {
    let mut session = Session::new(project());
    let added =
        run(&mut session, "automation.addPoint", json!({ "track": "Keys", "param": "volume", "beat": 4, "value": 0 }))
            .unwrap();
    assert_eq!((added["point"].as_u64(), added["points"].as_u64()), (Some(0), Some(1)));
    // In beat order: a point before the first goes first.
    let added = run(
        &mut session,
        "automation.addPoint",
        json!({ "track": 1, "param": "volume", "beat": 0, "value": -150.5, "curve": 0.5 }),
    )
    .unwrap();
    assert_eq!(added["point"], 0);
    run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 2, "value": -1 })).unwrap();
    let volume = &lanes(&session)[0];
    assert_eq!(volume.param, "volume");
    assert_eq!(
        volume.points,
        [AutomationPoint { beat: 0.0, value: 0.0, curve: Curve::bend(0.5) }, AutomationPoint::new(4.0, 1.0)]
    );

    // Moving the first point past the second puts it second.
    let moved = run(
        &mut session,
        "automation.movePoint",
        json!({ "track": 1, "param": "volume", "point": 0, "beat": 8, "value": -6 }),
    )
    .unwrap();
    assert_eq!(moved["point"], 1);
    assert_eq!(moved["value"], -6.0);
    assert_eq!(lanes(&session)[0].points[1].beat, 8.0);
    assert_eq!(lanes(&session)[0].points[1].curve, Curve::bend(0.5), "the curve stays");

    let list = run(&mut session, "automation.list", json!({ "track": 1 })).unwrap();
    assert_eq!(list["mixer"][0]["points"][1]["value"], -6.0, "the volume in dB");
    assert_eq!(list["mixer"][1]["param"], "pan");
    assert_eq!(list["devices"], json!([]));

    // Deleting the last point removes the lane.
    run(&mut session, "automation.deletePoint", json!({ "track": 1, "param": "pan", "point": 0 })).unwrap();
    assert_eq!(lanes(&session).len(), 1);
    let cleared = run(&mut session, "automation.clear", json!({ "track": 1 })).unwrap();
    assert_eq!(cleared["cleared"], 1);
    assert!(lanes(&session).is_empty());

    // Every edit was one undo step.
    for _ in 0..6 {
        run(&mut session, "edit.undo", json!({})).unwrap();
    }
    assert_eq!(session.project(), &project());
}

/// A whole-number parameter's lane, placed as the engine places it: a value between two whole
/// numbers sits where it falls, unrounded.
struct WholeNumbers(suhina_device_api::ParamInfo);

impl automation::Taper for WholeNumbers {
    fn position(&self, value: f64) -> f64 {
        self.0.to_normalized(value)
    }
    fn value(&self, position: f64) -> f64 {
        self.0.min + position * (self.0.max - self.0.min)
    }
}

/// A point added without a value on a whole-number lane keeps the envelope: it stores the value
/// between two numbers (the engine cuts the fraction off only as it plays), and the segments on
/// either side move toward it, not toward a rounded one.
#[test]
fn a_point_added_on_a_whole_number_lane_keeps_the_envelope() {
    let mut session = Session::new(project());
    run(&mut session, "device.add", json!({ "track": 1, "id": "sampler" })).unwrap();
    let poly = suhina_device_registry::info("sampler")
        .and_then(|info| info.params.iter().find(|p| p.id == "poly").copied())
        .unwrap();
    assert!(matches!(poly.kind, suhina_device_api::ParamKind::Integer));
    let taper = WholeNumbers(poly);
    let point = |beat: f64, value: Option<f64>| {
        let mut params = json!({ "track": 1, "device": 0, "param": "poly", "beat": beat });
        if let Some(value) = value {
            params["value"] = json!(value);
        }
        params
    };
    for (beat, value) in [(0.0, 1.0), (8.0, 4.0)] {
        run(&mut session, "automation.addPoint", point(beat, Some(value))).unwrap();
    }
    let lane = |session: &Session| session.project().channels[0].devices[0].automation[0].clone();
    let before = lane(&session);
    run(&mut session, "automation.addPoint", point(4.0, None)).unwrap();
    let after = lane(&session);
    assert_eq!(after.points.iter().map(|p| p.value).collect::<Vec<_>>(), [1.0, 2.5, 4.0]);
    for beat in [1.0, 2.0, 4.0, 6.0, 7.5] {
        let (old, new) = (before.value_at(beat, false, &taper), after.value_at(beat, false, &taper));
        assert!((old.unwrap() - new.unwrap()).abs() < 1e-9, "{beat}: {old:?} against {new:?}");
    }
    assert!((after.value_at(2.0, false, &taper).unwrap() - 1.75).abs() < 1e-9);
    let list = run(&mut session, "automation.list", json!({ "track": 1 })).unwrap();
    assert_eq!(list["devices"][0]["lanes"][0]["points"][1]["value"], json!(2.5), "{list}");
}

#[test]
fn a_manual_change_turns_a_lane_off_until_it_is_on_again() {
    let mut session = Session::new(project());
    run(&mut session, "device.add", json!({ "track": 1, "id": "utility" })).unwrap();
    run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "volume", "beat": 0, "value": -12 }))
        .unwrap();
    run(
        &mut session,
        "automation.addPoint",
        json!({ "track": 1, "device": 0, "param": "gain", "beat": 0, "value": 99 }),
    )
    .unwrap();
    // Kept in the parameter's range.
    assert_eq!(session.project().channels[0].devices[0].automation[0].points[0].value, 35.0);

    let set = run(&mut session, "track.setVolume", json!({ "tracks": [1], "db": -3 })).unwrap();
    assert_eq!(set["automation"], "off");
    assert!(!lanes(&session)[0].enabled);
    // Once off, another change says nothing more.
    let set = run(&mut session, "track.setVolume", json!({ "tracks": [1], "db": -4 })).unwrap();
    assert!(set.get("automation").is_none());
    let set =
        run(&mut session, "device.setParam", json!({ "track": 1, "device": 0, "param": "gain", "value": -6 })).unwrap();
    assert_eq!(set["automation"], "off");
    assert!(!session.project().channels[0].devices[0].automation[0].enabled);
    // A parameter without a lane is not touched.
    let set = run(&mut session, "track.setPan", json!({ "tracks": [1], "pan": 0.5 })).unwrap();
    assert!(set.get("automation").is_none());

    let list = run(&mut session, "automation.list", json!({ "track": 1 })).unwrap();
    assert_eq!(list["mixer"][0]["enabled"], false);
    assert_eq!(list["devices"][0]["lanes"][0]["enabled"], false);
    run(&mut session, "automation.setEnabled", json!({ "track": 1, "param": "volume", "enabled": true })).unwrap();
    run(&mut session, "automation.setEnabled", json!({ "track": 1, "device": 0, "param": "gain", "enabled": true }))
        .unwrap();
    assert!(lanes(&session)[0].enabled);
    assert!(session.project().channels[0].devices[0].automation[0].enabled);
    // The value set by hand stays, under the lane.
    assert_eq!(session.project().channels[0].devices[0].params, [DeviceParam::new("gain", -6.0)]);
}

#[test]
fn device_set_automation_takes_built_in_devices() {
    let mut session = Session::new(project());
    run(&mut session, "device.add", json!({ "track": 1, "id": "utility" })).unwrap();
    let set = run(
        &mut session,
        "device.setAutomation",
        json!({ "track": 1, "device": 0, "param": "gain", "points": [4, -6, 0, -100] }),
    )
    .unwrap();
    assert_eq!((set["param"].as_str(), set["points"].as_u64()), (Some("gain"), Some(2)));
    let lane = &session.project().channels[0].devices[0].automation[0];
    // In beat order, limited to the range.
    assert_eq!(lane.points.iter().map(|p| (p.beat, p.value)).collect::<Vec<_>>(), [(0.0, -70.0), (4.0, -6.0)]);
    assert!(
        run(&mut session, "device.setAutomation", json!({ "track": 1, "device": 0, "param": "nope", "points": [] }))
            .is_err()
    );
}

#[test]
fn sd_automation_uses_stable_ids_and_the_parameter_range() {
    let mut p = project();
    p.channels[0].devices.push(sd_device());
    let mut session = Session::new(p);
    let set = run(
        &mut session,
        "device.setAutomation",
        json!({ "track": 1, "device": 0, "param": "gain", "points": [4, 3, 0, -1] }),
    )
    .unwrap();
    assert_eq!(set["param"], "gain");
    assert_eq!(
        session.project().channels[0].devices[0].automation[0]
            .points
            .iter()
            .map(|p| (p.beat, p.value))
            .collect::<Vec<_>>(),
        [(0.0, 0.0), (4.0, 2.0)]
    );
    run(
        &mut session,
        "automation.addPoint",
        json!({ "track": 1, "device": 0, "param": "gain", "beat": 2, "value": 1.25 }),
    )
    .unwrap();
    assert_eq!(
        run(&mut session, "automation.list", json!({ "track": 1 })).unwrap()["devices"][0]["lanes"][0]["param"],
        "gain"
    );
    let set =
        run(&mut session, "device.setParam", json!({ "track": 1, "device": 0, "param": "gain", "value": 1 })).unwrap();
    assert_eq!(set["automation"], "off");
    assert!(session.project().channels[0].devices[0].automation.iter().all(|lane| !lane.enabled));
}

#[test]
fn malformed_automation_edits_are_refused_and_change_nothing() {
    let mut session = Session::new(project());
    run(&mut session, "device.add", json!({ "track": 1, "id": "utility" })).unwrap();
    run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 0, "value": 0 })).unwrap();
    let before = session.project().clone();
    for (id, params) in [
        ("automation.addPoint", json!({ "track": 1, "param": "loudness", "beat": 0, "value": 0 })),
        ("automation.addPoint", json!({ "track": 9, "param": "pan", "beat": 0, "value": 0 })),
        ("automation.addPoint", json!({ "track": 1, "param": "pan", "beat": -1, "value": 0 })),
        ("automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 1e300, "value": 0 })),
        ("automation.addPoint", json!({ "track": 1, "param": "pan", "beat": "soon", "value": 0 })),
        ("automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 0, "value": 2 })),
        ("automation.addPoint", json!({ "track": 1, "param": "volume", "beat": 0, "value": 7 })),
        ("automation.addPoint", json!({ "track": 1, "param": "mute", "beat": 0, "value": 0.5, "curve": 2 })),
        ("automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 0, "value": 0, "curve": "round" })),
        (
            "automation.addPoint",
            json!({ "track": 1, "param": "pan", "beat": 0, "value": 0, "curve": [0, 1.6, 0.5, 0.5] }),
        ),
        (
            "automation.addPoint",
            json!({ "track": 1, "param": "pan", "beat": 0, "value": 0, "curve": [-0.6, 0, 0.5, 0.5] }),
        ),
        ("automation.addPoint", json!({ "track": 1, "device": 1, "param": "gain", "beat": 0, "value": 0 })),
        ("automation.addPoint", json!({ "track": 1, "device": 0, "param": "volume", "beat": 0, "value": 0 })),
        ("automation.addPoint", json!({ "track": 1, "device": -1, "param": "gain", "beat": 0, "value": 0 })),
        ("automation.addPoint", json!({ "track": 1, "beat": 0, "value": 0 })),
        ("automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 0, "value": 0, "colour": 1 })),
        ("automation.movePoint", json!({ "track": 1, "param": "pan", "point": 1, "beat": 2 })),
        ("automation.movePoint", json!({ "track": 1, "param": "pan", "point": 0 })),
        ("automation.movePoint", json!({ "track": 1, "param": "volume", "point": 0, "beat": 2 })),
        ("automation.deletePoint", json!({ "track": 1, "param": "pan", "point": 5 })),
        ("automation.deletePoint", json!({ "track": 1, "param": "pan", "point": "first" })),
        ("automation.setEnabled", json!({ "track": 1, "param": "volume", "enabled": false })),
        ("automation.setEnabled", json!({ "track": 1, "param": "pan", "enabled": "no" })),
        ("automation.clear", json!({ "track": 1, "device": 4 })),
        ("automation.list", json!({ "track": "Nobody" })),
    ] {
        assert!(run(&mut session, id, params.clone()).is_err(), "{id} {params}");
        assert_eq!(session.project(), &before, "{id} {params}");
    }
    // One undo step for each good edit (the device and the point).
    run(&mut session, "edit.undo", json!({})).unwrap();
    run(&mut session, "edit.undo", json!({})).unwrap();
    assert!(run(&mut session, "edit.undo", json!({})).is_err());
}

#[test]
fn a_lane_holds_a_limited_number_of_points() {
    let mut p = project();
    let mut lane = AutomationLane::new("pan");
    lane.points = (0..automation::MAX_POINTS).map(|i| AutomationPoint::new(i as f64 * 0.001, 0.0)).collect();
    p.channels[0].automation.push(lane);
    let mut session = Session::new(p);
    let full = run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 0, "value": 0 }));
    assert!(full.unwrap_err().contains("at most"));
}

#[test]
fn several_points_move_together_in_one_undo_step() {
    let mut session = Session::new(project());
    for (beat, value) in [(0, -12), (4, -6), (8, 0)] {
        run(
            &mut session,
            "automation.addPoint",
            json!({ "track": 1, "param": "volume", "beat": beat, "value": value }),
        )
        .unwrap();
    }
    let before = session.project().clone();
    // The first two move two beats later and 3 dB up; the first passes no other point.
    let moved = run(
        &mut session,
        "automation.movePoints",
        json!({ "track": 1, "param": "volume", "points": [0, 1], "beats": 2, "value": 3 }),
    )
    .unwrap();
    assert_eq!(moved["points"], json!([0, 1]));
    let beats: Vec<f64> = lanes(&session)[0].points.iter().map(|p| p.beat).collect();
    assert_eq!(beats, [2.0, 6.0, 8.0]);
    let db = |v: f64| (20.0 * v.log10() * 100.0).round() / 100.0;
    assert_eq!(db(lanes(&session)[0].points[1].value), -3.0);
    // The top of the fader holds a point that would go past it.
    run(&mut session, "automation.movePoints", json!({ "track": 1, "param": "volume", "points": [2], "value": 10 }))
        .unwrap();
    assert!((db(lanes(&session)[0].points[2].value) - 6.0).abs() < 0.01);
    run(&mut session, "edit.undo", json!({})).unwrap();
    run(&mut session, "edit.undo", json!({})).unwrap();
    assert_eq!(session.project(), &before);
}

#[test]
fn moved_points_keep_beat_order_and_report_their_new_places() {
    let mut session = Session::new(project());
    for (beat, value) in [(0.0, 0.0), (4.0, 0.5), (8.0, -0.5)] {
        run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": beat, "value": value }))
            .unwrap();
    }
    // The first point jumps past the others; values stay in the pan's range.
    let moved = run(
        &mut session,
        "automation.movePoints",
        json!({ "track": 1, "param": "pan", "points": [0], "beats": 10, "value": -2 }),
    )
    .unwrap();
    assert_eq!(moved["points"], json!([2]));
    let points: Vec<(f64, f64)> = lanes(&session)[0].points.iter().map(|p| (p.beat, p.value)).collect();
    assert_eq!(points, [(4.0, 0.5), (8.0, -0.5), (10.0, -1.0)]);
}

/// A point dragged onto the beat of the next one stays before it, so the lane jumps from the
/// dragged point's value to the other's, as drawn while dragging, not the other way round; a
/// point dragged back onto the beat of the one before stays after it, and changing only the
/// value of the first of two points at one beat keeps their order.
#[test]
fn a_point_moved_onto_another_points_beat_keeps_its_side() {
    let pans =
        |session: &Session| -> Vec<(f64, f64)> { lanes(session)[0].points.iter().map(|p| (p.beat, p.value)).collect() };
    let mut session = Session::new(project());
    for (beat, value) in [(0.0, -0.5), (4.0, 0.5)] {
        run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": beat, "value": value }))
            .unwrap();
    }
    let moved = run(&mut session, "automation.movePoint", json!({ "track": 1, "param": "pan", "point": 0, "beat": 4 }))
        .unwrap();
    assert_eq!(moved["point"], json!(0));
    assert_eq!(pans(&session), [(4.0, -0.5), (4.0, 0.5)]);
    // Only the value of the first: the order stays.
    run(&mut session, "automation.movePoint", json!({ "track": 1, "param": "pan", "point": 0, "value": -0.25 }))
        .unwrap();
    assert_eq!(pans(&session), [(4.0, -0.25), (4.0, 0.5)]);
    // A point that comes from later goes after the ones at its beat.
    run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 8, "value": 1 })).unwrap();
    run(&mut session, "automation.movePoint", json!({ "track": 1, "param": "pan", "point": 2, "beat": 4 })).unwrap();
    assert_eq!(pans(&session), [(4.0, -0.25), (4.0, 0.5), (4.0, 1.0)]);
    // A selection moved later goes before the points at its beat, and a value change of a
    // selection keeps the order.
    let mut session = Session::new(project());
    for (beat, value) in [(0.0, -0.5), (4.0, 0.5)] {
        run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": beat, "value": value }))
            .unwrap();
    }
    run(&mut session, "automation.movePoints", json!({ "track": 1, "param": "pan", "points": [0], "beats": 4 }))
        .unwrap();
    assert_eq!(pans(&session), [(4.0, -0.5), (4.0, 0.5)]);
    let moved =
        run(&mut session, "automation.movePoints", json!({ "track": 1, "param": "pan", "points": [0], "value": 0.25 }))
            .unwrap();
    assert_eq!(moved["points"], json!([0]));
    assert_eq!(pans(&session), [(4.0, -0.25), (4.0, 0.5)]);
}

/// Two selected points at one beat (a jump) moved together onto a third point's beat keep their
/// order, whatever order the request names them in, and the places reported follow the request.
#[test]
fn points_at_one_beat_moved_together_keep_their_jump() {
    let pans =
        |session: &Session| -> Vec<(f64, f64)> { lanes(session)[0].points.iter().map(|p| (p.beat, p.value)).collect() };
    let mut session = Session::new(project());
    for (beat, value) in [(0.0, -0.5), (2.0, 0.5), (4.0, 1.0)] {
        run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": beat, "value": value }))
            .unwrap();
    }
    // A jump at beat 0, from -0.5 to 0.5.
    run(&mut session, "automation.movePoint", json!({ "track": 1, "param": "pan", "point": 1, "beat": 0 })).unwrap();
    assert_eq!(pans(&session), [(0.0, -0.5), (0.0, 0.5), (4.0, 1.0)]);
    // Moved later onto the third point's beat, named last first.
    let moved =
        run(&mut session, "automation.movePoints", json!({ "track": 1, "param": "pan", "points": [1, 0], "beats": 4 }))
            .unwrap();
    assert_eq!(moved["points"], json!([1, 0]));
    assert_eq!(pans(&session), [(4.0, -0.5), (4.0, 0.5), (4.0, 1.0)]);
    // And back earlier onto a point at beat 0: after it, still in their order.
    run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 0, "value": 0 })).unwrap();
    assert_eq!(pans(&session)[0], (0.0, 0.0));
    let moved = run(
        &mut session,
        "automation.movePoints",
        json!({ "track": 1, "param": "pan", "points": [2, 1], "beats": -4 }),
    )
    .unwrap();
    assert_eq!(moved["points"], json!([2, 1]));
    assert_eq!(pans(&session), [(0.0, 0.0), (0.0, -0.5), (0.0, 0.5), (4.0, 1.0)]);
}

#[test]
fn several_points_are_deleted_in_one_undo_step_and_the_lane_goes_with_the_last() {
    let mut session = Session::new(project());
    for beat in [0, 2, 4] {
        run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "mute", "beat": beat, "value": 1 }))
            .unwrap();
    }
    run(&mut session, "automation.deletePoints", json!({ "track": 1, "param": "mute", "points": [2, 0] })).unwrap();
    assert_eq!(lanes(&session)[0].points.len(), 1);
    assert_eq!(lanes(&session)[0].points[0].beat, 2.0);
    run(&mut session, "automation.deletePoints", json!({ "track": 1, "param": "mute", "points": [0] })).unwrap();
    assert!(lanes(&session).is_empty());
    run(&mut session, "edit.undo", json!({})).unwrap();
    run(&mut session, "edit.undo", json!({})).unwrap();
    assert_eq!(lanes(&session)[0].points.len(), 3);
}

#[test]
fn malformed_point_lists_change_nothing() {
    let mut session = Session::new(project());
    for beat in [0, 2] {
        run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": beat, "value": 0 }))
            .unwrap();
    }
    let before = session.project().clone();
    for (id, params) in [
        ("automation.movePoints", json!({ "track": 1, "param": "pan", "points": [0], "beats": -1 })),
        ("automation.movePoints", json!({ "track": 1, "param": "pan", "points": [0, 0], "beats": 1 })),
        ("automation.movePoints", json!({ "track": 1, "param": "pan", "points": [5], "beats": 1 })),
        ("automation.movePoints", json!({ "track": 1, "param": "pan", "points": [0] })),
        ("automation.movePoints", json!({ "track": 1, "param": "pan", "points": [], "beats": 1 })),
        ("automation.movePoints", json!({ "track": 1, "param": "pan", "points": [-1], "beats": 1 })),
        ("automation.movePoints", json!({ "track": 1, "param": "pan", "points": [0], "curve": 2 })),
        ("automation.deletePoints", json!({ "track": 1, "param": "pan", "points": [2] })),
        ("automation.deletePoints", json!({ "track": 1, "param": "pan", "points": "0" })),
        ("automation.deletePoints", json!({ "track": 1, "param": "volume", "points": [0] })),
    ] {
        assert!(run(&mut session, id, params.clone()).is_err(), "{id} {params}");
    }
    assert_eq!(session.project(), &before);
}

/// A curve whose points lie beyond the box can overshoot its own points; two halves could not
/// play as it does there, so a point is not added on it without a value.
#[test]
fn a_point_is_not_added_on_a_segment_that_overshoots_its_points() {
    let mut session = Session::new(project());
    let overshoot = json!([0.3, 1.5, 0.7, 1.5]);
    for (beat, value) in [(0, -1), (8, 1)] {
        let point = json!({ "track": 1, "param": "pan", "beat": beat, "value": value, "curve": overshoot });
        run(&mut session, "automation.addPoint", point).unwrap();
    }
    let before = lanes(&session)[0].clone();
    let refused = run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 4 }));
    assert!(refused.is_err());
    assert_eq!(lanes(&session)[0], before);
    // A cut before the overshoot is refused too when a half's points would need clamping.
    let refused = run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 1.925 }));
    assert!(refused.is_err());
    assert_eq!(lanes(&session)[0], before);
    // A value given, or the beat of a point, still works.
    run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 4, "value": 0 })).unwrap();
}

/// A half that cannot move is level only if its control points are: a curve that rises before
/// reaching the cut value of zero is not split into a straight first half.
#[test]
fn a_point_is_not_added_where_a_half_would_lose_its_rise() {
    let mut session = Session::new(project());
    let dips = json!([0.25, 1.0 / 6.0, 0.75, -0.5]);
    for (beat, value) in [(0, -1), (8, 1)] {
        let point = json!({ "track": 1, "param": "pan", "beat": beat, "value": value, "curve": dips });
        run(&mut session, "automation.addPoint", point).unwrap();
    }
    let before = lanes(&session)[0].clone();
    let refused = run(&mut session, "automation.addPoint", json!({ "track": 1, "param": "pan", "beat": 4 }));
    assert!(refused.is_err());
    assert_eq!(lanes(&session)[0], before);
}
