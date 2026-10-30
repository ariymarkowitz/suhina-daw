// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reverb: an algorithmic room reverb in the stages a designer thinks in. The design is in
//! `docs/spec/reverb.md`.
//!
//! ```text
//! in -> mono sum -> roll-off, Low Cut, High Cut -> predelay line -+-> reflection taps (Spin) -> Reflect -+
//!                                                                  |                                    +-> Stereo -> Dry/Wet
//!                                                                  +-> damping -> Shape delay ->        |
//!                                                                      direct taps -> network -> Diffuse
//! ```
//!
//! The early reflections are the measured tap pattern (docs/spec/reverb.md). At Density Low the
//! late reverb is Live's own network, as measured (`measured.rs`). At the other Densities the
//! network takes the direct taps again after the Shape delay, each into a line of its own: a
//! feedback delay network of 8, 12 or 16 lines (Density) mixed by a Householder reflection, the
//! textbook design (Jot and Chaigne, 1991), with a Schroeder allpass (Diffusion) in every line;
//! each line's gain sets the same decay time whatever its length, and shelves in the loop take
//! away more of the highs or lows per second. Size scales every delay with the cube root of
//! the room's volume, except that the Householder network's lines shrink less in small rooms. That
//! network's delay lengths and constants are this device's own.

use suhina_device_api::{
    DEFAULT_SAMPLE_RATE, Device, DeviceError, DeviceInfo, ParamBank, ParamInfo, ProcessContext, Role, Scale, Unit,
    check_sample_rate,
};
use suhina_dsp::biquad::BUTTERWORTH_Q;
use suhina_dsp::{Allpass, Biquad, Coefficients, DelayLine, FirstOrder, FirstOrderCoefficients, Smoother};

mod measured;

/// Ramp for levels, mix, stereo and most other changes.
pub const RAMP_SECONDS: f64 = 0.02;
/// Ramp for predelay changes: slower, so a long jump bends the pitch less on the way.
pub const PREDELAY_RAMP_SECONDS: f64 = 0.1;
/// Size glides with this time constant when Smooth is Slow.
pub const SMOOTH_SLOW_SECONDS: f64 = 0.045;
/// Size glides with this time constant when Smooth is Fast.
pub const SMOOTH_FAST_SECONDS: f64 = 0.015;
/// Frames between updates of delay lengths, tap levels and filters, counted from the last reset.
const CONTROL_FRAMES: u64 = 32;
/// However fast Smooth glides Size, no line or reflection lengthens or shortens by more than this
/// many frames a frame: faster, a line is read backwards and a jump across the range buzzes.
const GLIDE_SLEW: f64 = 0.5;

/// The reference room: delays below are for Size 100, and scale with `(size / 100)^(1/3)`.
const REFERENCE_SIZE: f64 = 100.0;
/// Which side an early reflection is heard on.
const LEFT: usize = 0;
/// See [`LEFT`].
const RIGHT: usize = 1;
/// A reflection heard as it is read.
const DIRECT: bool = false;
/// A reflection heard through the reflections' low-pass ([`Reverb::reflection_filters`]).
const FILTERED: bool = true;

/// One early reflection: its time after the predelay in the reference room (ms), its side, its
/// level against the first (before the fade) and whether it passes the reflections' low-pass.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Tap {
    ms: f64,
    side: usize,
    gain: f64,
    filtered: bool,
}

/// A [`Tap`].
const fn tap(ms: f64, side: usize, gain: f64, filtered: bool) -> Tap {
    Tap { ms, side, gain, filtered }
}

