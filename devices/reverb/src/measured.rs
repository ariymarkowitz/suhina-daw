// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The late reverb of Density Low, as measured (docs/spec/reverb.md › Decisions 19 and 24): six
//! lines mixed by one orthogonal matrix, each fed by its own mix of the direct reflections, and
//! each of those mixes heard again, low-passed, one offset later.
//!
//! ```text
//! direct taps -> 6 groups -> 6 families --+------------------------------> own line's input
//!                                          +-> offset -> low-pass (copy) --+
//! line outputs ----------------------------------------------------------+-> MIX -> lines' inputs
//! A, C and R are heard on the right, B, D and F on the left, all at one level
//! ```
//!
//! Times are frames at 44.1 kHz in the reference room (Size 100). A line is a delay of the first
//! length and an allpass of the second times Scale (%), both scaled with the room's size factor;
//! the input reaches the lines [`ENTRY_MS`] after each direct reflection, whatever the Size.
//! Every value was read from `reverb-network-exact` (Hi Cut off, Diffusion 0.1 %, Decay 60 s,
//! Stereo 120, Scale 5 to 100 %): with the input's fixed filter divided out each arrival is one
//! sample, and a simulation of this structure with these values matches Live's renders to −41 dB
//! over the first 0.5 s at every Scale and −35 dB from 1 to 2 s.

use suhina_dsp::{Allpass, Biquad, Coefficients, DelayLine};

use crate::{Damper, Damping, Line};

/// Lines in the loop: A, B, C, D, F and R.
pub(crate) const LOOP: usize = 6;
/// Groups of direct reflections the families are made of.
pub(crate) const GROUPS: usize = 6;
/// The frame rate the lengths below are at.
const RATE: f64 = 44_100.0;

/// The Scales (%) each length was measured at.
const SCALES: [f64; 6] = [5.0, 20.0, 40.0, 60.0, 80.0, 100.0];
/// The whole length, delay and allpass, of A, B, C, D, F and G at each of [`SCALES`] in the
/// reference room: Live's allpasses grow in steps of 8 frames, so a length is up to 4 frames off a
/// straight line in Scale, and between the measured Scales it is read off the straight line between
/// them. G is not a line of its own: R is A's and G's lengths and [`R_EXTRA`] together.
const LENGTHS: [[f64; 6]; 6] = [
    [2330.0, 2402.0, 2506.0, 2602.0, 2706.0, 2810.0],
    [2778.0, 2922.0, 3114.0, 3314.0, 3506.0, 3706.0],
    [3433.0, 3537.0, 3673.0, 3817.0, 3953.0, 4089.0],
    [3842.0, 4322.0, 4962.0, 5594.0, 6234.0, 6866.0],
    [6866.0, 7130.0, 7482.0, 7826.0, 8178.0, 8530.0],
    [7470.0, 7494.0, 7526.0, 7558.0, 7582.0, 7606.0],
];
/// How much of each length of [`LENGTHS`] is its allpass: frames per % of Scale.
const ALLPASS_RATES: [f64; 6] = [5.050, 9.768, 6.921, 31.832, 17.493, 1.444];
/// C runs this plain delay before its own length: its family reaches it unfiltered this long
/// after the entry, and nothing else hears the plain part.
const C_PLAIN: f64 = 1641.0;
/// R is this many frames longer than A and G together.
const R_EXTRA: f64 = 2.0;

/// Line `k`'s whole length and its allpass's share of it at Scale `percent`, reference room.
fn length_at(k: usize, percent: f64) -> (f64, f64) {
    let table = |row: usize| {
        let lengths = LENGTHS[row];
        let i = SCALES.iter().rposition(|&s| s <= percent).unwrap_or(0).min(SCALES.len() - 2);
        let t = (percent - SCALES[i]) / (SCALES[i + 1] - SCALES[i]);
        (lengths[i] + (lengths[i + 1] - lengths[i]) * t, ALLPASS_RATES[row] * percent)
    };
    match k {
        2 => {
            let (c, allpass) = table(2);
            (C_PLAIN + c, allpass)
        }
        5 => {
            let ((a, a_allpass), (g, g_allpass)) = (table(0), table(5));
            (a + g + R_EXTRA, a_allpass + g_allpass)
        }
        // A, B, D and F are rows 0, 1, 3 and 4 of the table.
        _ => table(k),
    }
}

