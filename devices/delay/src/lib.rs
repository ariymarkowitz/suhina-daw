// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Delay: a stereo delay with one line per side, a band-pass inside the feedback loop, one LFO
//! that moves the delay time and the band, three ways to change the time (Repitch, Fade, Jump),
//! ping-pong and freeze. The design is in `docs/spec/delay.md`.
//!
//! Per side and frame: the line is read at the delay time, which gives the wet signal; what goes
//! back into the line is the input plus the wet signal times the feedback, through the band-pass,
//! so every repeat passes the filter once more. Ping-pong feeds each side's wet signal into the
//! other line and the mono sum of the input into the left one, so repeats alternate left and
//! right. Freeze writes the wet signal back unchanged instead (no input, no filter, gain 1), so
//! the lines hold what they have.
//!
//! The delay time is a number of sixteenth notes at the tempo, made longer or shorter by the
//! offset (sync), or a time in seconds. Repitch glides the time to a new value, so the pitch
//! bends as on a tape delay; Fade crossfades from the old time to the new one; Jump changes it at
//! once, in whole frames. The LFO swings the time by up to a quarter of it and the band's centre
//! by up to two octaves either way.

use suhina_device_api::{
    Device, DeviceError, DeviceInfo, ParamInfo, ProcessContext, Role, Scale, Unit, check_sample_rate,
};
use suhina_dsp::biquad::BUTTERWORTH_Q;
use suhina_dsp::lfo::{Shape, wander, wave};
use suhina_dsp::{Biquad, Coefficients, DelayLine, OnePole, Smoother};

/// Ramp for feedback, mix, freeze and filter changes.
pub const RAMP_SECONDS: f64 = 0.02;
/// Longest delay, the LFO's swing included; synced times at very low tempos stop here.
pub const MAX_DELAY_SECONDS: f64 = 8.0;
/// Fastest change of the delay time in Repitch, in seconds of delay per second: the pitch moves by
/// at most this fraction while the time glides.
pub const REPITCH_RATE: f64 = 0.5;
/// Time constant that rounds the start and end of a Repitch glide.
pub const REPITCH_SECONDS: f64 = 0.02;
/// Length of a Fade crossfade.
pub const FADE_SECONDS: f64 = 0.05;
/// How far the LFO at 100 % swings the delay time, as a fraction of it.
pub const LFO_TIME_SWING: f64 = 0.25;
/// How far the LFO at 100 % swings the band's centre, in octaves either way.
pub const LFO_FILTER_OCTAVES: f64 = 2.0;
/// Time constant that softens the edges of the square and sample-and-hold LFO shapes.
pub const LFO_EDGE_SECONDS: f64 = 0.002;
/// The band's corners stay between these frequencies.
pub const FILTER_RANGE: (f64, f64) = (50.0, 18_000.0);
/// Frames between filter coefficient updates while the band moves, counted from the last reset.
const CONTROL_FRAMES: u64 = 4;

/// Parameter indices, in [`INFO`] order.
pub mod param {
    pub const LINK: usize = 0;
    pub const SYNC_LEFT: usize = 1;
    pub const SYNC_RIGHT: usize = 2;
    pub const SIXTEENTHS_LEFT: usize = 3;
    pub const SIXTEENTHS_RIGHT: usize = 4;
    pub const TIME_LEFT: usize = 5;
    pub const TIME_RIGHT: usize = 6;
    pub const OFFSET_LEFT: usize = 7;
    pub const OFFSET_RIGHT: usize = 8;
    pub const FEEDBACK: usize = 9;
    pub const FREEZE: usize = 10;
    pub const PING_PONG: usize = 11;
    pub const TRANSITION: usize = 12;
    pub const FILTER: usize = 13;
    pub const FILTER_FREQUENCY: usize = 14;
    pub const FILTER_WIDTH: usize = 15;
    pub const LFO_MODE: usize = 16;
    pub const LFO_RATE: usize = 17;
    pub const LFO_TIME: usize = 18;
    pub const LFO_SYNCED: usize = 19;
    pub const LFO_SIXTEENTHS: usize = 20;
    pub const LFO_SHAPE: usize = 21;
    pub const LFO_MORPH: usize = 22;
    pub const LFO_TO_TIME: usize = 23;
    pub const LFO_TO_FILTER: usize = 24;
    pub const MIX: usize = 25;
    pub const EQUAL_LOUDNESS: usize = 26;
    pub(crate) const COUNT: usize = 27;
}

