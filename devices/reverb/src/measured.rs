// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The late reverb of Densities Low, Mid and High, as measured (docs/spec/reverb.md › Decisions
//! 19, 24 and 27): lines mixed by one orthogonal matrix, each fed by its own mix of the direct
//! reflections, and each line's family of reflections heard again, low-passed, one offset later,
//! joining that line's output in the mix. The direct reflections come in groups that reach the
//! lines as one, and there are as many lines as groups: Low 6, Mid 8, High 10 ([`design`]).
//!
//! ```text
//! direct taps -> groups -> each line's input -------------------------------> own line
//!                       -> each line's family -> offset -> low-pass (copy) --+
//! line outputs -------------------------------------------------------------+-> MIX -> lines
//! ```
//!
//! At Low a line's family is its input. Its six lines are A, B, C, D, F and R: A, C and R heard
//! on the right, B, D and F on the left, all at one level. Mid's and High's lines are read
//! only at Size 100 (`reverb-network-density`), as below.
//!
//! Times are frames at 44.1 kHz. A line of Low is a delay and an allpass, each a whole number of 8-frame
//! blocks and one frame: the delay's blocks grow with the room's size factor, the allpass's with
//! the size factor times Scale (%), each rounding down at its own edge ([`length_at`]). The input
//! reaches the lines [`ENTRY_MS`] after each direct reflection, whatever the Size. Under Smooth Fast
//! or Slow the lengths keep to no blocks and A and B are read between frames ([`FAST_DELAYS`]).
//! Every value was read from `reverb-network-exact` (Hi Cut off, Diffusion 0.1 %, Decay 60 s,
//! Stereo 120, Scale 5 to 100 %): with the input's fixed filter divided out each arrival is one
//! sample, and a simulation of this structure with these values matches Live's renders to −41 dB
//! over the first 0.5 s at every Scale and −35 dB from 1 to 2 s.

use suhina_dsp::{Allpass, Biquad, Coefficients, DelayLine};

use crate::{Damper, Damping, Line};

/// Lines in Low's loop: A, B, C, D, F and R.
pub(crate) const LOOP: usize = 6;
/// Groups of Low's direct reflections the families are made of.
pub(crate) const GROUPS: usize = 6;
/// The most lines a network has (High's).
pub(crate) const MAX_LINES: usize = 10;
/// The most groups of direct reflections a network takes (High's).
pub(crate) const MAX_GROUPS: usize = 10;
/// Direct reflections in Densities Low, Mid and High (Sparse has four).
pub(crate) const DIRECT_TAPS: usize = 10;
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

/// Under Smooth Fast (and Slow, which renders as Fast) no length keeps to blocks
/// (`reverb-network-fast`, `reverb-network-fast-train`): each line's delay is `8 × y × room + 1`
/// frames, with y these (blocks), and its allpass `floor(8 × c × Scale × room +
/// FAST_ALLPASS_EDGE) + 1`. A's and B's delays keep their fraction of a frame and are read
/// between frames, linearly; C's, D's and F's round down to whole frames, and R's (A's delay and
/// G's together) rounds down past [`FAST_R_EDGE`]. Read at Size 1.53, 10.54 and 100 (Scale 40 %)
/// and Scale 5 and 100 % at Size 1.53: A and B to 0.005 frame, the rest to the frame.
const FAST_DELAYS: [f64; LOOP] = [287.0993, 340.0779, 628.6862, 459.9587, 846.774, 1220.6339];
/// Where R's delay rounds up to its next frame under Smooth Fast (frames).
const FAST_R_EDGE: f64 = 0.494;
/// Each line's allpass under Smooth Fast, as [`ALLPASSES`]: C's and R's are 0.1 and 0.2 % below
/// and above those, the middle of what their allpasses at Size 1.53 and Scale 100 % (172 and 160
/// frames) and the other reads allow.
const FAST_ALLPASSES: [f64; LOOP] = [0.63406, 1.22440, 0.8693, 3.97925, 2.18381, 0.8053];
/// Where an allpass rounds up to its next frame under Smooth Fast (frames).
const FAST_ALLPASS_EDGE: f64 = 0.15;
/// Where each family's copy starts under Smooth Fast (blocks, reference room), rounded down to a
/// whole frame: 0 to 6 frames before Smooth None's (396, 3191, 1640, 1336, 2983 and 2337 to the
/// peak at Size 100).
const FAST_FAMILY_OFFSETS: [f64; LOOP] = [49.4299, 398.832, 204.9654, 166.9265, 372.8029, 292.0821];
/// Under Smooth Fast the input reaches the lines this many frames sooner than [`ENTRY_MS`] gives,
/// and each line's loop is that much longer than its first pass (every path through two lines
/// arrives 5 frames after their first passes), at every Size (`reverb-network-fast-train`).
pub(crate) const FAST_EARLIER: f64 = 5.0;

/// `x` rounded down past `edge` to a whole frame, or, while things move, the straight line through
/// the middle of the steps.
fn frames(x: f64, edge: f64, settled: bool) -> f64 {
    if settled { (x + edge).floor() } else { x + edge - 0.5 }
}

/// Line `k`'s delay and allpass under Smooth Fast, as [`length_at`].
fn fast_length_at(k: usize, room: f64, percent: f64, settled: bool) -> (f64, f64) {
    let x = BLOCK * FAST_DELAYS[k] * room;
    let line = match k {
        0 | 1 => x + 1.0,
        5 => frames(x, FAST_R_EDGE, settled),
        _ => frames(x, 0.0, settled) + 1.0,
    };
    (line, frames(BLOCK * FAST_ALLPASSES[k] * percent * room, FAST_ALLPASS_EDGE, settled) + 1.0)
}

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

/// Where family `k`'s copy starts under Smooth Fast, as [`front`].
fn fast_front(k: usize, room: f64, settled: bool) -> f64 {
    frames(BLOCK * FAST_FAMILY_OFFSETS[k] * room, 0.0, settled)
}

