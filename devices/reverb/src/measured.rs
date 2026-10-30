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
//! Times are frames at 44.1 kHz. A line is a delay and an allpass, each a whole number of 8-frame
//! blocks and one frame: the delay's blocks grow with the room's size factor, the allpass's with
//! the size factor times Scale (%), each rounding down at its own edge ([`length_at`]). The input
//! reaches the lines [`ENTRY_MS`] after each direct reflection, whatever the Size.
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

/// Live builds every length of the network from blocks of 8 frames (at 44.1 kHz).
const BLOCK: f64 = 8.0;
/// Each line's delay before its allpass in the reference room (blocks) for A, B, C, D and F, and
/// G, the delay R runs after A's: a delay is `BLOCK * floor(blocks * room + DELAY_EDGE) + 1`
/// frames, G's `BLOCK * floor(blocks * room + G_EDGE)` with no frame more.
const DELAYS: [f64; LOOP] = [286.875, 339.649, 423.336, 459.936, 846.683, 934.943];
/// Where a delay rounds up to its next block (blocks).
const DELAY_EDGE: f64 = 0.85;
/// Where G rounds up to its next block (blocks).
const G_EDGE: f64 = -0.411;
/// Each line's allpass, A, B, C, D, F and R, in blocks per % of Scale in the reference room: it
/// is `BLOCK * floor(blocks * Scale * room + ALLPASS_EDGE) + 1` frames.
const ALLPASSES: [f64; LOOP] = [0.63406, 1.22440, 0.87055, 3.97925, 2.18381, 0.80374];
/// Where an allpass rounds up to its next block (blocks).
const ALLPASS_EDGE: f64 = 0.9;

/// `BLOCK` times `x` rounded down past `edge`, or, while things move, the straight line through
/// the middle of the steps, so a glide does not jump a block at a time.
fn blocks(x: f64, edge: f64, settled: bool) -> f64 {
    BLOCK * if settled { (x + edge).floor() } else { x + edge - 0.5 }
}

/// Line `k`'s delay and allpass (frames at 44.1 kHz) for size factor `room` and Scale `percent`,
/// in whole blocks when `settled`. C first runs its family's offset as a plain delay, and R is
/// A's delay, then G's, then its own allpass (`reverb-network-small`, `reverb-network-sections`).
fn length_at(k: usize, room: f64, percent: f64, settled: bool) -> (f64, f64) {
    let delay = |row: usize| blocks(DELAYS[row] * room, DELAY_EDGE, settled) + 1.0;
    let allpass = blocks(ALLPASSES[k] * percent * room, ALLPASS_EDGE, settled) + 1.0;
    let line = match k {
        2 => front(2, room, settled) + delay(2),
        5 => delay(0) + blocks(DELAYS[5] * room, G_EDGE, settled),
        _ => delay(k),
    };
    (line, allpass)
}

/// Where family `k`'s filtered reflections peak after their direct ones (frames at 44.1 kHz) for
/// size factor `room`, in whole blocks and a frame when `settled`.
pub(crate) fn peak(k: usize, room: f64, settled: bool) -> f64 {
    front(k, room, settled) + 1.0
}