/// The sync buttons, and how many sixteenth notes each is.
const SIXTEENTHS: [(&str, f64); 8] =
    [("1", 1.0), ("2", 2.0), ("3", 3.0), ("4", 4.0), ("5", 5.0), ("6", 6.0), ("8", 8.0), ("16", 16.0)];
const SIXTEENTH_NAMES: [&str; 8] = names(SIXTEENTHS);

/// The synced LFO rates, and one cycle's length in bars.
const SYNCED_RATES: [(&str, f64); 22] = [
    ("8 Bars", 8.0),
    ("6 Bars", 6.0),
    ("4 Bars", 4.0),
    ("3 Bars", 3.0),
    ("2 Bars", 2.0),
    ("1.5 Bars", 1.5),
    ("1 Bar", 1.0),
    ("3/4", 0.75),
    ("1/2", 0.5),
    ("3/8", 0.375),
    ("1/3", 1.0 / 3.0),
    ("5/16", 0.3125),
    ("1/4", 0.25),
    ("3/16", 0.1875),
    ("1/6", 1.0 / 6.0),
    ("1/8", 0.125),
    ("1/12", 1.0 / 12.0),
    ("1/16", 0.0625),
    ("1/24", 1.0 / 24.0),
    ("1/32", 0.03125),
    ("1/48", 1.0 / 48.0),
    ("1/64", 0.015625),
];
const SYNCED_RATE_NAMES: [&str; 22] = names(SYNCED_RATES);

