// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The late reverb, as measured (docs/spec/reverb.md › Late reverb): lines mixed by one orthogonal
//! matrix, each fed by its own mix of the direct reflections, and each line's family of
//! reflections heard again, low-passed, one offset later, joining that line's output in the mix.
//! The direct reflections come in groups that reach the lines as one, and there are as many lines
//! as groups: Sparse 4, Low 6, Mid 8, High 10 ([`design`]).
//!
//! ```text
//! direct taps -> groups -> each line's input ------------------------> own line
//! family lines (from the reflections, an entry later) -> low-pass ---+
//! line outputs ------------------------------------------------------+-> MIX -> lines
//! ```
//!
//! At Low a line's family is its input. A family's copy is its filtered reflections, an entry
//! later (the reverb's family lines hold both). Times are frames at 44.1 kHz. A line is a delay and an
//! allpass (two in series at Mid and High), each a whole number of 8-frame blocks and one frame
//! with Smooth None, whole frames under Fast ([`length`]). The input reaches the lines [`entry`]
//! after each direct reflection, whatever the Size. Every value was read from Live's impulse
//! responses at Diffusion 0.1 % and Decay 60 s (`reverb-network-exact`, `reverb-network-density`
//! and the runs named below), where each arrival is one sample once the input's fixed filters are
//! divided out.

use suhina_dsp::{Allpass, Biquad, Coefficients, DelayLine};

use crate::{Damper, Damping, Line};

/// Lines in Low's loop: A, B, C, D, F and R.
const LOOP: usize = 6;
/// Groups of Low's direct reflections the families are made of.
const GROUPS: usize = 6;
/// The most lines a network has (High's).
pub(crate) const MAX_LINES: usize = 10;
/// The most groups of direct reflections a network takes (High's).
pub(crate) const MAX_GROUPS: usize = 10;
/// The most direct reflections a Density has (Low, Mid and High; Sparse has four).
pub(crate) const DIRECT_TAPS: usize = 10;
/// The families of filtered reflections every Density draws on: Low's six (A, B, C, D, F and R's)
/// and the four only Mid and High have.
pub(crate) const FAMILY_COUNT: usize = 10;
/// The side each family's reflections and line are heard on (0 left, 1 right), at every Density.
pub(crate) const FAMILY_SIDES: [usize; FAMILY_COUNT] = [1, 0, 1, 0, 0, 1, 0, 1, 0, 1];
/// The frame rate the lengths below are at.
const RATE: f64 = 44_100.0;

/// Under Smooth None Live builds every delay of the reverb from blocks of 8 frames (at 44.1 kHz).
const BLOCK: f64 = 8.0;
/// ... rounding a length up to its next block unless it is less than this much of a block past
/// one ([`rounded`]): with 0.88 to 0.9 every length read fits, lines, allpasses, families, the
/// Shape entry and Stereo's delay.
const BLOCK_EDGE: f64 = 0.9;

/// A delay of `x` frames (at 44.1 kHz) as Live rounds it: under Smooth Fast (and Slow) down to a
/// whole frame, under None to whole blocks, up but for the last tenth of a block. While things
/// move, the straight line through the middle of the steps, so a glide does not jump a step at a
/// time.
pub(crate) fn rounded(x: f64, fast: bool, settled: bool) -> f64 {
    match (fast, settled) {
        (true, true) => x.floor(),
        (true, false) => x - 0.5,
        (false, true) => BLOCK * (x / BLOCK + BLOCK_EDGE).floor(),
        (false, false) => x + BLOCK * (BLOCK_EDGE - 0.5),
    }
}

/// Low's lines, A, B, C, D, F and R: each delay in blocks of the reference room, and its allpass
/// in blocks per % of Scale ([`length`]). C's delay takes in
/// its family's offset, and R's A's delay and a delay G, each as one rounded length: at Size 0.222
/// rounding the parts on their own makes C a block longer and R a block shorter
/// (`reverb-network-small`). Each is the middle of what both Smooths' reads allow, at Size 0.222 to
/// 500 (R's also with its 1854 blocks at Size 350, `reverb-network-size-level`).
const LOW_LINES: [[f64; 3]; LOOP] = [
    [287.0993, 0.63476, 0.0],
    [340.0779, 1.22412, 0.0],
    [628.7004, 0.87055, 0.0],
    [459.9281, 3.97956, 0.0],
    [846.6372, 2.18481, 0.0],
    [1220.5325, 0.80455, 0.0],
];

/// Each family's line in Sparse, Mid and High: its delay (blocks), and its two allpasses' blocks
/// per Scale %, the shorter first ([`length`]). Each is the middle of what every line's first pass
/// allows at Size 0.222, 100, 300, 400 and 500 with Smooth None and Fast and at Scale 0 to 100 %
/// at Size 100 (`reverb-network-density`, `reverb-network-density-size`,
/// `reverb-network-size-level`). How a line's length splits between its two allpasses shows only
/// above Diffusion 0.1 % (`reverb-network-density-decay` at 60 %, `reverb-network-density-split`
/// at 30 and 60 % and Size 10.54).
const SHARED_LINES: [[f64; 3]; FAMILY_COUNT] = [
    [1184.4905, 0.6886, 12.7517],
    [442.6705, 1.1039, 3.7124],
    [436.192, 1.3804, 5.9511],
    [1026.769, 1.7356, 3.4461],
    [356.0745, 2.4623, 5.7064],
    [560.7041, 0.7406, 8.3358],
    [621.4635, 2.9617, 3.7544],
    [1983.288, 0.7584, 2.6925],
    [379.8665, 0.8037, 5.6726],
    [501.2132, 1.1817, 3.8076],
];

/// Where each family's filtered reflection starts after the direct one (blocks, reference room,
/// [`rounded`]): 401, 3193, 1641, 1337, 2985, 2337, 961, 2073, 2393 and 2497 frames to its peak at
/// Size 100 under Smooth None, 0 to 6 frames sooner under Fast. Its copy into the mix is heard there
/// too, through the same low-pass. Each is the middle of what the reads at Size 0.222 to 500 allow,
/// None and Fast (`reverb-network-small`, `reverb-network-density`, `reverb-network-density-size`,
/// `reverb-network-fast-train`, `reverb-network-size-level`).
const FAMILY_LENGTHS: [f64; FAMILY_COUNT] =
    [49.3955, 398.8085, 204.9654, 166.9645, 372.7764, 292.0502, 120.0675, 258.9858, 299.0174, 311.7368];

