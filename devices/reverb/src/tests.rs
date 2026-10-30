// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use suhina_device_api::conformance;

use super::*;

suhina_device_api::conformance_test!(Reverb::new());

/// A reverb at `sample_rate`, fully wet, no modulation, then `settings`.
fn reverb(sample_rate: f64, settings: &[(usize, f64)]) -> Reverb {
    let mut r = Reverb::new();
    for &(index, value) in [(param::MIX, 1.0), (param::CHORUS, 0.0), (param::SPIN, 0.0)].iter().chain(settings) {
        r.set_param(index, value);
    }
    let _ = r.prepare(sample_rate);
    r
}

/// The response to an impulse of 0.5 on both sides, `seconds` long, in blocks of 256 frames.
fn impulse_response(r: &mut Reverb, sample_rate: f64, seconds: f64) -> Vec<[f32; 2]> {
    let frames = (seconds * sample_rate) as usize;
    let mut buffer = vec![0.0f32; frames * 2];
    buffer[0] = 0.5;
    buffer[1] = 0.5;
    for block in buffer.chunks_mut(512) {
        r.process(&ProcessContext::default(), block);
    }
    buffer.as_chunks::<2>().0.to_vec()
}

/// Energy (both sides) of `ir` from `from` to `to` seconds.
fn energy(ir: &[[f32; 2]], sample_rate: f64, from: f64, to: f64) -> f64 {
    let range = (from * sample_rate) as usize..(to * sample_rate) as usize;
    ir.get(range).unwrap_or_default().iter().map(|[l, r]| f64::from(*l).powi(2) + f64::from(*r).powi(2)).sum()
}

/// Decay time from the energy between two windows of the tail.
fn t60(ir: &[[f32; 2]], sample_rate: f64) -> f64 {
    let (a, b) = (energy(ir, sample_rate, 0.3, 0.5), energy(ir, sample_rate, 1.3, 1.5));
    60.0 / (10.0 * (a / b).log10())
}

fn correlation(ir: &[[f32; 2]], sample_rate: f64) -> f64 {
    let window = ir.get((0.3 * sample_rate) as usize..(1.5 * sample_rate) as usize).unwrap_or_default();
    let (mut ll, mut rr, mut lr) = (0.0f64, 0.0f64, 0.0f64);
    for [l, r] in window {
        let (l, r) = (f64::from(*l), f64::from(*r));
        ll += l * l;
        rr += r * r;
        lr += l * r;
    }
    lr / (ll * rr).sqrt()
}

#[test]
fn the_tail_decays_in_the_decay_time_at_every_sample_rate() {
    // The network is tuned to 0.9 of the setting, as the reference renders decay.
    for decay in [1.0, 3.0] {
        let mut times = Vec::new();
        for sr in [44_100.0, 96_000.0] {
            let mut r =
                reverb(sr, &[(param::DECAY, decay), (param::REFLECT, MIN_LEVEL_DB), (param::HIGH_DAMPING, 0.0)]);
            let ir = impulse_response(&mut r, sr, 1.6);
            times.push(t60(&ir, sr));
        }
        for t in &times {
            assert!((t / (decay * DECAY_RATIO) - 1.0).abs() < 0.1, "decay {decay}: {times:?}");
        }
        assert!((times[0] / times[1] - 1.0).abs() < 0.05, "decay {decay}: {times:?}");
    }
}