const fn names<const N: usize>(table: [(&'static str, f64); N]) -> [&'static str; N] {
    let mut names = [""; N];
    let mut i = 0;
    while i < N {
        names[i] = table[i].0;
        i += 1;
    }
    names
}

const TRANSITIONS: [&str; 3] = ["Repitch", "Fade", "Jump"];
const LFO_MODES: [&str; 6] = ["Rate", "Time", "Synced", "Triplet", "Dotted", "Sixteenths"];
const LFO_SHAPES: [&str; 7] = ["Sine", "Triangle", "Ramp Up", "Ramp Down", "Square", "Sample and Hold", "Wander"];

/// How the delay time follows a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transition {
    Repitch,
    Fade,
    Jump,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum LfoShape {
    /// A periodic wave, its cycle bent by Morph.
    Wave(Shape),
    Random,
    Wander,
}

/// The LFO's waveform and where Morph puts the middle of its cycle.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Lfo {
    shape: LfoShape,
    /// The first half of the waveform takes this much of the cycle, and the second half the rest.
    split: f64,
}

impl Lfo {
    /// The value, -1..1, at position `cycles`.
    #[inline]
    fn at(self, cycles: f64) -> f64 {
        match self.shape {
            LfoShape::Random => wave(Shape::Random, cycles),
            LfoShape::Wander => wander(cycles),
            LfoShape::Wave(shape) => {
                let whole = cycles.floor();
                let (phase, split) = (cycles - whole, self.split);
                let warped =
                    if phase < split { 0.5 * phase / split } else { 0.5 + 0.5 * (phase - split) / (1.0 - split) };
                wave(shape, whole + warped)
            }
        }
    }
}

const PARAMS: [ParamInfo; param::COUNT] = [
    ParamInfo::toggle("link", "Link", true),
    ParamInfo::toggle("syncLeft", "Sync Left", true),
    ParamInfo::toggle("syncRight", "Sync Right", true),
    ParamInfo::choice("sixteenthsLeft", "Sixteenths Left", &SIXTEENTH_NAMES, 2),
    ParamInfo::choice("sixteenthsRight", "Sixteenths Right", &SIXTEENTH_NAMES, 3),
    ParamInfo::continuous("timeLeft", "Time Left", (0.001, 5.0), 0.375, Unit::Seconds, Scale::Logarithmic),
    ParamInfo::continuous("timeRight", "Time Right", (0.001, 5.0), 0.375, Unit::Seconds, Scale::Logarithmic),
    ParamInfo::continuous("offsetLeft", "Offset Left", (-0.333, 0.333), 0.0, Unit::Percent, Scale::Linear),
    ParamInfo::continuous("offsetRight", "Offset Right", (-0.333, 0.333), 0.0, Unit::Percent, Scale::Linear),
    ParamInfo::continuous("feedback", "Feedback", (0.0, 0.95), 0.5, Unit::Percent, Scale::Linear),
    ParamInfo::toggle("freeze", "Freeze", false),
    ParamInfo::toggle("pingPong", "Ping Pong", false),
    ParamInfo::choice("transition", "Transition", &TRANSITIONS, 0),
    ParamInfo::toggle("filter", "Filter", true),
    ParamInfo::continuous(
        "filterFrequency",
        "Filter Frequency",
        FILTER_RANGE,
        1_000.0,
        Unit::Hertz,
        Scale::Logarithmic,
    ),
    ParamInfo::continuous("filterWidth", "Filter Width", (0.5, 9.0), 8.0, Unit::Octaves, Scale::Linear),
    ParamInfo::choice("lfoMode", "LFO Mode", &LFO_MODES, 0),
    ParamInfo::continuous("lfoRate", "LFO Rate", (0.01, 40.0), 0.5, Unit::Hertz, Scale::Logarithmic),
    ParamInfo::continuous("lfoTime", "LFO Time", (0.1, 200.0), 1.0, Unit::Seconds, Scale::Logarithmic),
    ParamInfo::choice("lfoSynced", "LFO Synced Rate", &SYNCED_RATE_NAMES, 4),
    ParamInfo::integer("lfoSixteenths", "LFO Sixteenths", (1, 64), 16, Unit::None),
    ParamInfo::choice("lfoShape", "LFO Shape", &LFO_SHAPES, 1),
    ParamInfo::continuous("lfoMorph", "LFO Morph", (-1.0, 1.0), 0.0, Unit::Percent, Scale::Linear),
    ParamInfo::continuous("lfoToTime", "LFO to Time", (0.0, 1.0), 0.0, Unit::Percent, Scale::Linear),
    ParamInfo::continuous("lfoToFilter", "LFO to Filter", (0.0, 1.0), 0.0, Unit::Percent, Scale::Linear),
    ParamInfo::continuous("mix", "Dry/Wet", (0.0, 1.0), 0.5, Unit::Percent, Scale::Linear),
    ParamInfo::toggle("equalLoudness", "Equal-Loudness Mix", false),
];

pub const INFO: DeviceInfo = DeviceInfo { id: "delay", name: "Delay", role: Role::Effect, params: &PARAMS };

const DEFAULT_SAMPLE_RATE: f64 = 48_000.0;
/// Tempo used when the host gives none that makes sense.
const FALLBACK_TEMPO: f64 = 120.0;

/// Where a side's line is read, and how the read position follows the delay time.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Reader {
    /// Repitch: the time after the rate limit, then after the rounding filter (frames).
    slewed: f64,
    glide: f64,
    /// Fade: the time being faded out and the one faded in, and how far the fade is (0..1; 1 when
    /// no fade runs).
    from: f64,
    to: f64,
    fade: f64,
}

impl Reader {
    /// A reader settled at `frames`, with no glide or fade running.
    fn at(frames: f64) -> Self {
        Self { slewed: frames, glide: frames, from: frames, to: frames, fade: 1.0 }
    }
}

/// One side: its line and band-pass.
#[derive(Debug, Clone, PartialEq, Default)]
struct Side {
    line: DelayLine,
    high_pass: Biquad,
    low_pass: Biquad,
    reader: Reader,
}

