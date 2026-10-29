// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The late reverb of Density Low, as measured (docs/spec/reverb.md › Decisions): seven loop
//! lines mixed by one orthogonal matrix, fed by four combinations of the direct reflections, some
//! of them through plain delays first.
//!
//! ```text
//! direct taps -> 6 groups -> 4 patterns -+-----------------------------------> loop lines
//!                                        +-> 4 plain delays (feed forward) --> loop lines
//! loop lines: A B C D F G (delay + allpass) and a 1641-frame plain delay, each line's output
//! mixed by MIX into every line's input; A and C are heard on the right, B, D and F on the left
//! ```
//!
//! Times are frames at 44.1 kHz in the reference room (Size 100). A line is a delay of the first
//! length and an allpass of the second times Scale (%), both scaled with the room's size factor;
//! the input reaches the lines [`ENTRY_MS`] after the first direct reflection, whatever the Size.

use suhina_dsp::{Allpass, DelayLine};

use crate::{Damper, Damping, Line};

/// Lines in the loop.
pub(crate) const LOOP: usize = 7;
/// Plain delays in front of the loop.
pub(crate) const PLAIN: usize = 4;
/// Patterns the direct reflections feed the network as.
const PATTERNS: usize = 4;
/// Groups of direct reflections the patterns are made of.
pub(crate) const GROUPS: usize = 6;
/// The frame rate the lengths below are at.
const RATE: f64 = 44_100.0;

/// The Scales (%) each loop line's length was measured at.
const SCALES: [f64; 6] = [5.0, 20.0, 40.0, 60.0, 80.0, 100.0];
/// Each loop line's whole length, delay and allpass, at each of [`SCALES`] in the reference room:
/// Live's allpasses grow in steps of 8 frames, so a line is up to 4 frames off a straight line in
/// Scale, and between the measured Scales it is read off the straight line between them. The
/// order is A, B, C, D, F, G and the plain 1641-frame line in front of C.
const LENGTHS: [[f64; 6]; LOOP] = [
    [2330.0, 2402.0, 2506.0, 2602.0, 2706.0, 2810.0],
    [2778.0, 2922.0, 3114.0, 3314.0, 3506.0, 3706.0],
    [3433.0, 3537.0, 3673.0, 3817.0, 3953.0, 4089.0],
    [3842.0, 4322.0, 4962.0, 5594.0, 6234.0, 6866.0],
    [6866.0, 7130.0, 7482.0, 7826.0, 8178.0, 8530.0],
    [7470.1, 7494.2, 7526.1, 7558.1, 7582.2, 7606.1],
    [1641.0, 1641.0, 1641.0, 1641.0, 1641.0, 1641.0],
];
/// How much of each loop line is its allpass: frames per % of Scale (the slope of [`LENGTHS`]).
const ALLPASS_RATES: [f64; LOOP] = [5.050, 9.768, 6.921, 31.832, 17.493, 1.444, 0.0];