/// Where family `k`'s copy starts (frames at 44.1 kHz) for size factor `room`: whole blocks when
/// `settled`; it peaks one frame later.
fn front(k: usize, room: f64, settled: bool) -> f64 {
    let x = FAMILY_OFFSETS[k] * room;
    BLOCK * if settled { x.round() } else { x }
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
/// Where each family's filtered reflection starts after the direct one (blocks, reference room):
/// 401, 3193, 1641, 1337, 2985 and 2337 frames to its peak at Size 100. Its copy into the mix is
/// heard there too, through the same low-pass.
const FAMILY_OFFSETS: [f64; LOOP] = [50.4258, 399.2770, 205.1187, 167.3337, 373.2037, 292.2697];
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
/// A and B run this much of a frame past their whole length, read by linear interpolation (0.99
/// of the frame and 0.01 of the next): a slight loss of the highs on every pass (0.18 dB at the
/// top) and none of the lows. The other lines are whole. No line loses anything else on a pass:
/// the whole tail falls with the decay's loss per second, at every Size (Decision 26).
const FRACTION: [f64; LOOP] = [0.01, 0.01, 0.0, 0.0, 0.0, 0.0];
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
        let frames = sample_rate / RATE;
        let whole = |x: f64| if settled { x.round() } else { x }.max(2.0);
        let percent = scale * 100.0;
        for k in 0..LOOP {
            let (line, allpass) = length_at(k, room, percent, settled);
            let (line, allpass) = (line * frames, allpass * frames);
            self.lines[k] = whole(line);
            self.allpasses[k] = whole(allpass);
            // The allpass loses its own time on each pass, so the echoes it holds back decay
            // as fast as the rest (as Live's tail does at every Diffusion).
            self.gains[k] = gain(line / sample_rate);
            self.losses[k] = gain(allpass / sample_rate);
            self.trips[k] = (line + allpass) / sample_rate / trip_seconds;
            self.fronts[k] = whole(front(k, room, settled) * frames);
        }
    }

    /// Each copy's level for size factor `room`, as the reflections fade by `fade` (dB per ms of
    /// their delay) over its family's offset.
    pub fn fade(&mut self, room: f64, fade: f64) {
        for (k, gain) in self.copy_gains.iter_mut().enumerate() {
            *gain = 10f64.powf(-fade * peak(k, room, false) * 1_000.0 / RATE / 20.0);
        }
    }

    /// The longest delay of the network (frames).
    pub fn longest(room: f64, sample_rate: f64) -> f64 {
        (0..LOOP).map(|k| length_at(k, room, 100.0, false).0).fold(0.0, f64::max) * sample_rate / RATE
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
            if FRACTION[k] != 0.0 {
                read += FRACTION[k] * (f64::from(line.delay.read_hermite(length + 1.0)) - read);
            }
            let y = f64::from(line.diffuser.process_lossy(read as f32, n.allpasses[k], diffusion, n.losses[k] as f32));
            *out = line.damper.damp(y, damping) * n.gains[k];
            if let Some(t) = tail.get_mut(SIDES[k]) {
                *t += *out;
            }
        }
        // Into the mix: each line's output and its family's copy.
        let into: [f64; LOOP] = std::array::from_fn(|j| outs[j] + copies[j]);
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

    /// Live's whole lengths, delay and allpass (frames, C's after its family's offset), read at
    /// each (Size, Scale %) Live's renders were read for.
    const MEASURED: [(f64, f64, [f64; LOOP]); 10] = [
        (100.0, 5.0, [2330.0, 2778.0, 5074.0, 3842.0, 6866.0, 9802.0]),
        (100.0, 20.0, [2402.0, 2922.0, 5178.0, 4322.0, 7130.0, 9898.0]),
        (100.0, 40.0, [2506.0, 3114.0, 5314.0, 4962.0, 7482.0, 10034.0]),
        (100.0, 100.0, [2810.0, 3706.0, 5730.0, 6866.0, 8530.0, 10418.0]),
        (50.0, 40.0, [1994.0, 2474.0, 4218.0, 3938.0, 5938.0, 7962.0]),
        (10.54, 5.0, [1106.0, 1314.0, 2394.0, 1826.0, 3250.0, 4634.0]),
        (10.54, 100.0, [1330.0, 1754.0, 2714.0, 3250.0, 4034.0, 4922.0]),
        (1.53, 5.0, [586.0, 698.0, 1258.0, 962.0, 1714.0, 2434.0]),
        (1.53, 40.0, [634.0, 786.0, 1322.0, 1242.0, 1866.0, 2490.0]),
        (1.53, 100.0, [706.0, 930.0, 1426.0, 1714.0, 2130.0, 2586.0]),
    ];

    #[test]
    fn a_line_is_its_measured_length() {
        for (size, percent, lengths) in MEASURED {
            let room = (size / 100.0f64).cbrt();
            for (k, want) in lengths.into_iter().enumerate() {
                let (line, allpass) = length_at(k, room, percent, true);
                assert_eq!(line + allpass, want, "line {k} at Size {size}, Scale {percent} %");
            }
        }
    }

    #[test]
    fn each_copy_peaks_where_measured() {
        let measured = [
            (100.0, [401.0, 3193.0, 1641.0, 1337.0, 2985.0, 2337.0]),
            (10.54, [193.0, 1513.0, 777.0, 633.0, 1409.0, 1105.0]),
            (0.22, [57.0, 417.0, 217.0, 177.0, 393.0, 305.0]),
        ];
        for (size, peaks) in measured {
            let room = (size / 100.0f64).cbrt();
            for (k, want) in peaks.into_iter().enumerate() {
                assert_eq!(peak(k, room, true), want, "family {k} at Size {size}");
            }
        }
    }

    #[test]
    fn a_glide_runs_through_the_middle_of_the_steps() {
        // Moving, a delay or an allpass is within half a block of where it settles.
        for size in [0.5, 3.0, 27.0, 100.0, 333.0] {
            let room = (size / 100.0f64).cbrt();
            for k in 0..LOOP {
                let (a, b) = (length_at(k, room, 40.0, true), length_at(k, room, 40.0, false));
                let most = if k == 2 || k == 5 { BLOCK } else { BLOCK / 2.0 };
                assert!((a.0 - b.0).abs() <= most, "line {k} at Size {size}");
                assert!((a.1 - b.1).abs() <= BLOCK / 2.0, "allpass {k} at Size {size}");
            }
        }
    }

    #[test]
    fn every_direct_reflection_of_low_has_a_group() {
        let direct = crate::TAPS_LOW.iter().filter(|tap| !tap.filtered).count();
        assert_eq!(direct, TAP_GROUPS.len());
    }
}