// The early reflections of each Density, as measured at 44.1 kHz (docs/spec/reverb.md
// › Decisions): times in the reference room, each a whole number of frames at 44.1 kHz plus the
// fraction by which Live rounds it to a frame (read from where it steps as the predelay moves by
// tenths of a frame). A filtered reflection keeps the fraction of the direct one it follows.
const TAPS_SPARSE: [Tap; 32] = [
    tap(0.0011, RIGHT, 1.5008, DIRECT),
    tap(26.5367, LEFT, -1.4980, DIRECT),
    tap(37.1666, RIGHT, 0.6930, FILTERED),
    tap(37.2120, RIGHT, 0.1117, FILTERED),
    tap(41.1259, LEFT, 1.4991, DIRECT),
    tap(52.9716, RIGHT, 1.2012, FILTERED),
    tap(53.0170, RIGHT, 0.1981, FILTERED),
    tap(55.6451, RIGHT, 1.4936, DIRECT),
    tap(63.7022, RIGHT, 0.0812, FILTERED),
    tap(67.6201, LEFT, -1.0370, FILTERED),
    tap(67.6655, LEFT, -0.1962, FILTERED),
    tap(72.3367, LEFT, 0.4495, FILTERED),
    tap(72.3821, LEFT, 0.0863, FILTERED),
    tap(78.2914, RIGHT, 1.5633, FILTERED),
    tap(78.3368, RIGHT, 0.2956, FILTERED),
    tap(79.5072, RIGHT, -0.7225, FILTERED),
    tap(79.5526, RIGHT, -0.1358, FILTERED),
    tap(92.8106, RIGHT, 0.5169, FILTERED),
    tap(92.8560, RIGHT, 0.0977, FILTERED),
    tap(94.0964, RIGHT, -0.1441, FILTERED),
    tap(94.1557, LEFT, -0.0974, FILTERED),
    tap(98.8723, LEFT, 1.6314, FILTERED),
    tap(98.9177, LEFT, 0.3114, FILTERED),
    tap(108.6156, RIGHT, -1.0976, FILTERED),
    tap(108.6610, RIGHT, -0.2090, FILTERED),
    tap(108.7449, LEFT, 0.8568, FILTERED),
    tap(108.7903, LEFT, 0.1599, FILTERED),
    tap(113.4615, LEFT, -0.0943, FILTERED),
    tap(123.2641, LEFT, -1.1785, FILTERED),
    tap(123.3095, LEFT, -0.2247, FILTERED),
    tap(127.9807, LEFT, -0.5721, FILTERED),
    tap(128.0261, LEFT, -0.1100, FILTERED),
];
const TAPS_LOW: [Tap; 71] = [
    tap(0.0011, RIGHT, 1.0006, DIRECT),
    tap(7.3707, RIGHT, -1.0015, DIRECT),
    tap(8.9580, RIGHT, 1.0133, FILTERED),
    tap(11.4138, LEFT, 0.9997, DIRECT),
    tap(16.3276, RIGHT, -1.0113, FILTERED),
    tap(17.9059, LEFT, -1.0005, DIRECT),
    tap(20.3707, RIGHT, 0.6052, FILTERED),
    tap(26.5367, LEFT, -0.9953, DIRECT),
    tap(26.8628, RIGHT, 0.0851, FILTERED),
    tap(28.4206, LEFT, 0.9985, DIRECT),
    tap(30.2732, LEFT, -0.6020, FILTERED),
    tap(34.1757, RIGHT, -0.9989, DIRECT),
    tap(35.4936, RIGHT, 0.0836, FILTERED),
    tap(37.1666, RIGHT, 0.1205, FILTERED),
    tap(37.3775, RIGHT, -0.3660, FILTERED),
    tap(37.6428, LEFT, 0.6036, FILTERED),
    tap(41.1259, LEFT, 0.9976, DIRECT),
    tap(41.6859, LEFT, 0.7245, FILTERED),
    tap(43.1326, RIGHT, -0.2250, FILTERED),
    tap(44.5362, RIGHT, -0.1168, FILTERED),
    tap(48.1780, LEFT, -0.3636, FILTERED),
    tap(48.2846, RIGHT, -0.9966, DIRECT),
    tap(48.5793, RIGHT, 0.5363, FILTERED),
    tap(50.0828, RIGHT, 0.6059, FILTERED),
    tap(52.9716, RIGHT, 0.2329, FILTERED),
    tap(55.0714, RIGHT, 0.8618, FILTERED),
    tap(55.6451, RIGHT, 0.9951, DIRECT),
    tap(56.8088, LEFT, -0.3581, FILTERED),
    tap(57.2415, RIGHT, 0.5709, FILTERED),
    tap(58.6927, LEFT, -0.1423, FILTERED),
    tap(60.3412, RIGHT, -0.2308, FILTERED),
    tap(63.7022, RIGHT, 0.8558, FILTERED),
    tap(64.3843, RIGHT, 0.4778, FILTERED),
    tap(64.4478, LEFT, -0.9218, FILTERED),
    tap(64.6020, RIGHT, -0.5960, FILTERED),
    tap(65.5861, RIGHT, 0.8453, FILTERED),
    tap(67.6201, LEFT, -0.6661, FILTERED),
    tap(70.8764, RIGHT, -0.1846, FILTERED),
    tap(71.3412, RIGHT, -0.0805, FILTERED),
    tap(71.3980, LEFT, 0.7380, FILTERED),
    tap(72.3367, LEFT, -0.1142, FILTERED),
    tap(74.9897, LEFT, 0.6729, FILTERED),
    tap(78.2914, RIGHT, 0.5238, FILTERED),
    tap(78.5567, LEFT, -0.1164, FILTERED),
    tap(79.0328, LEFT, 0.4080, FILTERED),
    tap(79.5072, RIGHT, -0.1849, FILTERED),
    tap(79.7063, LEFT, 0.1119, FILTERED),
    tap(81.3911, RIGHT, -0.5148, FILTERED),
    tap(81.4365, RIGHT, -0.0453, FILTERED),
    tap(83.7494, LEFT, 0.5699, FILTERED),
    tap(85.4501, RIGHT, -0.3523, FILTERED),
    tap(85.5249, LEFT, 0.6592, FILTERED),
    tap(85.9172, LEFT, 0.1134, FILTERED),
    tap(87.1462, RIGHT, 0.5185, FILTERED),
    tap(90.2415, LEFT, -0.7494, FILTERED),
    tap(92.8106, RIGHT, 0.3672, FILTERED),
    tap(94.0964, RIGHT, 0.4855, FILTERED),
    tap(94.1557, LEFT, 0.6641, FILTERED),
    tap(96.0396, LEFT, -0.5707, FILTERED),
    tap(98.8723, LEFT, -0.7425, FILTERED),
    tap(100.7562, LEFT, 0.6469, FILTERED),
    tap(101.2551, RIGHT, -1.0155, FILTERED),
    tap(101.7947, LEFT, 0.5313, FILTERED),
    tap(106.5113, LEFT, 0.6654, FILTERED),
    tap(108.6156, RIGHT, 1.0143, FILTERED),
    tap(108.7449, LEFT, 0.4033, FILTERED),
    tap(113.4615, LEFT, 0.5671, FILTERED),
    tap(115.9036, LEFT, 0.4828, FILTERED),
    tap(120.6202, LEFT, 0.3813, FILTERED),
    tap(123.2641, LEFT, -0.4962, FILTERED),
    tap(127.9807, LEFT, -0.3889, FILTERED),
];
const TAPS_MID: [Tap; 141] = [
    tap(0.0011, RIGHT, 0.9999, DIRECT),
    tap(7.3707, RIGHT, 0.9984, DIRECT),
    tap(8.9580, RIGHT, -0.3406, FILTERED),
    tap(9.0034, RIGHT, -0.0977, FILTERED),
    tap(11.4138, LEFT, 0.9975, DIRECT),
    tap(16.3276, RIGHT, -0.1072, FILTERED),
    tap(17.9059, RIGHT, -1.0004, DIRECT),
    tap(20.3707, RIGHT, -0.3169, FILTERED),
    tap(20.4161, RIGHT, -0.0825, FILTERED),
    tap(21.7698, LEFT, 0.2524, FILTERED),
    tap(21.8152, LEFT, 0.0602, FILTERED),
    tap(26.5367, LEFT, -0.9997, DIRECT),
    tap(26.8628, RIGHT, 0.3325, FILTERED),
    tap(26.9082, RIGHT, 0.0901, FILTERED),
    tap(28.4206, LEFT, 0.9947, DIRECT),
    tap(29.1394, LEFT, -0.5521, FILTERED),
    tap(29.1848, LEFT, -0.1745, FILTERED),
    tap(30.2732, LEFT, 0.5743, FILTERED),
    tap(30.3186, LEFT, 0.1573, FILTERED),
    tap(34.1757, RIGHT, -0.9990, DIRECT),
    tap(35.4936, RIGHT, 0.7019, FILTERED),
    tap(35.5390, RIGHT, 0.2115, FILTERED),
    tap(37.1666, RIGHT, 0.5107, FILTERED),
    tap(37.2120, RIGHT, 0.1449, FILTERED),
    tap(37.3775, RIGHT, -0.2382, FILTERED),
    tap(37.4229, RIGHT, -0.0697, FILTERED),
    tap(37.6428, LEFT, 0.3908, FILTERED),
    tap(37.6882, LEFT, 0.1097, FILTERED),
    tap(39.6746, LEFT, -0.2365, FILTERED),
    tap(39.7200, LEFT, -0.0703, FILTERED),
    tap(41.1259, LEFT, 0.9948, DIRECT),
    tap(41.6859, LEFT, -0.3987, FILTERED),
    tap(41.7313, LEFT, -0.1298, FILTERED),
    tap(43.1326, RIGHT, 0.1066, FILTERED),
    tap(44.5362, RIGHT, -0.0631, FILTERED),
    tap(48.1780, LEFT, -0.5616, FILTERED),
    tap(48.2234, LEFT, -0.1676, FILTERED),
    tap(48.2846, RIGHT, 0.9974, DIRECT),
    tap(48.3054, LEFT, 0.0661, FILTERED),
    tap(48.5793, RIGHT, 0.2133, FILTERED),
    tap(50.0828, RIGHT, 0.3925, FILTERED),
    tap(50.1282, RIGHT, 0.0989, FILTERED),
    tap(50.1893, LEFT, 0.7381, FILTERED),
    tap(50.2347, LEFT, 0.2099, FILTERED),
    tap(52.9716, RIGHT, -0.4531, FILTERED),
    tap(53.0170, RIGHT, -0.1261, FILTERED),
    tap(55.0714, RIGHT, -0.5074, FILTERED),
    tap(55.1168, RIGHT, -0.1552, FILTERED),
    tap(55.6451, RIGHT, 0.9977, DIRECT),
    tap(55.9444, LEFT, 0.5482, FILTERED),
    tap(55.9898, LEFT, 0.1524, FILTERED),
    tap(56.5317, RIGHT, 0.1187, FILTERED),
    tap(56.8088, LEFT, -0.3520, FILTERED),
    tap(56.8542, LEFT, -0.1065, FILTERED),
    tap(57.2415, RIGHT, 0.4944, FILTERED),
    tap(57.2869, RIGHT, 0.1250, FILTERED),
    tap(58.6927, LEFT, -0.2352, FILTERED),
    tap(58.7381, LEFT, -0.0722, FILTERED),
    tap(60.3412, RIGHT, -0.1112, FILTERED),
    tap(62.8946, LEFT, 0.3962, FILTERED),
    tap(62.9400, LEFT, 0.0981, FILTERED),
    tap(63.7022, RIGHT, 0.2058, FILTERED),
    tap(63.9013, RIGHT, -0.3854, FILTERED),
    tap(63.9467, RIGHT, -0.1070, FILTERED),
    tap(64.3843, RIGHT, -0.5616, FILTERED),
    tap(64.4297, RIGHT, -0.1646, FILTERED),
    tap(64.4478, LEFT, -0.3883, FILTERED),
    tap(64.4932, LEFT, -0.1149, FILTERED),
    tap(67.6201, LEFT, -0.0821, FILTERED),
    tap(67.9444, RIGHT, 0.3581, FILTERED),
    tap(67.9898, RIGHT, 0.1244, FILTERED),
    tap(70.8764, RIGHT, 0.4534, FILTERED),
    tap(70.9218, RIGHT, 0.1548, FILTERED),
    tap(71.3412, RIGHT, 0.0604, FILTERED),
    tap(71.3980, LEFT, 0.3937, FILTERED),
    tap(71.4434, LEFT, 0.1351, FILTERED),
    tap(72.3367, LEFT, -0.4124, FILTERED),
    tap(72.3821, LEFT, -0.1416, FILTERED),
    tap(74.4365, RIGHT, -0.1176, FILTERED),
    tap(74.9897, LEFT, 0.7106, FILTERED),
    tap(75.0351, LEFT, 0.2435, FILTERED),
    tap(77.4138, LEFT, -0.3345, FILTERED),
    tap(77.4592, LEFT, -0.1127, FILTERED),
    tap(78.2914, RIGHT, 0.3558, FILTERED),
    tap(78.3368, RIGHT, 0.1226, FILTERED),
    tap(78.5567, LEFT, 0.2807, FILTERED),
    tap(78.6021, LEFT, 0.0965, FILTERED),
    tap(79.0328, LEFT, 0.2079, FILTERED),
    tap(79.0782, LEFT, 0.0718, FILTERED),
    tap(79.5072, RIGHT, -0.3492, FILTERED),
    tap(79.5526, RIGHT, -0.1195, FILTERED),
    tap(79.7063, LEFT, 0.1164, FILTERED),
    tap(83.0673, RIGHT, 0.0799, FILTERED),
    tap(83.7494, LEFT, 0.5850, FILTERED),
    tap(83.7948, LEFT, 0.2020, FILTERED),
    tap(84.9512, RIGHT, -0.6379, FILTERED),
    tap(84.9966, RIGHT, -0.2192, FILTERED),
    tap(85.4501, RIGHT, -0.0907, FILTERED),
    tap(85.5249, LEFT, 0.0779, FILTERED),
    tap(85.9172, LEFT, -0.2990, FILTERED),
    tap(85.9626, LEFT, -0.1029, FILTERED),
    tap(87.1462, RIGHT, 0.1114, FILTERED),
    tap(90.2415, LEFT, 0.4140, FILTERED),
    tap(90.2869, LEFT, 0.1431, FILTERED),
    tap(90.7063, RIGHT, 0.3803, FILTERED),
    tap(90.7517, RIGHT, 0.1313, FILTERED),
    tap(92.8106, RIGHT, 0.8147, FILTERED),
    tap(92.8560, RIGHT, 0.2809, FILTERED),
    tap(94.0964, RIGHT, 0.3944, FILTERED),
    tap(94.1418, RIGHT, 0.1399, FILTERED),
    tap(94.1557, LEFT, 0.3849, FILTERED),
    tap(94.2011, LEFT, 0.1295, FILTERED),
    tap(96.0396, LEFT, 0.2794, FILTERED),
    tap(96.0850, LEFT, 0.0963, FILTERED),
    tap(97.6565, RIGHT, 0.2957, FILTERED),
    tap(97.7019, RIGHT, 0.0995, FILTERED),
    tap(98.8723, LEFT, -0.4452, FILTERED),
    tap(98.9177, LEFT, -0.1534, FILTERED),
    tap(100.7562, LEFT, 0.0486, FILTERED),
    tap(101.2551, RIGHT, -0.4889, FILTERED),
    tap(101.3005, RIGHT, -0.1700, FILTERED),
    tap(101.7947, LEFT, -0.7145, FILTERED),
    tap(101.8401, LEFT, -0.2464, FILTERED),
    tap(104.8152, RIGHT, -0.4333, FILTERED),
    tap(104.8606, RIGHT, -0.1486, FILTERED),
    tap(106.5113, LEFT, -0.1223, FILTERED),
    tap(108.6156, RIGHT, 0.2520, FILTERED),
    tap(108.6610, RIGHT, 0.0862, FILTERED),
    tap(108.7449, LEFT, 0.2963, FILTERED),
    tap(108.7903, LEFT, 0.1133, FILTERED),
    tap(112.1757, RIGHT, -0.3706, FILTERED),
    tap(112.2211, RIGHT, -0.1291, FILTERED),
    tap(113.4615, LEFT, 0.4535, FILTERED),
    tap(113.5069, LEFT, 0.1567, FILTERED),
    tap(115.9036, LEFT, -0.4425, FILTERED),
    tap(115.9490, LEFT, -0.1507, FILTERED),
    tap(120.6202, LEFT, 0.3906, FILTERED),
    tap(120.6656, LEFT, 0.1344, FILTERED),
    tap(123.2641, LEFT, -0.2013, FILTERED),
    tap(123.3095, LEFT, -0.0695, FILTERED),
    tap(127.9807, LEFT, 0.0725, FILTERED),
];
const TAPS_HIGH: [Tap; 178] = [
    tap(0.0011, RIGHT, 1.0004, DIRECT),
    tap(7.3707, RIGHT, 0.9988, DIRECT),
    tap(8.9580, RIGHT, -0.5414, FILTERED),
    tap(9.0034, RIGHT, -0.1130, FILTERED),
    tap(11.4138, LEFT, 1.0015, DIRECT),
    tap(16.3276, RIGHT, -0.1320, FILTERED),
    tap(17.9059, LEFT, -0.9986, DIRECT),
    tap(20.3707, RIGHT, -0.0587, FILTERED),
    tap(21.7698, LEFT, 0.5502, FILTERED),
    tap(21.8152, LEFT, 0.1266, FILTERED),
    tap(26.5367, LEFT, -0.9944, DIRECT),
    tap(26.8628, RIGHT, 0.6640, FILTERED),
    tap(26.9082, RIGHT, 0.1495, FILTERED),
    tap(28.4206, LEFT, 0.9996, DIRECT),
    tap(29.1394, LEFT, -0.1926, FILTERED),
    tap(33.1825, LEFT, -0.3564, FILTERED),
    tap(33.2279, LEFT, -0.0676, FILTERED),
    tap(34.1757, RIGHT, -0.9995, DIRECT),
    tap(35.4936, RIGHT, -0.5518, FILTERED),
    tap(35.5390, RIGHT, -0.1054, FILTERED),
    tap(37.1666, RIGHT, -0.3920, FILTERED),
    tap(37.2120, RIGHT, -0.0858, FILTERED),
    tap(37.3775, RIGHT, -0.0776, FILTERED),
    tap(37.6428, LEFT, -0.6022, FILTERED),
    tap(37.6882, LEFT, -0.1289, FILTERED),
    tap(39.6746, LEFT, 0.2935, FILTERED),
    tap(39.7200, LEFT, 0.0686, FILTERED),
    tap(41.1259, LEFT, 1.0016, DIRECT),
    tap(41.6859, LEFT, -0.2806, FILTERED),
    tap(41.7313, LEFT, -0.0746, FILTERED),
    tap(43.1326, RIGHT, -0.1072, FILTERED),
    tap(44.5362, RIGHT, 0.1515, FILTERED),
    tap(44.5816, RIGHT, 0.0418, FILTERED),
    tap(48.1780, LEFT, 0.4243, FILTERED),
    tap(48.2234, LEFT, 0.0891, FILTERED),
    tap(48.2846, RIGHT, 0.9996, DIRECT),
    tap(48.3054, LEFT, -0.5038, FILTERED),
    tap(48.3508, LEFT, -0.1114, FILTERED),
    tap(48.5793, RIGHT, -0.1971, FILTERED),
    tap(48.6247, RIGHT, -0.0469, FILTERED),
    tap(50.0828, RIGHT, 0.2066, FILTERED),
    tap(50.1893, LEFT, 0.2703, FILTERED),
    tap(50.2347, LEFT, 0.0603, FILTERED),
    tap(52.9716, RIGHT, 0.2971, FILTERED),
    tap(53.0170, RIGHT, 0.0592, FILTERED),
    tap(54.2415, LEFT, -0.3951, FILTERED),
    tap(54.2868, LEFT, -0.0754, FILTERED),
    tap(54.3322, RIGHT, -0.4101, FILTERED),
    tap(54.3775, RIGHT, -0.0850, FILTERED),
    tap(55.0714, RIGHT, -0.3133, FILTERED),
    tap(55.1168, RIGHT, -0.0847, FILTERED),
    tap(55.6451, RIGHT, 0.9986, DIRECT),
    tap(55.9444, LEFT, -0.2752, FILTERED),
    tap(55.9898, LEFT, -0.0503, FILTERED),
    tap(56.5317, RIGHT, -0.4672, FILTERED),
    tap(56.5771, RIGHT, -0.1130, FILTERED),
    tap(56.8088, LEFT, 0.4531, FILTERED),
    tap(56.8542, LEFT, 0.0984, FILTERED),
    tap(57.2415, RIGHT, 0.2579, FILTERED),
    tap(57.2869, RIGHT, 0.0506, FILTERED),
    tap(58.3753, RIGHT, 0.3353, FILTERED),
    tap(58.4206, RIGHT, 0.0662, FILTERED),
    tap(58.6927, LEFT, -0.1114, FILTERED),
    tap(60.3412, RIGHT, -0.1852, FILTERED),
    tap(60.3866, RIGHT, -0.0487, FILTERED),
    tap(61.6111, LEFT, -0.4870, FILTERED),
    tap(61.6564, LEFT, -0.1047, FILTERED),
    tap(62.8946, LEFT, -0.1875, FILTERED),
    tap(63.7022, RIGHT, -0.0933, FILTERED),
    tap(63.9013, RIGHT, -0.2343, FILTERED),
    tap(64.3843, RIGHT, 0.6327, FILTERED),
    tap(64.4297, RIGHT, 0.1422, FILTERED),
    tap(64.4478, LEFT, 0.4632, FILTERED),
    tap(64.4932, LEFT, 0.1039, FILTERED),
    tap(64.6020, RIGHT, 0.3493, FILTERED),
    tap(64.6474, RIGHT, 0.0694, FILTERED),
    tap(64.8674, RIGHT, -0.2754, FILTERED),
    tap(64.9127, RIGHT, -0.0610, FILTERED),
    tap(65.5861, RIGHT, 0.4961, FILTERED),
    tap(65.6315, RIGHT, 0.1092, FILTERED),
    tap(65.6542, LEFT, -0.2769, FILTERED),
    tap(65.6995, LEFT, -0.0508, FILTERED),
    tap(67.9444, RIGHT, 0.2871, FILTERED),
    tap(67.9898, RIGHT, 0.0764, FILTERED),
    tap(70.8764, RIGHT, 0.2908, FILTERED),
    tap(70.9218, RIGHT, 0.0761, FILTERED),
    tap(71.3412, RIGHT, 0.3763, FILTERED),
    tap(71.3866, RIGHT, 0.0986, FILTERED),
    tap(71.3980, LEFT, 0.3208, FILTERED),
    tap(71.4434, LEFT, 0.0844, FILTERED),
    tap(72.1463, LEFT, -0.1244, FILTERED),
    tap(72.3367, LEFT, 0.2501, FILTERED),
    tap(72.3821, LEFT, 0.0664, FILTERED),
    tap(73.4982, RIGHT, -0.3412, FILTERED),
    tap(73.5435, RIGHT, -0.0898, FILTERED),
    tap(74.4365, RIGHT, -0.1452, FILTERED),
    tap(74.4819, RIGHT, -0.0405, FILTERED),
    tap(74.9897, LEFT, -0.2268, FILTERED),
    tap(75.0351, LEFT, -0.0599, FILTERED),
    tap(75.3821, RIGHT, 0.5496, FILTERED),
    tap(75.4274, RIGHT, 0.1442, FILTERED),
    tap(77.4138, LEFT, -0.5528, FILTERED),
    tap(77.4592, LEFT, -0.1470, FILTERED),
    tap(78.2914, RIGHT, 0.3824, FILTERED),
    tap(78.3368, RIGHT, 0.1015, FILTERED),
    tap(78.5567, LEFT, -0.2883, FILTERED),
    tap(78.6021, LEFT, -0.0752, FILTERED),
    tap(79.0328, LEFT, 0.5694, FILTERED),
    tap(79.0782, LEFT, 0.1506, FILTERED),
    tap(79.5072, RIGHT, 0.3736, FILTERED),
    tap(79.5526, RIGHT, 0.0995, FILTERED),
    tap(79.7063, LEFT, -0.5745, FILTERED),
    tap(79.7517, LEFT, -0.1509, FILTERED),
    tap(80.7771, LEFT, 0.3132, FILTERED),
    tap(80.8224, LEFT, 0.0832, FILTERED),
    tap(81.1372, RIGHT, -0.0825, FILTERED),
    tap(81.3911, RIGHT, 0.2987, FILTERED),
    tap(81.4365, RIGHT, 0.0785, FILTERED),
    tap(82.6610, LEFT, 0.3816, FILTERED),
    tap(82.7063, LEFT, 0.1034, FILTERED),
    tap(83.7494, LEFT, -0.1817, FILTERED),
    tap(83.7948, LEFT, -0.0503, FILTERED),
    tap(84.9512, RIGHT, -0.4406, FILTERED),
    tap(84.9966, RIGHT, -0.1174, FILTERED),
    tap(85.4501, RIGHT, 0.4401, FILTERED),
    tap(85.4955, RIGHT, 0.1154, FILTERED),
    tap(85.9172, LEFT, -0.1973, FILTERED),
    tap(85.9626, LEFT, -0.0533, FILTERED),
    tap(87.1462, RIGHT, -0.2662, FILTERED),
    tap(87.1916, RIGHT, -0.0708, FILTERED),
    tap(88.0874, RIGHT, 0.2976, FILTERED),
    tap(88.1327, RIGHT, 0.0791, FILTERED),
    tap(88.4161, LEFT, -0.2689, FILTERED),
    tap(88.4614, LEFT, -0.0718, FILTERED),
    tap(90.2415, LEFT, -0.5131, FILTERED),
    tap(90.2869, LEFT, -0.1356, FILTERED),
    tap(90.7063, RIGHT, -0.4906, FILTERED),
    tap(90.7517, RIGHT, -0.1286, FILTERED),
    tap(92.8106, RIGHT, -0.4514, FILTERED),
    tap(92.8560, RIGHT, -0.1201, FILTERED),
    tap(94.0964, RIGHT, 0.2566, FILTERED),
    tap(94.1418, RIGHT, 0.0651, FILTERED),
    tap(94.1557, LEFT, -0.3112, FILTERED),
    tap(94.2011, LEFT, -0.0835, FILTERED),
    tap(95.2461, RIGHT, -0.5926, FILTERED),
    tap(95.2914, RIGHT, -0.1544, FILTERED),
    tap(95.3663, LEFT, -0.6659, FILTERED),
    tap(95.4116, LEFT, -0.1768, FILTERED),
    tap(96.0396, LEFT, -0.1242, FILTERED),
    tap(97.6565, RIGHT, 0.1359, FILTERED),
    tap(98.8723, LEFT, -0.2426, FILTERED),
    tap(98.9177, LEFT, -0.0631, FILTERED),
    tap(100.7562, LEFT, -0.4236, FILTERED),
    tap(100.8016, LEFT, -0.1119, FILTERED),
    tap(101.2551, RIGHT, 0.5379, FILTERED),
    tap(101.3005, RIGHT, 0.1418, FILTERED),
    tap(101.7947, LEFT, 0.6534, FILTERED),
    tap(101.8401, LEFT, 0.1723, FILTERED),
    tap(102.5250, LEFT, 0.1720, FILTERED),
    tap(102.5703, LEFT, 0.0449, FILTERED),
    tap(102.6066, RIGHT, 0.2381, FILTERED),
    tap(102.6519, RIGHT, 0.0647, FILTERED),
    tap(104.8152, RIGHT, -0.1643, FILTERED),
    tap(104.8606, RIGHT, -0.0447, FILTERED),
    tap(108.7449, LEFT, -0.5457, FILTERED),
    tap(108.7903, LEFT, -0.1357, FILTERED),
    tap(109.8855, LEFT, 0.1234, FILTERED),
    tap(112.1757, RIGHT, -0.6560, FILTERED),
    tap(112.2211, RIGHT, -0.1729, FILTERED),
    tap(113.4615, LEFT, 0.2213, FILTERED),
    tap(113.5069, LEFT, 0.0585, FILTERED),
    tap(115.9036, LEFT, 0.0994, FILTERED),
    tap(120.6202, LEFT, 0.4507, FILTERED),
    tap(120.6656, LEFT, 0.1144, FILTERED),
    tap(123.2641, LEFT, -0.2001, FILTERED),
    tap(123.3095, LEFT, -0.0567, FILTERED),
    tap(127.9807, LEFT, 0.2487, FILTERED),
    tap(128.0261, LEFT, 0.0656, FILTERED),
];