/// Line `k`'s whole length at Scale `percent` in the reference room (frames).
fn length_at(k: usize, percent: f64) -> f64 {
    let lengths = LENGTHS[k];
    let i = SCALES.iter().rposition(|&s| s <= percent).unwrap_or(0).min(SCALES.len() - 2);
    let t = (percent - SCALES[i]) / (SCALES[i + 1] - SCALES[i]);
    lengths[i] + (lengths[i + 1] - lengths[i]) * t
}
/// Whether each loop line has an allpass (the 1641-frame line has none).
const DIFFUSED: [bool; LOOP] = [true, true, true, true, true, true, false];
/// The plain delays in front of the loop (frames, reference room).
pub(crate) const DELAYS: [f64; PLAIN] = [400.75, 1336.75, 3192.75, 2129.0];
/// Which group each direct reflection of Density Low belongs to, in table order.
pub(crate) const TAP_GROUPS: [usize; 10] = [0, 0, 1, 2, 2, 4, 5, 1, 3, 3];
/// The four patterns over the six groups (each normalised to unit length below).
const PATTERN_VECTORS: [[f64; GROUPS]; PATTERNS] = [
    [0.97, 0.54, -0.08, -0.49, -0.33, 0.20],
    [-0.12, 0.50, 0.65, -0.32, 0.58, -0.57],
    [0.08, 0.41, -0.65, 0.31, 0.61, 0.10],
    [-0.46, 0.57, 0.28, 0.06, -0.09, 0.62],
];
/// The loop's mixing: row `i` is what line `i` takes from each line's output. Orthogonal to the
/// four places given; [`orthonormal`] makes it exactly so.
pub(crate) const MIX: [[f64; LOOP]; LOOP] = [
    [0.1416, 0.4606, 0.5290, -0.2829, 0.0081, 0.5804, 0.2664],
    [-0.4913, 0.4355, -0.0396, 0.4455, 0.6051, 0.0365, -0.0379],
    [-0.0284, 0.0522, -0.0547, -0.0540, 0.0575, -0.4242, 0.8985],
    [0.6914, 0.5046, -0.3213, -0.0969, 0.2531, -0.2504, -0.1672],
    [-0.4873, 0.4439, -0.0444, -0.5421, -0.3116, -0.3509, -0.2222],
    [-0.1282, 0.0830, -0.7788, -0.0894, -0.1688, 0.5403, 0.2042],
    [0.0764, 0.3700, 0.0619, 0.6382, -0.6639, -0.0645, 0.0351],
];
/// What each loop line takes from each plain delay's output.
const FROM_DELAYS: [[f64; PLAIN]; LOOP] = [
    [0.1773, -0.2062, 0.4730, 0.0],
    [-0.4841, 0.3838, 0.4457, 0.0],
    [0.0, 0.0, 0.0, 0.0],
    [0.6911, -0.1076, 0.5551, 0.0],
    [-0.4104, -0.3521, 0.3765, 0.6751],
    [-0.1798, -0.3095, -0.5207, -0.3550],
    [0.0578, 0.5641, 0.4787, 0.0],
];
/// What each loop line takes from each pattern.
const FROM_PATTERNS: [[f64; PATTERNS]; LOOP] = [
    [1.1116, 0.0, 0.0, 0.0],
    [0.0, 1.1537, 0.0, 0.0],
    [0.0, 0.0, 0.0, 0.0],
    [0.0, -0.0783, 0.0, 1.1632],
    [-0.3137, -0.2180, 0.4358, 0.2895],
    [0.0751, 0.1004, 0.0, 0.0554],
    [0.0, 0.0609, 1.8961, -0.0865],
];
/// What each plain delay takes from each pattern.
const DELAY_FROM_PATTERNS: [[f64; PATTERNS]; PLAIN] =
    [[1.0153, 0.0, 0.0, 0.0], [0.0, -0.1093, 0.0, 1.3217], [0.0, 0.9587, 0.0, 0.0], [0.8159, 0.0, 0.0, -0.5406]];
/// The 2129-frame delay also takes this much of the 3193-frame one's output.
const DELAY_CHAIN: f64 = 0.3364;
/// Each loop line's level in its side (before Stereo's rotation) and the side (0 left, 1 right).
/// The fit heard A and C on the right and B, D and F on the left; G is heard on the right too, at
/// a little above the others' level, though the fit gave it none. The loop shares its energy out evenly among its
/// lines once it has rung a while, so with two lines heard against three the right fell 2 to 3 dB
/// behind the left from 0.2 s on (when G's first arrivals come out), where Live's two sides stay
/// level; G evens them.
pub(crate) const OUTPUTS: [(f64, usize); LOOP] =
    [(1.1667, 1), (1.1510, 0), (0.9296, 1), (1.1588, 0), (0.9648, 0), (-1.5, 1), (0.0, 1)];
/// The input's delay to the lines after the first direct reflection, at Shape 0.5 (ms), and how
/// much more per unit of Shape (ms, times the size factor): 767 frames, 384 more per 0.25 of
/// Shape at Size 100.
pub(crate) const ENTRY_MS: (f64, f64) = (17.392, 34.83);
/// Longest delay a loop line needs (seconds, with room for Size 500).
pub(crate) const LINE_SECONDS: f64 = 0.35;
/// Longest allpass a loop line needs (seconds).
pub(crate) const ALLPASS_SECONDS: f64 = 0.13;
/// Longest plain delay (seconds).
pub(crate) const DELAY_SECONDS: f64 = 0.13;

/// The four patterns, each a unit vector over the groups.
fn patterns() -> [[f64; GROUPS]; PATTERNS] {
    PATTERN_VECTORS.map(|v| {
        let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
        v.map(|x| x / norm)
    })
}