/// With Smooth None, the lines of families A and B run this much of a frame past their whole
/// length, read by linear interpolation: a slight loss of the highs on every pass (0.18 dB at the
/// top), and the network's only loss besides the decay. Under Fast their delays keep their whole
/// fraction instead ([`length`]). Without it the tails at Size 0.222 keep up to 5 dB too much above
/// 12 kHz by 3 s, and no other hundredth fits as well (`reverb-network-density-size`).
const NONE_FRACTION: f64 = 0.01;

/// A line's delay and allpasses (frames at 44.1 kHz) from `table`: the delay of row `delay` and the
/// allpasses of row `family` (the second only when `second`), for size factor `room` and Scale
/// `percent`. Each part is its blocks times `room` (an allpass's also times Scale), [`rounded`],
/// but under Smooth Fast the delay of a line of family A or B keeps its fraction; and a frame.
#[allow(clippy::too_many_arguments)]
fn length(
    table: &[[f64; 3]],
    delay: usize,
    family: usize,
    second: bool,
    room: f64,
    percent: f64,
    settled: bool,
    fast: bool,
) -> (f64, f64, f64) {
    let [blocks_delay, _, _] = table[delay];
    let [_, shorter, longer] = table[family];
    let part = |blocks: f64| rounded(BLOCK * blocks, fast, settled);
    let line = if fast && family < 2 { BLOCK * blocks_delay * room } else { part(blocks_delay * room) };
    let first = part(shorter * percent * room) + 1.0;
    let second = if second { part(longer * percent * room) + 1.0 } else { 0.0 };
    (line + 1.0, first, second)
}

/// Where family `k`'s filtered reflections peak after their direct ones (frames at 44.1 kHz) for
/// size factor `room`, under Smooth Fast (or Slow) when `fast`: [`front`] and a frame.
pub(crate) fn peak(k: usize, room: f64, settled: bool, fast: bool) -> f64 {
    front(k, room, settled, fast) + 1.0
}

/// Where family `k`'s copy starts (frames at 44.1 kHz) for size factor `room`, at every Density.
pub(crate) fn front(k: usize, room: f64, settled: bool, fast: bool) -> f64 {
    rounded(BLOCK * FAMILY_LENGTHS[k] * room, fast, settled)
}

/// Which group each direct reflection of Density Low belongs to, in table order.
const TAP_GROUPS: [usize; DIRECT_TAPS] = [0, 0, 1, 2, 2, 4, 5, 1, 3, 3];
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
/// How much of its family each line takes, times its output level: the first pass of each line
/// against one direct reflection.
const FAMILY_GAINS: [f64; LOOP] = [1.7860, 1.7959, 1.8109, 1.8022, 1.8092, 1.8012];
/// The loop's mixing: row `i` is what line `i` takes from each line's output (and from each
/// family's copy, at the same weight). Read from the renders, it is orthogonal to 0.3 % without
/// being made so; [`orthonormal`] makes it exactly so.
const MIX: [[f64; LOOP]; LOOP] = [
    [0.1651, 0.43434, 0.7290, -0.2671, -0.0618, -0.4213],
    [-0.4845, 0.4152, -0.0838, 0.4661, 0.5380, -0.2813],
    [0.0587, 0.3811, 0.0875, 0.6188, -0.6243, 0.2665],
    [0.6681, 0.5295, -0.4350, -0.1034, 0.2579, 0.0826],
    [-0.3852, 0.2966, -0.4863, -0.4104, -0.4819, -0.3556],
    [-0.3740, 0.3540, 0.1679, -0.3865, 0.1348, 0.7342],
];
/// How much later the input reaches the lines per unit of Shape (frames at 44.1 kHz; 35 ms),
/// whatever the Size and the Density ([`entry`]).
const SHAPE_ENTRY: f64 = 1543.5;

/// The input's delay to the lines after each direct reflection (frames at 44.1 kHz) at `shape`:
/// [`SHAPE_ENTRY`] times Shape, [`rounded`], less a frame. With None −1, 391, 775, 1159 and 1543 frames at Shape 0, 0.25, 0.5, 0.75
/// and 1 (`reverb-network-exact`, `reverb-network-id`, `reverb-size` shape-*-tail), under Fast −1,
/// 384, 770, 1156 and 1542 (`reverb-structure` shape-*). Read at Size 100 and inferred at the
/// others: Shape 0.5's entry is 775 frames at every Size (`reverb-network-small`).
pub(crate) fn entry(shape: f64, settled: bool, fast: bool) -> f64 {
    rounded(SHAPE_ENTRY * shape, fast, settled) - 1.0
}
/// Longest delay a line needs (seconds, with room for Size 500): High's longest line.
const LINE_SECONDS: f64 = 0.65;
/// Longest allpass a line needs (seconds, with room for Size 500): Mid's and High's longest
/// line's Scale part.
const ALLPASS_SECONDS: f64 = 0.45;

/// The family each of Sparse's lines takes its delay from: its own, but the last's (family C), which
/// has A's (`reverb-network-density-size`).
const SPARSE_DELAYS: [usize; 4] = [4, 1, 5, 0];
/// Each of Sparse's lines' input, as [`MID_INPUTS`] (each weight is against a direct reflection of
/// the first one's level, whose sign it does not take).
const SPARSE_INPUTS: [[f64; 4]; 4] = [
    [-1.6120, -0.4120, -0.8280, 1.6400],
    [0.5990, 2.1400, -0.1890, 1.0230],
    [1.6190, -0.8600, -1.5630, 0.5690],
    [0.7570, -0.7360, 1.6880, 1.3990],
];
/// Sparse's inputs times this, fitted to Live's renders at Scale 5, 40 and 100 %.
const SPARSE_INPUT_GAIN: f64 = 0.9965;
/// Sparse's lines' families: they peak 2985, 3193, 2337 and 1641 frames after their direct
/// reflections.
const SPARSE_FAMILIES: [usize; 4] = [4, 1, 5, 2];
/// Sparse's mixing, as [`MID_MIX`], run with [`SPARSE_LEVELS`].
const SPARSE_MIX: [[f64; 4]; 4] = [
    [-0.166755, -0.334405, 0.663378, -0.648302],
    [0.871775, -0.077310, 0.418061, 0.243425],
    [-0.348542, -0.633013, 0.230244, 0.651767],
    [-0.301196, 0.693896, 0.576318, 0.309270],
];
/// Each of Sparse's lines' output level against the others', as [`MID_LEVELS`].
const SPARSE_LEVELS: [f64; 4] = [1.0032, 0.9986, 1.0023, 0.9960];