/// The tap tables in Density order.
const TAP_TABLES: [&[Tap]; 4] = [&TAPS_SPARSE, &TAPS_LOW, &TAPS_MID, &TAPS_HIGH];
/// The most taps a table has.
const MAX_TAPS: usize = 178;
/// The reflections are read this long after the predelay (seconds), as measured.
const TAP_OFFSET_SECONDS: f64 = 3.0 / 44_100.0;
/// Level of the first reflection against the input, before the predelay's fade.
const TAP_GAIN: f64 = 0.0942;
/// How fast the reflections fade, in dB per ms of their delay, times the Decay setting in
/// seconds: this plus [`TAP_FADE_PER_SHAPE`] times Shape.
const TAP_FADE: f64 = 0.0328;
/// How much faster the reflections fade per unit of Shape (dB per ms, times Decay in seconds).
const TAP_FADE_PER_SHAPE: f64 = 0.0668;
/// The whole wet signal is this many dB quieter per ms of predelay, times 1 / Decay in seconds
/// (0.0565 dB per ms at 0.2 s, 0.0094 at 1.2 s).
const PREDELAY_FADE: f64 = 0.0113;
/// Spin swings every reflection's whole delay, predelay included, by this fraction of it per unit
/// of Spin Amount, times [`spin_reach`] (docs/spec/reverb.md › Decisions).
const SPIN_FRACTION: f64 = 0.009_95;
/// Above the knee, [`spin_reach`] is this many ms over the predelay less
/// [`SPIN_REACH_OFFSET_MS`]: a long predelay swings by about the same 2 ms times the fraction
/// whatever the reflection's own delay.
const SPIN_REACH_MS: f64 = 2.0;
/// See [`SPIN_REACH_MS`].
const SPIN_REACH_OFFSET_MS: f64 = 3.02;
/// Spin's LFOs, one per direct reflection (the same ten times in every Density): the
/// reflection's time in the reference room (ms), its rate against Spin Rate and its phase at the
/// start (radians, of a sine that lengthens the delay), as tracked in Live's renders.
const SPIN_VOICES: [(f64, f64, f64); 10] = [
    (0.000, 0.983, 0.66),
    (7.370, 1.042, 1.69),
    (11.429, 0.992, 1.20),
    (17.914, 0.978, 1.54),
    (26.553, 1.016, 0.52),
    (28.435, 1.000, 0.86),
    (34.172, 1.025, 1.01),
    (41.134, 1.008, 0.17),
    (48.299, 1.009, 1.38),
    (55.646, 0.985, 0.33),
];
/// The rate the tap tables' frames count at.
const REFERENCE_RATE: f64 = 44_100.0;
/// A filtered reflection is a direct one this many frames later (reference room and rate), always
/// heard on this side, and swings as that one does. The offsets come in pairs two frames apart,
/// and each pair keeps to one side, as Live's renders show. So the filtered reflections are
/// played as one delayed mix of the direct ones per offset.
const FILTERED_OFFSETS: [(usize, usize); 20] = [
    (395, RIGHT),
    (397, RIGHT),
    (960, LEFT),
    (962, LEFT),
    (1335, LEFT),
    (1337, LEFT),
    (1639, RIGHT),
    (1641, RIGHT),
    (2071, RIGHT),
    (2073, RIGHT),
    (2336, RIGHT),
    (2338, RIGHT),
    (2392, LEFT),
    (2394, LEFT),
    (2493, RIGHT),
    (2495, RIGHT),
    (2982, LEFT),
    (2984, LEFT),
    (3190, LEFT),
    (3192, LEFT),
];
/// Longest delay a filtered offset reaches, at the largest Size (seconds).
const OFFSET_LINE_SECONDS: f64 = 0.13;
/// Longest delay from a direct reflection to the measured network, [`measured::ENTRY_MS`] at Shape
/// 1 and the largest Size (seconds).
const ENTRY_LINE_SECONDS: f64 = 0.06;
/// Peak swing of a line's delay per unit of Chorus Amount.
const CHORUS_SECONDS: f64 = 0.001_8;
/// The reflections' low-pass: a cookbook low-pass with this Q ...
const REFLECTION_Q: f64 = 0.508;
/// ... whose corner rises towards this frequency (Hz) with the Decay setting ...
const REFLECTION_TOP: f64 = 12_000.0;
/// ... being this many hertz lower at a decay of one second ...
const REFLECTION_SPAN: f64 = 818.0;
/// ... and less, as Decay to this power, for longer decays.
const REFLECTION_POWER: f64 = 0.886;
/// The input band's high cut never goes above this frequency, and the input passes a low-pass
/// at it when High Cut is off.
const TOP_CUT: f64 = 18_000.0;

/// Delay from the predelay to the network's taps: this plus the next times the size factor, plus the
/// next again times Shape - 0.5 and the size factor (ms). Puts the tail's onset where it sits for
/// the reference room.
const SHAPE_DELAY_MS: (f64, f64, f64) = (32.0, 18.5, 34.83);
/// Each line's diffuser delay at Scale 100 % in the reference room (ms), in line order. Shorter
/// ones turn every echo into a quick roll of clicks, where Live's echoes are scattered.
const DIFFUSERS: [f64; 16] =
    [4.23, 6.57, 9.99, 15.21, 5.19, 7.83, 11.91, 13.29, 3.57, 8.49, 10.83, 14.37, 4.71, 7.23, 9.39, 12.63];
/// The network's lines in the reference room (ms), in the order Density turns them on: the first
/// 8, 12 or all 16.
const LINES: [f64; 16] =
    [18.29, 39.93, 26.36, 56.83, 22.37, 47.00, 31.39, 69.95, 20.46, 43.11, 30.04, 63.29, 23.83, 51.68, 36.31, 77.25];
/// How many lines each Density runs. Sparse runs as many as Low, longer and less diffused:
/// Live's Sparse has fewer echoes than Low and yet a smoother spectrum, which takes more loop
/// delay than Low's, not less (docs/spec/reverb.md).
const DENSITY_LINES: [usize; 4] = [8, 8, 12, 16];
/// Each Density's lines against [`LINES`].
const DENSITY_STRETCH: [f64; 4] = [1.3, 1.0, 1.0, 1.0];
/// Each Density's diffusion against the Diffusion setting.
const DENSITY_DIFFUSION: [f32; 4] = [0.2, 1.0, 1.0, 1.0];
/// In rooms smaller than the reference one the lines shrink with the size factor to this power
/// only, so a small room keeps enough resonances not to ring: Live's tail at Size 1.53 is as
/// flat as this and rings at no lower pitch.
const SMALL_ROOM_LINE_POWER: f64 = 0.35;
/// Longer lines spread the same energy thinner, so a small room's tail is fed louder, by the
/// square root of how much longer its lines are than the room's, up to this factor (+4 dB, about
/// what Size 1.53 needs): more would make a Size glide from the smallest room ring.
const SMALL_ROOM_GAIN_LIMIT: f64 = 1.6;
/// The first, loudest direct reflection feeds this many lines, spread over the network, so the
/// tail's first echo is a few arrivals, as Live's is, not one click.
const FIRST_TAP_LINES: usize = 3;
/// Level of the tail for each Density against Low (dB), so the four sound equally loud, as
/// measured (docs/spec/reverb.md).
const DENSITY_TRIM_DB: [f64; 4] = [-0.3, 0.0, 0.0, 1.0];
/// Rooms larger than the reference one keep their tail's level: the network's input rises with
/// the size factor to this power (a larger network spreads the same energy over longer lines).
const LARGE_ROOM_POWER: f64 = 0.75;
/// Signs of the input into each line.
const INPUT_SIGNS: [f64; 16] = [1.0, 1.0, -1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, -1.0, 1.0, -1.0, 1.0];
/// Density Low, whose late reverb is the measured network ([`measured`]); the others run the
/// Householder network.
const LOW: usize = 1;
/// Level of the measured network's tail against the Householder network's (the same feed).
const MEASURED_GAIN: f64 = 0.486;
/// Sign of each line in the output.
const OUTPUT_SIGNS: [f64; 16] =
    [1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0];

