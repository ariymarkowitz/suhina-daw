---
description: The built-in algorithmic reverb - its parameters, signal flow (input band, early reflections, a measured feedback delay network per Density, chorus, freeze, stereo), how Live's Reverb imports into it, and how close it comes to Live's renders
read_when: reverb, algorithmic reverb, room, decay, early reflections, predelay, feedback delay network, FDN, damping, freeze, diffusion, importing Live's Reverb, Reverb elements
type: spec
status: current
last_verified: 2026-10-30
covers:
  - devices/reverb/**
  - crates/dsp/src/first_order.rs
---

# Reverb

Suhina's `reverb` (`devices/reverb`): an algorithmic room reverb with an input band, early
reflections, a feedback delay network for the late tail with damping in the loop, a chorus on the
network, freeze and a stereo width. Live's Reverb imports into it parameter for parameter
([`device-import.md`](device-import.md)), and the device is built to sound like Live's: its tap
patterns, network lengths and filter laws are read from Live's renders, not designed.

- **Observed with:** Live 12.4.6 at 44.1 kHz. The parameters come from a parameter dump and a saved
  set; the sound from renders of many hundreds of cases (impulses, noise, tones and clicks) analysed
  by the measurement harness of the Suhina device notes (`live-12/audio-effects/29.32-reverb.md`
  and `measurements/29.32-reverb/` in the device-specs repository, which hold the cases, the fits
  and the findings behind every constant here).
- **Sources:** Live 12 Reference Manual, 29.32 Reverb (what each control is for); the measurements
  above (how it sounds). Constants in the code are those measurements, named in their doc comments.

## Parameters

Values are plain values in their unit (ADR 0004). Ids are stable.

| # | Id | Name | Range | Default | Unit, scale |
|---|---|---|---|---|---|
| 0 | `predelay` | Predelay | 0.5 – 250 ms | 2.5 ms | seconds, logarithmic |
| 1 | `lowCut` | Low Cut | off/on | off | toggle |
| 2 | `highCut` | High Cut | off/on | on | toggle |
| 3 | `inputFrequency` | Input Frequency | 50 Hz – 18 kHz | 830 Hz | hertz, logarithmic |
| 4 | `inputWidth` | Input Width | 0.5 – 9 octaves | 5.85 | octaves, linear |
| 5 | `spin` | Spin | off/on | on | toggle |
| 6 | `spinRate` | Spin Rate | 0.07 – 1.3 Hz | 0.3 Hz | hertz, logarithmic |
| 7 | `spinAmount` | Spin Amount | 2 – 55 | 17.5 | plain number, logarithmic |
| 8 | `shape` | Shape | 0 – 1 | 0.5 | plain number, linear |
| 9 | `highDamping` | High Damping | off/on | on | toggle |
| 10 | `highDampingType` | High Damping Type | Shelf, Low-Pass | Shelf | choice |
| 11 | `highDampingFrequency` | High Damping Frequency | 20 Hz – 16 kHz | 4.5 kHz | hertz, logarithmic |
| 12 | `highDampingGain` | High Damping Gain | 0.2 – 1 | 0.7 | linear factor |
| 13 | `lowDamping` | Low Damping | off/on | off | toggle |
| 14 | `lowDampingFrequency` | Low Damping Frequency | 20 Hz – 15 kHz | 90 Hz | hertz, logarithmic |
| 15 | `lowDampingGain` | Low Damping Gain | 0.2 – 1 | 0.75 | linear factor |
| 16 | `diffusion` | Diffusion | 0.1 – 96 % | 60 % | percent, linear |
| 17 | `scale` | Scale | 5 – 100 % | 40 % | percent, linear |
| 18 | `chorus` | Chorus | off/on | on | toggle |
| 19 | `chorusRate` | Chorus Rate | 0.01 – 8 Hz | 0.02 Hz | hertz, logarithmic |
| 20 | `chorusAmount` | Chorus Amount | 0.01 – 4 | 0.02 | plain number, linear |
| 21 | `size` | Size | 0.222 – 500 (shown as 0.22) | 100 | plain number (a room volume), logarithmic |
| 22 | `smooth` | Smooth | None, Slow, Fast | Fast | choice |
| 23 | `decay` | Decay | 0.2 – 60 s | 1.2 s | seconds, logarithmic |
| 24 | `freeze` | Freeze | off/on | off | toggle |
| 25 | `flat` | Flat | off/on | on | toggle |
| 26 | `cut` | Cut | off/on | on | toggle |
| 27 | `stereo` | Stereo | 0 – 120 | 100 | plain number (degrees), linear |
| 28 | `density` | Density | Sparse, Low, Mid, High | Low | choice |
| 29 | `reflect` | Reflect | −30.46 – +6 dB | 0 dB | decibels, linear |
| 30 | `diffuse` | Diffuse | −30.46 – +6 dB | 0 dB | decibels, linear |
| 31 | `mix` | Dry/Wet | 0 – 100 % | 55 % | percent, linear |

Ranges, defaults and step lists are Live's (the defaults are its factory preset). Reflect and
Diffuse go down to a linear gain of 0.03, which Live shows as −30 dB.

## Behaviour

```text
in -> mono sum -> roll-off -> Low Cut -> High Cut -> predelay line -+-> reflection taps (Spin) --- Reflect -+
                                                                    |                                     +-> Stereo -> Dry/Wet
                                                                    +-> direct taps -> Shape entry -> network --- Diffuse -+
                                                                        (lines, allpasses, damping, Chorus, Freeze)
```

**Everything scales with the room.** The *size factor* is `(Size / 100)^(1/3)`, as for a room's
volume; Size 100 is the *reference room*, in which every delay below is given. Delays are quoted in
frames of 44.1 kHz and convert to the session rate.

### Input

- **Mono.** Both sides are summed, `(L + R) / 2`: Live's Reverb gives the same output for a
  left-only and a right-only input.
- **Roll-off.** The input always passes first-order high-passes at 127 Hz and 19 Hz.
- **Input band.** Low Cut is a first-order high-pass at 2.5 times the band's lower edge,
  `frequency / 2^(width/2)`, plus two first-order low shelves at 25 and 250 Hz whose gains follow the
  edge (see Low Cut's law below). It does nothing with the edge below 44.2 Hz. High Cut is a
  12 dB/octave Butterworth low-pass at `frequency × 2^(width/2)`, never above 18 kHz, with no
  delay beyond the filter's own. With High Cut off the input still passes the same low-pass at
  18 kHz, as Live's does (left out at sample rates where 18 kHz is above 0.45 of the
  rate). Both filters always run and their switches crossfade.
- **Predelay** delays the wet signal. The wet signal, reflections and tail alike, is also quieter
  by `0.0113 / decay` dB per ms of predelay (decay in seconds).

### Size and Smooth

Every delay of the reverb (taps, lines, allpasses) is its reference-room length times the size
factor. **Smooth** glides the factor with a time constant of 45 ms (Slow) or 15 ms (Fast), or not at
all (None), where Size changes glitch as in Live. However fast the glide, no line or reflection
changes by more than half a frame a frame: faster, a line is read backwards and buzzes.

### Early reflections

- **Taps.** Each Density has Live's own pattern, read tap by tap from its impulse responses: 4 direct
  taps at Sparse, 10 at Low, Mid and High (the same ten times), and filtered taps up to 130 ms
  (20 taps at Sparse, 71 at Low, 90 at Mid, 108 at High). Every tap has a time (a whole number of
  frames plus the fraction of a frame at which Live rounds it), a side and a level. The first tap
  arrives one frame before the predelay at a gain of 0.0942. The predelay and a tap's time round to
  a frame together.
- **Stereo's delay.** Below Stereo 120 the reflections heard, not the tail and not the network's
  input, come later: `floor(27.5625 × (1 − stereo / 120))` frames (0.625 ms at most) under Smooth
  Fast and Slow, rounded up to a whole 8-frame block under None (4 and 8 frames at the default 100).
- **Level.** Every tap is `0.92594 + 0.00074059 × Size` of its reference level (0.926 at the
  smallest Size, 1.296 at 500), and fades by `(0.0328 + 0.0668 × Shape) / decay` dB per ms of its
  delay.
- **Filtered taps** pass a cookbook low-pass of Q 0.5 per side. Its double pole is that of
  11.98 kHz plus `1.115 × (1 − exp(−t × size factor / decay))`, with `t` 0.0512 s at Low, 0.2092 at
  Sparse, 0.3200 at Mid and 0.2657 at High: the corner falls with a shorter Decay faster the denser
  the reflections (at Size 100, Low 8.5 kHz at 0.2 s, 11.3 at 1.2 s; Mid 1.4 and 8.4 kHz), and a
  smaller room's reflections are brighter (Low at 0.2 s: 10.2 kHz at Size 10.54, 11.0 at 1.53).
  The Decays were read at Size 100, the Sizes at Low (`reverb-size`, `reverb-smooth-slow`). Every filtered tap is a
  direct tap a whole number of frames later (20 offsets in pairs two frames apart, 395 to 3192
  frames, 9 to 72 ms), always on one side per pair. So they are played as one mix of the direct
  taps per offset, delayed by the offset. The ten pairs are ten *families*, the same at every
  Density (Sparse, Mid and High hear Low's six, and Mid and High four more). A family's peak is
  `8 × floor(n × size factor + 0.7) + 1` frames after its direct tap under None, `floor(8 × n' ×
  size factor) + 1` under Fast and Slow, with its own n and n' (blocks): under Fast it comes 0 to 6
  frames sooner.
- **Spin** swings each direct tap's whole delay `T` (predelay included) by
  `0.00995 × amount × reach(predelay) × T × sin(2π × m × rate × t + φ)`, where `reach` is 1 up to 5 ms
  of predelay and `2 / (predelay − 3.02)` above it (predelay in ms), so a long predelay swings every
  tap by about the same 2 ms × 0.00995 × amount. Each of the ten direct taps has its own LFO, rate
  `m` times Spin Rate (0.978 to 1.042) and a phase that restarts with the transport. A filtered tap
  swings as its direct tap did when it was read, one offset earlier. The tail's input swings with
  the reflections it comes from. Moving reflections are read by linear interpolation, as Live's
  (its reflections lose 0.4 dB at 6 kHz to 1.6 dB at 15 kHz as soon as Spin moves them); static
  ones are read at whole frames.

### Late reverb: the measured network

Live's network, identified from its renders, with a design per Density. A **line** is a delay and
a Schroeder allpass (Diffusion as its gain), two in series at Mid and High; lines are mixed after every trip by one matrix,
orthogonal between the lines as they are heard, and each line has its own mix of the direct taps as input.

| Density | Lines | Direct taps feed | Sides (left/right) |
|---|---|---|---|
| Sparse | 4 (X, Y, Z, W) | one line each | X, Y / Z, W |
| Low | 6 (A, B, C, D, F, R) | six groups of taps | B, D, F / A, C, R |
| Mid | 8 | eight groups | four each |
| High | 10 | ten taps | five each |

- **Input.** The direct taps are read after the **Shape entry**, `17.574 + s × (Shape − 0.5) × size
  factor` ms (s 35.19 below Shape 0.5 and 34.83 above), at their reflection levels and fades, so
  Shape and Decay shape the tail's onset as they shape the reflections. The network is mono: it
  hears no side of the input, and the input passes no damping. The level is
  `1.236 × 0.2383 × (Size level)²` times each line's input weight, where *Size level* is the
  reflections' `0.92594 + 0.00074059 × Size` above. Each Density's inputs and filtered-reflection
  copies are scaled to Live's levels (Sparse inputs ×0.9965, copies ×1.79; Mid and High inputs
  ×1.057, copies ×1.9).
- **Families.** Each line's *family* of filtered reflections (a mix of the direct taps, one per
  line, 401 to 3193 frames after them) is heard again, low-passed like the reflections, as part of
  that line's output into the mix.
- **Line levels.** Sparse's, Mid's and High's lines are heard at levels up to 1 % apart (the line
  with A's delay, and High's 2073 family's, the quietest: 0.992 to 0.996 against the others' 0.999
  to 1.004; Low's at one level). Their inputs and copies were read with these levels in them, so the device sums
  every line at one level and runs the mix between them as `level_i × mix_ij / level_j`, orthogonal
  between the lines taken at their levels.
- **Diffusion** is every Density's allpass gain, Sparse's too: at Diffusion 0.1 % a fifth of it
  leaves Sparse's tail 5 dB further from Live's at 1 to 3 s (Size 0.222), and at the default 60 %
  Live's first 0.3 s matches to −10 to −12 dB with the whole setting, against −3 to +0.5 with a fifth.
- **Lengths.** A line's delay and allpass are whole numbers of 8-frame blocks plus a frame (Live
  builds them so). At Low: the delay is `8 × floor(y × size factor + 0.85) + 1` frames, the allpass
  `8 × floor(c × Scale × size factor + 0.9) + 1` (Scale in %), with y 287.0993, 340.0779, 628.7004,
  459.9281, 846.6372 and 1220.5399 and c 0.63476, 1.22412, 0.87055, 3.97956, 2.18481 and 0.80455 for
  A, B, C, D, F and R, the same numbers as Smooth Fast's (below), rounded to blocks; each is the
  middle of what both settings' reads at Size 0.222 to 500 allow. C's delay is its family's offset
  and a plain delay, and R's is A's delay and a delay G, but each rounds as one length: at Size
  0.222 rounding the parts on their own makes C a block longer and R a block shorter, which takes
  the tail's waveform apart from Live's after 20 ms. At the
  default Scale in the reference room A, B, D and F are 57, 71, 113 and 170 ms, C 1640 frames then
  83 ms, R 228 ms. Sparse's, Mid's and High's lines follow the same law, from one table by family
  (the family of the line's copy): a delay y and two allpasses of c₁ and c₂ blocks per Scale %, each
  rounded on its own at the same edges (0.85 and 0.9) and a frame. A Mid or High line of family f
  is f's delay and both of f's allpasses; High has all ten families, Mid all but 7 and 8. A Sparse
  line has one allpass, its family's shorter: F, B and R are their families', and the C line has A's
  delay with C's allpass. y, c₁ and c₂ are A 1184.4905, 0.6886, 12.7517; B 442.6705, 1.1045, 3.7123;
  C 436.2265, 1.3804, 5.9511; D 1026.769, 1.7387, 3.4422; F 356.0745, 2.4623, 5.7064; R 560.7845,
  0.7399, 8.3358; and 6 to 9 621.4635, 2.9624, 3.7537; 1983.288, 0.7649, 2.6864; 379.8665, 0.8037,
  5.6726; 501.207, 1.183, 3.8067: each fits every line's first pass at Size 0.222, 100 and 500 with
  Smooth None and Fast and at Scale 0 to 100 % at Size 100 (but Sparse's F line at Scale 0, read a
  block long). Which allpass is the shorter shows only above Diffusion 0.1 % (Diffusion 60 %,
  where each first pass arrives as g² at the delay, −g(1 − g²) after each allpass and (1 − g²)²
  after both). These lines' lengths are not Low's. While
  Size or Scale move, each length follows the straight line through the middle of its steps.
- **Reading.** A and B are a hundredth of a frame longer, read by linear interpolation, which is
  all the highs the network loses on a pass; the other lines are whole frames and are not
  interpolated. So are Sparse's, Mid's and High's lines of families A and B under Smooth None
  (Sparse's C line, with A's delay, is whole): without it their tails keep up to 5 dB too much
  above 12 kHz by 3 s at Size 0.222, and no other hundredth fits as well. Lines with Chorus are read with cubic (Hermite) interpolation.
- **Smooth Fast and Slow** (Slow renders as Fast) change Low's lengths: none keeps to 8-frame
  blocks. Each length is its continuous value rounded down to a frame, and a frame: the allpass
  `floor(8 × c × Scale × size factor) + 1`, the delay `floor(8 × y × size factor) + 1`, but A's and
  B's delays keep their fraction of a frame (`8 × y × size factor + 1`, read by linear
  interpolation, most of the highs lost). The loops are 5 frames longer than their first passes and
  the input reaches them 5 frames sooner. The other Densities' lines do the same with their own
  lengths: whole frames, but the A and B lines' delays keep their fraction (Mid's and High's A and
  B lines, and Sparse's B line), and the input 5 frames sooner.
- **Decay.** The network is tuned to a T60 of 0.9 of the Decay setting. Every path fades with its
  whole time: each line's gain covers its delay, and its allpass loses its own delay's gain on every
  pass (`y = a·w[n−L] − g·w`, `w = x + g·a·w[n−L]`). The mix is made exactly orthogonal between the
  lines at their levels, so nothing is lost on a pass but the decay: the late tail falls by the same
  dB per second at every Size.

### Damping

In each line, after its gain, with the loss specified per second so it does not depend on Size:

- **High Damping, Shelf:** a second-order high shelf (Q 0.70) half way at 0.98 of the frequency.
  **Low Damping:** a second-order low shelf (Q 0.70) half way at the frequency over 0.98.
  The Gain is a linear factor per 0.17 s trip, raised to the power of the line's length plus its
  allpass's, in seconds, over 0.17 s. Sparse's shelves lose 1.37 times as much.
- **High Damping, Low-Pass:** one whole one-pole low-pass `y += (1 − p)(x − y)` with
  `p = e^(−2π × frequency / sample rate)` on every pass through every line, whatever its length;
  Gain does nothing with it, in Live's renders as here.

### Chorus

Swings every line's delay either way by up to `0.38 ms × amount` (at most half the line), each line
with its own phase of one LFO at the Chorus rate. Each side's lines are spread evenly round the
cycle, the right's half a step after the left's, so their Doppler shifts cancel in what the side
hears.

### Freeze

The loop's gain is set so the tail does not fall at all with Flat on, and 0.03 dB/s with Flat off
(Live's frozen tail falls 0.25 and 0.4 dB/s in all, nearly all of it from A's and B's fractions of a
frame and the high bands). With Flat on the damping is left out while frozen; with Flat off it
stays in. Cut decides whether new input still enters the network (it fades out over 20 ms, the
reflections with it); if it does, levels above 2 are compressed toward 4, so the tail cannot grow
without end. The early reflections go on.

### Output

Each side of the tail is the sum of the lines assigned to it, at one level. The reflections (times
Reflect) and the tail (times Diffuse) are added and rotated by `β = 45° × (1 − stereo / 120)`:
`L' = cos β L + sin β R`, `R' = sin β L + cos β R`. So 120 keeps the sides apart, 100 gives a
correlation of about 0.25 (Live's 0.25) and 0 makes them mono. Dry/Wet is an equal-power crossfade,
`dry × cos(mix·π/2) + wet × sin(mix·π/2)`. The dry signal is not delayed; the device adds no
latency.

### Changes while playing

Levels, mix, Stereo, every switch, the Spin and Chorus swings, Diffusion, Shape, Scale, Decay, the
input band and the damping frequencies and gains ramp over 20 ms; Predelay over 100 ms (a long jump
bends the pitch on the way). Delay lengths and gains are recomputed every 32 frames and
interpolated frame by frame in between. Switching Density starts or stops lines at once and the new
lines start from silence, with no crossfade: this was listened to in the app (2026-10-18) and no
click could be heard.

## Import

Live's Reverb is the element `Reverb`; every value is `<Element>/Manual@Value`
([`als-format.md`](als-format.md) › Devices). `crates/als/src/devices.rs` maps it:

| Element | Stored | Suhina |
|---|---|---|
| `On` | true/false | the device's on/off |
| `PreDelay`, `DecayTime` | ms | `predelay`, `decay` (seconds) |
| `BandHighOn`, `BandLowOn` | true/false | `lowCut`, `highCut`: the names are the other way round |
| `BandFreq`, `BandWidth` | Hz, octaves | `inputFrequency`, `inputWidth` |
| `SpinOn`, `EarlyReflectModFreq`, `EarlyReflectModDepth` | flag, Hz, 2 … 55 | `spin`, `spinRate`, `spinAmount` |
| `DiffuseDelay` | 0 … 1 | `shape` |
| `ShelfHighOn`, `HighFilterType`, `ShelfHiFreq`, `ShelfHiGain` | flag, 0 Shelf / 1 Lowpass, Hz, 0.2 … 1 | `highDamping`, `highDampingType`, `highDampingFrequency`, `highDampingGain` |
| `ShelfLowOn`, `ShelfLoFreq`, `ShelfLoGain` | flag, Hz, 0.2 … 1 | `lowDamping`, `lowDampingFrequency`, `lowDampingGain` |
| `AllPassGain`, `AllPassSize` | 0.001 … 0.96, 0.05 … 1 | `diffusion`, `scale` |
| `ChorusOn`, `SizeModFreq`, `SizeModDepth` | flag, Hz, 0.01 … 4 | `chorus`, `chorusRate`, `chorusAmount` |
| `RoomSize`, `SizeSmoothing` | 0.222 … 500, 0 None / 1 Slow / 2 Fast | `size`, `smooth` |
| `FreezeOn`, `FlatOn`, `CutOn` | true/false | `freeze`, `flat`, `cut` |
| `StereoSeparation` | 0 … 120 | `stereo` |
| `RoomType` | 0 Sparse … 3 High | `density` |
| `MixReflect`, `MixDiffuse` | linear gain 0.03 … 1.995 | `reflect`, `diffuse`: 20·log10 of it, dB |
| `MixDirect` | 0 … 1 | `mix` |

A stepped value outside its list is left at the default with a warning. Sets saved before Live
11.2 named some of these differently; only 12.4.6's names are read.

## Design notes

Why the device is built as it is.

- **Live's network, read from its renders, over a textbook feedback delay network.** A designed
  network (Householder or Hadamard mixing, one allpass per line) reached Live's class of tail but
  could not match its first arrivals, which are as strong as the reflections, nor the texture of its
  onset: Live's echoes stay distinct and sparse for hundreds of milliseconds, where a Hadamard
  network turns to noise in two trips. Impulse responses with Diffusion at its minimum (an allpass
  is then a plain delay), Decay at its maximum and several Scales show each arrival as one sample
  once the input's fixed filters are divided out, and tell lines, allpasses, mix and outputs apart.
  Cost: the constants are fitted values, not derivations; they are exact where read (Size 100 for
  Sparse, Mid and High) and extrapolated by the same laws elsewhere.
- **Early reflections are Live's tap pattern.** Every tap is a delay of the input rounded to a whole
  frame, either as it is or through one low-pass, and times scale with the cube root of Size. Used
  as read, the taps of the reflection-only cases land within 0.5 ms and 0.5 dB of Live's. The
  fraction of a frame is kept because Live steps each tap to the next frame at its own fraction as
  the predelay moves; without it four taps were a frame late at most predelays.
- **Filtered taps are played as delayed mixes of the direct ones.** Reading all 178 taps of High
  on its own cost 6.9 % of a core; one mix per offset into twenty lines read at whole frames costs
  4.7 %, and is exactly what Live does (a filtered tap is its direct tap delayed again).
- **Lengths in whole 8-frame blocks, without interpolation once settled.** Live builds every length
  from blocks, and fractional delays in the loop cut the 8 kHz octave's decay time by 10 to 38 %.
  While Size or Scale move, the straight line through the middle of the steps avoids a jump of a
  block at a time.
- **Nothing is lost on a pass but the decay.** Live's tail falls a fixed dB per second at every
  frequency the damping does not touch, at every Size. A loss per pass cannot be told apart from a
  loss per second at one Size, but at Size 0.222 it let the tail grow without end. The network's mix
  is exactly orthogonal between the lines at their levels, every path fades with its whole time (an allpass holds echoes back for
  several of its delays, so it loses its own), and the one real loss of the highs is A's and B's
  hundredth of a frame, read by linear interpolation. This reproduces Live's tail in every band to
  within 0.1 dB per second from Size 1.53 to 100.
- **Damping is a gain per 0.17 s trip, applied in proportion to each line's length.** The loop
  length is the one that fits every band Live was measured at, and it makes the loss per second
  independent of Size, as Live's top octaves are. Shelves are second-order: first-order ones had
  to sit off the set frequency and were 13 to 28 % off in octave decay time. The Low-Pass is a
  whole one-pole per pass, because a one-pole once a trip at the set frequency fits Live's decay
  per third-octave where an analog first-order low-pass needs separate corners. The input passes no
  damping: Live's lines each have their own and the input none (with an input pass, the first
  arrivals were 3 dB too dark).
- **Low Cut is a roll-off plus two shelves.** Live's Lo Cut has a first-order corner at 2.33 to 2.5
  times its band edge, and below it a gentle low shelf whose gain falls from +10.6 dB at 25 Hz for an
  edge of 132 Hz to −12.4 dB at 250 Hz for 841 Hz. Two shelves at 25 and 250 Hz take gains (dB) of
  (10.57, 1.17), (7.09, 1.30), (4.60, 0.71), (2.65, −3.49), (4.38, −7.11) and (7.58, −12.42) at edges
  of 132, 192, 264, 500, 658 and 841 Hz, joined linearly against the edge's logarithm and held
  beyond; they match Live's six edges to within 0.9 dB in every third-octave band. These are fitted
  values, not a law, and Live's Lo Cut does nothing for edges below about 44 Hz.
- **The 127 Hz and 19 Hz roll-off** is the shape of every reflection in Live's renders with both
  cuts off; the second brings the lowest band (32 Hz) within 0.2 dB.
- **Spin swings each direct tap's whole delay, by a share that falls with predelay.** Tracked in
  Live's renders of clicks at Predelay 0.5 to 250 ms, Amount 17.5 and 55, three Rates and three
  Sizes, every tap follows a sine of peak `0.00995 × amount × reach × T`, whatever the Rate, Amount
  and Size. Swinging only the delay after the predelay left the first tap still and over-swung the
  late ones at long predelays; swinging each tap by its own delay overstated the tone's FM 2.6 times
  at the default. Each tap has its own LFO as tracked in Live's renders, and a filtered tap moves as
  its direct tap did when it was read, because it is that tap, delayed. The network's input moves
  with its reflections, because Live's tail with Spin is uncorrelated with its tail without.
- **Linear interpolation for Spin, cubic for the lines.** Live reads Spin's reflections linearly, so
  this device does too. The lines move with Chorus, where linear interpolation would dull and
  modulate the highs for a saving of about 0.8 % of a core.
- **Chorus swings by 0.38 ms per unit of Amount**, fitted so a 1 kHz tone through the tail swings as
  far as through Live's (12 to 343 cents at five Amounts and Rates; Suhina 12 to 397). The loop adds
  each trip's shift to the last, so reading the swing as one line's gave 1.8 ms and a hiss. Evenly
  spread phases per side cancel Doppler shifts; golden-angle phases left a side's lines within a
  quarter cycle and rippled twice as much.
- **Decay is tuned to 0.9 of the setting**, which is what Live's renders show (0.87 to 0.9 from 0.8
  to 3.5 s). With every path fading with its whole time, the tail needs no level correction for
  short decays, and every window of the Decay cases is within 1.7 dB of Live's from 60 ms on.
- **The reflections' low-pass follows Decay by Density.** The corner of the copies was solved from
  Live's reflections alone at every Density and Decay 0.2 to 60 s: a cookbook low-pass of Q 0.50
  whose pole follows the law above (15 Hz rms over all 24 cases). High Cut does not change it.
- **Level follows Size** as `0.92594 + 0.00074059 × Size`, a pure gain on the reference
  response read at Sizes 0.222 to 500, and the network's input takes it twice. This one law fits all
  four Densities.
- **Stereo, not High Cut, delays the reflections.** The 8 frames once read as High Cut's were the
  default Stereo's delay under None; with it on the reflections alone and the tail left where it
  was, every Decay case lines up with Live's to the frame. Live's reflections are one frame before
  the predelay, not after it.
- **Two rounding rules, one set of lengths.** Under None every length is whole 8-frame blocks and a
  frame, under Fast and Slow whole frames, but both come from the same numbers: one y and c per
  line and two numbers per family fit both settings' reads from Size 0.222 to 500, and the
  families are the same at every Density. Reads at Size 500 and 0.222 settled lengths that the
  reads at 1.53 to 100 had left a frame or a block either way.
- **Smooth's glide times** (45 ms Slow, 15 ms Fast) are what one point of Live's glide gives for an
  exponential glide (50 ms after a jump Fast has finished, Slow has 30 to 40 % left); the curve
  itself was not measured. The half-frame slew limit stops lines being read backwards.
- **Freeze, Flat and Cut** follow Live's renders per band: with Flat on the frozen tail falls 0.1 dB/s
  at 1 kHz and up to 1.6 dB/s at 16 kHz; with Flat off the damping stays in and drains the shelves'
  bands as fast as the preset's filters do. The soft limit with Cut off is Suhina's.
- **Switching Density has no crossfade**: it was listened to, and no click could be heard.

## How close it is to Live

Measured against Live's renders of the same settings, imported and rendered by `suhina-cli render` at
44.1 kHz:

- **Early reflections:** every tap within 0.5 ms and 0.5 dB at every Density, the first 140 ms of
  the reflections alone within −35 to −42 dB of Live's energy at Size 100 and −29 to −43 dB at
  Size 0.222 and 500 (Low; every Density's first 200 ms at 500), −38 to −39 dB over the first
  80 ms at Size 1.53 and 10.54 (Low, at Decay 0.2 and 1.2 s and every Diffusion and Scale), and the level within 0.1
  dB in every octave at every Decay.
- **The network (Low):** waveform within −39 to −41 dB of Live's over the first 0.5 s and −35 dB
  from 1 to 2 s at every Scale at Size 100, −33 to −38 dB at Sizes 1.53 to 50. Energy in windows to
  4.8 s is within 0.5 dB in 941 of 1,084 cases (median 0.10 dB), and the decay time within 10 % in
  every band to 8 kHz.
- **Sparse, Mid and High:** waveform within −36 to −43 dB at Size 100 from 0.08 to 3.5 s at Scale 5,
  40 and 100 % (Diffusion 0.1 %, Decay 60 s), and level within 0.25 dB over 0.15 to 4 s. At Size
  0.222, Smooth None, −35 to −39 dB from 0.08 to 8 s; under Fast −36 to −38 dB to 0.3 s and −26 to
  −33 dB at 3 to 8 s. At Size 500 about −23 dB, where what is left is level (0.6 dB loud, as
  Low's): with that taken out, −37 to −43 dB to 8 s under None.

### What is further off

- **Smooth Fast at Size 500.** Every Density's tail, Low's too, drifts from Live's under Fast at
  Size 500 (with the level taken out, −32 to −37 dB over 0.08 to 0.3 s and −20 to −26 dB at 3 to
  8 s, against −40 to −44 and −38 to −43 under None), and Sparse's under Fast at Size 0.222 (−26 dB
  at 3 to 8 s); the mix and the line levels do not change it.
- **Sparse, Mid and High away from the defaults.** Their damping at Decay 1 s was not read: at the
  default damping the 8 to 16 kHz bands of Mid and High decay 1.4 to 1.6 dB/s too slowly.
- **Low beyond the settings read.** Lengths and levels were read at Size 0.222 to 100, Diffusion 0.1
  to 60 % and Decay 60 and 0.83 s; at Size 500 the waveform is within −24 dB, and the remaining
  difference is level (0.5 dB loud). Under Smooth Fast the entry was read at Shape 0.5 only.
- **Stereo's delay** was read at 44.1 kHz only; whether it is 0.625 ms or 27.5625 frames at other
  rates, and how it moves while Stereo is automated, were not read.
- **Low Cut** is extrapolated for edges between 44.2 and 132 Hz and above 841 Hz.
- **Spin.** Tap by tap the swing is within 0.02 ms of Live's. Live's moving reflections carry a flat
  floor of noise from 16 to 22 kHz that follows Spin's speed (−50 dB at the default to −29 dB at the
  extreme, against 1 kHz), which this device's reads do not make, so above 12 kHz Live's highs vary
  less at Spin's rate (0.4 to 1.3 dB against 2.3 to 4.7 at Amount 55 and 1.3 Hz). At Spin Rate 1.3
  Hz Live's LFOs run 0.6 % faster than at 0.3.
- **Echo texture.** The tail's echoes thicken differently from Live's: kurtosis 26 from 100 to 150 ms
  and 5.8 from 150 to 200 against Live's 15 and 10.5, and about four fifths of Live's strong
  clicks from 50 to 250 ms.
- **Smooth None against Fast at Decay 3.46 s** (Sparse, Mid, High): Live's two settings differ by up
  to 2 dB by 2.4 s; this device follows Fast, the preset's setting, there.
- **Long and small-room highs** (Sparse, Mid, High): a fixed shelf per trip cannot follow Live's
  highs, which fall fastest in the first seconds and more slowly after.

## Cost

One instance at its defaults: 2.8 to 3.2 % of one core at 48 kHz (60 s rendered in 1.7 to 1.9 s,
release build, Apple M-series, 2026-10-30), over the 1 % budget as the device notes allow for
reverbs: ten direct taps, up to twenty offset lines for the filtered taps, and 4 to 10 modulated
lines with one allpass (two at Mid and High) and damping each, every frame. Density Sparse costs 2.3 %, and 2.4 % with Spin
off. Run `cargo test --release -p suhina-device-reverb cost -- --ignored --nocapture` to
measure it.

## Tests

`devices/reverb/src/tests.rs`: the device contract (`device_api::conformance`); the decay time at 1
and 3 s within 10 % at 44.1 and 96 kHz, and at 0.2 s for every Diffusion; the smallest room still
decaying at the longest Decay; the same level at both rates; Stereo's correlation at 0, 100 and
120; Reflect and Diffuse independent; the dry signal undelayed and the mix equal-power; the loop
keeping its energy with each line at its level; Mid's lines passing two allpasses (a first pass at Diffusion 60 % keeps
(1 − g²)² of its level); the first
reflection after the predelay; the predelay lowering the wet signal in inverse proportion to Decay;
the reflections' fade with Shape and Decay; Spin's reach, swing, restart and filtered taps following
one direct tap each; Sparse and Low each playing their own reflections; Freeze holding the tail,
Cut keeping new input out and Flat off keeping the damping; silence decaying to exact zeros; every
control moved from one end to the other without a click.

`devices/reverb/src/golden_tests.rs`: golden renders of nine scenarios (the defaults, no modulation,
the densities, damping, Chorus and Spin, Reflect and Diffuse, Freeze with and without Cut, a size
change), compared within 1e-6 with the files in `devices/reverb/tests/golden`; its README lists
them and how to make them again when a change to the sound is intended.