#[test]
fn the_measured_network_decays_in_the_decay_time_at_every_diffusion() {
    // Every path fades with its whole time: the allpasses' echoes and the plain delays too, so
    // even the shortest decay falls as tuned once the first arrivals are out (Decision 20).
    let sr = 44_100.0;
    for diffusion in [0.001, 0.6, 0.96] {
        let mut r = reverb(
            sr,
            &[
                (param::DENSITY, LOW as f64),
                (param::DECAY, 0.2),
                (param::DIFFUSION, diffusion),
                (param::REFLECT, MIN_LEVEL_DB),
                (param::HIGH_DAMPING, 0.0),
                (param::LOW_DAMPING, 0.0),
            ],
        );
        let ir = impulse_response(&mut r, sr, 0.6);
        let (a, b) = (energy(&ir, sr, 0.15, 0.25), energy(&ir, sr, 0.35, 0.45));
        let t60 = 60.0 * 0.2 / (10.0 * (a / b).log10());
        assert!((t60 / (0.2 * DECAY_RATIO) - 1.0).abs() < 0.12, "diffusion {diffusion}: {t60}");
    }
}

#[test]
fn the_smallest_room_still_decays_at_the_longest_decay() {
    // At Size 0.22 and Decay 60 s a trip through a line loses under 0.1 %, so no line may pass
    // more than it takes in, or the mode that lives in it grows without end.
    let sr = 44_100.0;
    let mut r = reverb(
        sr,
        &[
            (param::DENSITY, LOW as f64),
            (param::SIZE, 0.22),
            (param::SMOOTH, 0.0),
            (param::DECAY, 60.0),
            (param::DIFFUSION, 0.001),
            (param::REFLECT, MIN_LEVEL_DB),
            (param::HIGH_DAMPING, 0.0),
            (param::LOW_DAMPING, 0.0),
        ],
    );
    let ir = impulse_response(&mut r, sr, 30.0);
    let (early, late) = (energy(&ir, sr, 4.0, 6.0), energy(&ir, sr, 24.0, 26.0));
    let fall = 10.0 * (early / late).log10() / 20.0;
    assert!(fall > 0.8, "the tail falls {fall} dB/s from 5 to 25 s");
}

/// A 1 kHz sine burst, 0.2 s long, then silence, `seconds` in all.
fn burst(sample_rate: f64, seconds: f64) -> Vec<f32> {
    let frames = (seconds * sample_rate) as usize;
    (0..frames)
        .flat_map(|n| {
            let t = n as f64 / sample_rate;
            let v = if t < 0.2 { (std::f64::consts::TAU * 1_000.0 * t).sin() as f32 * 0.5 } else { 0.0 };
            [v, v]
        })
        .collect()
}

#[test]
fn levels_match_across_sample_rates() {
    // Energy per second of the response to the same sound, at two rates.
    let levels: Vec<f64> = [44_100.0, 96_000.0]
        .map(|sr| {
            let mut r = reverb(sr, &[]);
            let mut signal = burst(sr, 1.5);
            for block in signal.chunks_mut(512) {
                r.process(&ProcessContext::default(), block);
            }
            let frames = signal.as_chunks::<2>().0.to_vec();
            10.0 * (energy(&frames, sr, 0.2, 1.5) / sr).log10()
        })
        .to_vec();
    assert!((levels[0] - levels[1]).abs() < 0.5, "{levels:?}");
}

#[test]
fn stereo_turns_two_uncorrelated_sides_into_one() {
    let sr = 44_100.0;
    for (stereo, expected) in [(120.0, 0.0), (100.0, (std::f64::consts::FRAC_PI_4 / 6.0 * 2.0).sin()), (0.0, 1.0)] {
        let mut r = reverb(sr, &[(param::STEREO, stereo), (param::REFLECT, MIN_LEVEL_DB)]);
        let c = correlation(&impulse_response(&mut r, sr, 1.6), sr);
        assert!((c - expected).abs() < 0.1, "stereo {stereo}: {c} against {expected}");
    }
}