/// The size factor of the network's lines in a room of size factor `room`: the room's own above
/// the reference size, shrinking less below it.
fn line_room(room: f64) -> f64 {
    if room < 1.0 { room.powf(SMALL_ROOM_LINE_POWER) } else { room }
}

/// The side that hears line `k`: each side hears every other line. The mixing passes most of each
/// line on to the next one, so an echo is heard again on alternate sides, as Live's are, and the
/// sides share no echo and come out nearly uncorrelated.
const fn side_of(k: usize) -> usize {
    k % 2
}
/// Level of the late reverb against the input, for the longest decays; the Householder network's
/// tail starts lower for shorter ones, as if it had already decayed for [`TAIL_FADE_SECONDS`] when
/// it is heard.
const TAIL_GAIN: f64 = 1.236;
/// How long the shortest decays' tail counts as having decayed already when it is heard.
const TAIL_FADE_SECONDS: f64 = 0.0118;
/// The shortest decays' tail is quieter again by this many dB over the Decay setting squared
/// (seconds): 3 dB at 0.2 s, under 0.2 dB from 0.8 s up.
const TAIL_SHORT_DB: f64 = 0.12;
/// The tail is fed as if the reflections came this much later (ms): it takes their fade, Shape's
/// share included, over this much more delay than their own.
const TAIL_LAG_MS: f64 = 20.0;
/// The damping filters' loss is per trip round a loop this long (seconds, reference room).
const LOSS_TRIP_SECONDS: f64 = 0.17;
/// High Damping's Low-Pass is, per trip, a one-pole low-pass `y += (1 - p)(x - y)` at the
/// frequency, its pole `p = e^(-2π f / rate)` taken at this rate (Live's, as measured).
const LOW_PASS_RATE: f64 = 44_100.0;
/// The network's input passes this much of a trip of the Low-Pass (against [`INPUT_TRIPS`] of
/// the other damping): Live's tail is that much darker from its first arrivals.
const INPUT_PASS_TRIPS: f64 = 0.8;
/// The network's input already loses this share of one trip's damping, as Live's tail does from
/// its first arrivals on.
const INPUT_TRIPS: f64 = 0.21;
/// Corner of the first-order low-pass the network's input passes (Hz).
const FEED_CUT: f64 = 15_000.0;
/// The damping shelves are second-order (a cookbook shelf with this Q, half way at the set
/// frequency), steeper than first-order ones.
const SHELF_Q: f64 = BUTTERWORTH_Q;
/// A loss of the highs built into the loop: a high shelf half way at this frequency, this many dB
/// per trip up to [`AIR_DECAY`]; longer decays lose less, with the decay to the power
/// [`AIR_POWER`].
const AIR_FREQUENCY: f64 = 8_000.0;
/// The fixed loss of the highs per trip, in dB, for decays up to [`AIR_DECAY`].
const AIR_LOSS_DB: f64 = 0.96;
/// Decay time (seconds) above which the fixed loss of the highs shrinks.
const AIR_DECAY: f64 = 3.46;
/// Above [`AIR_DECAY`] the fixed loss of the highs shrinks with the decay to this power.
const AIR_POWER: f64 = 0.15;
/// The same for the measured network (Density Low) while Smooth is Slow or Fast: Live's highs
/// decay at 4 to 16 kHz about as this gives at Decay 14.4 and 60 s (the Householder network's
/// own losses need the smaller power). 0.33 left the 60 s tail 4 dB bright at 8 kHz by 4 s.
const MEASURED_AIR_POWER: f64 = 0.25;
/// The measured network's fixed loss of the highs per trip with Smooth None (dB), whatever the
/// decay. Live's loss of the highs comes almost all with Smooth Slow or Fast: with None its
/// 16 kHz octave decays only 0.3 to 0.4 dB/s faster than 1 kHz at Decay 3.46, 14.4 and 60 s,
/// against 2 to 3.5 dB/s with Fast.
const MEASURED_AIR_NONE_DB: f64 = 0.06;
/// In rooms below this size factor (Size 10.5) the measured network's loss of the highs with
/// Smooth Slow or Fast grows per second as the room shrinks: Live's 8 kHz octave decays 3.8 dB/s
/// faster than 1 kHz at Size 1.53 against 1.5 to 1.9 dB/s at Size 10.54, 100 and 500.
const MEASURED_AIR_ROOM: f64 = 0.47;
/// A frozen tail loses its highs as one of this decay (seconds) does: Live's frozen tail with
/// Flat on decays 0.2, 0.55, 1.0 and 1.5 dB/s faster at 2, 4, 8 and 16 kHz than at 1 kHz, as the
/// 60 s tail does.
const FROZEN_AIR_DECAY: f64 = 60.0;
/// Decay time the network is tuned to, against the Decay setting.
const DECAY_RATIO: f64 = 0.9;
/// Low Cut: a first-order shelf this deep (linear), half way at the band's lower edge.
const LOW_CUT_DEPTH: f64 = 0.2;
/// The input always passes a first-order high-pass at this frequency, as measured
/// (docs/spec/reverb.md).
const RUMBLE_FREQUENCY: f64 = 127.0;
/// ... and a second one at this frequency.
const SUBSONIC_FREQUENCY: f64 = 19.0;
/// A frozen tail still falls this many dB per second with Flat on, besides its loss of the highs
/// ([`FROZEN_AIR_DECAY`]), as measured: 0.25 dB/s in all, most of it from 2 to 8 kHz ...
const FREEZE_DRAIN: f64 = 0.07;
/// ... and this many with Flat off, where the damping stays in too.
const FREEZE_DRAIN_UNFLAT: f64 = 0.2;
/// Inside a frozen loop that still takes input, levels above this are compressed so the tail
/// cannot grow without end.
const FREEZE_LIMIT: f64 = 2.0;

/// Longest delay the predelay line needs, with some room.
const PREDELAY_LINE_SECONDS: f64 = 1.0;
/// Longest delay a network line needs, with some room.
const LINE_SECONDS: f64 = 0.3;
/// Longest delay a diffuser needs, with some room.
const DIFFUSER_SECONDS: f64 = 0.035;

/// Parameter indices, in [`INFO`] order.
pub mod param {
    /// Time from the input to the first reflection, in seconds.
    pub const PREDELAY: usize = 0;
    /// Low Cut on the input band.
    pub const LOW_CUT: usize = 1;
    /// High Cut on the input band.
    pub const HIGH_CUT: usize = 2;
    /// Centre of the input band in hertz.
    pub const INPUT_FREQUENCY: usize = 3;
    /// Width of the input band in octaves.
    pub const INPUT_WIDTH: usize = 4;
    /// Spin, which swings the early reflections' delays, on or off.
    pub const SPIN: usize = 5;
    /// How fast Spin swings, in hertz.
    pub const SPIN_RATE: usize = 6;
    /// How far Spin swings.
    pub const SPIN_AMOUNT: usize = 7;
    /// How the reflections fade and when the tail sets in, 0..1.
    pub const SHAPE: usize = 8;
    /// High damping in the loop on or off.
    pub const HIGH_DAMPING: usize = 9;
    /// High damping as a shelf (0) or a low-pass (1).
    pub const HIGH_DAMPING_TYPE: usize = 10;
    /// High damping's frequency in hertz.
    pub const HIGH_DAMPING_FREQUENCY: usize = 11;
    /// High damping's gain per second, linear.
    pub const HIGH_DAMPING_GAIN: usize = 12;
    /// Low damping in the loop on or off.
    pub const LOW_DAMPING: usize = 13;
    /// Low damping's frequency in hertz.
    pub const LOW_DAMPING_FREQUENCY: usize = 14;
    /// Low damping's gain per second, linear.
    pub const LOW_DAMPING_GAIN: usize = 15;
    /// The diffusers' allpass gain.
    pub const DIFFUSION: usize = 16;
    /// The diffusers' delays as a fraction of their longest.
    pub const SCALE: usize = 17;
    /// Chorus, which swings the network's line lengths, on or off.
    pub const CHORUS: usize = 18;
    /// How fast Chorus swings, in hertz.
    pub const CHORUS_RATE: usize = 19;
    /// How far Chorus swings.
    pub const CHORUS_AMOUNT: usize = 20;
    /// The room's size; 100 is the reference room.
    pub const SIZE: usize = 21;
    /// How Size glides to a new value: None, Slow or Fast.
    pub const SMOOTH: usize = 22;
    /// Decay time of the tail, in seconds.
    pub const DECAY: usize = 23;
    /// Holds the tail without decay.
    pub const FREEZE: usize = 24;
    /// While frozen, leaves the damping out.
    pub const FLAT: usize = 25;
    /// While frozen, takes no new input.
    pub const CUT: usize = 26;
    /// Width of the wet signal, 0 (mono) to 120.
    pub const STEREO: usize = 27;
    /// How many lines the network runs: Sparse, Low, Mid or High.
    pub const DENSITY: usize = 28;
    /// Level of the early reflections in decibels.
    pub const REFLECT: usize = 29;
    /// Level of the late reverb in decibels.
    pub const DIFFUSE: usize = 30;
    /// Share of the wet signal in the output.
    pub const MIX: usize = 31;
    /// How many parameters there are.
    pub(crate) const COUNT: usize = 32;
}

/// Lowest Reflect and Diffuse level: a linear gain of 0.03.
pub const MIN_LEVEL_DB: f64 = -30.4576;

/// The parameters, in [`param`] order.
const PARAMS: [ParamInfo; param::COUNT] = [
    ParamInfo::continuous("predelay", "Predelay", (0.000_5, 0.25), 0.002_5, Unit::Seconds, Scale::Logarithmic)
        .in_group("Input"),
    ParamInfo::toggle("lowCut", "Low Cut", false).in_group("Input"),
    ParamInfo::toggle("highCut", "High Cut", true).in_group("Input"),
    ParamInfo::continuous(
        "inputFrequency",
        "Input Frequency",
        (50.0, 18_000.0),
        830.0,
        Unit::Hertz,
        Scale::Logarithmic,
    )
    .in_group("Input"),
    ParamInfo::continuous("inputWidth", "Input Width", (0.5, 9.0), 5.85, Unit::Octaves, Scale::Linear)
        .in_group("Input"),
    ParamInfo::toggle("spin", "Spin", true).in_group("Early Reflections"),
    ParamInfo::continuous("spinRate", "Spin Rate", (0.07, 1.3), 0.3, Unit::Hertz, Scale::Logarithmic)
        .in_group("Early Reflections"),
    ParamInfo::continuous("spinAmount", "Spin Amount", (2.0, 55.0), 17.5, Unit::None, Scale::Logarithmic)
        .in_group("Early Reflections"),
    ParamInfo::continuous("shape", "Shape", (0.0, 1.0), 0.5, Unit::None, Scale::Linear).in_group("Early Reflections"),
    ParamInfo::toggle("highDamping", "High Damping", true).in_group("Damping"),
    ParamInfo::choice("highDampingType", "High Damping Type", &["Shelf", "Low-Pass"], 0).in_group("Damping"),
    ParamInfo::continuous(
        "highDampingFrequency",
        "High Damping Frequency",
        (20.0, 16_000.0),
        4_500.0,
        Unit::Hertz,
        Scale::Logarithmic,
    )
    .in_group("Damping"),
    ParamInfo::continuous("highDampingGain", "High Damping Gain", (0.2, 1.0), 0.7, Unit::None, Scale::Linear)
        .in_group("Damping"),
    ParamInfo::toggle("lowDamping", "Low Damping", false).in_group("Damping"),
    ParamInfo::continuous(
        "lowDampingFrequency",
        "Low Damping Frequency",
        (20.0, 15_000.0),
        90.0,
        Unit::Hertz,
        Scale::Logarithmic,
    )
    .in_group("Damping"),
    ParamInfo::continuous("lowDampingGain", "Low Damping Gain", (0.2, 1.0), 0.75, Unit::None, Scale::Linear)
        .in_group("Damping"),
    ParamInfo::continuous("diffusion", "Diffusion", (0.001, 0.96), 0.6, Unit::Percent, Scale::Linear)
        .in_group("Diffusion"),
    ParamInfo::continuous("scale", "Scale", (0.05, 1.0), 0.4, Unit::Percent, Scale::Linear).in_group("Diffusion"),
    ParamInfo::toggle("chorus", "Chorus", true).in_group("Diffusion"),
    ParamInfo::continuous("chorusRate", "Chorus Rate", (0.01, 8.0), 0.02, Unit::Hertz, Scale::Logarithmic)
        .in_group("Diffusion"),
    ParamInfo::continuous("chorusAmount", "Chorus Amount", (0.01, 4.0), 0.02, Unit::None, Scale::Linear)
        .in_group("Diffusion"),
    ParamInfo::continuous("size", "Size", (0.22, 500.0), 100.0, Unit::None, Scale::Logarithmic).in_group("Decay"),
    ParamInfo::choice("smooth", "Smooth", &["None", "Slow", "Fast"], 2).in_group("Decay"),
    ParamInfo::continuous("decay", "Decay", (0.2, 60.0), 1.2, Unit::Seconds, Scale::Logarithmic).in_group("Decay"),
    ParamInfo::toggle("freeze", "Freeze", false).in_group("Decay"),
    ParamInfo::toggle("flat", "Flat", true).in_group("Decay"),
    ParamInfo::toggle("cut", "Cut", true).in_group("Decay"),
    ParamInfo::continuous("stereo", "Stereo", (0.0, 120.0), 100.0, Unit::None, Scale::Linear).in_group("Global"),
    ParamInfo::choice("density", "Density", &["Sparse", "Low", "Mid", "High"], 1).in_group("Global"),
    ParamInfo::continuous("reflect", "Reflect", (MIN_LEVEL_DB, 6.0), 0.0, Unit::Decibels, Scale::Linear)
        .in_group("Output"),
    ParamInfo::continuous("diffuse", "Diffuse", (MIN_LEVEL_DB, 6.0), 0.0, Unit::Decibels, Scale::Linear)
        .in_group("Output"),
    ParamInfo::continuous("mix", "Dry/Wet", (0.0, 1.0), 0.55, Unit::Percent, Scale::Linear).in_group("Output"),
];