/// Mid's and High's inputs times this, fitted to Live's renders as Sparse's.
const MID_HIGH_INPUT_GAIN: f64 = 1.057;

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
/// Mid's lines' families: they peak 3193, 2985, 961, 1337, 2497, 1641, 2337 and 401 frames after
/// their direct reflections at Size 100.
const MID_FAMILIES: [usize; 8] = [1, 4, 6, 3, 9, 2, 5, 0];
/// Mid's mixing, as [`MIX`]: first read from the two-line paths, orthogonal to 1 %, then fitted as
/// a rotation of that (`M · exp(A)`, `A` antisymmetric, by Gauss-Newton) to Live's whole tails at
/// Size 0.222 and 500 (`reverb-network-density-size`), moving no entry more than 0.004 (Sparse's
/// 0.001, High's 0.007).
#[allow(clippy::approx_constant, reason = "a fitted value that is near Euler's constant by chance")]
const MID_MIX: [[f64; 8]; 8] = [
    [0.368161, 0.108775, -0.581296, -0.568820, 0.086265, 0.031482, 0.343733, -0.254124],
    [0.271656, -0.064204, 0.048059, -0.466431, 0.070065, 0.122714, -0.509307, 0.650271],
    [0.365622, 0.394272, 0.422556, -0.057502, -0.466574, 0.509036, 0.004660, -0.228428],
    [0.357116, -0.837262, -0.069221, 0.129876, -0.330419, 0.086300, -0.069221, -0.168481],
    [0.395007, -0.137308, 0.576006, -0.067730, 0.387312, -0.252981, 0.506916, 0.133311],
    [0.398688, 0.136657, -0.322884, 0.612085, 0.381779, 0.390011, 0.022096, 0.212413],
    [0.296946, 0.254839, -0.185537, 0.234442, -0.543292, -0.575630, 0.117608, 0.342265],
    [0.355373, 0.158482, 0.077321, 0.067185, 0.264291, -0.412222, -0.588521, -0.501964],
];
/// Each of Mid's lines' output level against the others'. Live's lines are not all heard at one
/// level, and [`MID_INPUTS`] were read with these in them, so the loop runs the
/// mixing as `level_i · mix_ij / level_j` ([`loop_mix`]). A free fit of each mix to every trip at
/// Size 0.222 (`reverb-network-density-size`) comes out as this mixing between lines at these levels
/// to 0.0002; they hold at Size 100 and 500 too.
const MID_LEVELS: [f64; 8] = [0.9992, 1.0013, 1.0026, 0.9997, 1.0009, 1.0017, 1.0028, 0.9919];

/// Each of High's lines' input, as [`MID_INPUTS`]; its direct reflections each reach the lines on
/// their own.
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
/// High's lines' families, as [`MID_FAMILIES`]: they peak 3193, 2393, 2985, 961, 1337, 2497, 1641,
/// 2337, 401 and 2073 frames after their direct reflections.
const HIGH_FAMILIES: [usize; 10] = [1, 8, 4, 6, 3, 9, 2, 5, 0, 7];
/// High's mixing, as [`MID_MIX`], run with [`HIGH_LEVELS`].
const HIGH_MIX: [[f64; 10]; 10] = [
    [-0.348113, 0.431273, -0.190544, -0.054873, 0.003126, -0.328649, 0.142605, 0.102578, 0.534750, 0.478192],
    [0.085721, -0.327071, -0.250251, 0.447068, -0.525203, 0.000062, -0.212144, 0.201146, 0.472698, -0.196046],
    [0.005687, -0.173832, 0.137437, -0.457475, -0.280688, 0.109642, 0.539417, -0.392038, 0.375058, -0.255809],
    [-0.319107, -0.011948, -0.485638, -0.138487, -0.133577, 0.531357, -0.315474, -0.430384, -0.090890, 0.223175],
    [-0.129695, -0.552898, -0.434898, -0.127689, 0.437665, -0.494356, -0.024158, -0.148806, 0.023321, -0.113284],
    [-0.531302, 0.283413, 0.077284, 0.191711, -0.292916, -0.376628, -0.061197, -0.272901, -0.275947, -0.461145],
    [-0.416906, -0.203191, 0.330902, -0.512201, -0.037540, 0.065516, -0.435820, 0.435421, 0.127347, -0.107735],
    [0.385716, 0.390266, -0.501488, -0.462877, -0.182072, -0.138763, -0.106304, 0.241407, -0.110776, -0.314508],
    [-0.107498, -0.309132, -0.097002, -0.110037, -0.501957, -0.170334, 0.331383, 0.257650, -0.484688, 0.423402],
    [0.369272, -0.038617, 0.284305, -0.163968, -0.252294, -0.398535, -0.478311, -0.446345, 0.039609, 0.319961],
];
/// Each of High's lines' output level against the others', as [`MID_LEVELS`].
const HIGH_LEVELS: [f64; 10] = [0.9988, 1.0030, 1.0019, 1.0025, 1.0007, 1.0022, 1.0044, 1.0013, 0.9930, 0.9922];

/// One Density's network.
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
    /// Each line's family: its copy's offset, its side, and whether it is an A or B line.
    families: [usize; MAX_LINES],
    /// The lengths, by family ([`length`]).
    lengths: &'static [[f64; 3]],
    /// The family each line takes its delay from, when not its own.
    delays: Option<&'static [usize]>,
    /// Whether each line runs a second allpass.
    second: bool,
    /// The mixing as read; [`orthonormal`] makes it exactly orthogonal.
    mix: [[f64; MAX_LINES]; MAX_LINES],
    /// Each line's output level against the others' ([`loop_mix`]).
    levels: [f64; MAX_LINES],
    /// How many times as much of the damping shelves' loss a line takes per second as Low's law
    /// gives ([`Lengths::trips`]).
    trip_scale: f64,
}