/// Where Low's family `k`'s copy starts (frames at 44.1 kHz) for size factor `room`: whole
/// blocks when `settled`; it peaks one frame later.
fn front(k: usize, room: f64, settled: bool) -> f64 {
    front_of(FAMILY_OFFSETS[k], room, settled)
}

/// Where a copy `offset` blocks from its direct reflections in the reference room starts in a room
/// of size factor `room`, as [`front`].
fn front_of(offset: f64, room: f64, settled: bool) -> f64 {
    let x = offset * room;
    BLOCK * if settled { x.round() } else { x }
}

/// A line of Sparse, Mid or High, delay and allpass (frames at 44.1 kHz), from its `row` of
/// [`SPARSE_LENGTHS`], [`MID_LENGTHS`] or [`HIGH_LENGTHS`], for size factor `room` and Scale
/// `percent`; the allpass is `extra` frames more than its blocks.
fn read_length_at(row: &[f64; 7], extra: f64, room: f64, percent: f64, settled: bool) -> (f64, f64) {
    // Blocks at the Scales read, linearly between them (and on past the last two).
    let at = |i: usize| (READ_SCALES[i], row[i]);
    let i = READ_SCALES.iter().rposition(|&s| s <= percent).unwrap_or(0).min(READ_SCALES.len() - 2);
    let ((s0, n0), (s1, n1)) = (at(i), at(i + 1));
    let n = n0 + (n1 - n0) * (percent - s0) / (s1 - s0);
    let delay = row[0];
    (blocks(delay * room, 0.5, settled) + 2.0, blocks((n - delay).max(0.0) * room, 0.5, settled) + extra)
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
/// With Smooth None, A and B run this much of a frame past their whole length, read by linear
/// interpolation (0.99 of the frame and 0.01 of the next): a slight loss of the highs on every
/// pass (0.18 dB at the top) and none of the lows. The other lines are whole. No line loses
/// anything else on a pass: the whole tail falls with the decay's loss per second, at every Size
/// (Decision 26). Under Smooth Fast A's and B's fractions are larger ([`FAST_DELAYS`]), and they
/// are all of Fast's loss of the highs.
const FRACTION: [f64; LOOP] = [0.01, 0.01, 0.0, 0.0, 0.0, 0.0];
/// The side each line is heard on (0 left, 1 right), all at one level before Stereo's rotation.
pub(crate) const SIDES: [usize; LOOP] = [1, 0, 1, 0, 0, 1];
/// The input's delay to the lines after each direct reflection, at Shape 0.5 (ms), and how much
/// more per unit of Shape below and above 0.5 (ms, times the size factor): −1, 775 and 1543
/// frames at Shape 0, 0.5 and 1 at Size 100, so 776 frames for the half below and 768 above.
pub(crate) const ENTRY_MS: (f64, f64, f64) = (17.574, 35.193, 34.830);
/// Longest delay a line needs (seconds, with room for Size 500): High's longest line.
pub(crate) const LINE_SECONDS: f64 = 0.65;
/// Longest allpass a line needs (seconds, with room for Size 500): Mid's and High's longest
/// line's Scale part.
pub(crate) const ALLPASS_SECONDS: f64 = 0.45;
/// Longest family offset (seconds, with room for Size 500).
const FRONT_SECONDS: f64 = 0.2;

/// Density Mid's and High's lines, in [`Design`] order: the delay (blocks, its length `8 n + 2`
/// frames), then the delay and allpass together (blocks, `8 n + 3` frames) at each Scale of
/// [`READ_SCALES`], read at Size 100 (`reverb-network-density`, first passes and the second passes
/// through each pair of lines). The allpass is the part that grows with Scale; at other Sizes
/// both parts take the size factor, as Low's do, which is not measured. The steps between Scales
/// are not one rounding of a straight line (some 20 % steps are a block short), so the blocks are
/// kept as read and joined linearly.
const MID_LENGTHS: [[f64; 7]; 8] = [
    [444.0, 468.0, 540.0, 637.0, 733.0, 829.0, 926.0],
    [357.0, 398.0, 521.0, 684.0, 847.0, 1010.0, 1174.0],
    [623.0, 656.0, 757.0, 892.0, 1026.0, 1160.0, 1295.0],
    [1028.0, 1054.0, 1131.0, 1235.0, 1339.0, 1442.0, 1546.0],
    [503.0, 527.0, 603.0, 703.0, 802.0, 902.0, 1002.0],
    [437.0, 474.0, 584.0, 731.0, 877.0, 1024.0, 1171.0],
    [562.0, 607.0, 743.0, 925.0, 1107.0, 1288.0, 1469.0],
    [1185.0, 1253.0, 1454.0, 1723.0, 1993.0, 2261.0, 2530.0],
];
/// High's lines, as [`MID_LENGTHS`]: Mid's and two more, the second and the last.
const HIGH_LENGTHS: [[f64; 7]; 10] = [
    [444.0, 468.0, 540.0, 637.0, 733.0, 829.0, 926.0],
    [381.0, 413.0, 510.0, 640.0, 770.0, 899.0, 1029.0],
    [357.0, 398.0, 521.0, 684.0, 847.0, 1010.0, 1174.0],
    [623.0, 656.0, 757.0, 892.0, 1026.0, 1160.0, 1295.0],
    [1028.0, 1054.0, 1131.0, 1235.0, 1339.0, 1442.0, 1546.0],
    [503.0, 527.0, 603.0, 703.0, 802.0, 902.0, 1002.0],
    [437.0, 474.0, 584.0, 731.0, 877.0, 1024.0, 1171.0],
    [562.0, 607.0, 743.0, 925.0, 1107.0, 1288.0, 1469.0],
    [1185.0, 1253.0, 1454.0, 1723.0, 1993.0, 2261.0, 2530.0],
    [1985.0, 2002.0, 2054.0, 2123.0, 2192.0, 2261.0, 2330.0],
];
/// The Scales (%) of [`MID_LENGTHS`]' columns: 0 is the delay alone.
const READ_SCALES: [f64; 7] = [0.0, 5.0, 20.0, 40.0, 60.0, 80.0, 100.0];

/// Sparse's lines X, Y, Z and W, as [`MID_LENGTHS`] but `8 n + 2` frames in all: the delay is
/// `8 n + 2` and the allpass `8 n` (`reverb-network-density`). Y and Z are not Mid's lines;
/// X, Z and W have Mid's fixed delays within a block.
const SPARSE_LENGTHS: [[f64; 7]; 4] = [
    [357.0, 369.0, 406.0, 455.0, 504.0, 553.0, 603.0],
    [443.0, 449.0, 465.0, 488.0, 510.0, 532.0, 554.0],
    [561.0, 565.0, 576.0, 591.0, 606.0, 621.0, 635.0],
    [1185.0, 1192.0, 1213.0, 1241.0, 1268.0, 1296.0, 1323.0],
];
/// Sparse's four direct reflections each reach the lines on their own.
const SPARSE_TAP_GROUPS: [usize; DIRECT_TAPS] = pad_groups([0, 1, 2, 3]);
/// Each of Sparse's lines' input, as [`MID_INPUTS`] (each weight is against a direct reflection of
/// the first one's level, whose sign it does not take).
const SPARSE_INPUTS: [[f64; 4]; 4] = [
    [-1.6120, -0.4120, -0.8280, 1.6400],
    [0.5990, 2.1400, -0.1890, 1.0230],
    [1.6190, -0.8600, -1.5630, 0.5690],
    [0.7570, -0.7360, 1.6880, 1.3990],
];
/// Sparse's direct reflections are 1.5 times as loud as the others' and four, where the input
/// normalises by the root of the sum of their squares: this much of the 3.16 that Low's ten have.
const SPARSE_INPUT_GAIN: f64 = 0.9487;
/// Each of Sparse's lines' family, as [`MID_COPIES`].
#[allow(clippy::approx_constant, reason = "a fitted value that is near π/8 by chance")]
const SPARSE_COPIES: [[f64; 4]; 4] = [
    [-0.7930, 0.0336, 0.6542, -0.8982],
    [0.3394, -1.2442, -0.0412, -0.4358],
    [0.9219, 0.5558, -0.0835, -0.8390],
    [0.5325, -0.0322, 1.1925, 0.3927],
];
/// Where each of Sparse's families' copy starts: it peaks 2985, 3193, 2337 and 1641 frames after
/// its direct reflections, in Low's blocks.
const SPARSE_OFFSETS: [f64; 4] = [FAMILY_OFFSETS[4], FAMILY_OFFSETS[1], FAMILY_OFFSETS[5], FAMILY_OFFSETS[2]];
/// Sparse's mixing, as [`MID_MIX`].
const SPARSE_MIX: [[f64; 4]; 4] = [
    [-0.1673, -0.3341, 0.6631, -0.6486],
    [0.8718, -0.0777, 0.4175, 0.2440],
    [-0.3485, -0.6331, 0.2299, 0.6518],
    [-0.3002, 0.6939, 0.5770, 0.3090],
];
/// Sparse's sides: X and Y on the left, Z and W on the right.
const SPARSE_SIDES: [usize; 4] = [0, 0, 1, 1];

/// `groups` for the first reflections, and none for the rest.
const fn pad_groups<const N: usize>(groups: [usize; N]) -> [usize; DIRECT_TAPS] {
    let mut out = [usize::MAX; DIRECT_TAPS];
    let mut i = 0;
    while i < N {
        out[i] = groups[i];
        i += 1;
    }
    out
}

/// Mid's direct reflections reach the lines in 8 groups: the first and the fourth reflection as
/// one, with weights equal and opposite to their levels, and so the second and the seventh.
const MID_TAP_GROUPS: [usize; DIRECT_TAPS] = [0, 1, 2, 0, 3, 4, 1, 5, 6, 7];
/// Each of Mid's lines' input: its weight on each group times its output level, the first pass
/// against one direct reflection of the group.
const MID_INPUTS: [[f64; 8]; 8] = [
    [0.0752, -0.6203, -1.4157, 0.2637, -1.3792, 0.8928, 0.2110, 0.8309],
    [0.3001, 1.5956, 0.1161, -0.1601, -1.1467, 0.6674, 0.1731, -1.2484],
    [1.2471, -0.5603, 1.0326, 0.9641, -0.1397, 0.8984, -1.1400, 0.0150],
    [0.2102, -0.4120, -0.1727, -2.0335, 0.3170, 0.8667, -0.8004, -0.1686],
    [-0.6229, 0.3253, 1.4170, -0.3365, -0.1671, 0.9650, 0.9465, 1.2416],
    [0.9590, 0.5226, -0.7926, 0.3384, 1.5033, 0.9734, 0.9348, 0.0527],
    [-1.4164, 0.8425, -0.4587, 0.6245, 0.5737, 0.7272, -1.3265, 0.2895],
    [-0.9832, -1.1993, 0.1881, 0.3803, 0.1573, 0.8441, 0.6337, -1.4033],
];
/// Each of Mid's lines' family, as it joins the line's output: 1.84 times the family's filtered
/// reflections against their direct ones, as Low's [`FAMILY_GAINS`] are.
const MID_COPIES: [[f64; 8]; 8] = [
    [-0.5279, 0.1311, 0.7484, 0.5700, 0.0491, 0.5809, 0.5327, 0.0836],
    [-0.0887, 0.9085, 0.2623, -0.4871, 0.3573, 0.3796, -0.5605, -0.2580],
    [0.3048, -0.6822, -0.0194, 0.0000, 0.9165, 0.4822, 0.0134, -0.4271],
    [0.7103, 0.4840, -0.5156, 0.4487, -0.3001, 0.5026, 0.3616, -0.3816],
    [0.1273, -0.4879, 0.4566, -0.0889, -0.8140, 0.3788, -0.5535, -0.4727],
    [0.6341, -0.0710, 0.2364, -0.2251, -0.0330, 0.4529, -0.1009, 1.0362],
    [-0.5791, -0.1232, -0.7189, 0.4483, 0.0072, 0.5108, -0.6251, 0.3192],
    [-0.4236, -0.1177, -0.3877, -0.8803, -0.2954, 0.4799, 0.6006, -0.0194],
];
/// Where each of Mid's families' copy starts (blocks, reference room): it peaks 3193, 2985, 961,
/// 1337, 2497, 1641, 2337 and 401 frames after its direct reflections at Size 100, those Low has
/// in Low's blocks.
const MID_OFFSETS: [f64; 8] = [
    FAMILY_OFFSETS[1],
    FAMILY_OFFSETS[4],
    120.0,
    FAMILY_OFFSETS[3],
    312.0,
    FAMILY_OFFSETS[2],
    FAMILY_OFFSETS[5],
    FAMILY_OFFSETS[0],
];
/// Mid's mixing, as [`MIX`]: made orthogonal from the fit, whose rows were orthogonal to 1 %.
#[allow(clippy::approx_constant, reason = "a fitted value that is near Euler's constant by chance")]
const MID_MIX: [[f64; 8]; 8] = [
    [0.3683, 0.1089, -0.5817, -0.5682, 0.0864, 0.0322, 0.3445, -0.2530],
    [0.2738, -0.0658, 0.0484, -0.4655, 0.0708, 0.1216, -0.5105, 0.6491],
    [0.3658, 0.3945, 0.4225, -0.0576, -0.4669, 0.5088, 0.0048, -0.2277],
    [0.3560, -0.8371, -0.0694, 0.1303, -0.3314, 0.0861, -0.0680, -0.1699],
    [0.3949, -0.1376, 0.5762, -0.0673, 0.3871, -0.2537, 0.5069, 0.1322],
    [0.3996, 0.1362, -0.3218, 0.6134, 0.3823, 0.3886, 0.0205, 0.2107],
    [0.2975, 0.2551, -0.1855, 0.2348, -0.5423, -0.5772, 0.1158, 0.3410],
    [0.3532, 0.1574, 0.0777, 0.0653, 0.2640, -0.4116, -0.5876, -0.5058],
];
/// Mid's sides: four lines heard on the left, four on the right.
const MID_SIDES: [usize; 8] = [0, 0, 0, 0, 1, 1, 1, 1];

/// High's direct reflections each reach the lines on their own.
const HIGH_TAP_GROUPS: [usize; DIRECT_TAPS] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
/// Each of High's lines' input, as [`MID_INPUTS`].
const HIGH_INPUTS: [[f64; 10]; 10] = [
    [0.3489, 1.1670, -0.1361, 1.0533, -0.4631, 0.0100, 1.3012, -0.8455, -0.7958, 0.2516],
    [-0.5246, -0.4824, 1.1040, -0.8033, -0.6152, -1.2950, 1.1572, 0.2154, 0.0009, 0.4947],
    [1.3323, -0.6285, -1.1228, -0.4324, 0.3345, -0.6819, 0.9199, 0.0170, 0.2692, -0.9601],
    [-0.7756, 0.5521, -0.3357, -0.0285, -1.1936, -0.3286, -0.2226, -0.7851, 1.3005, -1.0464],
    [-0.0633, -0.2775, -0.3145, -1.3490, -1.0629, 1.0638, 0.0569, -0.3118, -1.1971, -0.3588],
    [-0.1499, -1.1368, 0.4698, 0.6943, 0.1877, -0.7194, -0.6787, -1.3025, -0.9237, -0.6666],
    [-1.0738, -0.2636, -1.2635, -0.5013, 0.8118, -0.0878, 0.3134, -1.0224, 0.1623, 1.0732],
    [-0.2554, -0.7696, -1.1345, 0.9573, -1.2317, -0.4484, -0.2701, 0.9381, -0.3424, 0.5893],
    [0.7784, 1.0137, -0.2645, -0.7414, -0.2340, -1.1961, -1.1568, -0.2559, -0.4088, 0.6124],
    [-1.1453, 0.7659, -0.3891, -0.0994, 0.6801, -0.5935, 0.0942, 0.8846, -0.9501, -1.0664],
];
/// Each of High's lines' family, as [`MID_COPIES`].
const HIGH_COPIES: [[f64; 10]; 10] = [
    [0.2995, -0.6917, -0.2195, 0.6124, 0.2888, -0.5071, -0.0015, 0.2651, 0.5601, 0.2923],
    [-0.4620, -0.5734, -0.3215, 0.1208, -0.3785, 0.4638, 0.3233, -0.7983, 0.2007, 0.1312],
    [-0.0337, -0.2706, 0.6835, -0.0384, 0.3726, -0.1296, -0.7833, -0.6487, 0.1161, -0.2414],
    [0.6603, -0.2093, -0.4159, -0.3505, 0.6107, 0.3185, 0.3194, -0.2003, 0.0210, -0.6625],
    [0.0204, -0.7125, -0.3447, -0.5079, -0.5377, -0.1207, -0.5633, 0.3839, -0.3476, -0.2346],
    [-0.5555, -0.2500, 0.3440, 0.1707, -0.0013, -0.5266, 0.5881, 0.1393, -0.1951, -0.7866],
    [-0.4775, 0.1863, -0.2334, 0.3818, 0.1013, 0.6029, -0.4544, 0.4569, 0.5311, -0.5420],
    [0.3548, -0.2258, 0.7600, -0.3432, -0.4446, 0.3560, 0.3205, 0.3014, 0.6396, -0.0254],
    [-0.6510, -0.1396, -0.0624, -0.7974, 0.6341, -0.0779, 0.1094, 0.2235, 0.2998, 0.4018],
    [-0.0424, -0.4754, 0.3909, 0.3332, 0.4051, 0.6567, 0.0896, 0.3525, -0.6984, 0.2841],
];
/// Where each of High's families' copy starts, as [`MID_OFFSETS`]: it peaks 3193, 2393, 2985,
/// 961, 1337, 2497, 1641, 2337, 401 and 2073 frames after its direct reflections.
const HIGH_OFFSETS: [f64; 10] = [
    FAMILY_OFFSETS[1],
    299.0,
    FAMILY_OFFSETS[4],
    120.0,
    FAMILY_OFFSETS[3],
    312.0,
    FAMILY_OFFSETS[2],
    FAMILY_OFFSETS[5],
    FAMILY_OFFSETS[0],
    259.0,
];
/// High's mixing, as [`MID_MIX`].
const HIGH_MIX: [[f64; 10]; 10] = [
    [-0.3505, 0.4295, -0.1924, -0.0537, 0.0075, -0.3268, 0.1435, 0.1060, 0.5352, 0.4771],
    [0.0860, -0.3274, -0.2498, 0.4474, -0.5296, 0.0010, -0.2102, 0.2026, 0.4694, -0.1917],
    [0.0058, -0.1742, 0.1377, -0.4571, -0.2860, 0.1088, 0.5422, -0.3906, 0.3727, -0.2503],
    [-0.3179, -0.0114, -0.4861, -0.1392, -0.1307, 0.5316, -0.3165, -0.4312, -0.0892, 0.2223],
    [-0.1294, -0.5533, -0.4348, -0.1269, 0.4352, -0.4947, -0.0239, -0.1498, 0.0246, -0.1194],
    [-0.5303, 0.2847, 0.0787, 0.1914, -0.2975, -0.3769, -0.0621, -0.2746, -0.2756, -0.4573],
    [-0.4159, -0.2044, 0.3309, -0.5132, -0.0401, 0.0648, -0.4358, 0.4347, 0.1265, -0.1082],
    [0.3864, 0.3906, -0.5008, -0.4627, -0.1853, -0.1402, -0.1052, 0.2412, -0.1112, -0.3123],
    [-0.1085, -0.3080, -0.0982, -0.1086, -0.4948, -0.1701, 0.3284, 0.2563, -0.4894, 0.4303],
    [0.3696, -0.0376, 0.2832, -0.1628, -0.2488, -0.3990, -0.4773, -0.4456, 0.0437, 0.3254],
];
/// High's sides: five lines on each.
const HIGH_SIDES: [usize; 10] = [0, 0, 0, 0, 0, 1, 1, 1, 1, 1];

/// How a design's lines get their lengths.
#[derive(Debug, PartialEq)]
enum Laws {
    /// Low's: [`length_at`], and [`fast_length_at`] under Smooth Fast.
    Low,
    /// Read at Size 100 at each Scale ([`read_length_at`]), the same under Smooth Fast.
    Read(&'static [[f64; 7]], f64),
}

/// One Density's measured network.
#[derive(Debug, PartialEq)]
pub(crate) struct Design {
    /// Lines in the loop.
    pub lines: usize,
    /// The group each direct reflection feeds, in table order.
    pub tap_groups: [usize; DIRECT_TAPS],
    /// Each line's input: its weight on each group ...
    inputs: [[f64; MAX_GROUPS]; MAX_LINES],
    /// ... times this.
    input_gains: [f64; MAX_LINES],
    /// Each line's family, when it is not its input (as at Low).
    copies: Option<[[f64; MAX_GROUPS]; MAX_LINES]>,
    /// Where each line's family's copy starts (blocks, reference room).
    offsets: [f64; MAX_LINES],
    /// The mixing as read; [`orthonormal`] makes it exactly orthogonal.
    mix: [[f64; MAX_LINES]; MAX_LINES],
    /// The side each line is heard on.
    pub sides: [usize; MAX_LINES],
    /// How much of a frame past its length each line is read with Smooth None.
    fractions: [f64; MAX_LINES],
    /// How the lines get their lengths.
    laws: Laws,
    /// How much of the Diffusion setting the allpasses take.
    pub diffusion: f32,
}

/// `row` padded with zeros to `M` values.
const fn pad<const N: usize, const M: usize>(row: [f64; N]) -> [f64; M] {
    let mut out = [0.0; M];
    let mut i = 0;
    while i < N {
        out[i] = row[i];
        i += 1;
    }
    out
}

/// `rows` padded with zeros to `M` rows of `M` values.
const fn pad_rows<const N: usize, const C: usize, const M: usize>(rows: [[f64; C]; N]) -> [[f64; M]; M] {
    let mut out = [[0.0; M]; M];
    let mut i = 0;
    while i < N {
        out[i] = pad(rows[i]);
        i += 1;
    }
    out
}

/// `sides` padded to [`MAX_LINES`]; a line past the last is heard nowhere.
const fn pad_sides<const N: usize>(sides: [usize; N]) -> [usize; MAX_LINES] {
    let mut out = [2; MAX_LINES];
    let mut i = 0;
    while i < N {
        out[i] = sides[i];
        i += 1;
    }
    out
}

/// Low's network: each line's input is its family.
static LOW: Design = Design {
    lines: LOOP,
    tap_groups: TAP_GROUPS,
    inputs: pad_rows(FAMILIES),
    input_gains: pad(FAMILY_GAINS),
    copies: None,
    offsets: pad(FAMILY_OFFSETS),
    mix: pad_rows(MIX),
    sides: pad_sides(SIDES),
    fractions: pad(FRACTION),
    laws: Laws::Low,
    diffusion: 1.0,
};
/// Sparse's network.
static SPARSE: Design = Design {
    lines: 4,
    tap_groups: SPARSE_TAP_GROUPS,
    inputs: pad_rows(SPARSE_INPUTS),
    input_gains: pad([SPARSE_INPUT_GAIN; 4]),
    copies: Some(pad_rows(SPARSE_COPIES)),
    offsets: pad(SPARSE_OFFSETS),
    mix: pad_rows(SPARSE_MIX),
    sides: pad_sides(SPARSE_SIDES),
    fractions: [0.0; MAX_LINES],
    laws: Laws::Read(&SPARSE_LENGTHS, 0.0),
    // The fifth that the Householder network took (Decision 17), not read from the measured one.
    diffusion: 0.2,
};
/// Mid's network.
static MID: Design = Design {
    lines: 8,
    tap_groups: MID_TAP_GROUPS,
    inputs: pad_rows(MID_INPUTS),
    input_gains: [1.0; MAX_LINES],
    copies: Some(pad_rows(MID_COPIES)),
    offsets: pad(MID_OFFSETS),
    mix: pad_rows(MID_MIX),
    sides: pad_sides(MID_SIDES),
    fractions: [0.0; MAX_LINES],
    laws: Laws::Read(&MID_LENGTHS, 1.0),
    diffusion: 1.0,
};
/// High's network.
static HIGH: Design = Design {
    lines: 10,
    tap_groups: HIGH_TAP_GROUPS,
    inputs: pad_rows(HIGH_INPUTS),
    input_gains: [1.0; MAX_LINES],
    copies: Some(pad_rows(HIGH_COPIES)),
    offsets: pad(HIGH_OFFSETS),
    mix: pad_rows(HIGH_MIX),
    sides: pad_sides(HIGH_SIDES),
    fractions: [0.0; MAX_LINES],
    laws: Laws::Read(&HIGH_LENGTHS, 1.0),
    diffusion: 1.0,
};

/// The measured network of Density `density` (0 Sparse to 3 High); an unknown one runs Low's.
pub(crate) fn design(density: usize) -> &'static Design {
    match density {
        0 => &SPARSE,
        2 => &MID,
        3 => &HIGH,
        _ => &LOW,
    }
}

/// `design`'s mixing with its rows made exactly orthonormal (Gram-Schmidt), so the loop neither
/// gains nor loses energy by itself however long it rings.
fn orthonormal(design: &Design) -> [[f64; MAX_LINES]; MAX_LINES] {
    let n = design.lines;
    let mut rows = design.mix;
    for i in 0..n {
        for j in 0..i {
            let dot: f64 = rows[i][..n].iter().zip(&rows[j][..n]).map(|(a, b)| a * b).sum();
            let done = rows[j];
            for (x, d) in rows[i][..n].iter_mut().zip(done) {
                *x -= dot * d;
            }
        }
        let norm = rows[i][..n].iter().map(|x| x * x).sum::<f64>().sqrt();
        for x in &mut rows[i][..n] {
            *x /= norm;
        }
    }
    rows
}

/// The lengths and gains the measured network runs with, worked out at the control rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Lengths {
    /// Each line's delay (frames).
    pub lines: [f64; MAX_LINES],
    /// How much of a frame past its delay each line is read, by linear interpolation.
    pub fractions: [f64; MAX_LINES],
    /// Each line's allpass (frames).
    pub allpasses: [f64; MAX_LINES],
    /// Each line's gain per trip through its delay.
    pub gains: [f64; MAX_LINES],
    /// Each line's allpass gain per pass through the allpass's delay.
    pub losses: [f64; MAX_LINES],
    /// Each line's share of a damping trip.
    pub trips: [f64; MAX_LINES],
    /// Each family's delay to its copy's low-pass (frames).
    pub fronts: [f64; MAX_LINES],
    /// Each copy's level: a filtered reflection fades as the reflections do over its own offset.
    pub copy_gains: [f64; MAX_LINES],
}