/// What the reverb is and the parameters it has.
pub const INFO: DeviceInfo = DeviceInfo { id: "reverb", name: "Reverb", role: Role::Effect, params: &PARAMS };

/// What the control-rate update works out for the taps and lines.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Tuning {
    /// Each tap's delay after the predelay (frames).
    tap_delays: [f64; MAX_TAPS],
    /// Each tap's gain (linear).
    tap_gains: [f64; MAX_TAPS],
    /// Each tap's share of the network's input (zero for filtered taps).
    feed_gains: [f64; MAX_TAPS],
    /// Each of [`FILTERED_OFFSETS`] (frames).
    offsets: [f64; FILTERED_OFFSETS.len()],
    /// Delay from the predelay to the network's taps (frames).
    shape_delay: f64,
    /// The lines' diffusers' lengths (frames).
    diffusers: [f64; 16],
    /// The lines' lengths (frames).
    lines: [f64; 16],
    /// Each line's gain per trip.
    gains: [f64; 16],
    /// Gain of the input into the network.
    feed: f64,
    /// The measured network's lengths and gains (Density Low).
    measured: measured::Lengths,
    /// Delay from the predelay to the measured network's input (frames).
    entry: f64,
    /// Gain of the input into the measured network.
    measured_feed: f64,
    /// Gain of the filtered reflections into the measured network, from their level as heard to a
    /// direct reflection's share of the input.
    copy_feed: f64,
}

impl Default for Tuning {
    /// Silent taps and lines at the shortest lengths, until the first tuning.
    fn default() -> Self {
        Self {
            tap_delays: [2.0; MAX_TAPS],
            tap_gains: [0.0; MAX_TAPS],
            feed_gains: [0.0; MAX_TAPS],
            offsets: [2.0; FILTERED_OFFSETS.len()],
            shape_delay: 2.0,
            diffusers: [2.0; 16],
            lines: [2.0; 16],
            gains: [0.0; 16],
            feed: 0.0,
            measured: measured::Lengths::default(),
            entry: 2.0,
            measured_feed: 0.0,
            copy_feed: 0.0,
        }
    }
}

impl Tuning {
    /// The values `t` (0..1) of the way from `from` to `to`.
    fn between(&mut self, from: &Self, to: &Self, t: f64) {
        fn mix<const N: usize>(out: &mut [f64; N], a: &[f64; N], b: &[f64; N], t: f64) {
            for ((o, a), b) in out.iter_mut().zip(a).zip(b) {
                *o = a + (b - a) * t;
            }
        }
        mix(&mut self.tap_delays, &from.tap_delays, &to.tap_delays, t);
        mix(&mut self.tap_gains, &from.tap_gains, &to.tap_gains, t);
        mix(&mut self.feed_gains, &from.feed_gains, &to.feed_gains, t);
        mix(&mut self.offsets, &from.offsets, &to.offsets, t);
        mix(&mut self.diffusers, &from.diffusers, &to.diffusers, t);
        mix(&mut self.lines, &from.lines, &to.lines, t);
        mix(&mut self.gains, &from.gains, &to.gains, t);
        self.shape_delay = from.shape_delay + (to.shape_delay - from.shape_delay) * t;
        self.feed = from.feed + (to.feed - from.feed) * t;
        self.measured.between(&from.measured, &to.measured, t);
        self.entry = from.entry + (to.entry - from.entry) * t;
        self.measured_feed = from.measured_feed + (to.measured_feed - from.measured_feed) * t;
        self.copy_feed = from.copy_feed + (to.copy_feed - from.copy_feed) * t;
    }
}

/// One line of the network and its damping.
#[derive(Debug, Clone, Default, PartialEq)]
struct Line {
    /// The line's delay.
    delay: DelayLine,
    /// The allpass the line's output passes on every trip.
    diffuser: Allpass,
    /// The line's damping.
    damper: Damper,
}

/// The network's damping: the high shelf and the low-pass (High damping's two types), the low
/// shelf and the fixed loss of the highs. Every filter always runs and the switches crossfade,
/// so turning one on, off or to the other type does not click.
#[derive(Debug, Clone, Default, PartialEq)]
struct Damper {
    /// High damping as a shelf.
    high_shelf: Biquad,
    /// High damping as a low-pass: two first-order shelves, each with half its loss.
    low_pass: [FirstOrder; 2],
    /// Low damping.
    low_shelf: Biquad,
    /// The fixed loss of the highs.
    air: FirstOrder,
}

/// A filter section the damping runs.
trait Section {
    /// One sample through the section.
    fn run(&mut self, x: f64) -> f64;
    /// Silences the section.
    fn clear(&mut self);
}

impl Section for FirstOrder {
    fn run(&mut self, x: f64) -> f64 {
        self.process(x)
    }

    fn clear(&mut self) {
        self.reset();
    }
}

impl Section for [FirstOrder; 2] {
    fn run(&mut self, x: f64) -> f64 {
        self.iter_mut().fold(x, |y, section| section.process(y))
    }

    fn clear(&mut self) {
        self.iter_mut().for_each(FirstOrder::reset);
    }
}

impl Section for Biquad {
    fn run(&mut self, x: f64) -> f64 {
        self.process(x)
    }

    fn clear(&mut self) {
        self.reset();
    }
}

impl Line {
    /// Silences the line and its damping.
    fn clear(&mut self) {
        self.delay.clear();
        self.diffuser.clear();
        self.damper.clear();
    }
}

impl Damper {
    /// Silences the filters.
    fn clear(&mut self) {
        self.high_shelf.reset();
        self.low_pass.clear();
        self.low_shelf.reset();
        self.air.reset();
    }

    /// `y` through the damping, each filter heard in proportion to its amount. A filter that is
    /// not heard does not run, and starts again from silence (faded in by its amount).
    #[inline]
    fn damp(&mut self, y: f64, amounts: &Damping) -> f64 {
        fn heard(filter: &mut impl Section, x: f64, amount: f64) -> f64 {
            if amount > 0.0 {
                filter.run(x)
            } else {
                filter.clear();
                x
            }
        }
        let y = if amounts.high > 0.0 {
            let shelf = heard(&mut self.high_shelf, y, 1.0 - amounts.low_pass);
            let pass = heard(&mut self.low_pass, y, amounts.low_pass);
            let high = shelf + (pass - shelf) * amounts.low_pass;
            y + (high - y) * amounts.high
        } else {
            self.high_shelf.reset();
            self.low_pass.clear();
            y
        };
        let y = y + (heard(&mut self.low_shelf, y, amounts.low) - y) * amounts.low;
        self.air.process(y)
    }
}

/// How much of each damping filter is heard, 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Damping {
    /// High damping.
    high: f64,
    /// The low-pass's share of high damping (the shelf has the rest).
    low_pass: f64,
    /// Low damping.
    low: f64,
}