/// [`MIX`] with its rows made exactly orthonormal (Gram-Schmidt), so the loop neither gains nor
/// loses energy by itself however long it rings.
fn orthonormal() -> [[f64; LOOP]; LOOP] {
    let mut rows = MIX;
    for i in 0..LOOP {
        for j in 0..i {
            let dot: f64 = rows[i].iter().zip(&rows[j]).map(|(a, b)| a * b).sum();
            let done = rows[j];
            for (x, d) in rows[i].iter_mut().zip(done) {
                *x -= dot * d;
            }
        }
        let norm = rows[i].iter().map(|x| x * x).sum::<f64>().sqrt();
        for x in &mut rows[i] {
            *x /= norm;
        }
    }
    rows
}

/// The lengths and gains the measured network runs with, worked out at the control rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Lengths {
    /// Each loop line's delay (frames).
    pub lines: [f64; LOOP],
    /// Each loop line's allpass (frames).
    pub allpasses: [f64; LOOP],
    /// Each loop line's gain per trip.
    pub gains: [f64; LOOP],
    /// Each loop line's share of a damping trip.
    pub trips: [f64; LOOP],
    /// The plain delays (frames).
    pub delays: [f64; PLAIN],
}

impl Default for Lengths {
    fn default() -> Self {
        Self {
            lines: [2.0; LOOP],
            allpasses: [2.0; LOOP],
            gains: [0.0; LOOP],
            trips: [0.0; LOOP],
            delays: [1.0; PLAIN],
        }
    }
}

impl Lengths {
    /// The values `t` (0..1) of the way from `from` to `to`.
    pub fn between(&mut self, from: &Self, to: &Self, t: f64) {
        fn mix<const N: usize>(out: &mut [f64; N], a: &[f64; N], b: &[f64; N], t: f64) {
            for ((o, a), b) in out.iter_mut().zip(a).zip(b) {
                *o = a + (b - a) * t;
            }
        }
        mix(&mut self.lines, &from.lines, &to.lines, t);
        mix(&mut self.allpasses, &from.allpasses, &to.allpasses, t);
        mix(&mut self.gains, &from.gains, &to.gains, t);
        mix(&mut self.trips, &from.trips, &to.trips, t);
        mix(&mut self.delays, &from.delays, &to.delays, t);
    }

    /// The lengths for size factor `room`, Scale `scale` (0..1) at `sample_rate`; `gain` gives a
    /// line's gain per trip from its length in seconds. Whole frames when `settled`.
    pub fn tune(
        &mut self,
        room: f64,
        scale: f64,
        sample_rate: f64,
        settled: bool,
        gain: impl Fn(f64) -> f64,
        trip_seconds: f64,
    ) {
        let frames = room * sample_rate / RATE;
        let whole = |x: f64, least: f64| if settled { x.round() } else { x }.max(least);
        for (k, rate) in ALLPASS_RATES.iter().enumerate() {
            let percent = scale * 100.0;
            let allpass = if DIFFUSED[k] { rate * percent * frames } else { 0.0 };
            let line = length_at(k, percent) * frames - allpass;
            self.lines[k] = whole(line, 2.0);
            self.allpasses[k] = whole(allpass, 2.0);
            let seconds = (line + allpass) / sample_rate;
            self.gains[k] = gain(seconds);
            self.trips[k] = seconds / trip_seconds;
        }
        for (d, base) in self.delays.iter_mut().zip(DELAYS) {
            *d = whole(base * frames, 1.0);
        }
    }

    /// The longest delay of the network (frames).
    pub fn longest(room: f64, sample_rate: f64) -> f64 {
        LENGTHS.iter().flatten().copied().fold(0.0, f64::max) * room * sample_rate / RATE
    }
}

/// The measured network's lines and state.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Network {
    /// The loop lines, each with its allpass and damping.
    pub lines: [Line; LOOP],
    /// The plain delays in front of the loop.
    delays: [DelayLine; PLAIN],
    /// Each pattern as a unit vector over the groups.
    patterns: [[f64; GROUPS]; PATTERNS],
    /// The loop's mixing, exactly orthogonal.
    mix: [[f64; LOOP]; LOOP],
}