#[test]
fn reflect_and_diffuse_are_independent_gains() {
    let sr = 44_100.0;
    let tail = |reflect: f64| {
        let mut r = reverb(sr, &[(param::REFLECT, reflect)]);
        energy(&impulse_response(&mut r, sr, 1.0), sr, 0.3, 1.0)
    };
    // The tail after the reflections does not change with Reflect.
    assert!((tail(MIN_LEVEL_DB) / tail(6.0) - 1.0).abs() < 1e-6);
    let early = |diffuse: f64| {
        let mut r = reverb(sr, &[(param::DIFFUSE, diffuse), (param::SHAPE, 1.0)]);
        energy(&impulse_response(&mut r, sr, 0.1), sr, 0.0, 0.03)
    };
    // The reflections before the tail starts do not change with Diffuse.
    assert!((early(MIN_LEVEL_DB) / early(6.0) - 1.0).abs() < 1e-6);
}

#[test]
fn the_dry_signal_is_not_delayed_and_the_mix_is_equal_power() {
    let sr = 44_100.0;
    let mut r = reverb(sr, &[(param::MIX, 0.55)]);
    let ir = impulse_response(&mut r, sr, 0.01);
    let dry = (0.55 * std::f64::consts::FRAC_PI_2).cos() * 0.5;
    assert!((f64::from(ir[0][0]) - dry).abs() < 1e-6, "{:?}", ir[0]);
    assert_eq!(r.latency(), 0);
}

#[test]
fn the_first_reflection_arrives_after_the_predelay() {
    let sr = 48_000.0;
    for predelay in [0.0005, 0.01, 0.25] {
        let mut r = reverb(sr, &[(param::PREDELAY, predelay), (param::DIFFUSE, MIN_LEVEL_DB)]);
        let ir = impulse_response(&mut r, sr, 0.3);
        let first = ir.iter().skip(1).position(|[l, r]| l.abs().max(r.abs()) > 1e-4).map(|n| n + 1);
        // A frame (at 44.1 kHz) before the predelay, and High Cut's own delay later, as measured.
        let expected = ((predelay + TAP_OFFSET_SECONDS + HIGH_CUT_DELAY_SECONDS) * sr).round() as usize;
        assert!(first.is_some_and(|n| n.abs_diff(expected) <= 2), "{predelay}: {first:?} against {expected}");
    }
}

#[test]
fn predelay_lowers_the_wet_signal_in_inverse_proportion_to_the_decay() {
    // Live's drop from 2.5 ms to 250 ms of predelay, in the first 0.4 s after the first
    // reflection (reflections and tail together): 5.6, 2.3 and 0.8 dB at Decay 0.5, 1.2 and
    // 3.46 s.
    let sr = 44_100.0;
    for (decay, drop) in [(0.5, 5.6), (1.2, 2.3), (3.46, 0.8)] {
        let wet = |predelay: f64| {
            let mut r = reverb(sr, &[(param::PREDELAY, predelay), (param::DECAY, decay)]);
            let ir = impulse_response(&mut r, sr, predelay + 0.5);
            energy(&ir, sr, predelay, predelay + 0.4)
        };
        let measured = 10.0 * (wet(0.0025) / wet(0.25)).log10();
        assert!((measured - drop).abs() < 0.5, "Decay {decay}: {measured:.2} dB against {drop}");
    }
}

/// When one reflection arrives after each click of a train (one every 0.2 s for 9.6 s), in
/// seconds after its click: the loudest frame within `half` seconds of `around` on `side`,
/// refined by a parabola. Reflections only, with Spin at `amount` and 0.3 Hz.
fn arrivals(sr: f64, predelay: f64, amount: f64, side: usize, around: f64, half: f64) -> Vec<f64> {
    let mut r = reverb(sr, &[(param::PREDELAY, predelay), (param::DIFFUSE, MIN_LEVEL_DB), (param::DECAY, 0.2)]);
    r.set_param(param::SPIN, 1.0);
    r.set_param(param::SPIN_AMOUNT, amount);
    r.set_param(param::SPIN_RATE, 0.3);
    let period = (0.2 * sr) as usize;
    let mut buffer = vec![0.0f32; period * 48 * 2];
    for click in 0..48 {
        buffer[click * period * 2] = 0.5;
        buffer[click * period * 2 + 1] = 0.5;
    }
    for block in buffer.chunks_mut(512) {
        r.process(&ProcessContext::default(), block);
    }
    let frames = buffer.as_chunks::<2>().0;
    let (from, to) = (((around - half) * sr) as usize, ((around + half) * sr) as usize);
    (0..48)
        .map(|click| {
            let level = |n: usize| frames.get(click * period + n).map_or(0.0, |f| f64::from(f[side].abs()));
            let peak = (from..to).max_by(|&a, &b| level(a).total_cmp(&level(b))).unwrap_or(from);
            let (a, b, c) = (level(peak - 1), level(peak), level(peak + 1));
            let bend = a - 2.0 * b + c;
            (peak as f64 + if bend == 0.0 { 0.0 } else { 0.5 * (a - c) / bend }) / sr
        })
        .collect()
}