impl Side {
    fn new(sample_rate: f64) -> Self {
        Self { line: DelayLine::for_seconds(MAX_DELAY_SECONDS, sample_rate), ..Self::default() }
    }

    fn clear(&mut self) {
        self.line.clear();
        self.high_pass.reset();
        self.low_pass.reset();
        self.reader = Reader::default();
    }

    /// The wet sample at `target` frames of delay, the read position following it by `transition`.
    fn read(&mut self, target: f64, transition: Transition, fresh: bool, rates: &Rates) -> f32 {
        let r = &mut self.reader;
        if fresh {
            *r = Reader::at(target);
        }
        match transition {
            Transition::Repitch => {
                let slewed = r.slewed + (target - r.slewed).clamp(-REPITCH_RATE, REPITCH_RATE);
                let glide = slewed + (r.glide - slewed) * rates.repitch;
                *r = Reader { slewed, ..Reader::at(glide) };
                self.line.read_hermite(glide)
            }
            Transition::Fade => {
                if r.fade >= 1.0 && (target - r.to).abs() >= 1.0 {
                    r.from = r.to;
                    r.to = target;
                    r.fade = 0.0;
                }
                let (from, to) = (r.from, r.to);
                let x = if r.fade >= 1.0 {
                    self.line.read_hermite(to)
                } else {
                    r.fade = (r.fade + rates.fade_step).min(1.0);
                    let w = (0.5 - 0.5 * (std::f64::consts::PI * r.fade).cos()) as f32;
                    self.line.read_hermite(from) * (1.0 - w) + self.line.read_hermite(to) * w
                };
                r.slewed = r.to;
                r.glide = r.to;
                x
            }
            Transition::Jump => {
                let whole = target.round();
                *r = Reader::at(whole);
                self.line.read(whole as usize)
            }
        }
    }

    /// Writes the next sample into the line: `x`, through the band-pass in proportion to
    /// `filter`, or `held` unchanged in proportion to `freeze`. The band-pass always runs, so
    /// switching it crossfades.
    fn write(&mut self, x: f64, held: f32, filter: f32, freeze: f32) {
        let filtered = self.low_pass.process(self.high_pass.process(x));
        let fed = x + (filtered - x) * f64::from(filter);
        self.line.push(fed as f32 * (1.0 - freeze) + held * freeze);
    }
}

/// Per-frame constants from the sample rate.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rates {
    /// Repitch's rounding filter coefficient.
    repitch: f64,
    /// How far a fade moves per frame.
    fade_step: f64,
}

impl Rates {
    fn new(sample_rate: f64) -> Self {
        Self { repitch: (-1.0 / (REPITCH_SECONDS * sample_rate)).exp(), fade_step: 1.0 / (FADE_SECONDS * sample_rate) }
    }
}

/// The dry and wet gains for the mix and the share of the equal-loudness law, worked out again
/// only when one of them changes.
struct MixLaw {
    key: (f32, f32),
    gains: (f64, f64),
}

impl MixLaw {
    fn new() -> Self {
        // NaN never equals a key, so the first frame works the gains out.
        Self { key: (f32::NAN, f32::NAN), gains: (0.0, 0.0) }
    }

    #[inline]
    fn gains(&mut self, mix: f32, loudness: f32) -> (f64, f64) {
        if (mix, loudness) != self.key {
            self.key = (mix, loudness);
            let (mix, loudness) = (f64::from(mix), f64::from(loudness));
            let (sin, cos) = (mix * std::f64::consts::FRAC_PI_2).sin_cos();
            self.gains = ((1.0 - mix) + (cos - (1.0 - mix)) * loudness, mix + (sin - mix) * loudness);
        }
        self.gains
    }
}