/// Which group each direct reflection of Density Low belongs to, in table order.
pub(crate) const TAP_GROUPS: [usize; 10] = [0, 0, 1, 2, 2, 4, 5, 1, 3, 3];
/// Each line's family: its weight on each group, the level of the family's filtered reflection
/// against the direct reflection it follows (the same for both reflections of a group, to 1 %).
const FAMILIES: [[f64; GROUPS]; LOOP] = [
    [1.044, 0.625, -0.0915, -0.604, -0.385, 0.237],
    [-0.1175, 0.5865, 0.7715, -0.399, 0.663, -0.686],
    [0.1255, 0.5465, -0.8845, 0.382, 0.880, 0.091],
    [-0.624, 0.755, 0.3725, 0.1175, -0.148, 0.955],
    [-0.692, 0.419, -0.684, -0.506, -0.590, -0.546],
    [0.241, 0.5005, 0.198, 1.050, -0.555, -0.533],
];
/// Where each family's filtered reflection peaks after the direct one (frames, reference room):
/// its copy into the mix is heard there too, through the same low-pass.
const FAMILY_OFFSETS: [f64; LOOP] = [401.0, 3193.0, 1641.0, 1337.0, 2985.0, 2337.0];
/// How much of its family each line takes, times its output level: the first pass of each line
/// against one direct reflection.
const FAMILY_GAINS: [f64; LOOP] = [1.7860, 1.7959, 1.8109, 1.8022, 1.8092, 1.8012];
/// The loop's mixing: row `i` is what line `i` takes from each line's output (and from each
/// family's copy, at the same weight). Read from the renders, it is orthogonal to 0.3 % without
/// being made so; [`orthonormal`] makes it exactly so.
pub(crate) const MIX: [[f64; LOOP]; LOOP] = [
    [0.1651, 0.43434, 0.7290, -0.2671, -0.0618, -0.4213],
    [-0.4845, 0.4152, -0.0838, 0.4661, 0.5380, -0.2813],
    [0.0587, 0.3811, 0.0875, 0.6188, -0.6243, 0.2665],
    [0.6681, 0.5295, -0.4350, -0.1034, 0.2579, 0.0826],
    [-0.3852, 0.2966, -0.4863, -0.4104, -0.4819, -0.3556],
    [-0.3740, 0.3540, 0.1679, -0.3865, 0.1348, 0.7342],
];
/// What each line's output keeps on its way into the mix, beyond the decay's own loss (the length
/// of each column of the mix as read). Not while frozen, so a frozen tail does not drain by it.
const LOSSES: [f64; LOOP] = [0.9961, 0.9939, 0.9928, 0.9954, 0.9956, 0.9944];
/// A and B hear their input this much again one frame later on every pass, a slight loss of the
/// highs (0.18 dB at the top); the other lines have none.
const NEXT_FRAME: [f64; LOOP] = [0.0105, 0.0109, 0.0, 0.0, 0.0, 0.0];
/// The side each line is heard on (0 left, 1 right), all at one level before Stereo's rotation.
pub(crate) const SIDES: [usize; LOOP] = [1, 0, 1, 0, 0, 1];
/// The input's delay to the lines after each direct reflection, at Shape 0.5 (ms), and how much
/// more per unit of Shape below and above 0.5 (ms, times the size factor): −1, 775 and 1543
/// frames at Shape 0, 0.5 and 1 at Size 100, so 776 frames for the half below and 768 above.
pub(crate) const ENTRY_MS: (f64, f64, f64) = (17.574, 35.193, 34.830);
/// Longest delay a line needs (seconds, with room for Size 500).
pub(crate) const LINE_SECONDS: f64 = 0.45;
/// Longest allpass a line needs (seconds).
pub(crate) const ALLPASS_SECONDS: f64 = 0.13;
/// Longest family offset (seconds, with room for Size 500).
const FRONT_SECONDS: f64 = 0.2;

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
    /// Each line's delay (frames).
    pub lines: [f64; LOOP],
    /// Each line's allpass (frames).
    pub allpasses: [f64; LOOP],
    /// Each line's gain per trip through its delay.
    pub gains: [f64; LOOP],
    /// Each line's allpass gain per pass through the allpass's delay.
    pub losses: [f64; LOOP],
    /// Each line's share of a damping trip.
    pub trips: [f64; LOOP],
    /// Each family's delay to its copy's low-pass (frames).
    pub fronts: [f64; LOOP],
    /// Each copy's level: a filtered reflection fades as the reflections do over its own offset.
    pub copy_gains: [f64; LOOP],
}