/// Declares [`Ramps`] from one list of fields, so the smoothers and their targets are paired by
/// name and cannot fall out of step.
macro_rules! ramps {
    ($($(#[$doc:meta])* $field:ident,)*) => {
        /// Values that ramp to their settings, so moving a control does not click:
        /// `Ramps<Smoother>` holds the smoothers and `Ramps<f64>` their targets.
        #[derive(Debug, Clone, Copy, PartialEq)]
        struct Ramps<T> {
            $($(#[$doc])* $field: T,)*
        }

        impl<T: Copy> Ramps<T> {
            /// Every field set to `value`.
            fn splat(value: T) -> Self {
                Self { $($field: value,)* }
            }
        }

        impl Ramps<Smoother> {
            /// Calls `f` with each smoother and its target.
            fn each(&mut self, targets: &Ramps<f64>, mut f: impl FnMut(&mut Smoother, f32)) {
                $(f(&mut self.$field, targets.$field as f32);)*
            }
        }
    };
}

ramps! {
    /// Predelay (frames).
    predelay,
    /// Reflect (linear).
    reflect,
    /// Diffuse (linear).
    diffuse,
    /// Dry/Wet.
    mix,
    /// Stereo's mixing angle (radians).
    angle,
    /// Gain of the input into the network (0 while frozen with Cut on).
    feed,
    /// How much of Low Cut is heard (0 off, 1 on).
    low_cut,
    /// How much of High Cut is heard (0 off, 1 on).
    high_cut,
    /// Spin's swing, as a fraction of each reflection's whole delay before [`spin_reach`].
    spin,
    /// Chorus's swing (frames).
    chorus,
    /// Shape, read at the control rate.
    shape,
    /// Scale, read at the control rate.
    scale,
    /// The natural log of Decay, read at the control rate.
    decay,
    /// log2 of the input band's centre, read at the control rate.
    centre,
    /// The input band's width in octaves, read at the control rate.
    width,
    /// The diffusers' gain.
    diffusion,
    /// log2 of high damping's frequency, read at the control rate.
    high_frequency,
    /// High damping's gain, read at the control rate.
    high_gain,
    /// log2 of low damping's frequency, read at the control rate.
    low_frequency,
    /// Low damping's gain, read at the control rate.
    low_gain,
    /// How much high damping is heard (see [`Damping`]).
    high,
    /// The low-pass's share of high damping (see [`Damping`]).
    low_pass,
    /// How much low damping is heard (see [`Damping`]).
    low,
}

impl Ramps<Smoother> {
    /// Whether the values read at the control rate are still moving.
    fn control_moving(&self) -> bool {
        [
            &self.shape,
            &self.scale,
            &self.decay,
            &self.high_frequency,
            &self.high_gain,
            &self.low_frequency,
            &self.low_gain,
        ]
        .iter()
        .any(|s| !s.is_settled())
    }

    /// Whether the input band's centre or width is still moving.
    fn band_moving(&self) -> bool {
        !self.centre.is_settled() || !self.width.is_settled()
    }
}

/// The algorithmic room reverb: input filters, predelay, early reflections and a feedback delay
/// network.
pub struct Reverb {
    /// The sample rate it was prepared for.
    sample_rate: f64,
    /// Every parameter's value, in [`param`] order.
    params: ParamBank<{ param::COUNT }>,
    /// Values that ramp to their settings.
    ramps: Ramps<Smoother>,
    /// Low Cut, a first-order low shelf.
    low_cut: FirstOrder,
    /// The fixed roll-off of the lowest octaves at the input.
    rumble: FirstOrder,
    /// A second, lower high-pass of the lowest octave at the input.
    subsonic: FirstOrder,
    /// High Cut, a second-order low-pass.
    high_cut: Biquad,
    /// The low-pass the input passes while High Cut is off.
    top_cut: Biquad,
    /// The reflections' low-pass, for the filtered taps of each side and for each family of them
    /// the measured network takes again ([`measured::COPY_OFFSETS`]).
    reflection_filters: [Biquad; 2 + measured::COPIES],
    /// The early reflections of the Density setting.
    taps: &'static [Tap],
    /// The Density whose reflections [`taps`](Self::taps) are.
    reflections: usize,
    /// One line per [`FILTERED_OFFSETS`] entry, holding the mix of direct reflections that
    /// reflections at that offset play.
    offset_lines: [DelayLine; FILTERED_OFFSETS.len()],
    /// Which of [`offset_lines`](Self::offset_lines) the Density setting's reflections play.
    offsets_used: [bool; FILTERED_OFFSETS.len()],
    /// The predelay line, which the taps and the Shape delay read.
    input: DelayLine,
    /// The predelayed input after the loop's damping, once for [`INPUT_TRIPS`] of a trip, which
    /// the network's taps read.
    feed_line: DelayLine,
    /// The measured network's input groups, each the sum of its direct reflections as they were
    /// read (Spin's shift included), delayed by the Shape entry before the network takes them; the
    /// last ones hold the families of filtered reflections it takes as well.
    group_lines: [DelayLine; measured::GROUPS + measured::COPIES],
    /// The damping in front of [`feed_line`](Self::feed_line).
    input_damper: Damper,
    /// The low-pass after [`input_damper`](Self::input_damper), at [`FEED_CUT`].
    feed_cut: FirstOrder,
    /// The network's lines; the first [`active`](Self::active) run.
    lines: [Line; 16],
    /// The measured network, which Density Low runs instead of [`lines`](Self::lines).
    measured: measured::Network,
    /// Whether the measured network ran in the last frame.
    measured_running: bool,
    /// Lines running (Density).
    active: usize,
    /// Each line's share of the input.
    weights: [f64; 16],
    /// Cosine and sine of each line's phase offset.
    chorus_offsets: [(f64, f64); 16],
    /// How each reflection is read.
    routes: [Route; MAX_TAPS],
    /// The size factor `(size / 100)^(1/3)` as it glides.
    room: f64,
    /// The tuning at the last control point.
    from: Tuning,
    /// The tuning at the next control point.
    to: Tuning,
    /// The tuning in between, for this frame.
    now: Tuning,
    /// Whether `from` and `to` differ, so frames in between need `now`.
    moving: bool,
    /// Spin's LFO positions, in cycles (0..1), one per [`SPIN_VOICES`].
    spin_phases: [f64; SPIN_VOICES.len()],
    /// Chorus's LFO position, in cycles (0..1).
    chorus_phase: f64,
    /// Whether the transport ran in the last block: Spin's LFOs start again when it starts.
    playing: bool,
    /// Frames since the last reset, for the control-rate grid.
    frame: u64,
    /// Whether a parameter changed since the last control update.
    dirty: bool,
}

impl Default for Reverb {
    /// The same as [`Reverb::new`].
    fn default() -> Self {
        Self::new()
    }
}

/// A level in decibels as a linear gain.
fn db_to_gain(db: f64) -> f64 {
    10.0f64.powf(db / 20.0)
}

/// Cosine and sine of golden-angle phase offsets, from the `first`th on, so no two taps or lines
/// swing together.
fn phase_offsets<const N: usize>(first: usize) -> [(f64, f64); N] {
    std::array::from_fn(|k| {
        let p = std::f64::consts::TAU * ((k + first) as f64 * 0.381_966_011_250_105).fract();
        (p.cos(), p.sin())
    })
}

/// How far Spin reaches with `predelay_ms` of predelay, as a share of [`SPIN_FRACTION`]: all of
/// it up to the knee (5 ms), then [`SPIN_REACH_MS`] over the predelay less
/// [`SPIN_REACH_OFFSET_MS`] (0.29 at 10 ms, 0.0083 at 250 ms).
fn spin_reach(predelay_ms: f64) -> f64 {
    SPIN_REACH_MS / (predelay_ms - SPIN_REACH_OFFSET_MS).max(SPIN_REACH_MS)
}

/// How a reflection is read.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Route {
    /// From the predelay line at its own delay, swinging by it on this LFO of [`SPIN_VOICES`].
    Own(usize),
    /// As the direct reflection `source` (its place in the table), one of [`FILTERED_OFFSETS`]
    /// later.
    Follows { source: usize, offset: usize },
}

/// Each tap's [`Reverb::routes`] entry: a direct tap is read at its own delay, on the LFO of its
/// time; a filtered one follows the direct tap it is one of [`FILTERED_OFFSETS`] on its side
/// after, or is read at its own delay if it follows none.
fn routes(taps: &[Tap]) -> [Route; MAX_TAPS] {
    let voice = |ms: f64| {
        SPIN_VOICES
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1.0 - ms).abs().total_cmp(&(b.1.0 - ms).abs()))
            .map_or(0, |(k, _)| k)
    };
    let mut routes = [Route::Own(0); MAX_TAPS];
    for (route, tap) in routes.iter_mut().zip(taps) {
        let followed = FILTERED_OFFSETS.iter().enumerate().filter(|&(_, &(_, side))| side == tap.side).find_map(
            |(offset, &(frames, _))| {
                let ms = frames as f64 * 1_000.0 / REFERENCE_RATE;
                taps.iter().position(|d| !d.filtered && (tap.ms - d.ms - ms).abs() < 0.0015).map(|d| (d, offset))
            },
        );
        *route = match followed {
            Some((source, offset)) if tap.filtered => Route::Follows { source, offset },
            _ => Route::Own(voice(tap.ms)),
        };
    }
    routes
}

/// Spin's LFO positions at the start, in cycles.
fn spin_start() -> [f64; SPIN_VOICES.len()] {
    SPIN_VOICES.map(|(_, _, phase)| phase / std::f64::consts::TAU)
}

/// The sign of each line's input with `active` lines running (zero for lines not running).
fn input_weights(active: usize) -> [f64; 16] {
    std::array::from_fn(|k| if k < active { INPUT_SIGNS.get(k).copied().unwrap_or(1.0) } else { 0.0 })
}

/// Mixes the first `n` lines' outputs with a Householder reflection, `I - (2/n) 11ᵀ`, then passes
/// each line's share on to the next one. Each line keeps most of itself (three quarters at 8
/// lines) and sends a little to every other one, so the echoes thicken over several trips as
/// Live's do (docs/spec/reverb.md › Decisions); the shift stops two trips undoing each other.
#[inline]
fn mix_lines(v: &mut [f64; 16], n: usize) {
    if let Some(lines) = v.get_mut(..n) {
        let share = lines.iter().sum::<f64>() * 2.0 / n.max(1) as f64;
        for x in lines.iter_mut() {
            *x -= share;
        }
        lines.rotate_left(1);
    }
}

/// Soft limit for a frozen loop: unchanged up to [`FREEZE_LIMIT`], then compressed towards twice it.
#[inline]
fn limit(x: f64) -> f64 {
    let over = x.abs() - FREEZE_LIMIT;
    if over <= 0.0 { x } else { x.signum() * (FREEZE_LIMIT + over / (1.0 + over / FREEZE_LIMIT)) }
}

impl Reverb {
    /// A reverb with every parameter at its default, prepared for the default sample rate
    /// (allocates).
    pub fn new() -> Self {
        let sr = DEFAULT_SAMPLE_RATE;
        let mut reverb = Self {
            sample_rate: sr,
            params: ParamBank::new(&PARAMS),
            ramps: Ramps::splat(Smoother::new(0.0, 1)),
            low_cut: FirstOrder::default(),
            rumble: FirstOrder::default(),
            subsonic: FirstOrder::default(),
            high_cut: Biquad::default(),
            top_cut: Biquad::default(),
            reflection_filters: Default::default(),
            taps: &TAPS_LOW,
            reflections: LOW,
            offset_lines: Default::default(),
            offsets_used: [false; FILTERED_OFFSETS.len()],
            input: DelayLine::default(),
            feed_line: DelayLine::default(),
            group_lines: std::array::from_fn(|_| DelayLine::default()),
            input_damper: Damper::default(),
            feed_cut: FirstOrder::default(),
            lines: Default::default(),
            measured: measured::Network::default(),
            measured_running: true,
            active: 8,
            weights: input_weights(8),
            chorus_offsets: phase_offsets(7),
            routes: routes(&TAPS_LOW),
            room: 1.0,
            from: Tuning::default(),
            to: Tuning::default(),
            now: Tuning::default(),
            moving: false,
            spin_phases: spin_start(),
            playing: false,
            chorus_phase: 0.0,
            frame: 0,
            dirty: true,
        };
        reverb.allocate();
        reverb.rebuild();
        reverb
    }

    /// Allocates the lines for the current sample rate.
    fn allocate(&mut self) {
        let sr = self.sample_rate;
        self.input = DelayLine::for_seconds(PREDELAY_LINE_SECONDS, sr);
        self.feed_line = DelayLine::for_seconds(PREDELAY_LINE_SECONDS, sr);
        self.group_lines = std::array::from_fn(|_| DelayLine::for_seconds(ENTRY_LINE_SECONDS, sr));
        self.offset_lines = std::array::from_fn(|_| DelayLine::for_seconds(OFFSET_LINE_SECONDS, sr));
        self.lines = std::array::from_fn(|_| Line {
            delay: DelayLine::for_seconds(LINE_SECONDS, sr),
            diffuser: Allpass::for_seconds(DIFFUSER_SECONDS, sr),
            damper: Damper::default(),
        });
        self.measured = measured::Network::new(sr);
    }

    /// The value of parameter `index`, 0.0 for an unknown one.
    fn value(&self, index: usize) -> f64 {
        self.params.get(index)
    }

    /// Whether switch `index` is on.
    fn on(&self, index: usize) -> bool {
        self.params.on(index)
    }

    /// Whether Freeze is on.
    fn frozen(&self) -> bool {
        self.on(param::FREEZE)
    }

    /// Where each ramped value is heading, from the parameters.
    fn targets(&self) -> Ramps<f64> {
        let sr = self.sample_rate;
        let flag = |index| if self.on(index) { 1.0 } else { 0.0 };
        // Frozen with Flat on, the damping is left out; with Flat off it stays in, as Live's frozen
        // tail goes on losing the bands the shelves take. The fixed loss of the highs stays in
        // either way, at its longest decay's (FREEZE_DRAIN).
        let filtered = if self.frozen() && self.on(param::FLAT) { 0.0 } else { 1.0 };
        Ramps {
            predelay: self.value(param::PREDELAY) * sr,
            reflect: db_to_gain(self.value(param::REFLECT)),
            diffuse: db_to_gain(self.value(param::DIFFUSE)),
            mix: self.value(param::MIX),
            angle: std::f64::consts::FRAC_PI_4 * (1.0 - self.value(param::STEREO) / 120.0),
            feed: if self.frozen() && self.on(param::CUT) { 0.0 } else { 1.0 },
            low_cut: flag(param::LOW_CUT),
            high_cut: flag(param::HIGH_CUT),
            spin: flag(param::SPIN) * SPIN_FRACTION * self.value(param::SPIN_AMOUNT),
            chorus: flag(param::CHORUS) * CHORUS_SECONDS * self.value(param::CHORUS_AMOUNT) * sr,
            shape: self.value(param::SHAPE),
            scale: self.value(param::SCALE),
            decay: self.value(param::DECAY).max(0.01).ln(),
            centre: self.value(param::INPUT_FREQUENCY).max(1.0).log2(),
            width: self.value(param::INPUT_WIDTH),
            diffusion: self.value(param::DIFFUSION),
            high_frequency: self.value(param::HIGH_DAMPING_FREQUENCY).max(1.0).log2(),
            high_gain: self.value(param::HIGH_DAMPING_GAIN),
            low_frequency: self.value(param::LOW_DAMPING_FREQUENCY).max(1.0).log2(),
            low_gain: self.value(param::LOW_DAMPING_GAIN),
            high: flag(param::HIGH_DAMPING) * filtered,
            low_pass: flag(param::HIGH_DAMPING_TYPE),
            low: flag(param::LOW_DAMPING) * filtered,
        }
    }

    /// The size factor `(size / 100)^(1/3)` for the Size setting.
    fn room_target(&self) -> f64 {
        (self.value(param::SIZE) / REFERENCE_SIZE).cbrt()
    }

    /// Every state cleared, every ramped value at its setting.
    fn rebuild(&mut self) {
        let ramp = Smoother::ramp_frames(RAMP_SECONDS, self.sample_rate);
        let targets = self.targets();
        self.ramps.each(&targets, |smoother, target| {
            smoother.set_ramp(ramp);
            smoother.reset(target);
        });
        self.ramps.predelay.set_ramp(Smoother::ramp_frames(PREDELAY_RAMP_SECONDS, self.sample_rate));
        self.low_cut.reset();
        self.rumble = FirstOrder::new(FirstOrderCoefficients::highpass(self.sample_rate, RUMBLE_FREQUENCY));
        self.subsonic = FirstOrder::new(FirstOrderCoefficients::highpass(self.sample_rate, SUBSONIC_FREQUENCY));
        self.high_cut.reset();
        self.top_cut = Biquad::new(Coefficients::lowpass(self.sample_rate, TOP_CUT, BUTTERWORTH_Q));
        if TOP_CUT >= 0.45 * self.sample_rate {
            self.top_cut.set(Coefficients::IDENTITY);
        }
        for filter in &mut self.reflection_filters {
            filter.reset();
        }
        self.input.clear();
        self.feed_line.clear();
        for line in &mut self.group_lines {
            line.clear();
        }
        for line in &mut self.offset_lines {
            line.clear();
        }
        self.input_damper.clear();
        self.feed_cut = FirstOrder::new(if FEED_CUT < 0.45 * self.sample_rate {
            FirstOrderCoefficients::lowpass(self.sample_rate, FEED_CUT)
        } else {
            FirstOrderCoefficients::IDENTITY
        });
        for line in &mut self.lines {
            line.clear();
        }
        self.measured.clear();
        self.measured_running = self.value(param::DENSITY) as usize == LOW;
        self.set_active(self.density_lines());
        self.room = self.room_target();
        self.spin_phases = spin_start();
        self.chorus_phase = 0.0;
        self.frame = 0;
        self.tune_input();
        self.tune();
        self.from = self.to;
        self.now = self.to;
        self.moving = false;
        self.dirty = false;
    }

    /// Runs the first `lines` lines of the network, and the Density setting's reflections.
    fn set_active(&mut self, lines: usize) {
        self.active = lines.clamp(1, LINES.len());
        self.reflections = self.value(param::DENSITY) as usize;
        self.taps = TAP_TABLES.get(self.reflections).copied().unwrap_or(&TAPS_LOW);
        self.routes = routes(self.taps);
        let mut used = [false; FILTERED_OFFSETS.len()];
        for route in self.routes.iter().take(self.taps.len()) {
            if let &Route::Follows { offset, .. } = route
                && let Some(slot) = used.get_mut(offset)
            {
                *slot = true;
            }
        }
        // Offset lines that start again begin silent.
        for ((line, &now), was) in self.offset_lines.iter_mut().zip(&used).zip(self.offsets_used) {
            if now && !was {
                line.clear();
            }
        }
        self.offsets_used = used;
        self.weights = input_weights(self.active);
    }

    /// How many lines the Density setting runs.
    fn density_lines(&self) -> usize {
        DENSITY_LINES.get(self.value(param::DENSITY) as usize).copied().unwrap_or(8)
    }

    /// The input filters from the band's (ramped) centre and width.
    fn tune_input(&mut self) {
        let sr = self.sample_rate;
        let centre = 2.0f64.powf(f64::from(self.ramps.centre.current()));
        let half = f64::from(self.ramps.width.current()) / 2.0;
        let (low, high) = (centre / 2.0f64.powf(half), centre * 2.0f64.powf(half));
        self.low_cut.set(FirstOrderCoefficients::low_shelf(sr, low, LOW_CUT_DEPTH));
        let high = high.min(TOP_CUT);
        self.high_cut.set(if high < sr * 0.45 {
            Coefficients::lowpass(sr, high, BUTTERWORTH_Q)
        } else {
            Coefficients::IDENTITY
        });
    }

    /// Delay lengths, tap levels, line gains and damping for the current size and settings, into
    /// `to`.
    fn tune(&mut self) {
        let sr = self.sample_rate;
        let room = self.room;
        let ms = sr / 1_000.0;
        let shape = f64::from(self.ramps.shape.current());
        let scale = f64::from(self.ramps.scale.current());
        let setting = f64::from(self.ramps.decay.current()).exp();
        let fade = (TAP_FADE + TAP_FADE_PER_SHAPE * shape) / setting;
        let predelay_ms = self.value(param::PREDELAY) * 1_000.0;
        let frozen = self.frozen();
        let decay = setting * DECAY_RATIO;
        // Once nothing moves, the lines are whole frames long, so a line without chorus is read
        // without interpolation, which would take away highs on every trip. While Size or Decay
        // glide they move smoothly between frames.
        let settled = self.room == self.room_target() && !self.ramps.control_moving();
        let density_index = self.value(param::DENSITY) as usize;
        let density = DENSITY_TRIM_DB.get(density_index).copied().unwrap_or(0.0);
        let line_room = line_room(room);
        let stretch = DENSITY_STRETCH.get(density_index).copied().unwrap_or(1.0) * line_room / room;
        let lines = LINES.map(|line| line * stretch);
        let predelay_gain = db_to_gain(-PREDELAY_FADE * predelay_ms / setting);
        let taps = self.taps;
        // The network is fed by the direct reflections, each at its own level and fade.
        let norm = taps.iter().filter(|tap| !tap.filtered).map(|tap| tap.gain * tap.gain).sum::<f64>().sqrt().max(1e-9);
        let t = &mut self.to;
        t.tap_gains = [0.0; MAX_TAPS];
        t.feed_gains = [0.0; MAX_TAPS];
        for (((delay, gain), feed), tap) in
            t.tap_delays.iter_mut().zip(&mut t.tap_gains).zip(&mut t.feed_gains).zip(taps)
        {
            let time = tap.ms * room;
            *delay = time * ms + TAP_OFFSET_SECONDS * sr;
            let level = tap.gain * db_to_gain(-fade * time);
            *gain = TAP_GAIN * predelay_gain * level;
            if !tap.filtered {
                *feed = level / norm;
            }
        }
        for (offset, (frames, _)) in t.offsets.iter_mut().zip(FILTERED_OFFSETS) {
            let frames = frames as f64 * room * sr / REFERENCE_RATE;
            *offset = if settled { frames.round() } else { frames }.max(2.0);
        }
        let (a, b, c) = SHAPE_DELAY_MS;
        t.shape_delay = ((a + (b + c * (shape - 0.5)) * room) * ms).max(0.0);
        for (length, base) in t.diffusers.iter_mut().zip(DIFFUSERS) {
            let frames = base * scale * room * ms;
            *length = if settled { frames.round() } else { frames }.max(2.0);
        }
        for (((length, gain), base), diffuser) in t.lines.iter_mut().zip(t.gains.iter_mut()).zip(lines).zip(DIFFUSERS) {
            let frames = base * room * ms;
            *length = if settled { frames.round() } else { frames }.max(2.0);
            // A trip is the line and its diffuser.
            let seconds = (base + diffuser * scale) * room / 1_000.0;
            *gain = if frozen {
                db_to_gain(-seconds * if self.params.on(param::FLAT) { FREEZE_DRAIN } else { FREEZE_DRAIN_UNFLAT })
            } else {
                db_to_gain(-60.0 * seconds / decay)
            };
        }
        let gain = |seconds: f64| {
            if frozen {
                db_to_gain(-seconds * if self.params.on(param::FLAT) { FREEZE_DRAIN } else { FREEZE_DRAIN_UNFLAT })
            } else {
                db_to_gain(-60.0 * seconds / decay)
            }
        };
        t.measured.tune(room, scale, sr, settled, gain, LOSS_TRIP_SECONDS);
        let (entry, per_shape) = measured::ENTRY_MS;
        t.entry = ((entry + per_shape * (shape - 0.5) * room) * ms).max(0.0);
        let short = 60.0 * TAIL_FADE_SECONDS / setting + TAIL_SHORT_DB / (setting * setting) + fade * TAIL_LAG_MS;
        t.feed = TAIL_GAIN * predelay_gain * db_to_gain(density) * room.max(1.0).powf(LARGE_ROOM_POWER);
        // The measured network's lines scale with the room's own size factor, small rooms too.
        // Every path through it fades with its whole time, so it needs no cut for short decays;
        // it keeps Shape's share of the fade over the 20 ms, about Shape 0.5.
        t.measured_feed =
            t.feed * MEASURED_GAIN * db_to_gain(-TAP_FADE_PER_SHAPE * (shape - 0.5) * TAIL_LAG_MS / setting);
        t.copy_feed = t.measured_feed / (TAP_GAIN * predelay_gain * norm);
        t.feed *= db_to_gain(-short);
        t.feed *= (line_room / room).sqrt().min(SMALL_ROOM_GAIN_LIMIT);
        let corner = (REFLECTION_TOP - REFLECTION_SPAN * setting.powf(-REFLECTION_POWER)).clamp(20.0, 0.45 * sr);
        for filter in &mut self.reflection_filters {
            filter.set(Coefficients::lowpass(sr, corner, REFLECTION_Q));
        }

        let air_setting = if frozen { FROZEN_AIR_DECAY } else { setting };
        let air_db = AIR_LOSS_DB * (AIR_DECAY / air_setting).powf(AIR_POWER).min(1.0);
        let measured_air_db = if self.value(param::SMOOTH) as usize == 0 {
            MEASURED_AIR_NONE_DB
        } else {
            AIR_LOSS_DB
                * (AIR_DECAY / air_setting).powf(MEASURED_AIR_POWER).min(1.0)
                * (MEASURED_AIR_ROOM / room).max(1.0)
        };
        let r = &self.ramps;
        let (high_f, high_g) = (2.0f64.powf(f64::from(r.high_frequency.current())), f64::from(r.high_gain.current()));
        let (low_f, low_g) = (2.0f64.powf(f64::from(r.low_frequency.current())), f64::from(r.low_gain.current()));
        // A line's share of the Low-Pass's one-pole is two first-order high shelves, each with half
        // that share of its loss at the top, half way where the one-pole has lost a quarter and
        // three quarters of it: one shelf alone has too narrow a slope for a fraction of a trip.
        let pole = (-std::f64::consts::TAU * high_f / LOW_PASS_RATE).exp();
        let ratio = (1.0 + pole) / (1.0 - pole).max(1e-9);
        let pass_db = 20.0 * ratio.log10();
        // Where the one-pole's loss is `share` of its loss at the top (Hz).
        let reach = |share: f64| {
            let q = ratio.powf(2.0 * share) * (1.0 - pole) * (1.0 - pole);
            ((1.0 + pole * pole - q) / (2.0 * pole).max(1e-9)).clamp(-1.0, 1.0).acos() * LOW_PASS_RATE
                / std::f64::consts::TAU
        };
        let pass_corners = [reach(0.25), reach(0.75)];
        let tune = |damper: &mut Damper, trips: f64, pass_trips: f64, air_db: f64| {
            let db = |g: f64| 20.0 * g.max(1e-6).log10() * trips;
            damper.high_shelf.set(Coefficients::high_shelf(sr, high_f, SHELF_Q, db(high_g)));
            for (section, corner) in damper.low_pass.iter_mut().zip(pass_corners) {
                section.set(FirstOrderCoefficients::high_shelf(sr, corner, db_to_gain(-pass_db * pass_trips / 2.0)));
            }
            damper.low_shelf.set(Coefficients::low_shelf(sr, low_f, SHELF_Q, db(low_g)));
            damper.air.set(FirstOrderCoefficients::high_shelf(sr, AIR_FREQUENCY, db_to_gain(-air_db * trips)));
        };
        for ((line, base), diffuser) in self.lines.iter_mut().zip(lines).zip(DIFFUSERS) {
            // The loss is per trip round the reference loop; a line of another length, or in a
            // room of another size, loses in proportion to its length, so the loss per second
            // does not depend on the room.
            let trips = (base + diffuser * scale) * room / 1_000.0 / LOSS_TRIP_SECONDS;
            tune(&mut line.damper, trips, trips, air_db);
        }
        for (line, &trips) in self.measured.lines.iter_mut().zip(&self.to.measured.trips) {
            tune(&mut line.damper, trips, trips, measured_air_db);
        }
        let trips = self.to.measured.return_trips;
        tune(&mut self.measured.return_line.damper, trips, trips, measured_air_db);
        tune(&mut self.input_damper, INPUT_TRIPS, INPUT_PASS_TRIPS, air_db);
    }

    /// The longest delay Size moves, a line or a direct reflection, in a room of size factor
    /// `room` (frames).
    fn longest_delay(&self, room: f64) -> f64 {
        let stretch = DENSITY_STRETCH.get(self.value(param::DENSITY) as usize).copied().unwrap_or(1.0);
        let line = LINES.iter().copied().fold(0.0, f64::max) * stretch * line_room(room);
        let tap = self.taps.iter().map(|tap| tap.ms).fold(0.0, f64::max) * room;
        let longest = line.max(tap) * self.sample_rate / 1_000.0;
        if self.value(param::DENSITY) as usize == LOW {
            longest.max(measured::Lengths::longest(room, self.sample_rate))
        } else {
            longest
        }
    }

    /// Size moves toward its setting, as fast as Smooth says, over one control period.
    fn glide_room(&mut self) {
        let target = self.room_target();
        let seconds = match self.value(param::SMOOTH) as usize {
            1 => SMOOTH_SLOW_SECONDS,
            2 => SMOOTH_FAST_SECONDS,
            _ => 0.0,
        };
        let frames = seconds * self.sample_rate;
        let keep = if frames > 1.0 { (-(CONTROL_FRAMES as f64) / frames).exp() } else { 0.0 };
        let mut room = target + (self.room - target) * keep;
        let moved = (self.longest_delay(room) - self.longest_delay(self.room)).abs();
        let most = GLIDE_SLEW * CONTROL_FRAMES as f64;
        if frames > 1.0 && moved > most {
            room = self.room + (room - self.room) * most / moved;
        }
        let room = if (room - target).abs() < 1e-6 { target } else { room };
        if room != self.room {
            self.room = room;
            self.dirty = true;
        }
    }

    /// At a control point: what the next stretch of frames glides to.
    fn control(&mut self) {
        self.from = self.to;
        self.glide_room();
        if self.ramps.band_moving() {
            self.tune_input();
        }
        let moving = self.ramps.control_moving();
        if self.dirty || moving {
            self.tune();
            // One more update once the ramps stop, so the lines settle on whole frames.
            self.dirty = moving;
        }
        self.moving = self.from != self.to;
    }
}

impl Device for Reverb {
    /// The reverb's [`INFO`].
    fn info(&self) -> &'static DeviceInfo {
        &INFO
    }

    /// Allocates the lines for `sample_rate` and starts silent.
    fn prepare(&mut self, sample_rate: f64) -> Result<(), DeviceError> {
        self.sample_rate = check_sample_rate(sample_rate)?;
        self.allocate();
        self.rebuild();
        Ok(())
    }

    /// Silences the reverb and settles every ramped value at its setting.
    fn reset(&mut self) {
        self.rebuild();
    }

    /// Sets parameter `index`, clamped to its range; the ramped values move to it.
    fn set_param(&mut self, index: usize, value: f64) {
        if self.params.set(index, value).is_none() {
            return;
        }
        let targets = self.targets();
        self.ramps.each(&targets, |smoother, target| smoother.set_target(target));
        self.dirty = true;
    }

    /// The value of parameter `index`, or `None` for an unknown one.
    fn param(&self, index: usize) -> Option<f64> {
        self.params.param(index)
    }

    /// Runs interleaved stereo `buffer` through the reverb in place.
    fn process(&mut self, context: &ProcessContext, buffer: &mut [f32]) {
        let sr = self.sample_rate;
        // Live's Spin LFOs start from the same phases whenever playback starts.
        if context.playing && !self.playing {
            self.spin_phases = spin_start();
        }
        self.playing = context.playing;
        let wanted = self.density_lines();
        // Sparse and Low run as many lines, but not the same reflections.
        if wanted != self.active || self.reflections != self.value(param::DENSITY) as usize {
            // Lines that start again begin silent.
            for line in self.lines.iter_mut().skip(self.active).take(wanted.saturating_sub(self.active)) {
                line.clear();
            }
            self.set_active(wanted);
        }
        let active = self.active;
        let density_index = self.value(param::DENSITY) as usize;
        let measured = density_index == LOW;
        if measured != self.measured_running {
            // The network that takes over starts silent.
            if measured {
                self.measured.clear();
            } else {
                for line in &mut self.lines {
                    line.clear();
                }
            }
            self.measured_running = measured;
        }
        let diffusion_scale = DENSITY_DIFFUSION.get(density_index).copied().unwrap_or(1.0);
        // Each side hears half the lines (the measured network's levels are its own).
        let norm = if measured { 1.0 } else { (2.0 / active as f64).sqrt() };
        let (spin_step, chorus_step) = (self.value(param::SPIN_RATE) / sr, self.value(param::CHORUS_RATE) / sr);
        let frozen = self.frozen();
        // Stereo's rotation and the mix's gains, worked out again only when their inputs move
        // (NaN never equals an input, so the first frame works them out).
        let mut output_key = (f64::NAN, f64::NAN);
        let (mut rotation, mut gains) = ((0.0, 0.0), (0.0, 0.0));

        for frame in buffer.as_chunks_mut::<2>().0 {
            let step = self.frame % CONTROL_FRAMES;
            if step == 0 {
                self.control();
            }
            if self.moving {
                self.now.between(&self.from, &self.to, (step + 1) as f64 / CONTROL_FRAMES as f64);
            }
            let r = &mut self.ramps;
            let [predelay, reflect, diffuse, mix, angle, feed, low_cut, high_cut, spin_swing, chorus_swing] = [
                &mut r.predelay,
                &mut r.reflect,
                &mut r.diffuse,
                &mut r.mix,
                &mut r.angle,
                &mut r.feed,
                &mut r.low_cut,
                &mut r.high_cut,
                &mut r.spin,
                &mut r.chorus,
            ]
            .map(|s| f64::from(s.next_value()));
            for s in [
                &mut r.shape,
                &mut r.scale,
                &mut r.decay,
                &mut r.centre,
                &mut r.width,
                &mut r.high_frequency,
                &mut r.high_gain,
                &mut r.low_frequency,
                &mut r.low_gain,
            ] {
                s.next_value();
            }
            let diffusion = r.diffusion.next_value() * diffusion_scale;
            let damping = Damping {
                high: f64::from(r.high.next_value()),
                low_pass: f64::from(r.low_pass.next_value()),
                low: f64::from(r.low.next_value()),
            };

            let input = frame.map(|s| if s.is_finite() { f64::from(s) } else { 0.0 });
            // Both cuts always run, so switching one crossfades instead of clicking; High Cut
            // takes the place of the fixed top cut.
            let mut x = self.subsonic.process(self.rumble.process((input[0] + input[1]) / 2.0));
            x += (self.low_cut.process(x) - x) * low_cut;
            let top = self.top_cut.process(x);
            x = top + (self.high_cut.process(x) - top) * high_cut;
            let t = if self.moving { &self.now } else { &self.to };

            // Early reflections and the network's input, read from the predelay line. Spin swings
            // each direct reflection's whole delay on its own LFO, by a share of it that falls as
            // the predelay grows, and the network's input with it; a filtered one is the direct
            // one it follows, a whole number of frames later, so it is played from a mix of the
            // direct ones per offset. Without Spin, and with nothing moving, the taps fall on
            // whole frames.
            let mut spin = [0.0f64; SPIN_VOICES.len()];
            if spin_swing > 0.0 {
                let reach = spin_swing * spin_reach(predelay * 1_000.0 / sr);
                for (s, phase) in spin.iter_mut().zip(&self.spin_phases) {
                    *s = reach * (std::f64::consts::TAU * phase).sin();
                }
            }
            let whole = spin_swing == 0.0 && !self.moving && self.ramps.predelay.is_settled();
            let entry_whole = t.entry.round();
            let read_from = |line: &DelayLine, at: f64| {
                f64::from(if whole { line.read(at.round() as usize) } else { line.read_hermite(at) })
            };
            // Direct and filtered reflections, left and right; each direct one also feeds a line
            // of its own, in turn, after the Shape delay, and the first feeds a few.
            let (mut direct, mut filtered) = ([0.0f64; 2], [0.0f64; 2]);
            let mut inputs = [0.0f64; 16];
            let mut groups = [0.0f64; measured::GROUPS + measured::COPIES];
            let mut direct_index = 0;
            let mut next_line = 0;
            let mut spread = FIRST_TAP_LINES.clamp(1, active);
            let mut reads = [0.0f64; MAX_TAPS];
            for (((((tap, &delay), &gain), &share), route), read) in self
                .taps
                .iter()
                .zip(&t.tap_delays)
                .zip(&t.tap_gains)
                .zip(&t.feed_gains)
                .zip(&self.routes)
                .zip(&mut reads)
            {
                let &Route::Own(voice) = route else { continue };
                let at = predelay + delay + spin.get(voice).copied().unwrap_or(0.0) * (predelay + delay);
                *read = read_from(&self.input, at);
                let sides = if tap.filtered { &mut filtered } else { &mut direct };
                if let Some(side) = sides.get_mut(tap.side) {
                    *side += *read * gain;
                }
                if !tap.filtered && measured {
                    // Read where the reflection is, and delayed by the entry below, so the network
                    // hears the shift Spin gave the reflection, not the one it has an entry later.
                    // At whole frames the two reads still add to the entry and delay rounded once.
                    let from = if whole { at + t.entry - entry_whole } else { at };
                    let u = read_from(&self.feed_line, from) * share * feed * t.measured_feed;
                    if let Some(group) = measured::TAP_GROUPS.get(direct_index).and_then(|&g| groups.get_mut(g)) {
                        *group += u;
                    }
                    direct_index += 1;
                } else if !tap.filtered {
                    let u =
                        read_from(&self.feed_line, at + t.shape_delay) * share * feed * t.feed / (spread as f64).sqrt();
                    for j in 0..spread {
                        if let Some(input) = inputs.get_mut((next_line + j * active / spread) % active) {
                            *input += u;
                        }
                    }
                    next_line = (next_line + 1) % active;
                    spread = 1;
                }
            }
            let mut mixes = [0.0f64; FILTERED_OFFSETS.len()];
            for (route, &gain) in self.routes.iter().zip(&t.tap_gains).take(self.taps.len()) {
                if let &Route::Follows { source, offset } = route
                    && let Some(mix) = mixes.get_mut(offset)
                {
                    *mix += reads.get(source).copied().unwrap_or(0.0) * gain;
                }
            }
            let mut copies = [0.0f64; measured::COPIES];
            for ((((line, mix), &offset), &(frames, side)), _) in self
                .offset_lines
                .iter_mut()
                .zip(mixes)
                .zip(&t.offsets)
                .zip(&FILTERED_OFFSETS)
                .zip(&self.offsets_used)
                .filter(|(_, used)| **used)
            {
                let y = if self.moving { line.read_hermite(offset) } else { line.read(offset.round() as usize) };
                if let Some(out) = filtered.get_mut(side) {
                    *out += f64::from(y);
                }
                if let Some(copy) = measured::COPY_OFFSETS.iter().position(|o| o.contains(&frames)) {
                    copies[copy] += f64::from(y);
                }
                line.push(mix as f32);
            }
            let [left_filter, right_filter, copy_filters @ ..] = &mut self.reflection_filters;
            let early = [direct[0] + left_filter.process(filtered[0]), direct[1] + right_filter.process(filtered[1])];
            if measured {
                for ((group, filter), copy) in groups[measured::GROUPS..].iter_mut().zip(copy_filters).zip(copies) {
                    *group = filter.process(copy) * feed * t.copy_feed;
                }
            }
            for (group, line) in groups.iter_mut().zip(&mut self.group_lines) {
                let late = if whole { line.read(entry_whole as usize) } else { line.read_hermite(t.entry) };
                line.push(*group as f32);
                *group = f64::from(late);
            }

            // The network. Without Chorus the swing is zero whatever the phase.
            let (chorus_sin, chorus_cos) =
                if chorus_swing > 0.0 { (std::f64::consts::TAU * self.chorus_phase).sin_cos() } else { (0.0, 0.0) };
            let mut tail = [0.0f64; 2];
            if measured {
                let offsets = &self.chorus_offsets;
                let swing = |k: usize, length: f64| {
                    let (c, s) = offsets.get(k).copied().unwrap_or((0.0, 0.0));
                    chorus_swing.min(0.5 * length) * (chorus_sin * c + chorus_cos * s)
                };
                tail = self.measured.process(&groups, &t.measured, diffusion, &damping, swing, frozen);
            } else {
                let mut damped = [0.0f64; 16];
                for (((((line, slot), &length), &spread), &gain), &(c, s)) in self
                    .lines
                    .iter_mut()
                    .zip(&mut damped)
                    .zip(&t.lines)
                    .zip(&t.diffusers)
                    .zip(&t.gains)
                    .zip(&self.chorus_offsets)
                    .take(active)
                {
                    let swing = chorus_swing.min(0.5 * length) * (chorus_sin * c + chorus_cos * s);
                    let y =
                        f64::from(line.diffuser.process(line.delay.read_hermite(length + swing), spread, diffusion));
                    *slot = line.damper.damp(y, &damping) * gain;
                }
                // The tail is heard after each line's gain and damping, so even a line's first arrival
                // has decayed for as long as the line is.
                let outs = damped;
                mix_lines(&mut damped, active);
                for (k, ((((line, &y), &d), &weight), &u)) in self
                    .lines
                    .iter_mut()
                    .zip(&outs)
                    .zip(&damped)
                    .zip(&self.weights)
                    .zip(&inputs)
                    .take(active)
                    .enumerate()
                {
                    let mut next = d + u * weight;
                    if frozen {
                        next = limit(next);
                    }
                    line.delay.push(next as f32);
                    if let Some(out) = tail.get_mut(side_of(k)) {
                        *out += OUTPUT_SIGNS.get(k).copied().unwrap_or(0.0) * y;
                    }
                }
            }
            self.input.push(x as f32);
            self.feed_line.push(self.feed_cut.process(self.input_damper.damp(x, &damping)) as f32);

            // Output: the two parts, Stereo's rotation, then the mix.
            let wet = [reflect * early[0] + diffuse * norm * tail[0], reflect * early[1] + diffuse * norm * tail[1]];
            if (angle, mix) != output_key {
                output_key = (angle, mix);
                rotation = angle.sin_cos();
                let (wet_gain, dry_gain) = (mix * std::f64::consts::FRAC_PI_2).sin_cos();
                gains = (dry_gain, wet_gain);
            }
            let ((sin, cos), (dry_gain, wet_gain)) = (rotation, gains);
            let wet = [cos * wet[0] + sin * wet[1], sin * wet[0] + cos * wet[1]];
            for ((sample, dry), wet) in frame.iter_mut().zip(input).zip(wet) {
                *sample = (dry * dry_gain + wet * wet_gain) as f32;
            }

            for (phase, (_, rate, _)) in self.spin_phases.iter_mut().zip(SPIN_VOICES) {
                *phase = (*phase + spin_step * rate).fract();
            }
            self.chorus_phase = (self.chorus_phase + chorus_step).fract();
            self.frame = self.frame.wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod golden_tests;
#[cfg(test)]
mod tests;