pub struct Delay {
    sample_rate: f64,
    rates: Rates,
    values: [f64; param::COUNT],
    /// How much of the band-pass is heard (0 off, 1 on).
    filter: Smoother,
    feedback: Smoother,
    mix: Smoother,
    freeze: Smoother,
    /// log2 of the band's centre, and its width in octaves.
    centre: Smoother,
    width: Smoother,
    /// The LFO's depths on the time and the band.
    to_time: Smoother,
    to_filter: Smoother,
    /// How much of the equal-loudness mix law is used (0 linear, 1 equal loudness).
    loudness: Smoother,
    sides: [Side; 2],
    lfo_edge: OnePole,
    /// Free-running LFO position: whole cycles and the fraction of the current one.
    lfo_cycles: u64,
    lfo_phase: f64,
    /// Frames since the last reset, for the filter update grid.
    frame: u64,
    /// Whether the readers and the LFO filter still have to start at their first values.
    fresh: bool,
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

impl Delay {
    pub fn new() -> Self {
        let sr = DEFAULT_SAMPLE_RATE;
        let still = Smoother::new(0.0, 1);
        let mut delay = Self {
            sample_rate: sr,
            rates: Rates::new(sr),
            values: PARAMS.map(|p| p.default),
            filter: still,
            feedback: still,
            mix: still,
            freeze: still,
            centre: still,
            width: still,
            to_time: still,
            to_filter: still,
            loudness: still,
            sides: [Side::new(sr), Side::new(sr)],
            lfo_edge: OnePole::new(LFO_EDGE_SECONDS, sr),
            lfo_cycles: 0,
            lfo_phase: 0.0,
            frame: 0,
            fresh: true,
        };
        delay.rebuild();
        delay
    }

    fn value(&self, index: usize) -> f64 {
        self.values.get(index).copied().unwrap_or(0.0)
    }

    fn on(&self, index: usize) -> bool {
        self.value(index) >= 0.5
    }

    fn targets(&self) -> [f32; 9] {
        let flag = |index| if self.on(index) { 1.0 } else { 0.0 };
        [
            flag(param::FILTER),
            self.value(param::FEEDBACK) as f32,
            self.value(param::MIX) as f32,
            flag(param::FREEZE),
            self.value(param::FILTER_FREQUENCY).max(1.0).log2() as f32,
            self.value(param::FILTER_WIDTH) as f32,
            self.value(param::LFO_TO_TIME) as f32,
            self.value(param::LFO_TO_FILTER) as f32,
            flag(param::EQUAL_LOUDNESS),
        ]
    }

    fn smoothers(&mut self) -> [&mut Smoother; 9] {
        [
            &mut self.filter,
            &mut self.feedback,
            &mut self.mix,
            &mut self.freeze,
            &mut self.centre,
            &mut self.width,
            &mut self.to_time,
            &mut self.to_filter,
            &mut self.loudness,
        ]
    }

    fn rebuild(&mut self) {
        let ramp = Smoother::ramp_frames(RAMP_SECONDS, self.sample_rate);
        let targets = self.targets();
        for (smoother, target) in self.smoothers().into_iter().zip(targets) {
            smoother.set_ramp(ramp);
            smoother.reset(target);
        }
        for side in &mut self.sides {
            side.clear();
        }
        self.lfo_cycles = 0;
        self.lfo_phase = 0.0;
        self.frame = 0;
        self.fresh = true;
        self.tune_filters(0.0);
    }

    fn transition(&self) -> Transition {
        match self.value(param::TRANSITION) as usize {
            1 => Transition::Fade,
            2 => Transition::Jump,
            _ => Transition::Repitch,
        }
    }

    /// One side's delay time in seconds before the LFO, at `tempo`. With Link on, the right side
    /// takes the left side's settings except its offset.
    fn base_seconds(&self, right: bool, tempo: f64) -> f64 {
        let own = |l: usize, r: usize| if right { r } else { l };
        let linked = |l: usize, r: usize| if right && !self.on(param::LINK) { r } else { l };
        let seconds = if self.on(linked(param::SYNC_LEFT, param::SYNC_RIGHT)) {
            let step = self.value(linked(param::SIXTEENTHS_LEFT, param::SIXTEENTHS_RIGHT)) as usize;
            let sixteenths = SIXTEENTHS.get(step).map_or(1.0, |s| s.1);
            sixteenths * 15.0 / tempo * (1.0 + self.value(own(param::OFFSET_LEFT, param::OFFSET_RIGHT)))
        } else {
            self.value(linked(param::TIME_LEFT, param::TIME_RIGHT))
        };
        seconds.min(MAX_DELAY_SECONDS / (1.0 + LFO_TIME_SWING))
    }