impl Design {
    /// The side line `k` is heard on: its family's.
    pub fn side(&self, k: usize) -> usize {
        FAMILY_SIDES[self.families[k]]
    }
}

/// `items` padded with `fill` to `M` values.
const fn pad<T: Copy, const N: usize, const M: usize>(items: [T; N], fill: T) -> [T; M] {
    let mut out = [fill; M];
    let mut i = 0;
    while i < N {
        out[i] = items[i];
        i += 1;
    }
    out
}

/// `rows` padded with zeros to `M` rows of `M` values.
const fn pad_rows<const N: usize, const C: usize, const M: usize>(rows: [[f64; C]; N]) -> [[f64; M]; M] {
    let mut out = [[0.0; M]; M];
    let mut i = 0;
    while i < N {
        out[i] = pad(rows[i], 0.0);
        i += 1;
    }
    out
}

/// Low's network: each line's input is its family.
static LOW: Design = Design {
    lines: LOOP,
    tap_groups: TAP_GROUPS,
    inputs: pad_rows(FAMILIES),
    input_gains: pad(FAMILY_GAINS, 0.0),
    families: pad([0, 1, 2, 3, 4, 5], 0),
    lengths: &LOW_LINES,
    delays: None,
    second: false,
    mix: pad_rows(MIX),
    levels: [1.0; MAX_LINES],
    trip_scale: 1.0,
};
/// Sparse's network: its four direct reflections each reach the lines on their own.
static SPARSE: Design = Design {
    lines: 4,
    tap_groups: pad([0, 1, 2, 3], usize::MAX),
    inputs: pad_rows(SPARSE_INPUTS),
    input_gains: pad([SPARSE_INPUT_GAIN; 4], 0.0),
    families: pad(SPARSE_FAMILIES, 0),
    lengths: &SHARED_LINES,
    delays: Some(&SPARSE_DELAYS),
    second: false,
    mix: pad_rows(SPARSE_MIX),
    levels: pad(SPARSE_LEVELS, 1.0),
    // Fitted to the High and Low Shelves at 4 kHz, 1 kHz and 500 Hz and the default damping at
    // Size 1.53, 10.54 and 100 (`reverb-network-density-filters`); the exponent of the length came
    // out at 0.99.
    trip_scale: 1.37,
};
/// Mid's network.
static MID: Design = Design {
    lines: 8,
    tap_groups: MID_TAP_GROUPS,
    inputs: pad_rows(MID_INPUTS),
    input_gains: [MID_HIGH_INPUT_GAIN; MAX_LINES],
    families: pad(MID_FAMILIES, 0),
    lengths: &SHARED_LINES,
    delays: None,
    second: true,
    mix: pad_rows(MID_MIX),
    levels: pad(MID_LEVELS, 1.0),
    trip_scale: 1.0,
};
/// High's network.
static HIGH: Design = Design {
    lines: 10,
    tap_groups: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    inputs: pad_rows(HIGH_INPUTS),
    input_gains: [MID_HIGH_INPUT_GAIN; MAX_LINES],
    families: HIGH_FAMILIES,
    lengths: &SHARED_LINES,
    delays: None,
    second: true,
    mix: pad_rows(HIGH_MIX),
    levels: HIGH_LEVELS,
    trip_scale: 1.0,
};

/// Cosine and sine of each line's Chorus phase: each side's lines spread evenly round the cycle,
/// the right's half a step after the left's. A side's lines then never swing the same way
/// together, so their Doppler shifts cancel in what it hears (with golden-angle phases Low's
/// highs above 16 kHz rose and fell 1.1 to 1.2 dB at the Chorus rate against Live's 0.6; with
/// these 0.3 to 0.5 dB, `reverb-modulation-density`).
fn chorus_phases(design: &Design) -> [(f64, f64); MAX_LINES] {
    let sides: Vec<usize> = (0..design.lines).map(|k| design.side(k)).collect();
    std::array::from_fn(|k| {
        let Some(&side) = sides.get(k) else { return (0.0, 0.0) };
        let count = sides.iter().filter(|&&s| s == side).count();
        let place = sides[..k].iter().filter(|&&s| s == side).count();
        let p = std::f64::consts::TAU * (place as f64 + 0.5 * side as f64) / count as f64;
        (p.cos(), p.sin())
    })
}

/// The network of Density `density` (0 Sparse to 3 High); an unknown one runs Low's.
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

/// The mixing the loop runs: [`orthonormal`]'s, between the lines as heard, each at its level.
/// The loop keeps its energy measured with each line taken at its level.
fn loop_mix(design: &Design) -> [[f64; MAX_LINES]; MAX_LINES] {
    let mut mix = orthonormal(design);
    let levels = &design.levels[..design.lines];
    for (row, level) in mix.iter_mut().zip(levels) {
        for (m, from) in row.iter_mut().zip(levels) {
            *m *= level / from;
        }
    }
    mix
}

/// The lengths and gains the network runs with, worked out at the control rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Lengths {
    /// Each line's delay (frames).
    pub lines: [f64; MAX_LINES],
    /// How much of a frame past its delay each line is read, by linear interpolation.
    pub fractions: [f64; MAX_LINES],
    /// Each line's allpass (frames).
    pub allpasses: [f64; MAX_LINES],
    /// Each line's second allpass, in series after the first (frames; 0 for none).
    pub seconds: [f64; MAX_LINES],
    /// Each line's gain per trip through its delay.
    pub gains: [f64; MAX_LINES],
    /// Each line's allpass gain per pass through the allpass's delay.
    pub losses: [f64; MAX_LINES],
    /// The same for the second allpass.
    pub second_losses: [f64; MAX_LINES],
    /// Each line's share of a damping trip.
    pub trips: [f64; MAX_LINES],
}

impl Default for Lengths {
    fn default() -> Self {
        Self {
            lines: [2.0; MAX_LINES],
            fractions: [0.0; MAX_LINES],
            allpasses: [2.0; MAX_LINES],
            seconds: [0.0; MAX_LINES],
            gains: [0.0; MAX_LINES],
            losses: [0.0; MAX_LINES],
            second_losses: [0.0; MAX_LINES],
            trips: [0.0; MAX_LINES],
        }
    }
}