impl Network {
    /// A network for `sample_rate` (allocates).
    pub fn new(sample_rate: f64) -> Self {
        Self {
            lines: std::array::from_fn(|_| Line {
                delay: DelayLine::for_seconds(LINE_SECONDS, sample_rate),
                diffuser: Allpass::for_seconds(ALLPASS_SECONDS, sample_rate),
                damper: Damper::default(),
            }),
            delays: std::array::from_fn(|_| DelayLine::for_seconds(DELAY_SECONDS, sample_rate)),
            patterns: patterns(),
            mix: orthonormal(),
        }
    }

    /// Silences every line.
    pub fn clear(&mut self) {
        for line in &mut self.lines {
            line.clear();
        }
        for delay in &mut self.delays {
            delay.clear();
        }
    }

    /// One frame: `groups` is the input of each group of direct reflections; returns the tail of
    /// each side. `swing` gives each loop line's chorus swing (frames) from its index.
    #[inline]
    pub fn process(
        &mut self,
        groups: &[f64; GROUPS],
        n: &Lengths,
        diffusion: f32,
        damping: &Damping,
        swing: impl Fn(usize, f64) -> f64,
        frozen: bool,
    ) -> [f64; 2] {
        let p: [f64; PATTERNS] = self.patterns.map(|v| v.iter().zip(groups).map(|(a, g)| a * g).sum());
        let pattern = |row: &[f64; PATTERNS]| row.iter().zip(&p).map(|(a, x)| a * x).sum::<f64>();

        let mut outs = [0.0f64; LOOP];
        let mut tail = [0.0f64; 2];
        for (k, (line, out)) in self.lines.iter_mut().zip(&mut outs).enumerate() {
            let length = n.lines[k];
            let read = f64::from(line.delay.read_hermite(length + swing(k, length)));
            let y = if DIFFUSED[k] {
                f64::from(line.diffuser.process(read as f32, n.allpasses[k], diffusion))
            } else {
                read
            };
            *out = line.damper.damp(y, damping) * n.gains[k];
            let (level, side) = OUTPUTS[k];
            if let Some(t) = tail.get_mut(side) {
                *t += level * *out;
            }
        }
        let delayed: [f64; PLAIN] =
            std::array::from_fn(|q| f64::from(self.delays[q].read_hermite(n.delays[q].max(2.0))));
        for (q, delay) in self.delays.iter_mut().enumerate() {
            let mut x = pattern(&DELAY_FROM_PATTERNS[q]);
            if q == 3 {
                x += DELAY_CHAIN * delayed[2];
            }
            delay.push(x as f32);
        }
        for (k, line) in self.lines.iter_mut().enumerate() {
            let mixed: f64 = self.mix[k].iter().zip(&outs).map(|(m, y)| m * y).sum();
            let fed: f64 = FROM_DELAYS[k].iter().zip(&delayed).map(|(m, y)| m * y).sum();
            let mut next = mixed + fed + pattern(&FROM_PATTERNS[k]);
            if frozen {
                next = crate::limit(next);
            }
            line.delay.push(next as f32);
        }
        tail
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loop_mix_is_orthogonal_and_close_to_the_measured_one() {
        let mix = orthonormal();
        for i in 0..LOOP {
            for j in 0..LOOP {
                let dot: f64 = (0..LOOP).map(|k| mix[k][i] * mix[k][j]).sum();
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((dot - want).abs() < 1e-12, "columns {i} and {j}: {dot}");
                assert!((mix[i][j] - MIX[i][j]).abs() < 2e-3, "{i}, {j}: {} against {}", mix[i][j], MIX[i][j]);
            }
        }
    }

    #[test]
    fn a_line_is_its_measured_length_at_the_measured_scales() {
        for (k, lengths) in LENGTHS.iter().enumerate() {
            for (&s, &length) in SCALES.iter().zip(lengths) {
                assert!((length_at(k, s) - length).abs() < 1e-9, "line {k} at {s} %");
            }
            let between = length_at(k, 30.0);
            assert!(between > lengths[1] - 1e-9 && between < lengths[2] + 1e-9, "line {k}: {between}");
        }
    }

    #[test]
    fn every_direct_reflection_of_low_has_a_group() {
        let direct = crate::TAPS_LOW.iter().filter(|tap| !tap.filtered).count();
        assert_eq!(direct, TAP_GROUPS.len());
    }
}