    /// The LFO's length of one cycle in beats when it follows the tempo, or `None` when it runs in
    /// hertz or seconds.
    fn lfo_beats(&self) -> Option<f64> {
        let synced = || SYNCED_RATES.get(self.value(param::LFO_SYNCED) as usize).map_or(4.0, |r| r.1 * 4.0);
        match self.value(param::LFO_MODE) as usize {
            2 => Some(synced()),
            3 => Some(synced() * 2.0 / 3.0),
            4 => Some(synced() * 1.5),
            5 => Some(self.value(param::LFO_SIXTEENTHS) * 0.25),
            _ => None,
        }
    }

    fn lfo_hertz(&self) -> f64 {
        match self.value(param::LFO_MODE) as usize {
            1 => 1.0 / self.value(param::LFO_TIME).max(1e-3),
            _ => self.value(param::LFO_RATE),
        }
    }

    /// The LFO's waveform. Morph moves the middle of the cycle: the first half of the waveform
    /// takes (1 + morph) / 2 of the cycle and the second half the rest.
    fn lfo(&self) -> Lfo {
        let shape = match self.value(param::LFO_SHAPE) as usize {
            1 => LfoShape::Wave(Shape::Triangle),
            2 => LfoShape::Wave(Shape::SawUp),
            3 => LfoShape::Wave(Shape::SawDown),
            4 => LfoShape::Wave(Shape::Square),
            5 => LfoShape::Random,
            6 => LfoShape::Wander,
            _ => LfoShape::Wave(Shape::Sine),
        };
        Lfo { shape, split: ((1.0 + self.value(param::LFO_MORPH)) / 2.0).clamp(0.02, 0.98) }
    }

    /// Band-pass coefficients for the current (smoothed) centre and width, moved by `lfo`
    /// (-1..1).
    fn tune_filters(&mut self, lfo: f64) {
        let sr = self.sample_rate;
        let swing = lfo * f64::from(self.to_filter.current()) * LFO_FILTER_OCTAVES;
        let centre = f64::from(self.centre.current()) + swing;
        let half = f64::from(self.width.current()) / 2.0;
        let top = FILTER_RANGE.1.min(sr * 0.49);
        let low = 2.0f64.powf(centre - half).clamp(FILTER_RANGE.0, top);
        let high = 2.0f64.powf(centre + half).clamp(FILTER_RANGE.0, top);
        let (hp, lp) = (Coefficients::highpass(sr, low, BUTTERWORTH_Q), Coefficients::lowpass(sr, high, BUTTERWORTH_Q));
        for side in &mut self.sides {
            side.high_pass.set(hp);
            side.low_pass.set(lp);
        }
    }
}

impl Device for Delay {
    fn info(&self) -> &'static DeviceInfo {
        &INFO
    }

    fn prepare(&mut self, sample_rate: f64) -> Result<(), DeviceError> {
        let sr = check_sample_rate(sample_rate)?;
        self.sample_rate = sr;
        self.rates = Rates::new(sr);
        self.sides = [Side::new(sr), Side::new(sr)];
        self.lfo_edge = OnePole::new(LFO_EDGE_SECONDS, sr);
        self.rebuild();
        Ok(())
    }

    fn reset(&mut self) {
        self.rebuild();
    }

    fn set_param(&mut self, index: usize, value: f64) {
        let Some(info) = PARAMS.get(index) else { return };
        let Some(slot) = self.values.get_mut(index) else { return };
        *slot = info.clamp(value);
        let targets = self.targets();
        for (smoother, target) in self.smoothers().into_iter().zip(targets) {
            smoother.set_target(target);
        }
    }