impl Default for Lengths {
    fn default() -> Self {
        Self {
            lines: [2.0; LOOP],
            allpasses: [2.0; LOOP],
            gains: [0.0; LOOP],
            losses: [0.0; LOOP],
            trips: [0.0; LOOP],
            fronts: [1.0; LOOP],
            copy_gains: [0.0; LOOP],
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
        mix(&mut self.losses, &from.losses, &to.losses, t);
        mix(&mut self.trips, &from.trips, &to.trips, t);
        mix(&mut self.fronts, &from.fronts, &to.fronts, t);
        mix(&mut self.copy_gains, &from.copy_gains, &to.copy_gains, t);
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
        let percent = scale * 100.0;
        for (k, &offset) in FAMILY_OFFSETS.iter().enumerate() {
            let (length, allpass) = length_at(k, percent);
            let (line, allpass) = ((length - allpass) * frames, allpass * frames);
            self.lines[k] = whole(line, 2.0);
            self.allpasses[k] = whole(allpass, 2.0);
            // The allpass loses its own time on each pass, so the echoes it holds back decay
            // as fast as the rest (as Live's tail does at every Diffusion).
            self.gains[k] = gain(line / sample_rate);
            self.losses[k] = gain(allpass / sample_rate);
            self.trips[k] = (line + allpass) / sample_rate / trip_seconds;
            // The copy's low-pass peaks one frame after it starts.
            self.fronts[k] = whole((offset - 1.0) * frames, 2.0);
        }
    }

    /// Each copy's level for size factor `room`, as the reflections fade by `fade` (dB per ms of
    /// their delay) over its family's offset.
    pub fn fade(&mut self, room: f64, fade: f64) {
        for (gain, offset) in self.copy_gains.iter_mut().zip(FAMILY_OFFSETS) {
            *gain = 10f64.powf(-fade * offset * room * 1_000.0 / RATE / 20.0);
        }
    }

    /// The longest delay of the network (frames).
    pub fn longest(room: f64, sample_rate: f64) -> f64 {
        (0..LOOP).map(|k| length_at(k, 100.0).0).fold(0.0, f64::max) * room * sample_rate / RATE
    }
}

/// The measured network's lines and state.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Network {
    /// The lines, each with its allpass and damping.
    pub lines: [Line; LOOP],
    /// Each family on its way to its copy.
    fronts: [DelayLine; LOOP],
    /// The copies' low-pass, the reflections' own.
    copy_filters: [Biquad; LOOP],
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
            fronts: std::array::from_fn(|_| DelayLine::for_seconds(FRONT_SECONDS, sample_rate)),
            copy_filters: Default::default(),
            mix: orthonormal(),
        }
    }

    /// Silences every line.
    pub fn clear(&mut self) {
        for line in &mut self.lines {
            line.clear();
        }
        for front in &mut self.fronts {
            front.clear();
        }
        for filter in &mut self.copy_filters {
            filter.reset();
        }
    }

    /// Sets the copies' low-pass, the one the filtered reflections pass.
    pub fn set_copy_filter(&mut self, coefficients: Coefficients) {
        for filter in &mut self.copy_filters {
            filter.set(coefficients);
        }
    }

    /// One frame: `groups` is the input of each group of direct reflections, already delayed by
    /// the entry; returns the tail of each side. `swing` gives each line's chorus swing (frames)
    /// from its index and length.
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
        let families: [f64; LOOP] =
            std::array::from_fn(|k| FAMILY_GAINS[k] * FAMILIES[k].iter().zip(groups).map(|(w, g)| w * g).sum::<f64>());
        let mut copies = [0.0f64; LOOP];
        for ((((front, filter), copy), (&family, &delay)), &level) in self
            .fronts
            .iter_mut()
            .zip(&mut self.copy_filters)
            .zip(&mut copies)
            .zip(families.iter().zip(&n.fronts))
            .zip(&n.copy_gains)
        {
            *copy = level * filter.process(f64::from(front.read_hermite(delay.max(2.0))));
            front.push(family as f32);
        }

        let mut outs = [0.0f64; LOOP];
        let mut tail = [0.0f64; 2];
        for (k, (line, out)) in self.lines.iter_mut().zip(&mut outs).enumerate() {
            let length = n.lines[k] + swing(k, n.lines[k]);
            let mut read = f64::from(line.delay.read_hermite(length));
            if NEXT_FRAME[k] != 0.0 {
                read += NEXT_FRAME[k] * f64::from(line.delay.read_hermite(length + 1.0));
            }
            let y = f64::from(line.diffuser.process_lossy(read as f32, n.allpasses[k], diffusion, n.losses[k] as f32));
            *out = line.damper.damp(y, damping) * n.gains[k];
            if let Some(t) = tail.get_mut(SIDES[k]) {
                *t += *out;
            }
        }
        // Into the mix: each line's output and its family's copy, through the loss a pass keeps
        // (while frozen, an exactly orthogonal mix of a pass that keeps its level).
        let into: [f64; LOOP] = std::array::from_fn(|j| {
            if frozen { outs[j] / (1.0 + NEXT_FRAME[j]) + copies[j] } else { LOSSES[j] * (outs[j] + copies[j]) }
        });
        for (k, line) in self.lines.iter_mut().enumerate() {
            let mixed: f64 = self.mix[k].iter().zip(&into).map(|(m, y)| m * y).sum();
            let mut next = families[k] + mixed;
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
        for (k, lengths) in LENGTHS.iter().enumerate().take(5) {
            let line = [0, 1, 2, 3, 4][k];
            for (&s, &length) in SCALES.iter().zip(lengths) {
                let want = if line == 2 { C_PLAIN + length } else { length };
                assert!((length_at(line, s).0 - want).abs() < 1e-9, "line {line} at {s} %");
            }
        }
        // R is A's and G's lengths and two frames: 10,034 frames at Scale 40 %.
        assert!((length_at(5, 40.0).0 - 10_034.0).abs() < 1e-9);
    }

    #[test]
    fn every_direct_reflection_of_low_has_a_group() {
        let direct = crate::TAPS_LOW.iter().filter(|tap| !tap.filtered).count();
        assert_eq!(direct, TAP_GROUPS.len());
    }
}