/// Half the spread of `times`, in ms.
fn swing_ms(times: &[f64]) -> f64 {
    let (lo, hi) = times.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &t| (lo.min(t), hi.max(t)));
    (hi - lo) * 500.0
}

#[test]
fn spin_reaches_less_far_as_the_predelay_grows() {
    // Live's swing of each reflection's whole delay per unit of Amount, tracked tap by tap (in
    // % of the delay): the same up to 5 ms of predelay, then falling as 2 ms over the predelay
    // less 3 ms, whatever Amount, Rate and Size.
    for (predelay_ms, share) in [
        (0.5, 0.994),
        (2.5, 0.994),
        (5.0, 0.994),
        (6.5, 0.567),
        (8.0, 0.398),
        (10.0, 0.285),
        (25.0, 0.0907),
        (50.0, 0.0425),
        (100.0, 0.0206),
        (250.0, 0.0081),
    ] {
        let ours = SPIN_FRACTION * spin_reach(predelay_ms) * 100.0;
        assert!((ours / share - 1.0).abs() < 0.03, "{predelay_ms} ms: {ours:.4} % against {share}");
    }
}

#[test]
fn spin_swings_each_reflection_by_its_whole_delay() {
    // Live's left reflection at 11.43 ms (Amount 17.5) swings 2.08 ms either way at 0.5 ms of
    // predelay and 0.40 ms at 100 ms; the first, right at the predelay, 0.09 and 0.36 ms.
    let sr = 44_100.0;
    for (predelay, side, tap, live) in
        [(0.0005, 0, 11.429, 2.08), (0.1, 0, 11.429, 0.40), (0.0005, 1, 0.0, 0.09), (0.1, 1, 0.0, 0.36)]
    {
        let at = predelay + TAP_OFFSET_SECONDS + tap / 1_000.0;
        let ours = swing_ms(&arrivals(sr, predelay, 17.5, side, at + HIGH_CUT_DELAY_SECONDS, 0.003));
        let expected = 17.5 * SPIN_FRACTION * spin_reach(predelay * 1_000.0) * at * 1_000.0;
        assert!(
            (ours / expected - 1.0).abs() < 0.1 || (ours - expected).abs() < 0.03,
            "{predelay} s, {tap} ms: {ours:.3} ms against {expected:.3}"
        );
        assert!((ours - live).abs() < 0.1 + 0.05 * live, "{predelay} s, {tap} ms: {ours:.3} ms against Live's {live}");
    }
}

#[test]
fn filtered_reflections_swing_with_the_direct_one_they_follow() {
    // The left filtered reflection at 30.27 ms is the first (right) one 30.27 ms later: Live's
    // hardly moves at 0.5 ms of predelay, where one swinging by its own delay would move 1.5 ms
    // either way at Amount 5.
    let sr = 44_100.0;
    let at = 0.0005 + TAP_OFFSET_SECONDS + HIGH_CUT_DELAY_SECONDS + 0.030_272;
    let ours = swing_ms(&arrivals(sr, 0.0005, 5.0, 0, at, 0.0003));
    assert!(ours < 0.05, "{ours:.3} ms");
}