    fn param(&self, index: usize) -> Option<f64> {
        self.values.get(index).copied()
    }

    fn process(&mut self, context: &ProcessContext, buffer: &mut [f32]) {
        let sr = self.sample_rate;
        let tempo = if context.tempo.is_finite() && context.tempo > 0.0 {
            context.tempo.clamp(1.0, 1_000.0)
        } else {
            FALLBACK_TEMPO
        };
        let beats_per_frame = tempo / 60.0 / sr;
        let lfo_beats = self.lfo_beats();
        let lfo_wave = self.lfo();
        let follows_song = lfo_beats.is_some() && context.playing && context.beat.is_finite();
        let cycles_per_frame = lfo_beats.map_or(self.lfo_hertz() / sr, |beats| beats_per_frame / beats.max(1e-6));
        let bases = [self.base_seconds(false, tempo) * sr, self.base_seconds(true, tempo) * sr];
        let transition = self.transition();
        let ping_pong = self.on(param::PING_PONG);
        let longest = MAX_DELAY_SECONDS * sr;
        let mut law = MixLaw::new();

        for (n, frame) in buffer.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let cycles = match lfo_beats {
                Some(beats) if follows_song => (context.beat + n as f64 * beats_per_frame) / beats.max(1e-6),
                _ => self.lfo_cycles as f64 + self.lfo_phase,
            };
            let raw = lfo_wave.at(cycles);
            if self.fresh {
                self.lfo_edge.reset(raw);
            }
            let lfo = self.lfo_edge.process(raw);
            let (filter, feedback, mix, freeze) =
                (self.filter.next_value(), self.feedback.next_value(), self.mix.next_value(), self.freeze.next_value());
            let band_moves = !(self.centre.is_settled() && self.width.is_settled() && self.to_filter.is_settled())
                || self.to_filter.current() > 0.0;
            self.centre.next_value();
            self.width.next_value();
            let to_time = f64::from(self.to_time.next_value());
            self.to_filter.next_value();
            // The band is retuned every few frames while it moves, so it glides without zipper
            // noise; a settled band keeps its coefficients.
            if band_moves && self.frame.is_multiple_of(CONTROL_FRAMES) {
                self.tune_filters(lfo);
            }

            let fresh = self.fresh;
            let swing = 1.0 + LFO_TIME_SWING * to_time * lfo;
            let [left, right] = &mut self.sides;
            let wet = [
                left.read((bases[0] * swing).clamp(2.0, longest), transition, fresh, &self.rates),
                right.read((bases[1] * swing).clamp(2.0, longest), transition, fresh, &self.rates),
            ];
            let input = frame.map(|s| if s.is_finite() { f64::from(s) } else { 0.0 });
            let fb = f64::from(feedback);
            if ping_pong {
                left.write((input[0] + input[1]) / 2.0 + fb * f64::from(wet[1]), wet[1], filter, freeze);
                right.write(fb * f64::from(wet[0]), wet[0], filter, freeze);
            } else {
                left.write(input[0] + fb * f64::from(wet[0]), wet[0], filter, freeze);
                right.write(input[1] + fb * f64::from(wet[1]), wet[1], filter, freeze);
            }

            let (dry_gain, wet_gain) = law.gains(mix, self.loudness.next_value());
            for ((sample, x), y) in frame.iter_mut().zip(input).zip(wet) {
                *sample = (x * dry_gain + f64::from(y) * wet_gain) as f32;
            }

            self.fresh = false;
            self.frame = self.frame.wrapping_add(1);
            if !follows_song {
                self.lfo_phase += cycles_per_frame;
                if self.lfo_phase >= 1.0 {
                    self.lfo_cycles = self.lfo_cycles.wrapping_add(self.lfo_phase as u64) % (1 << 32);
                    self.lfo_phase = self.lfo_phase.fract();
                }
            }
        }
    }
}

#[cfg(test)]
mod golden_tests;
#[cfg(test)]
mod tests;