impl Lengths {
    /// The values `t` (0..1) of the way from `from` to `to`.
    pub fn between(&mut self, from: &Self, to: &Self, t: f64) {
        use crate::lerp;
        lerp(&mut self.lines, &from.lines, &to.lines, t);
        lerp(&mut self.fractions, &from.fractions, &to.fractions, t);
        lerp(&mut self.allpasses, &from.allpasses, &to.allpasses, t);
        lerp(&mut self.seconds, &from.seconds, &to.seconds, t);
        lerp(&mut self.gains, &from.gains, &to.gains, t);
        lerp(&mut self.losses, &from.losses, &to.losses, t);
        lerp(&mut self.second_losses, &from.second_losses, &to.second_losses, t);
        lerp(&mut self.trips, &from.trips, &to.trips, t);
    }

    /// `design`'s lengths for size factor `room`, Scale `scale` (0..1) at `sample_rate`, under
    /// Smooth Fast (or Slow) when `fast`; `gain` gives a line's gain per trip from its length in
    /// seconds. Whole frames when `settled`, but for the A and B lines' fractions under Fast.
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
        for k in 0..design.lines {
            let family = design.families[k];
            let delay = design.delays.map_or(family, |d| d[k]);
            let (line, allpass, second) =
                length(design.lengths, delay, family, design.second, room, percent, settled, fast);
            let (line, allpass, second) = (line * frames, allpass * frames, second * frames);
            self.seconds[k] = if second > 0.0 { whole(second) } else { 0.0 };
            self.second_losses[k] = gain(second / sample_rate);
            if fast && family < 2 {
                // Read between frames, linearly, moving or not.
                self.lines[k] = line.floor().max(2.0);
                self.fractions[k] = line - self.lines[k];
            } else {
                self.lines[k] = whole(line);
                self.fractions[k] = if family < 2 { NONE_FRACTION } else { 0.0 };
            }
            self.allpasses[k] = whole(allpass);
            // The allpass loses its own time on each pass, so the echoes it holds back decay
            // as fast as the rest (as Live's tail does at every Diffusion).
            self.gains[k] = gain(line / sample_rate);
            self.losses[k] = gain(allpass / sample_rate);
            self.trips[k] = design.trip_scale * (line + allpass + second) / sample_rate / trip_seconds;
        }
    }

    /// The longest delay of `design`'s network (frames).
    pub fn longest(design: &Design, room: f64, sample_rate: f64) -> f64 {
        let line = |k: usize| {
            let family = design.families[k];
            let delay = design.delays.map_or(family, |d| d[k]);
            length(design.lengths, delay, family, design.second, room, 100.0, false, false).0
        };
        (0..design.lines).map(line).fold(0.0, f64::max) * sample_rate / RATE
    }
}

/// The network's lines and state.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Network {
    /// The lines, each with its allpass and damping; the design's first ones run.
    pub lines: [Line; MAX_LINES],
    /// The copies' low-pass, the reflections' own.
    copy_filters: [Biquad; MAX_LINES],
    /// The network running.
    design: &'static Design,
    /// Its mixing ([`loop_mix`]).
    mix: [[f64; MAX_LINES]; MAX_LINES],
    /// Cosine and sine of each line's Chorus phase.
    chorus_phases: [(f64, f64); MAX_LINES],
}