#[test]
fn every_filtered_reflection_follows_exactly_one_direct_one() {
    // As Live's renders with Spin show, at every Density: the faint right taps at 63.95 ms (Mid)
    // and 56.58 ms (High) follow the ones at 7.37 and 0 ms, and of the two taps at 64.44 ms the
    // left follows the right one at 34.17 ms and the right the left one at 11.43 ms.
    for taps in TAP_TABLES {
        for tap in taps.iter().filter(|tap| tap.filtered) {
            let matches = FILTERED_OFFSETS
                .iter()
                .filter(|&&(_, side)| side == tap.side)
                .flat_map(|&(frames, _)| {
                    let offset = frames as f64 * 1_000.0 / REFERENCE_RATE;
                    taps.iter().filter(move |d| !d.filtered && (tap.ms - d.ms - offset).abs() < 0.0015)
                })
                .count();
            assert_eq!(matches, 1, "{tap:?}");
        }
    }
    // Times to within a frame (each table time carries its fraction of a frame).
    let source = |taps: &[Tap], ms: f64, side: usize| {
        let k = taps.iter().position(|t| t.filtered && t.side == side && (t.ms - ms).abs() < 0.02);
        match k.map(|k| routes(taps)[k]) {
            Some(Route::Follows { source, .. }) => taps[source].ms,
            _ => f64::NAN,
        }
    };
    for (taps, ms, side, expected) in [
        (&TAPS_MID[..], 63.946, RIGHT, 7.370),
        (&TAPS_HIGH[..], 56.576, RIGHT, 0.0),
        (&TAPS_HIGH[..], 64.444, LEFT, 34.172),
        (&TAPS_HIGH[..], 64.444, RIGHT, 11.429),
    ] {
        let found = source(taps, ms, side);
        assert!((found - expected).abs() < 0.02, "{ms} ms: {found} against {expected}");
    }
}

#[test]
fn sparse_and_low_play_their_own_reflections() {
    let mut r = reverb(44_100.0, &[(param::DENSITY, 0.0)]);
    let mut block = vec![0.0f32; 64];
    r.process(&ProcessContext::default(), &mut block);
    assert_eq!(r.taps.len(), TAPS_SPARSE.len());
    r.set_param(param::DENSITY, 1.0);
    r.process(&ProcessContext::default(), &mut block);
    assert_eq!(r.taps.len(), TAPS_LOW.len());
}

#[test]
fn spin_starts_again_when_playback_starts() {
    // Live's LFOs have the same phases whenever playback starts, from bar 1 or bar 2 alike.
    let mut r = reverb(44_100.0, &[(param::SPIN, 1.0)]);
    let stopped = ProcessContext::default();
    let playing = ProcessContext { playing: true, ..stopped };
    let mut block = vec![0.0f32; 512 * 2];
    for _ in 0..100 {
        r.process(&stopped, &mut block);
    }
    assert_ne!(r.spin_phases, spin_start());
    r.process(&playing, &mut block);
    let step = 0.3 * 512.0 / 44_100.0;
    for ((ours, start), (_, rate, _)) in r.spin_phases.iter().zip(spin_start()).zip(SPIN_VOICES) {
        assert!((ours - (start + step * rate)).abs() < 1e-9, "{ours} against {start}");
    }
    // Playing on does not start it again.
    let before = r.spin_phases;
    r.process(&playing, &mut block);
    assert_ne!(r.spin_phases, before);
    assert!(r.spin_phases.iter().zip(before).all(|(p, b)| p > &b));
}