impl Default for Lengths {
    fn default() -> Self {
        Self {
            lines: [2.0; MAX_LINES],
            fractions: [0.0; MAX_LINES],
            allpasses: [2.0; MAX_LINES],
            gains: [0.0; MAX_LINES],
            losses: [0.0; MAX_LINES],
            trips: [0.0; MAX_LINES],
            fronts: [1.0; MAX_LINES],
            copy_gains: [0.0; MAX_LINES],
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
        mix(&mut self.fractions, &from.fractions, &to.fractions, t);
        mix(&mut self.allpasses, &from.allpasses, &to.allpasses, t);
        mix(&mut self.gains, &from.gains, &to.gains, t);
        mix(&mut self.losses, &from.losses, &to.losses, t);
        mix(&mut self.trips, &from.trips, &to.trips, t);
        mix(&mut self.fronts, &from.fronts, &to.fronts, t);
        mix(&mut self.copy_gains, &from.copy_gains, &to.copy_gains, t);
    }

    /// `design`'s lengths for size factor `room`, Scale `scale` (0..1) at `sample_rate`, under
    /// Smooth Fast (or Slow) when `fast`; `gain` gives a line's gain per trip from its length in
    /// seconds. Whole frames when `settled`, but for A's and B's fractions under Fast. Only Low's
    /// lines are measured under Fast: Mid's and High's keep None's lengths.
    #[allow(clippy::too_many_arguments)]
    pub fn tune(
        &mut self,
        design: &Design,
        room: f64,
        scale: f64,
        sample_rate: f64,
        settled: bool,
        fast: bool,
        gain: impl Fn(f64) -> f64,
        trip_seconds: f64,
    ) {
        let frames = sample_rate / RATE;
        let whole = |x: f64| if settled { x.round() } else { x }.max(2.0);
        let percent = scale * 100.0;
        let low = design.laws == Laws::Low;
        let fast = fast && low;
        for k in 0..design.lines {
            let (line, allpass) = match design.laws {
                Laws::Low if fast => fast_length_at(k, room, percent, settled),
                Laws::Low => length_at(k, room, percent, settled),
                Laws::Read(rows, extra) => read_length_at(&rows[k], extra, room, percent, settled),
            };
            let (line, allpass) = (line * frames, allpass * frames);
            if fast && k < 2 {
                // Read between frames, linearly, moving or not.
                self.lines[k] = line.floor().max(2.0);
                self.fractions[k] = line - self.lines[k];
            } else {
                self.lines[k] = whole(line);
                self.fractions[k] = if fast { 0.0 } else { design.fractions[k] };
            }
            self.allpasses[k] = whole(allpass);
            // The allpass loses its own time on each pass, so the echoes it holds back decay
            // as fast as the rest (as Live's tail does at every Diffusion).
            self.gains[k] = gain(line / sample_rate);
            self.losses[k] = gain(allpass / sample_rate);
            self.trips[k] = (line + allpass) / sample_rate / trip_seconds;
            let front = if fast { fast_front(k, room, settled) } else { front_of(design.offsets[k], room, settled) };
            self.fronts[k] = whole(front * frames);
        }
    }

    /// Each of `design`'s copies' level for size factor `room`, as the reflections fade by `fade`
    /// (dB per ms of their delay) over its family's offset.
    pub fn fade(&mut self, design: &Design, room: f64, fade: f64) {
        for (gain, &offset) in self.copy_gains.iter_mut().zip(&design.offsets).take(design.lines) {
            let peak = front_of(offset, room, false) + 1.0;
            *gain = 10f64.powf(-fade * peak * 1_000.0 / RATE / 20.0);
        }
    }

    /// The longest delay of `design`'s network (frames).
    pub fn longest(design: &Design, room: f64, sample_rate: f64) -> f64 {
        let line = |k: usize| match design.laws {
            Laws::Low => length_at(k, room, 100.0, false).0,
            Laws::Read(rows, extra) => read_length_at(&rows[k], extra, room, 100.0, false).0,
        };
        (0..design.lines).map(line).fold(0.0, f64::max) * sample_rate / RATE
    }
}

/// The measured network's lines and state.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Network {
    /// The lines, each with its allpass and damping; the design's first ones run.
    pub lines: [Line; MAX_LINES],
    /// Each family on its way to its copy.
    fronts: [DelayLine; MAX_LINES],
    /// The copies' low-pass, the reflections' own.
    copy_filters: [Biquad; MAX_LINES],
    /// The network running.
    design: &'static Design,
    /// Its mixing, exactly orthogonal.
    mix: [[f64; MAX_LINES]; MAX_LINES],
}