impl Default for Network {
    fn default() -> Self {
        Self {
            lines: Default::default(),
            copy_filters: Default::default(),
            design: &LOW,
            mix: loop_mix(&LOW),
            chorus_phases: chorus_phases(&LOW),
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
                second: Allpass::for_seconds(ALLPASS_SECONDS, sample_rate),
                damper: Damper::default(),
            }),
            ..Self::default()
        }
    }

    /// Runs `design`, silent from its start if it was not running.
    pub fn set_design(&mut self, design: &'static Design) {
        if !std::ptr::eq(self.design, design) {
            self.design = design;
            self.mix = loop_mix(design);
            self.chorus_phases = chorus_phases(design);
            self.clear();
        }
    }

    /// Silences every line.
    pub fn clear(&mut self) {
        for line in &mut self.lines {
            line.clear();
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

    /// One frame: `groups` is the input of each group of direct reflections and `copies` each
    /// family's filtered reflections, both already delayed by the entry; returns the tail of each
    /// side. `swing` gives each line's chorus swing (frames)
    /// from the cosine and sine of its phase and its length.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        groups: &[f64; MAX_GROUPS],
        copies: &[f64; FAMILY_COUNT],
        n: &Lengths,
        diffusion: f32,
        damping: &Damping,
        swing: impl Fn((f64, f64), f64) -> f64,
        frozen: bool,
    ) -> [f64; 2] {
        let d = self.design;
        let lines = d.lines;
        let weigh = |row: &[f64; MAX_GROUPS]| row.iter().zip(groups).map(|(w, g)| w * g).sum::<f64>();
        let families: [f64; MAX_LINES] = std::array::from_fn(|k| d.input_gains[k] * weigh(&d.inputs[k]));
        let mut heard = [0.0f64; MAX_LINES];
        for ((copy, filter), &family) in heard.iter_mut().zip(&mut self.copy_filters).zip(&d.families).take(lines) {
            *copy = filter.process(copies[family]);
        }

        let mut outs = [0.0f64; MAX_LINES];
        let mut tail = [0.0f64; 2];
        for (k, (line, out)) in self.lines.iter_mut().zip(&mut outs).enumerate().take(lines) {
            let length = n.lines[k] + swing(self.chorus_phases[k], n.lines[k]);
            let mut read = f64::from(line.delay.read_hermite(length));
            if n.fractions[k] != 0.0 {
                read += n.fractions[k] * (f64::from(line.delay.read_hermite(length + 1.0)) - read);
            }
            let mut y = line.diffuser.process_lossy(read as f32, n.allpasses[k], diffusion, n.losses[k] as f32);
            if n.seconds[k] > 0.0 {
                y = line.second.process_lossy(y, n.seconds[k], diffusion, n.second_losses[k] as f32);
            }
            *out = line.damper.damp(f64::from(y), damping) * n.gains[k];
            tail[d.side(k)] += *out;
        }
        // Into the mix: each line's output and its family's copy.
        let into: [f64; MAX_LINES] = std::array::from_fn(|j| outs[j] + heard[j]);
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

    /// Low's line `k`: its delay and allpass ([`length`]).
    fn low(k: usize, room: f64, percent: f64, settled: bool, fast: bool) -> (f64, f64) {
        let (line, allpass, _) = length(&LOW_LINES, k, k, false, room, percent, settled, fast);
        (line, allpass)
    }

    /// A Sparse, Mid or High line's lengths ([`length`]).
    fn shared(delay: usize, family: usize, second: bool, room: f64, percent: f64, fast: bool) -> (f64, f64, f64) {
        length(&SHARED_LINES, delay, family, second, room, percent, true, fast)
    }

    #[test]
    fn every_densitys_a_and_b_lines_are_its_families_a_and_b() {
        // They run a hundredth of a frame long under Smooth None; Sparse's C line has A's delay but
        // is whole (`reverb-network-density-size`).
        for (design, lines) in [(&LOW, &[0, 1][..]), (&SPARSE, &[1]), (&MID, &[0, 7]), (&HIGH, &[0, 8])] {
            let ab: Vec<usize> = (0..design.lines).filter(|&k| design.families[k] < 2).collect();
            assert_eq!(ab, lines, "{} lines", design.lines);
        }
    }

    #[test]
    fn the_entry_is_its_measured_length_at_every_shape() {
        // Live's entries (frames) at Shape 0, 0.25, 0.5, 0.75 and 1, Smooth None then Fast.
        for (fast, frames) in
            [(false, [-1.0, 391.0, 775.0, 1159.0, 1543.0]), (true, [-1.0, 384.0, 770.0, 1156.0, 1542.0])]
        {
            for (shape, want) in [0.0, 0.25, 0.5, 0.75, 1.0].into_iter().zip(frames) {
                assert_eq!(entry(shape, true, fast), want, "Shape {shape}, fast {fast}");
            }
        }
    }

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
    fn the_loop_keeps_its_energy_with_each_line_at_its_level() {
        for design in [&SPARSE, &LOW, &MID, &HIGH] {
            let (mix, levels, n) = (loop_mix(design), &design.levels, design.lines);
            // Taken at their levels, the lines mix orthogonally.
            let heard = |k: usize, i: usize| mix[k][i] * levels[i] / levels[k];
            for i in 0..n {
                for j in 0..n {
                    let dot: f64 = (0..n).map(|k| heard(k, i) * heard(k, j)).sum();
                    let want = if i == j { 1.0 } else { 0.0 };
                    assert!((dot - want).abs() < 1e-12, "{n} lines, columns {i} and {j}: {dot}");
                }
            }
            // Within 1 % of each other, about one.
            let mean = (levels[..n].iter().map(|l| l.ln()).sum::<f64>() / n as f64).exp();
            assert!((mean - 1.0).abs() < 1e-4, "{n} lines: {mean}");
            assert!(levels[..n].iter().all(|l| (l - 1.0).abs() < 0.01), "{n} lines: {levels:?}");
        }
        assert_eq!(loop_mix(&LOW), orthonormal(&LOW));
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
        for (k, lengths) in measured.into_iter().enumerate() {
            for (percent, want) in [5.0, 20.0, 40.0, 60.0, 80.0, 100.0].into_iter().zip(lengths) {
                let (line, allpass, second) = shared(SPARSE_DELAYS[k], SPARSE.families[k], false, 1.0, percent, false);
                assert_eq!(line + allpass + second, want, "line {k} at Scale {percent} %");
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
        for (k, lengths) in measured.into_iter().enumerate() {
            let family = HIGH.families[k];
            for (percent, want) in [5.0, 20.0, 40.0, 60.0, 80.0, 100.0].into_iter().zip(lengths) {
                let (line, first, second) = shared(family, family, true, 1.0, percent, false);
                assert_eq!(line + first + second, want, "line {k} at Scale {percent} %");
            }
        }
        // Mid's are High's but the second and the last.
        let mid: Vec<_> = HIGH.families[..HIGH.lines].iter().filter(|f| ![8, 7].contains(*f)).copied().collect();
        assert_eq!(mid, MID.families[..MID.lines]);
    }

    #[test]
    fn d_and_the_2073_familys_allpasses_split_as_measured_at_size_10_54() {
        // Each allpass (frames) where Mid's and High's tails match Live's at Diffusion 30 and
        // 60 % (`reverb-network-density-split`). With the allpasses split as first read, D's
        // second came a block short at Scale 40 %, and at 100 % D's first and the 2073 family's
        // came a block long and the 2073 family's second a block short; at Diffusion 0.1 % only
        // their sums show, which hold at every Size and Scale read.
        let room = (10.54f64 / 100.0).cbrt();
        for (family, percent, want) in
            [(3, 40.0, (265.0, 529.0)), (3, 100.0, (657.0, 1305.0)), (7, 100.0, (289.0, 1025.0))]
        {
            let (_, first, second) = shared(family, family, true, room, percent, false);
            assert_eq!((first, second), want, "family {family} at Scale {percent} %");
        }
    }

    #[test]
    fn every_densitys_lines_are_their_measured_lengths_at_the_ends_of_the_size_range() {
        // Live's first passes less the entry (frames), Scale 40 %, at Size 0.222 and 500, with
        // Smooth None and Fast (`reverb-network-density-size`); under Fast the A and B lines' are
        // between frames.
        let small = (0.222f64 / 100.0).cbrt();
        let large = (500.0f64 / 100.0).cbrt();
        let high: [[f64; 4]; 10] = [
            [675.0, 8699.0, 664.92, 8693.68],
            [683.0, 8747.0, 668.0, 8742.0],
            [723.0, 9355.0, 714.0, 9343.0],
            [939.0, 12187.0, 930.0, 12178.0],
            [1291.0, 16891.0, 1289.0, 16883.0],
            [747.0, 9595.0, 733.0, 9588.0],
            [771.0, 9987.0, 763.0, 9981.0],
            [979.0, 12651.0, 965.0, 12639.0],
            [1811.0, 23571.0, 1799.16, 23559.6],
            [2219.0, 29035.0, 2215.0, 29020.0],
        ];
        // Sparse's last line at Size 0.222 under Fast overlaps others and is not read.
        let sparse: [[f64; 4]; 4] = [
            [482.0, 6226.0, 475.0, 6220.0],
            [514.0, 6666.0, 509.98, 6661.68],
            [626.0, 8082.0, 617.0, 8077.0],
            [1306.0, 16970.0, f64::NAN, 16960.0],
        ];
        let check = |name: &str, k: usize, delay: usize, family: usize, second: bool, want: [f64; 4]| {
            for ((room, fast), want) in
                [(small, false), (large, false), (small, true), (large, true)].into_iter().zip(want)
            {
                if want.is_nan() {
                    continue;
                }
                let (line, first, second) = shared(delay, family, second, room, 40.0, fast);
                let got = line + first + second;
                assert!((got - want).abs() < 0.06, "{name} line {k}, room {room}, fast {fast}: {got} against {want}");
            }
        };
        for (k, want) in high.into_iter().enumerate() {
            check("High", k, HIGH.families[k], HIGH.families[k], true, want);
        }
        for (k, want) in sparse.into_iter().enumerate() {
            check("Sparse", k, SPARSE_DELAYS[k], SPARSE.families[k], false, want);
        }
    }

    #[test]
    fn each_line_of_mid_and_high_has_its_own_family() {
        // As at Low, each family's copy joins one line, and none shares its offset.
        for design in [&MID, &HIGH] {
            let mut peaks: Vec<_> =
                design.families[..design.lines].iter().map(|&k| peak(k, 1.0, true, false) as i64).collect();
            peaks.sort_unstable();
            peaks.dedup();
            assert_eq!(peaks.len(), design.lines);
        }
        assert_eq!(peak(MID.families[2], 1.0, true, false), 961.0);
        assert_eq!(peak(HIGH.families[9], 1.0, true, false), 2073.0);
    }

    #[test]
    fn every_direct_reflection_of_mid_and_high_has_a_group() {
        for (taps, design) in [(&crate::TAPS_MID[..], &MID), (&crate::TAPS_HIGH[..], &HIGH)] {
            assert_eq!(taps.len(), DIRECT_TAPS);
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
                let (line, allpass) = low(k, room, percent, true, false);
                assert_eq!(line + allpass, want, "line {k} at Size {size}, Scale {percent} %");
            }
        }
    }

    #[test]
    fn the_smallest_rooms_c_and_r_are_their_measured_lengths() {
        // Read from Live's tail at Size 0.222 (`reverb-network-small`): C's family offset and
        // delay, and R's A delay and G, are each one rounded length, not two.
        let room = (0.222 / 100.0f64).cbrt();
        assert_eq!(low(2, room, 40.0, true, false).0, 8.0 * 82.0 + 1.0);
        assert_eq!(low(5, room, 40.0, true, false).0, 8.0 * 160.0 + 1.0);
    }

    #[test]
    fn the_largest_rooms_r_is_its_measured_length() {
        // Live's tail at Size 500 (`reverb-size decay-3.46s-size-500`) has R a block shorter than
        // the reads at Size 0.222 to 100 put it: 2087 blocks and a frame.
        let room = 5.0f64.cbrt();
        assert_eq!(low(5, room, 40.0, true, false).0, 8.0 * 2087.0 + 1.0);
    }

    #[test]
    fn r_at_size_350_is_its_measured_length() {
        // Low's tail at Size 350 (`reverb-network-size-level`) has R 1854 blocks and a frame, a
        // block longer than the reads at Size 0.222 to 100 and 500 alone put it.
        let room = 3.5f64.cbrt();
        assert_eq!(low(5, room, 40.0, true, false).0, 8.0 * 1854.0 + 1.0);
    }

    #[test]
    fn mids_and_highs_lines_are_their_measured_lengths_at_size_400() {
        // Live's first passes less the entry (frames) at Size 400, Scale 40 %, Smooth None
        // (`reverb-network-size-level`), where the 3193, 2337, 961 and 2497 families' lines are a
        // block shorter than the reads at the other Sizes alone put them.
        let room = 4.0f64.cbrt();
        for (family, want) in [(1, 8075.0), (5, 11739.0), (6, 11315.0), (9, 8907.0)] {
            let (line, first, second) = shared(family, family, true, room, 40.0, false);
            assert_eq!(line + first + second, want, "family {family}");
        }
    }

    #[test]
    fn mids_and_highs_lines_are_their_measured_lengths_at_size_300_and_400() {
        // Every family's delay and allpasses (frames) at Scale 40 %, Smooth None, where Mid's and
        // High's tails match Live's (`reverb-network-size-level`, −37 to −40 dB to 3 s).
        type Line = (f64, f64, f64);
        let measured: [(f64, [Line; FAMILY_COUNT]); 2] = [
            (
                300.0,
                [
                    (13673.0, 321.0, 5889.0),
                    (5113.0, 513.0, 1721.0),
                    (5033.0, 641.0, 2753.0),
                    (11849.0, 809.0, 1593.0),
                    (4113.0, 1137.0, 2641.0),
                    (6473.0, 345.0, 3849.0),
                    (7177.0, 1369.0, 1737.0),
                    (22889.0, 353.0, 1249.0),
                    (4385.0, 377.0, 2625.0),
                    (5785.0, 553.0, 1761.0),
                ],
            ),
            (
                400.0,
                [
                    (15049.0, 353.0, 6481.0),
                    (5625.0, 561.0, 1889.0),
                    (5545.0, 705.0, 3025.0),
                    (13041.0, 889.0, 1753.0),
                    (4529.0, 1257.0, 2905.0),
                    (7121.0, 377.0, 4241.0),
                    (7897.0, 1505.0, 1913.0),
                    (25193.0, 393.0, 1369.0),
                    (4825.0, 409.0, 2889.0),
                    (6369.0, 601.0, 1937.0),
                ],
            ),
        ];
        for (size, lines) in measured {
            let room = (size / 100.0f64).cbrt();
            for (family, want) in lines.into_iter().enumerate() {
                assert_eq!(shared(family, family, true, room, 40.0, false), want, "family {family} at Size {size}");
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
        // C's, D's, F's and R's at the ends of the range (`reverb-network-density-size`).
        for (size, loops) in [(0.222, [694.0, 647.0, 976.0, 1308.0]), (500.0, [9078.0, 8470.0, 12778.0, 17138.0])] {
            let room = (size / 100.0f64).cbrt();
            for (k, want) in (2..LOOP).zip(loops) {
                let (line, allpass) = low(k, room, 40.0, true, true);
                assert_eq!(line + allpass, want, "line {k} at Size {size}");
            }
        }
        for (size, percent, loops) in measured {
            let room = (size / 100.0f64).cbrt();
            for (k, want) in loops.into_iter().enumerate() {
                let (line, allpass) = low(k, room, percent, true, true);
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
    fn under_smooth_fast_an_allpass_is_its_measured_length() {
        // Read at Diffusion 48 and 60 %, where a frame between a line's delay and its allpass
        // shows (`reverb-impulse` reverb-decay, reverb-decay-none, reverb-texture): A to R.
        let measured = [
            (100.0, 40.0, [204.0, 392.0, 279.0, 1274.0, 700.0, 258.0]),
            (10.54, 40.0, [96.0, 186.0, 132.0, 602.0, 331.0, 122.0]),
            (1.53, 5.0, [7.0, 13.0, 9.0, 40.0, 22.0, 8.0]),
            (1.53, 40.0, [51.0, 98.0, 70.0, 317.0, 174.0, 64.0]),
            (1.53, 100.0, [127.0, 244.0, 173.0, 791.0, 434.0, 160.0]),
            (500.0, 40.0, [348.0, 670.0, 477.0, 2178.0, 1196.0, 441.0]),
        ];
        for (size, percent, allpasses) in measured {
            let room = (size / 100.0f64).cbrt();
            for (k, want) in allpasses.into_iter().enumerate() {
                assert_eq!(low(k, room, percent, true, true).1, want, "line {k} at Size {size}, Scale {percent} %");
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
        // A's and D's filtered reflections at Size 500 (`reverb-structure size-500-er`).
        let room = 5.0f64.cbrt();
        assert_eq!(peak(0, room, true, true), 676.0);
        assert_eq!(peak(3, room, true, true), 2285.0);
        for (size, peaks) in measured {
            let room = (size / 100.0f64).cbrt();
            for (k, want) in peaks.into_iter().enumerate() {
                assert_eq!(peak(k, room, true, true), want, "family {k} at Size {size}");
            }
        }
    }

    #[test]
    fn each_copy_peaks_where_measured() {
        let measured = [
            (100.0, [401.0, 3193.0, 1641.0, 1337.0, 2985.0, 2337.0]),
            (10.54, [193.0, 1513.0, 777.0, 633.0, 1409.0, 1105.0]),
            (0.222, [57.0, 417.0, 217.0, 177.0, 393.0, 305.0]),
            // `reverb-size decay-3.46s-size-500`: A and B read, the rest where a block either way
            // fits Live's tail worse.
            (500.0, [681.0, 5457.0, 2809.0, 2289.0, 5105.0, 4001.0]),
            // Low's reflections alone at Size 250 to 400 (`reverb-network-size-level`), where the
            // 401 family is a block shorter at 250 and 350 and the 3193 family a block longer at
            // 250 and 300 than the reads up to 100 and at 500 alone put them.
            (250.0, [537.0, 4337.0, 2233.0, 1817.0, 4049.0, 3177.0]),
            (300.0, [577.0, 4609.0, 2369.0, 1929.0, 4305.0, 3377.0]),
            (350.0, [601.0, 4849.0, 2497.0, 2033.0, 4529.0, 3553.0]),
            (400.0, [633.0, 5065.0, 2609.0, 2121.0, 4737.0, 3713.0]),
        ];
        for (size, peaks) in measured {
            let room = (size / 100.0f64).cbrt();
            for (k, want) in peaks.into_iter().enumerate() {
                assert_eq!(peak(k, room, true, false), want, "family {k} at Size {size}");
            }
        }
    }

    #[test]
    fn mids_and_highs_own_families_peak_where_measured() {
        // Mid's and High's reflections alone at Size 0.222 and 500 (`reverb-network-density-size`):
        // the 961, 2073, 2393 and 2497 families, None then Fast.
        let measured = [
            (0.222, [129.0, 273.0, 313.0, 329.0], [126.0, 271.0, 313.0, 326.0]),
            (500.0, [1649.0, 3545.0, 4097.0, 4265.0], [1643.0, 3543.0, 4091.0, 4265.0]),
        ];
        // At Size 300 and 400 under None, where Mid's and High's tails match Live's
        // (`reverb-network-size-level`).
        for (size, none) in [(300.0, [1393.0, 2993.0, 3457.0, 3601.0]), (400.0, [1529.0, 3297.0, 3801.0, 3961.0])] {
            let room = (size / 100.0f64).cbrt();
            for (i, want) in none.into_iter().enumerate() {
                assert_eq!(peak(6 + i, room, true, false), want, "family {} at Size {size}", 6 + i);
            }
        }
        for (size, none, fast) in measured {
            let room = (size / 100.0f64).cbrt();
            for (i, (none, fast)) in none.into_iter().zip(fast).enumerate() {
                assert_eq!(peak(6 + i, room, true, false), none, "family {} at Size {size}", 6 + i);
                assert_eq!(peak(6 + i, room, true, true), fast, "family {} at Size {size}, Fast", 6 + i);
            }
        }
    }

    #[test]
    fn a_glide_runs_through_the_middle_of_the_steps() {
        // Moving, a delay or an allpass is within half a block of where it settles.
        for size in [0.5, 3.0, 27.0, 100.0, 333.0] {
            let room = (size / 100.0f64).cbrt();
            for k in 0..LOOP {
                let (a, b) = (low(k, room, 40.0, true, false), low(k, room, 40.0, false, false));
                let most = if k == 2 || k == 5 { BLOCK } else { BLOCK / 2.0 };
                assert!((a.0 - b.0).abs() <= most, "line {k} at Size {size}");
                assert!((a.1 - b.1).abs() <= BLOCK / 2.0, "allpass {k} at Size {size}");
            }
        }
    }

    #[test]
    fn every_direct_reflection_of_low_has_a_group() {
        let direct = crate::TAPS_LOW.len();
        assert_eq!(direct, TAP_GROUPS.len());
    }
}