#[test]
fn the_reflections_fade_with_shape_and_decay_as_measured() {
    // The left reflection at 41.13 ms against the one at 11.43 ms, both read on whole frames at
    // the default predelay, so they keep the same shape through the input filters.
    let sr = 44_100.0;
    for (shape, decay) in [(0.0, 1.2), (0.5, 1.2), (1.0, 0.5)] {
        let settings =
            [(param::SHAPE, shape), (param::DECAY, decay), (param::DIFFUSE, MIN_LEVEL_DB), (param::SIZE, 100.0)];
        let mut r = reverb(sr, &settings);
        let ir = impulse_response(&mut r, sr, 0.06);
        let tap = |ms: f64| {
            let at = ((0.0025 + TAP_OFFSET_SECONDS + HIGH_CUT_DELAY_SECONDS + ms / 1_000.0) * sr).round() as usize;
            ir.get(at - 2..at + 3).unwrap_or_default().iter().map(|[l, _]| l.abs()).fold(0.0f32, f32::max)
        };
        let fall = 20.0 * f64::from(tap(41.134) / tap(11.429)).log10();
        let expected = -(TAP_FADE + TAP_FADE_PER_SHAPE * shape) / decay * (41.134 - 11.429);
        assert!((fall - expected).abs() < 0.05, "shape {shape}, decay {decay}: {fall} against {expected}");
    }
}

#[test]
fn freeze_holds_the_tail_and_cut_keeps_new_input_out() {
    let sr = 44_100.0;
    let mut r = reverb(sr, &[(param::REFLECT, MIN_LEVEL_DB), (param::DECAY, 0.5)]);
    let run = |r: &mut Reverb, signal: &mut [f32]| {
        for block in signal.chunks_mut(512) {
            r.process(&ProcessContext::default(), block);
        }
    };
    let level = |b: &[f32]| 10.0 * (b.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / b.len() as f64).log10();
    // A burst, and Freeze while its tail is still loud: the tail then holds its level.
    let mut start = burst(sr, 0.3);
    run(&mut r, &mut start);
    r.set_param(param::FREEZE, 1.0);
    let mut held = vec![0.0f32; 44_100 * 2 * 2];
    run(&mut r, &mut held);
    // Over 0.4 s or more: a sine's frozen tail beats between the modes near it.
    let (early, late) = (level(&held[17_640..52_920]), level(&held[123_480..]));
    // Nearly lossless, but the energy spreads evenly over the lines, which moves the output a
    // little (Live's frozen tail rises up to 2.7 dB the same way).
    assert!((early - late).abs() < 3.0, "{early} dB then {late} dB");
    // With Cut on, another burst while frozen adds nothing to the tail.
    let mut more = burst(sr, 1.0);
    run(&mut r, &mut more);
    let after = level(&more[44_100..]);
    assert!((after - late).abs() < 2.0, "{late} dB, then {after} dB after more input");
}

#[test]
fn a_frozen_tail_keeps_its_damping_with_flat_off() {
    // Frozen with Flat off, high damping goes on taking the highs, as Live's does (8 kHz falls
    // 14 dB/s, 1 kHz 0.2); with Flat on the highs fall barely faster than the mids.
    let sr = 44_100.0;
    let band = |flat: f64| {
        let settings = [(param::REFLECT, MIN_LEVEL_DB), (param::FLAT, flat), (param::CUT, 0.0), (param::MIX, 1.0)];
        let mut r = reverb(sr, &settings);
        r.set_param(param::FREEZE, 1.0);
        let mut signal = conformance::test_signal(4_410);
        signal.resize(44_100 * 2 * 4, 0.0);
        for block in signal.chunks_mut(512) {
            r.process(&ProcessContext::default(), block);
        }
        // The level above 6 kHz against the level below it (dB), 0.1 s and 1 s in: with Flat off
        // the highs are mostly gone by then.
        let highs = |from: usize| {
            let left: Vec<f64> =
                signal[from * 2..(from + 22_050) * 2].iter().step_by(2).map(|v| f64::from(*v)).collect();
            let (mut high, mut all) = (0.0, 0.0);
            // A first difference is a crude high-pass (it weights the mids in too, so the gap is
            // smaller than the highs' own), enough to tell the two settings apart.
            for w in left.windows(2) {
                high += (w[1] - w[0]).powi(2);
                all += w[1].powi(2);
            }
            10.0 * (high / all).log10()
        };
        highs(4_410) - highs(44_100)
    };
    let (flat, unflat) = (band(1.0), band(0.0));
    assert!(flat < 2.0, "Flat on: the highs fell {flat} dB more than the rest");
    assert!(unflat > flat + 1.0, "Flat off: the highs fell {unflat} dB more than the rest, against {flat}");
}