impl Default for Network {
    fn default() -> Self {
        Self {
            lines: Default::default(),
            fronts: Default::default(),
            copy_filters: Default::default(),
            design: &LOW,
            mix: orthonormal(&LOW),
        }
    }
}

impl Network {
    /// A network for `sample_rate`, running Low's design (allocates).
    pub fn new(sample_rate: f64) -> Self {
        Self {
            lines: std::array::from_fn(|_| Line {
                delay: DelayLine::for_seconds(LINE_SECONDS, sample_rate),
                diffuser: Allpass::for_seconds(ALLPASS_SECONDS, sample_rate),
                damper: Damper::default(),
            }),
            fronts: std::array::from_fn(|_| DelayLine::for_seconds(FRONT_SECONDS, sample_rate)),
            ..Self::default()
        }
    }

    /// Runs `design`, silent from its start if it was not running.
    pub fn set_design(&mut self, design: &'static Design) {
        if !std::ptr::eq(self.design, design) {
            self.design = design;
            self.mix = orthonormal(design);
            self.clear();
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
        groups: &[f64; MAX_GROUPS],
        n: &Lengths,
        diffusion: f32,
        damping: &Damping,
        swing: impl Fn(usize, f64) -> f64,
        frozen: bool,
    ) -> [f64; 2] {
        let d = self.design;
        let lines = d.lines;
        let weigh = |row: &[f64; MAX_GROUPS]| row.iter().zip(groups).map(|(w, g)| w * g).sum::<f64>();
        let families: [f64; MAX_LINES] = std::array::from_fn(|k| d.input_gains[k] * weigh(&d.inputs[k]));
        let sources = match &d.copies {
            Some(copies) => std::array::from_fn(|k| weigh(&copies[k])),
            None => families,
        };
        let mut copies = [0.0f64; MAX_LINES];
        for ((((front, filter), copy), (&family, &delay)), &level) in self
            .fronts
            .iter_mut()
            .zip(&mut self.copy_filters)
            .zip(&mut copies)
            .zip(sources.iter().zip(&n.fronts))
            .zip(&n.copy_gains)
            .take(lines)
        {
            *copy = level * filter.process(f64::from(front.read_hermite(delay.max(2.0))));
            front.push(family as f32);
        }

        let mut outs = [0.0f64; MAX_LINES];
        let mut tail = [0.0f64; 2];
        for (k, (line, out)) in self.lines.iter_mut().zip(&mut outs).enumerate().take(lines) {
            let length = n.lines[k] + swing(k, n.lines[k]);
            let mut read = f64::from(line.delay.read_hermite(length));
            if n.fractions[k] != 0.0 {
                read += n.fractions[k] * (f64::from(line.delay.read_hermite(length + 1.0)) - read);
            }
            let y = f64::from(line.diffuser.process_lossy(read as f32, n.allpasses[k], diffusion, n.losses[k] as f32));
            *out = line.damper.damp(y, damping) * n.gains[k];
            if let Some(t) = tail.get_mut(d.sides[k]) {
                *t += *out;
            }
        }
        // Into the mix: each line's output and its family's copy.
        let into: [f64; MAX_LINES] = std::array::from_fn(|j| outs[j] + copies[j]);
        for (k, line) in self.lines.iter_mut().enumerate().take(lines) {
            let mixed: f64 = self.mix[k][..lines].iter().zip(&into).map(|(m, y)| m * y).sum();
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
        for design in [&SPARSE, &LOW, &MID, &HIGH] {
            let mix = orthonormal(design);
            let n = design.lines;
            for i in 0..n {
                for j in 0..n {
                    let dot: f64 = (0..n).map(|k| mix[k][i] * mix[k][j]).sum();
                    let want = if i == j { 1.0 } else { 0.0 };
                    assert!((dot - want).abs() < 1e-12, "{n} lines, columns {i} and {j}: {dot}");
                    let read = design.mix[i][j];
                    assert!((mix[i][j] - read).abs() < 2e-3, "{n} lines, {i}, {j}: {} against {read}", mix[i][j]);
                }
            }
        }
    }

    #[test]
    fn sparses_lines_are_their_measured_lengths() {
        // Live's first passes at Size 100 less the entry (frames), as below.
        let measured: [[f64; 6]; 4] = [
            [2954.0, 3250.0, 3642.0, 4034.0, 4426.0, 4826.0],
            [3594.0, 3722.0, 3906.0, 4082.0, 4258.0, 4434.0],
            [4522.0, 4610.0, 4730.0, 4850.0, 4970.0, 5082.0],
            [9538.0, 9706.0, 9930.0, 10146.0, 10370.0, 10586.0],
        ];
        for (k, (row, lengths)) in SPARSE_LENGTHS.iter().zip(measured).enumerate() {
            for (percent, want) in [5.0, 20.0, 40.0, 60.0, 80.0, 100.0].into_iter().zip(lengths) {
                let (line, allpass) = read_length_at(row, 0.0, 1.0, percent, true);
                assert_eq!(line + allpass, want, "line {k} at Scale {percent} %");
            }
        }
    }

    #[test]
    fn mids_and_highs_lines_are_their_measured_lengths() {
        // Live's first passes at Size 100 less the entry (frames), Scale 5, 20, 40, 60, 80 and
        // 100 % (`reverb-network-density`).
        let measured: [[f64; 6]; 10] = [
            [3747.0, 4323.0, 5099.0, 5867.0, 6635.0, 7411.0],
            [3307.0, 4083.0, 5123.0, 6163.0, 7195.0, 8235.0],
            [3187.0, 4171.0, 5475.0, 6779.0, 8083.0, 9395.0],
            [5251.0, 6059.0, 7139.0, 8211.0, 9283.0, 10363.0],
            [8435.0, 9051.0, 9883.0, 10715.0, 11539.0, 12371.0],
            [4219.0, 4827.0, 5627.0, 6419.0, 7219.0, 8019.0],
            [3795.0, 4675.0, 5851.0, 7019.0, 8195.0, 9371.0],
            [4859.0, 5947.0, 7403.0, 8859.0, 10307.0, 11755.0],
            [10027.0, 11635.0, 13787.0, 15947.0, 18091.0, 20243.0],
            [16019.0, 16435.0, 16987.0, 17539.0, 18091.0, 18643.0],
        ];
        for (k, (row, lengths)) in HIGH_LENGTHS.iter().zip(measured).enumerate() {
            for (percent, want) in [5.0, 20.0, 40.0, 60.0, 80.0, 100.0].into_iter().zip(lengths) {
                let (line, allpass) = read_length_at(row, 1.0, 1.0, percent, true);
                assert_eq!(line + allpass, want, "line {k} at Scale {percent} %");
            }
        }
        // Mid's are High's but the second and the last.
        let mid: Vec<_> =
            HIGH_LENGTHS.iter().enumerate().filter(|(k, _)| ![1, 9].contains(k)).map(|(_, r)| *r).collect();
        assert_eq!(mid, MID_LENGTHS);
    }

    #[test]
    fn each_line_of_mid_and_high_has_its_own_family() {
        // As at Low, each family's copy joins one line, and none shares its offset.
        for design in [&MID, &HIGH] {
            let mut peaks: Vec<_> =
                design.offsets[..design.lines].iter().map(|&o| (front_of(o, 1.0, true) + 1.0) as i64).collect();
            peaks.sort_unstable();
            peaks.dedup();
            assert_eq!(peaks.len(), design.lines);
        }
        assert_eq!(front_of(MID.offsets[2], 1.0, true) + 1.0, 961.0);
        assert_eq!(front_of(HIGH.offsets[9], 1.0, true) + 1.0, 2073.0);
    }

    #[test]
    fn every_direct_reflection_of_mid_and_high_has_a_group() {
        for (taps, design) in [(&crate::TAPS_MID[..], &MID), (&crate::TAPS_HIGH[..], &HIGH)] {
            assert_eq!(taps.iter().filter(|tap| !tap.filtered).count(), DIRECT_TAPS);
            assert!(design.tap_groups.iter().all(|&g| g < design.lines));
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
    fn under_smooth_fast_a_line_is_its_measured_loop() {
        // Live's loops under Smooth Fast (frames: each line's first pass and 5), read at each
        // (Size, Scale %) of `reverb-network-fast`.
        let measured = [
            (100.0, 40.0, [2501.795, 3113.620, 5309.0, 4954.0, 7474.0, 10023.0]),
            (10.54, 40.0, [1181.926, 1472.137, 2508.0, 2341.0, 3531.0, 4735.0]),
            (1.53, 5.0, [578.192, 689.411, 1258.0, 954.0, 1704.0, 2433.0]),
            (1.53, 40.0, [622.191, 774.411, 1319.0, 1231.0, 1856.0, 2489.0]),
            (1.53, 100.0, [698.192, 920.409, 1422.0, 1705.0, 2116.0, 2585.0]),
        ];
        for (size, percent, loops) in measured {
            let room = (size / 100.0f64).cbrt();
            for (k, want) in loops.into_iter().enumerate() {
                let (line, allpass) = fast_length_at(k, room, percent, true);
                let most = if k < 2 { 0.01 } else { 0.0 };
                assert!(
                    (line + allpass - want).abs() <= most,
                    "line {k} at Size {size}, Scale {percent} %: {}",
                    line + allpass
                );
            }
        }
    }

    #[test]
    fn under_smooth_fast_each_copy_starts_where_measured() {
        let measured = [
            (100.0, [396.0, 3191.0, 1640.0, 1336.0, 2983.0, 2337.0]),
            (10.54, [187.0, 1508.0, 775.0, 631.0, 1409.0, 1104.0]),
            (1.53, [99.0, 793.0, 408.0, 332.0, 741.0, 581.0]),
        ];
        for (size, peaks) in measured {
            let room = (size / 100.0f64).cbrt();
            for (k, want) in peaks.into_iter().enumerate() {
                assert_eq!(fast_front(k, room, true) + 1.0, want, "family {k} at Size {size}");
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