#[test]
fn silence_decays_to_exact_silence() {
    let sr = 44_100.0;
    let mut r = reverb(sr, &[(param::DECAY, 0.2), (param::CHORUS, 1.0), (param::SPIN, 1.0)]);
    let mut signal = conformance::test_signal(4_410);
    signal.resize(44_100 * 2 * 20, 0.0);
    for block in signal.chunks_mut(512) {
        r.process(&ProcessContext::default(), block);
    }
    assert!(signal[signal.len() - 44_100..].iter().all(|v| *v == 0.0));
}

/// Largest step from one frame to the next, on either side.
fn largest_step(signal: &[f32]) -> f32 {
    let frames = signal.as_chunks::<2>().0;
    frames.windows(2).map(|w| (w[1][0] - w[0][0]).abs().max((w[1][1] - w[0][1]).abs())).fold(0.0, f32::max)
}

#[test]
fn moving_any_control_does_not_click() {
    // A low sine through the reverb, 50 % wet; each parameter jumps from one end to the other in
    // the middle. The output must not step much further than the sine and the tail do anyway.
    let sr = 44_100.0;
    let sine = |frames: usize| -> Vec<f32> {
        (0..frames)
            .flat_map(|n| {
                let v = (std::f64::consts::TAU * 110.0 * n as f64 / sr).sin() as f32 * 0.5;
                [v, v]
            })
            .collect()
    };
    let render = |index: usize, jump: Option<f64>| {
        let mut r = reverb(sr, &[(param::MIX, 0.5), (param::SMOOTH, 2.0)]);
        if let Some(info) = PARAMS.get(index) {
            r.set_param(index, info.min);
        }
        let _ = r.prepare(sr);
        let mut signal = sine(44_100);
        let (first, second) = signal.split_at_mut(22_050 * 2);
        for block in first.chunks_mut(512) {
            r.process(&ProcessContext::default(), block);
        }
        if let (Some(value), Some(info)) = (jump, PARAMS.get(index)) {
            let _ = info;
            r.set_param(index, value);
        }
        for block in second.chunks_mut(512) {
            r.process(&ProcessContext::default(), block);
        }
        largest_step(signal.get(22_050 * 2 - 2_000..22_050 * 2 + 8_000).unwrap_or_default())
    };
    for (index, info) in PARAMS.iter().enumerate() {
        // Switching Density on or off lines, and Freeze, change the sound by design.
        if matches!(index, param::DENSITY | param::FREEZE | param::SMOOTH) {
            continue;
        }
        let still = render(index, None);
        let moved = render(index, Some(info.max));
        assert!(moved < still * 1.5 + 0.01, "{}: a step of {moved} against {still}", info.id);
    }
}

/// The cost of one instance at its defaults: run with `cargo test --release -p suhina-device-reverb
/// cost -- --ignored --nocapture`. The budget (docs/spec/reverb.md › Cost) is checked by hand, as CI
/// machines vary too much for a timing test.
#[test]
#[ignore]
fn cost_of_a_minute_at_48_khz() {
    let mut device = Reverb::new();
    let _ = device.prepare(48_000.0);
    let mut signal = conformance::test_signal(48_000 * 60);
    let start = std::time::Instant::now();
    for block in signal.chunks_mut(512) {
        device.process(&ProcessContext::default(), block);
    }
    let seconds = start.elapsed().as_secs_f64();
    println!("60 s at 48 kHz in {seconds:.3} s: {:.2} % of a core", seconds / 60.0 * 100.0);
}
