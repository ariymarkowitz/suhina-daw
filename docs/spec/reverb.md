---
description: The built-in algorithmic reverb - its parameters, signal flow (input band, early reflections, a damped feedback delay network, chorus, freeze, stereo), how Live's Reverb imports into it, and how close it comes to Live's renders
read_when: reverb, algorithmic reverb, room, decay, early reflections, predelay, feedback delay network, FDN, damping, freeze, diffusion, importing Live's Reverb, Reverb elements
type: spec
status: current
last_verified: 2026-10-29
covers:
  - devices/reverb/**
  - crates/dsp/src/first_order.rs
---

# Reverb

Suhina's `reverb` (`devices/reverb`): an algorithmic room reverb with an input band, early
reflections, a feedback delay network for the late tail with damping in the loop, a chorus on the
network, freeze and a stereo width. It is designed so that Live's Reverb imports into it
parameter for parameter ([`device-import.md`](device-import.md)).

- **Observed with:** Live 12.4.6, 44.1 kHz. The parameters come from the parameter dump and a
  saved set; the sound from five behaviour runs of 98 cases (`reverb-structure`, `-decay`,
  `-filters`, `-texture`, `-size`) and the later runs of the same note (Lo Cut, network
  frequencies, Freeze, Smooth, Spin), rendered in Live and analysed by the measurement harness of
  the Suhina device notes (`live-12/audio-effects/29.32-reverb.md` and
  `measurements/29.32-reverb/` in the device-specs repository; candidate runs not made yet are in
  its `runs.py`).
- **Sources:** Live 12 Reference Manual, 29.32 Reverb (what each control is for); the
  measurements above (how it sounds); the feedback delay network of Jot and Chaigne (1991) and
  the Schroeder allpass, textbook designs. The early reflections' tap pattern is read from Live's
  renders (Decisions 12), and so is Density Low's network (Decisions 19); the other networks'
  delay lengths and the other constants are this device's own, tuned to the measured laws below.

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
| 21 | `size` | Size | 0.22 – 500 | 100 | plain number (a room volume), logarithmic |
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
in -> mono sum -> 127 + 19 Hz roll-off -> Low Cut -> High Cut -> predelay line -+-> reflection taps (Spin) ------ Reflect -+
                                                              |                                       +-> Stereo -> Dry/Wet
                                                              +-> damping, 15 kHz low-pass -> Shape delay -> direct taps, one line each
                                                                  -> feedback delay network, an allpass in      |
                                                                     every line (Size, Decay, Diffusion, Scale, |
                                                                     damping, Chorus, Freeze) ----- Diffuse ----+
```

- **Mono input.** Both sides are summed, `(L + R) / 2`: Live's Reverb gives the same output for a
  left-only and a right-only input.
- **Low roll-off.** The input always passes two first-order high-passes, at 127 Hz and 19 Hz:
  the first is the shape of every reflection in Live's renders with both cuts off, the second
  brings the lowest band (32 Hz) within 0.2 dB.
- **Input band.** Low Cut is a first-order shelf 14 dB deep, half way at
  `frequency / 2^(width/2)`; High Cut a 12 dB/octave Butterworth low-pass at
  `frequency × 2^(width/2)`, never above 18 kHz. With High Cut off the input still passes the same
  low-pass at 18 kHz, as Live's does (left out at sample rates where 18 kHz is above 0.45 of the
  rate). Both always run and their switches crossfade.
- **Size.** Every delay of the reverb (taps, Shape delay, diffusers, lines) is its length in the
  reference room times `(size / 100)^(1/3)`, as for a room's volume, except that below Size 100
  the lines shrink only with that factor to the power 0.35. Smooth glides the factor with a time
  constant of 45 ms (Slow) or 15 ms (Fast), but never so fast that the longest line or reflection
  changes by more than half a frame a frame; or not at all (None), which makes Size changes glitch
  as in Live.
- **Early reflections.** Live's own tap pattern for each Density (10 direct taps and 61 filtered
  ones over 129 ms at Low; 4, 10 and 10 direct taps at Sparse, Mid and High, every table to about
  130 ms), each time a whole number of frames of 44.1 kHz plus its tap's fraction of a frame in
  the reference room, each on one side with its sign and level. The first tap is 3 frames (at 44.1 kHz) after the predelay, with a gain of 0.0942. The
  filtered taps pass, per side, a cookbook low-pass of Q 0.508 at `12 kHz − 818 Hz × decay^−0.886`
  (8.5 kHz at 0.2 s, 11.3 kHz at 1.2 s). Every tap fades by `(0.0328 + 0.0668 × shape) / decay`
  dB per ms of its delay (decay in seconds), and the whole wet signal, tail included, is
  `0.0113 / decay` dB quieter per ms of predelay (0.0565 at 0.2 s, 0.0094 at 1.2 s, −2.8 dB
  at 250 ms for Decay 1 s: Live's drop at 250 ms is 5.6, 2.3, 0.8 and 0.2 dB at Decay 0.5, 1.2,
  3.46 and 14.4 s, the same for the reflections and the tail). Spin swings each direct tap's whole
  delay `T` (predelay included) by `0.00995 × amount × reach(predelay) × T × sin(2π × m × rate ×
  t + φ)`, where `reach` is 1 up to 5 ms of predelay and `2 / (predelay − 3.02)` above it
  (predelay in ms: 0.29 at 10 ms, 0.0081 at 250 ms), so a long predelay swings every tap by about
  the same 2 ms × 0.00995 × amount. Each of the ten direct taps has its own LFO, its rate `m` times
  Spin Rate (0.978 to 1.042) and its phase φ when playback starts, as tracked in Live (Decision 10). A
  filtered tap is a direct one a whole number of frames later (twenty offsets in pairs two frames
  apart, 395 and 397 frames of 44.1 kHz to 3190 and 3192 frames, 8.96 to 72.38 ms, in the
  reference room; each pair always heard on one side) and swings as that one did when it was
  read. So each direct tap is read once, the filtered taps at each offset are played as one mix
  of the direct taps (each at its filtered tap's level), delayed by the offset, and the network's
  input taps swing with their reflections.
- **Network input.** The direct taps again, read after the Shape delay of `32 + (18.5 + 34.83 ×
  (shape − 0.5)) × size factor` ms from a copy of the predelay line whose input has passed the
  loop's damping (below) once, at 0.21 of a trip, and a first-order low-pass at 15 kHz. Each
  direct tap feeds one line of its own, in turn (the first tap the first line, and so on,
  starting again at the first line when the taps outnumber the lines), except that the first tap
  feeds three lines spread over the network at 1/√3 each, with the line's sign, at its level and
  fade (so Shape and Decay shape the tail's onset as they shape the reflections), normalised so
  the taps' energies sum to one. Its level is `1.236 × 10^(−(0.708 / decay + 0.12 / decay²) / 20)`,
  and the taps feeding it take their fade over 20 ms more than their own delay (so the tail falls
  with Shape twice as much as the reflections' first part does, as Live's does); in rooms
  larger than the reference it rises with the size factor to the power 0.75, in smaller ones with
  the square root of how much longer the lines are than the room's (at most 1.6 times, +4 dB),
  and Density Sparse and High are trimmed by −0.3 and +1 dB. Freeze with Cut on fades it out (20 ms).
- **Network, Density Low.** Live's own network, as measured (Decisions 19; `devices/reverb/src/measured.rs`
  has the numbers). The ten direct taps are summed in six groups and the groups into four
  patterns (unit vectors over the groups), read `17.39 + 34.83 × (shape − 0.5) × size factor` ms
  after the first direct tap from the same damped copy of the predelay line, at the same levels
  and fades as above, times 0.486, with Shape's share of the fade over 20 ms more (about Shape
  0.5) and none of the cuts for short decays of Decision 5 (Decision 20). Seven loop lines, mixed after every trip by one 7 × 7
  orthogonal matrix: six are a delay and a Schroeder allpass (Diffusion as its gain), 52 to 194 ms
  long at Scale 40 % in the reference room, of which the allpass is 0.03 to 2.9 ms per 10 % of
  Scale; the seventh is a plain 1641-frame (37.2 ms) delay that feeds the third line. Each line's
  length at Scale 5, 20, 40, 60, 80 and 100 % is Live's, and in between it is read off the straight
  line between them. The patterns enter the lines directly and through four plain delays (9.1,
  30.3, 72.4 and 48.3 ms) that feed the lines without taking anything back. Five lines are heard,
  two on the right and three on the left, each at its own level. A return line (Decision 21)
  takes the groups again, runs the first and sixth lines' delays and 2 frames more (227.5 ms at
  Scale 40 %) through one allpass as long as both of theirs, takes 0.757 of its own output and a share of every loop line's, and
  is heard on the right only; it feeds nothing back. Three families of filtered reflections enter
  as well, one offset after their mix of the groups (Decision 23). Every length scales with the size
  factor, small rooms too; damping and chorus are as below. Every path fades with its whole time
  from the input: each line's gain covers its delay, its allpass loses its own delay's gain on
  every pass (`y = a·w[n−L] − g·w`, `w = x + g·a·w[n−L]`), and each plain delay its own.
- **Network, Density Sparse, Mid and High.** 8, 12 or 16 lines (Density Sparse, Mid, High), 18 to 77 ms long in the
  reference room, 1.3 times that at Sparse. Each line's output passes a Schroeder allpass with the
  Diffusion as its gain (0.2 of it at Sparse) and a delay of its own,
  3.6 to 15.2 ms times Scale and the size factor, on every trip. The lines are mixed after every
  trip by a Householder reflection, `I − (2/N) 11ᵀ`, followed by a shift of one line: each line
  keeps `1 − 2/N` of itself (three quarters at 8 lines), passed on to the next line, and sends
  `2/N` to every other one. So the first arrivals are single echoes of the direct taps, the first
  about 70 ms after the first reflection, as Live's is, and the echoes thicken over the next
  few trips. Each line's gain per trip is `10^(−3 × (length + allpass delay) / (0.9 × decay))`, so
  every line decays in the same time whatever its length: the tail falls 60 dB in 0.9 of the Decay
  setting, as Live's mid bands do.
- **Damping.** In each line, after its gain: High Damping as a second-order high shelf (the
  cookbook shelf, Q 0.707, half way at the frequency) or, as Low-Pass, a one-pole low-pass
  `y += (1 − p)(x − y)` with `p = e^(−2π × frequency / 44.1 kHz)` per trip; Low Damping as a
  second-order low shelf (the same, below the frequency). A line's share of the one-pole is two
  first-order high shelves, each with half that share of its loss at the top, half way where the
  one-pole has lost a quarter and three quarters of its loss there. Their gain is a linear factor per trip round a
  0.17 s loop, raised to the power of the line's length and its allpass delay (in the room as it
  is) over 0.17 s, so the loss per second does not depend on Size. A fixed loss of the highs is
  always in the loop: a first-order high shelf half way at 8 kHz, 0.96 dB per 0.17 s trip, less for
  decays over 3.46 s (by the ratio to the power 0.15). In the measured network (Density Low)
  it shrinks with the power 0.25, grows in rooms below Size 10.5 by the ratio of the size
  factors (twice at Size 1.53), and with Smooth None it is 0.06 dB per trip at every decay:
  Live's loss of the highs comes almost all with Smooth Slow or Fast (with None its 16 kHz octave
  decays 0.3 to 0.4 dB/s faster than 1 kHz at Decay 3.46, 14.4 and 60 s, against 2 to 3.5 dB/s
  with Fast), which the measurements of the decay per band were made with. Lines and their allpasses are whole frames
  long once Size, Scale and Decay have settled, so a line without chorus is not interpolated.
- **Chorus.** Swings every line's delay either way by up to `1.8 ms × amount` (at most half the
  line), each with its own phase of one LFO at the Chorus rate; lines are read with cubic
  interpolation.
- **Output.** Each side of the tail is the sum of every other line, with a sign per line: the
  left the first, third and so on, the right the others. The sides share no echo, and as the
  mixing passes most of each line on to the next, an echo is heard again on alternate sides, as
  Live's are. The reflections (times Reflect) and the tail (times Diffuse) are added and rotated
  by `β = 45° × (1 − stereo / 120)`: `L' = cos β L + sin β R`, `R' = sin β L + cos β R`. So 120 keeps
  them apart, 100 gives a correlation of about 0.25 (Live's 0.25) and 0 makes them mono. Dry/Wet is an equal-power
  crossfade, `dry × cos(mix·π/2) + wet × sin(mix·π/2)`. The dry signal is not delayed; the device
  adds no latency.
- **Freeze.** Every line's gain is set so the tail falls 0.07 dB per second with Flat on and 0.2
  with Flat off; the fixed loss of the highs stays in at its 60 s amount, and with Flat off the
  damping stays in too, so a frozen tail drains as Live's does: 0.25 and 0.4 dB per second in
  all, most of it above 2 kHz (Decision 15). Cut decides whether
  new input still enters the network; if it does, levels above 2 are compressed toward 4 so the
  tail cannot grow without end. The early reflections go on.
- **Changes while playing.** Levels, mix, Stereo, every switch, the Spin and Chorus swings,
  Diffusion, Shape, Scale, Decay, the input band and the damping frequencies and gains ramp over
  20 ms, Predelay over 100 ms (a long jump bends the pitch on the way); delay lengths and gains are recomputed every 32 frames and interpolated frame
  by frame in between. Switching Density starts or stops lines at once, and the mixing matrix
  changes with them; the new lines start from silence. This is deliberate and has no crossfade:
  changing Density while playing was listened to in the app (2026-10-18) and no click could be
  heard. The `size-change` render covers the moment (Density Mid at 1 s).

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
| `RoomSize`, `SizeSmoothing` | 0.22 … 500, 0 None / 1 Slow / 2 Fast | `size`, `smooth` |
| `FreezeOn`, `FlatOn`, `CutOn` | true/false | `freeze`, `flat`, `cut` |
| `StereoSeparation` | 0 … 120 | `stereo` |
| `RoomType` | 0 Sparse … 3 High | `density` |
| `MixReflect`, `MixDiffuse` | linear gain 0.03 … 1.995 | `reflect`, `diffuse`: 20·log10 of it, dB |
| `MixDirect` | 0 … 1 | `mix` |

A stepped value outside its list is left at the default with a warning. Sets saved before Live
11.2 named some of these differently; only 12.4.6's names are read.

## Decisions

Each entry: what was chosen, what it costs, why.

1. **A feedback delay network of 8 lines (Density Low) with an allpass in every line and Live's
   early-reflection taps,** over a Schroeder reverb of combs: a denser, smoother tail with the
   same decay at every frequency the damping does not touch, in the same class as Live's (whose
   manual describes filters and modulated delay lines). Cost: see 16. Dragonfly Reverb (GPL-3.0),
   the ROADMAP's earlier candidate, was not ported: its algorithms and controls do not map onto
   Live's stages one to one, and an own network of textbook parts, tuned to the measurements, is
   smaller and needs no port.
2. **Each direct tap feeds a line of its own, and the diffusion is an allpass in every line,**
   over the sum of the taps through four allpasses into every line. That sum was one fixed comb
   in front of the whole tail: from 0.15 to 1.5 s its spectrum rippled 5.0 dB (the spread of the
   1/6-octave-detrended spectrum from 300 Hz to 6 kHz) against Live's 2.6, with a 2 to 4 dB bump
   at 250 Hz, and it was heard as tonal. The allpasses in front also smeared the onset into noise
   at once (a kurtosis of 2.8 in 10 ms windows from 100 to 200 ms, against Live's 15), where Live's
   is heard as distinct echoes. Now the ripple is 2.6 dB, the onset's kurtosis 7.3, and Live's
   response to Diffusion (sparsest near 0 and near 0.96) is what an allpass per trip gives. The
   allpasses' fractional delays had also dulled the input; without them 10 to 16 kHz came out 1 to
   3 dB loud, which the 15 kHz low-pass takes back. The taps go to the lines in turn: Live's first
   echo of the tail, about 70 ms after the first reflection at the default Shape, is the first
   tap through the shortest line, and orders searched to fit the onset's level better (0.5 dB rms
   from 50 to 600 ms against 0.8) made its texture worse. Cost: an allpass per line instead of
   four in all, 4.05 % instead of 3.65 % at Density High, the same at the defaults.

   The first tap, the loudest, feeds three lines: through one line its first echo, 69 ms after
   the first reflection at the defaults, was a single spike about 5 dB above Live's, heard as a
   click; Live's has arrivals at 69.5, 74.2 and 76.9 ms, this device's now at 68.8, 72.9 and
   76.9.

   The allpasses' delays (3.6 to 15.2 ms at Scale 100 %, 1.4 to 6.1 ms at the default 40 %) are
   three times what they were. An allpass turns every echo into a train of echoes one delay
   apart, falling by the Diffusion each time; at 0.5 to 2 ms apart that train is heard as a quick
   roll of clicks, and the tail's onset as bunches of rolls, where Live's clicks are scattered.
   Measured on the strong clicks (2.5 times the 20 ms rms) from 50 to 250 ms, the spread of the
   gaps between them against their mean (1 for clicks at random) is 0.93 at the defaults and at
   Shape 0, and 1.01 at Shape 1, against Live's 0.78, 0.79 and 1.05; it was 1.07 to 1.28 with
   the shorter allpasses. Two short allpasses in front of the network, tried to soften the first
   echoes, bunched the clicks further (1.22 to 1.51), even at Diffusion 0.1 % (1.24 against
   Live's 0.84), and were left out. Cost: 12 fewer octave decay times within 10 % (317 of 376
   against 329 of 368), and 27 fewer `compare.py` checks, against 14 more tail windows within
   0.5 dB.
3. **Householder mixing (with a shift), each side hearing every other line,** over a Hadamard
   matrix heard on both sides with two sign patterns. A Hadamard matrix sends every line into
   every other one equally, so the echoes turned to noise within two trips: even at Diffusion
   0.1 % the tail was Gaussian from 200 ms, where Live's stays sparse to 500 ms. A Householder
   reflection keeps three quarters of each line at 8 lines and passes it on to the next, so a path
   is heard again on alternate sides, as in Live's renders, and the echoes thicken over several
   trips. Hearing every other line on each side, the sides correlate by 0.24 (Live's 0.25; 0.29
   with every line on both sides) and the ripple is 2.6 dB instead of 3.1. Cost: 2N operations
   instead of N·log N, less.
4. **Cubic interpolation for the taps, the lines and their allpasses.** Linear interpolation
   would save about 0.8 % of a core but dulls and modulates the highs while Spin or Chorus moves.
   Without Spin, and with nothing moving, the taps are read at whole frames, and once settled the
   lines' allpasses are whole frames too (fractional ones in the loop cut the 8 kHz octave's decay
   time by 10 to 38 %).
5. **The tail is tuned to 0.9 of the Decay setting** and its level follows
   `1.236 × 10^(−(0.708 / decay + 0.12 / decay²) / 20)` with the taps' fade over 20 ms more
   (Network input): the decay is what Live's renders show (0.87 to 0.9 of the setting from 0.8 to
   3.5 s), and the level law is fitted to the steady level of Live's tail from 0.3 s on in every
   case (0.2 s and later for Decay 0.2 s); the factor was refitted for the network of Decisions 2
   and 3 (1.194, 1.215 and 1.236 tried, the last leaving the most of the tail's windows within
   0.5 dB). A level law
   of one term in `1 / decay` left Decay 0.2 s 2 dB loud; the second term is 0.2 dB or less from
   0.8 s up. The 20 ms are what Shape's effect on the tail asks for (2.4 dB from Shape 0 to 1 at
   Decay 1.2 s, against 1.3 dB from the taps' own fades). The reflections fade at the same rate:
   with Shape's share taken out, their fade is a T60 of 0.9 of the setting too.
6. **The damping is a gain per trip round a 0.17 s loop** (the loop length that fits every
   measured band of Live's), applied to each line in proportion to its length in the room as it
   is, so it sets the same decay per band whatever the line and the Size. Live's renders keep the
   top octaves' decay with Size; a loss per trip would shorten it in small rooms. The shelves
   are second-order: first-order ones, the simpler choice, had to sit off the set frequency to fit
   it and still left the octave band either side 13 to 28 % off in decay time; second-order ones
   at the set frequency fit within 8 %. Cost: two biquads instead of two first-order sections per
   line. The Low-Pass type is Live's one-pole once a trip, at the set frequency: from the decay
   rate per third-octave over Live's undamped one, `reverb-filters` at 1 kHz and
   `reverb-network-freq` at 4 kHz fit it together within 2.4 dB/s rms (the corner at 0.99 of the
   setting), where an analog first-order low-pass needs a corner and a trip rate of its own for
   each. Gain does nothing with it, in Live's renders as here. A share of a trip is two shelves,
   not a low-pass with its corner raised: that matches the one-pole's loss only below the corner
   and lost highs hundreds of dB/s too fast above it (at 1 kHz the 4 kHz third-octave lost 143
   dB/s more than undamped, Live's 76; now 69), and one shelf alone is up to 28 dB/s off below
   1 kHz, two within 3 dB/s from 250 Hz up.
7. **A fixed 90 Hz roll-off at the input,** first-order, the simplest that brings the lowest two
   octaves within about 2 dB of Live's in every case (they were 3 to 8 dB too loud without it).
8. **The fixed loss of the highs** (an 8 kHz shelf, 0.96 dB a trip, less above 3.46 s) is there
   because Live's tail is shorter above 2 kHz even with both filters off. Above 3.46 s it shrinks
   with the decay to the power 0.15: Live's own extra loss above 8 kHz hardly shrinks from 3.46 to
   60 s (at 8 kHz 3.0 and 2.3 dB per second more than at 1 kHz). With the cube root, and then
   0.27, the 10 to 16 kHz third-octaves decayed 1 to 2 dB per second too slowly at 14.4 and 60 s,
   heard as a harsh top; with 0.15 they are within 0.4 dB per second at 14.4 s and 1.2 at 60 s.
   Below 3.46 s the loss stays at 0.96 dB: letting it grow further took 2 dB per second too much
   from 6 to 8 kHz at 0.83 s.
9. **Low Cut is a 14 dB first-order shelf** half way at the band's lower edge. Live's Lo Cut fits
   no simple filter (3 to 8 dB/octave, flattening at the bottom); at 1 kHz, width 5.85 the shelf is
   within 0.6 dB rms of its measured response (1.3 dB at worst, at 57 Hz), and at 5 kHz it is
   0.6 dB above Live's −3 dB point and 1.3 dB above its −6 dB point.
10. **Spin swings each direct reflection's whole delay on its own LFO, by a share that falls with
   the predelay; a filtered reflection swings with the direct one it follows, and the tail's
   input with them.** Tracked in Live's renders of a click every 0.4 s (and 0.2 s), reflections
   only, at Predelay 0.5 to 250 ms, Amount 17.5 and 55, Rate 0.07, 0.3 and 1.3 Hz, Size 10.54,
   100 and 500: each tap's arrival follows a sine whose peak swing is
   `0.00995 × amount × reach × T`, with `T` the tap's delay from the input, predelay included, and
   `reach` 1 up to 5 ms of predelay and `2 / (predelay − 3.02)` above (ms). Over the ten direct
   taps the fit leaves 1 % at every predelay from 10 to 250 ms, and `reach` comes out the same at
   both Amounts, all three Rates and all three Sizes (within 1 to 2 %). So at the default 2.5 ms the
   swing is about 1 % of the whole delay per unit of Amount, and at 250 ms every tap swings by
   about the same 0.36 ms at Amount 17.5 (the first tap and the one at 41 ms alike). Live's
   swing does not stop at the predelay: stretching the delay after it alone, as before, left the
   first tap still and over-swung the late ones at a long predelay.
   Each of the ten direct times (the same in every Density) has its own LFO: a rate 0.978 to
   1.042 times Spin Rate (the same ratios at 0.3 and 1.3 Hz, 0.6 % faster overall at 1.3) and a
   phase that is the same whenever playback starts, from bar 1 or bar 2 (0.17 to 1.71 rad), so the LFOs start again with the transport, as ours do. A
   filtered tap is a direct one 8.96, 30.27, 52.97, 67.62 … ms later (the offsets every table
   repeats) and moves exactly as it does: at 0.5 ms the left one at 30.27 ms, a copy of the
   first, swings 0.09 ms where its own delay would give 5 ms. The offsets come in pairs 0.045 ms
   apart and each pair is heard on one side only (8.96, 37.17, 46.96, 52.97 and 56.53 ms on the
   right; 21.77, 30.27, 54.24, 67.62 and 72.34 ms on the left), which assigns every filtered tap
   of every Density to one direct tap. Live's renders of each Density with Spin
   (`reverb-spin-filtered`) confirm the cases the times alone left open: the right taps at
   63.95 ms (Mid) and 56.58 ms (High) follow the ones at 7.37 and 0 ms (the missing partner of
   56.53), and of the two taps at 64.44 ms the right follows the left one at 11.43 ms and the left
   the right one at 34.17 ms; the left one's level moving is its level still, so it is one copy,
   not two. Every offset is a whole number of frames of 44.1 kHz (395, 397, 960, 962 … 3192), so
   a filtered tap is its direct tap delayed again: it swings as the direct tap did when that one
   was read, the offset earlier, not as it does now. Live's renders show this: scoring both tracks
   against `reverb-spin-filtered` at Amount 17.5, 273 of the 275 filtered taps whose tracks part by
   more than 0.15 ms are louder along the one sampled at the direct tap's time, 3 to 20 times;
   Sparse's late copies sit on it within 0.06 ms and 0.4 to 0.5 ms off the other. Ours did the
   other until the filtered taps were played as delayed mixes; the same scoring of our renders of
   the same train now picks the direct tap's time for every tap, by the same margins. Swinging each tap by its own
   delay overstated the tone's FM 2.6 times at the default predelay (1.24 semitones peak to peak
   against 0.48) and gave the reflections +9 dB at Amount 55 in 150 to 300 ms with Diffuse at its
   minimum; both are now within the measures' noise (Known differences). Live's tail with Spin is
   uncorrelated with its tail without (r −0.04 after the noise), so the network's input taps swing
   with their reflections. Cost: about 0.4 % of a core with Spin on (ten sines a frame, 0.2 %
   more than one LFO for all). Chorus swings every line's delay
   either way by 1.8 ms per unit of Amount, each line with its own phase; that figure comes from
   the FM of a tone through the tail, which overstated Spin's swing, and is kept until Chorus is
   tracked line by line.
11. **Level corrections for large rooms and for Density** (the size factor to the power 0.75
    above the reference room; −0.3 dB for Sparse, +1 dB for High) are fitted to Live's renders: a
    larger network spreads the same energy over longer lines and comes out quieter, which Live's
    does not. Below the reference room the lines shrink less than the room (the size factor to
    the power 0.35), so a small room keeps enough modes not to ring: at Size 1.53 the tail
    peaked at 461 and 709 Hz, 5 dB above Live's strongest peaks (940 Hz to 4.6 kHz), and its
    ripple was 8.3 dB against Live's 4.9; now it is 4.0. Longer lines spread the energy thinner,
    so the tail is fed louder by the square root of the stretch, capped at 1.6 times: beyond that
    a Size glide from the smallest room rang.
12. **The early reflections are Live's own tap pattern, read from its renders.** Every tap of Live's
   reflections is a delay of the input rounded to a whole frame, either as it is or through one
   low-pass whose corner follows Decay; their times scale with the cube root of Size, and with the fade taken out
   the direct taps all have a level of exactly ±1. So the pattern was read tap by tap from the
   impulse responses (deconvolved from the input filters) and is used as measured, which puts
   every tap of the reflection-only cases within 0.5 ms and 0.5 dB of Live's. This reverses the
   earlier choice of an own pattern that only kept Live's laws: the per-tap checks of the
   comparison cannot pass without the real times, and the pattern is a measured behaviour of the
   device, read from its output alone. Two corrections came from reflections-only renders with
   Diffuse at its minimum and Spin off (`reverb-tap-frac`, `reverb-spin-filtered`):
   - **Each tap's time carries a fraction of a frame**, which Live rounds away: with the predelay
     moved by tenths of a frame around 5 ms, every tap steps to the next frame at its own fraction
     (the ten direct taps at 0.05, 0.05, 0.35, 0.65, 0.27, 0.35, 0.15, 0.65, 0.35 and 0.95 of a
     frame past a whole one, the same in every Density; a filtered tap at its direct one's). The
     tables were read at 2.5 ms (0.25 of a frame), which put four direct taps and their copies one
     frame late at most other predelays. The tables now hold the fractions, and every one of the
     229 taps that steps clearly steps where Live's does.
   - **Sparse, Mid and High go on past 67 ms**, where Live's tail hid them before: their reflections
     continue to 130 ms as Low's do, about a tenth of the reflections' energy. They follow the same
     structure (a direct tap plus one of the offsets, with its partner two frames later), so the late
     taps were fitted as such, 23, 73 and 96 of them above a gain of 0.04, together with the
     faint taps' gains (117 changed, most by 0.5 to 2.6 dB). Live against ours over the reflections'
     first 140 ms went from −4.9 to −27 dB of the energy at Sparse (−26 at Mid and High, −30 at Low,
     at every fraction of a frame).

   The network's line lengths and diffusers stay this device's own:
   Live's network was only partly identified (Known differences).
13. **The network's input passes the damping once, at 0.21 of a trip** (one more set of the
   loop's filters on the mono input, no measurable cost). Live's tail is damped from its first
   arrivals on: against the same settings with the filters off, its 65 to 120 ms lose 2.2 times
   what a line's own share of a trip gives, for the high shelf, the low-pass and the low shelf
   alike. Without it the highs came out 2 to 5 dB loud for the first 0.3 s with High Damping
   on, and the tail's broadband decay read 15 % short. The Low-Pass is passed at 0.8 of a trip
   instead: at 0.21 the tail's first 0.2 s at Low-Pass 1 kHz had 8 kHz 1 dB above 1 kHz against
   Live's 7 dB below, and 0.8 brings 2 to 16 kHz within 1 dB of Live's there (4 kHz within 1 dB
   too).
14. **Smooth's glide times (45 ms Slow, 15 ms Fast)** are what one point of Live's glide gives for
   an exponential glide (`reverb-smooth-glide`: 50 ms after a jump Fast has finished, Slow has 30
   to 40 % left); the curve itself was not measured.
15. **Freeze, Flat and Cut** follow Live's renders: a frozen tail drains 0.25 dB per second with
   Flat on and 0.4 with Flat off, and Cut keeps new input out entirely. Per band
   (`reverb-freeze-static`, 2 to 10 s), Live's frozen tail with Flat on falls 0.1 dB/s at 1 kHz
   and 0.2, 0.6, 1.1 and 1.6 at 2, 4, 8 and 16 kHz, the loss of the highs of its 60 s tail; with
   Flat off the shelves stay in: 3.7 dB/s at 4 kHz, 14 at 8 kHz and 1.7 at 125 Hz with the
   preset's filters, against 0.2 at 1 kHz. Leaving out every filter and draining all bands alike
   had Suhina's frozen tail 30 dB bright at 4 kHz and over 100 dB at 16 kHz after 8 s with Flat
   off, and 4 to 7 dB bright at 8 to 16 kHz with Flat on; now each band's drain is within
   0.2 dB/s of Live's from 125 Hz to 8 kHz. The soft limit with Cut
   off is Suhina's, so a frozen tail fed for minutes cannot overflow.
16. **Cost: about 3.2 % of a core at the defaults,** over the 1 % budget, as the device notes allow
    for reverbs: the ten direct taps (cubic while Spin moves them), the filtered taps (up to 168,
    at High) as one mix per offset into up to twenty delay lines read at whole frames once the
    room settles (Decision 10), and 8 to 16 modulated lines with an allpass and three to four
    filters each run every frame. Reading each of the 178 taps of High on its own, cubic while
    Spin moved them, cost 6.9 % (5.4 % with Spin off); the mixes take it to 4.7 % (4.2 %), and Mid
    from 5.6 % to 4.0 % (4.1 to 3.5 %), with Sparse 2.3 % and Low 2.8 % (30 s of noise at 48 kHz,
    the best of three, 2026-10-30). Cutting further would mean fewer lines or linear interpolation
    (Decision 4).

17. **Density Sparse is 8 lines 1.3 times as long, with a fifth of the Diffusion,** not 4 lines.
    Live's Sparse has sparser echoes than Low and yet a smoother spectrum, which takes more loop
    delay, not less: 4 lines reached Live's ripple only at three times their length, with nothing
    arriving before 150 ms. Its kurtosis from 150 to 200, 200 to 300 and 300 to 400 ms is 36, 16
    and 7.1 against Live's 27, 12 and 6.2 (5.6, 3.2 and 2.9 with 4 lines), and its ripple 2.2 dB
    against 2.0 (5.6). The diffusion was set by the late tail, where Live keeps distinct clicks: at
    0.35 of the setting it held a third fewer clicks per second than Live's after 400 ms, which
    was heard as a slower, drier rhythm. Cost: as many lines as Low, though it stays cheaper for
    its 9 taps.
18. **Size glides at most half a frame a frame** in the longest line or reflection. Faster, as
    Fast's first milliseconds are across most of the range, a line is read backwards and buzzes
    (the jump from the default Size to the largest stepped the output by 0.030 against the
    still signal's 0.012). Only near-instant jumps across most of the range glide more slowly than
    Smooth says.
19. **Density Low runs Live's own network, identified from its renders,** in place of the
    Householder network (which Sparse, Mid and High keep: their networks are different and not
    measured). Live's impulse responses at Diffusion 0.1 % (the allpasses then pass the signal on
    as plain delays) and Decay 60 s, at Size 100 and Scale 5 to 100 %, were read as sparse
    arrivals; each arrival moves with Scale by the allpass length on its path, which tells the
    lines apart, and the network (patterns, lines, plain delays, mixing, outputs) was fitted to
    all six Scales at once with the mixing held orthogonal. Live renders the same tail every time
    (two renders bit-identical over 10 s), so the fit is against the waveform itself. Against
    Live's renders it does not see (`reverb-network-id default`, Smooth None) the tail explains 93 %
    of the energy of each side to 135 ms, 81 / 77 % (left / right) to 204 ms, 67 / 59 % to 272 ms
    and 57 / 31 % to 363 ms, with no timing lag; the old network was uncorrelated past its first
    arrivals. What is not found: Live's own allpasses (one per line here; the return line's alone is
    measured, Decision 21) and how Smooth Fast changes the network (Live's Fast renders run its delays a
    little shorter, up to 11 frames by 0.25 s and 39 by 0.9 s at Size 500, so they do not match
    its Smooth None renders sample for sample). Cost: 7 lines and 4 plain delays against 8 lines,
    and a 7 × 7 mix.
20. **In the measured network every path fades with its whole time** (Density Low). Live's tail
    at Decay 0.2 s falls at 336 dB/s from its first arrivals down (a T60 of 0.18 s, 0.9 of the
    setting); Suhina's fell half as fast (0.33 s) and its first arrivals were 8 to 12 dB weak, 10 dB
    in all. Two paths decayed less than their time: an allpass holds echoes back for several of
    its delays while the line gain counted it once (at Diffusion 0.6 the T60 at 0.2 s was 0.256 s
    against 0.199 s at 0.1 %), and the plain delays in front (up to 121 ms through the chained
    pair) fed the loop undecayed, which came out 12 dB loud after 150 ms. With both fading, the
    cuts of Decision 5 (fitted to the Householder network) were what made the first arrivals weak:
    without them every window of the reverb-decay cases is within 1.7 dB of Live's from 60 ms on, at
    every Decay. The level is refitted (0.545 to 0.486) and keeps Shape's share of the 20 ms fade
    (Shape 0 to 1 then within 0.7 dB of Live's, against 1 dB without). Against Live: Decay 0.2 s
    +0.5 dB and T60 0.20 s (was −9.9 dB, 0.31 s), 0.83 s +0.1 dB (−1.1), the default −0.1 dB
    (−0.5), 3.46 s −0.6 dB and 3.01 s against 3.01, 60 s −0.1 dB (+0.9).

21. **Density Low's network has a return line heard on the right,** which the first fit missed
    (it explained a third of Live's right side past 0.27 s). Live hears every direct reflection
    again on the right alone, A's and G's lengths and 2 frames after it reaches the network
    (10,034 frames at Size 100 and Scale 40 %, moving with Scale by A's and G's allpasses and with
    Size like the lines), and again one length later at about three quarters; the 48.3 and 55.7
    ms reflections' return is 13 times the tail's rms, the echo heard at 1.37 s at Size 500. It
    was found in `reverb-network-taps` (Stereo 120, so each side apart): each arrival's source
    reflection is told by how Shape tilts it (Shape 0 against 1 at Decay 0.2 s moves a
    reflection's level by 0.336 dB per ms of its own delay), which put every reflection's return
    at the same delay with one weight per group of the network's input. Its allpass was read in
    `reverb-network-allpass` (Diffusion 0.1, 30 and 96 %, Scale 100 %): the return comes as one
    pulse and copies 649 frames apart (A's and G's allpass lengths together), at −g, 1 − g² and
    (1 − g²)·g of it with g the Diffusion setting, so one allpass of both lengths and not one each
    (two allpasses had fitted the energy windows better, which is why fitting alone could not
    choose). So the return line takes the six groups (`RETURN_FEED`), runs A's and G's delays
    through that allpass, takes 0.757 of its own output and a share of each loop line's
    (`RETURN_LOOP`, fitted to the renders' waveforms, the least sure part), and is heard on the
    right at 1.266, the level that makes its first return match Live's against the left side's
    reflections; G, heard on the right at −1.5 before to even the sides, is not heard (Live is
    silent where G's first pass would be). Live's input to the network is mono: impulses on the
    left alone and on the right alone (`reverb-network-sides`) give identical renders, so the
    return line hears no side of the input. The Size 500 echo now peaks at 1.383 s, 19 dB over the
    tail's rms against Live's 1.382 s and 17 dB (it was missing). Over the `reverb-impulse`
    comparison set the right side's late windows are closer (1 to 2 s −0.08 dB median against
    −0.54, 2 to 4 s 0.00 against −0.27), but 0.15 to 1 s are now 0.5 to 0.9 dB quiet (−0.13 and
    −0.73 before), so 787 of 1,084 energy windows are within 1 dB (824 before): Live's right side
    has energy there that this network does not explain (Decision 22 found it is not in the
    network's input). Cost: one line and one allpass, about 0.3 % of a core.
22. **The measured network hears each direct reflection as it was read, Spin's shift included,**
    delayed by the Shape entry (`ENTRY_MS`), in one short line per input group. It read the feed
    line an entry later at the shift Spin had then, which is the same with Spin still but not while
    it moves. Found in `reverb-network-spin` and `reverb-network-spin2` (an impulse every 2 s at
    Spin Rate 0.3 and 0.37 Hz, Amount 30, Stereo 120, Diffusion 0.1 %, Decay 0.2 s, Hi Cut off):
    with each reflection's position at each impulse known, the tail of each impulse is the sum of
    the reflections' own responses, each moved by its reflection's shift, which twenty impulses
    solve per frequency. Live's tail fits that to −37 dB (right) and −29 dB (left) below 4 kHz with
    the shift each reflection had when it was read, and to −6 to −11 dB with it taken 383 or 767
    frames later; Suhina's fitted only at 767 frames (−24 dB) and now fits at none, to −48 and −42
    dB. The same solve confirms the input groups (`TAP_GROUPS`) and their signs and fades: one
    response per group predicts an impulse left out to −38 dB (right) and −31 dB (left) below 1
    kHz, as well as one per reflection, and the two reflections of a group have responses of
    coherence 0.97 to 1.00 at the same level. The six groups' responses carry Live's energy to
    within about 1 dB from 50 to 300 ms and correlate with Live's at 0.79 to 0.99 (Known
    differences for their timing), so the right side's later shortfall (Decision 21) is in the
    loop, not the input. Positions were read by fitting each reflection's law to the
    reflections-only render at all frequencies with its LFO taken when the reflection is read
    (reading it at the impulse made every phase look 0.06 to 0.18 rad late, growing with the delay).
    Cost: six lines read once a frame.

23. **Density Low's network also takes four families of filtered reflections,** each a fixed
    number of frames after the direct ones (`COPY_OFFSETS`). Each family of filtered reflections
    (Decision 12: a direct one 395, 1335, 1639, 2336, 2982 or 3190 frames later, on one side) is a
    mix of the six groups, the same for both reflections of a group, and those mixes are the
    network's inputs: the 395 family's is the first pattern (1.04 to 1.19 times it, group by
    group), 3190's the second, 1639's the third (which also feeds the 1641-frame line), 1335's the
    fourth, 2336's the return line's `RETURN_FEED` (1.31 to 1.48 times), and the plain delays in
    front of the loop are those offsets (400.75, 1336.75 and 3192.75 frames). The network takes some
    mixes again one offset later, as the filtered reflections themselves, which the first fit
    missed. Found in `reverb-network-taps` (Stereo 120, Diffusion 0.1 %, Decay 60 s), arrival by
    arrival against Suhina's render of the same cases: a missing arrival's levels over the ten
    direct reflections follow one family's weights (fit 0.96 to 1.00), its move with Scale names
    the lines it passed, and Shape tilts a copy as the filtered reflection it is (at 101 ms), not
    as its source (48 ms). So:
    - the return line takes the 2336 family at 0.51 of a direct reflection's share (its returns
      came at 0.57 to 0.66 of their sources' own, with no repeat at twice the offset), and the 395
      family (the first pattern's) at −0.36 (moving with Scale by A's and G's allpasses, 409
      frames after the 2336 family would be at Scale 5, 40 and 100 %);
    - the 2982 family (left) feeds F straight from the groups (0.51), and B (0.44) and the
      1641-frame line (−0.76, heard on the right through C) one offset later;
    - the 1639 family feeds A (0.20) and C (−0.18) one offset later, where Live's path through
      the 1641-frame line and then A was 3.9 dB stronger than the mix gave, and through it and
      then C 1.2 dB weaker.

    Each copy is read from the filtered reflections' own offset lines before their side's low-pass,
    passes a low-pass like theirs and waits the Shape entry as the groups do. With these in,
    `RETURN_LOOP` (fitted when they were missing) made the right's late tail 0.6 dB loud, so it
    keeps its fitted shape at 0.85 of its size; at zero the right falls 2.1 dB short, so Live's
    return line does take a share of the loop. In `reverb-network-taps` at Decay 60 s the left
    is now within 0.6 dB of Live's from 0.15 s at every Scale and Size, and within 0.3 dB past 0.7
    s (0.6 to 1.7 dB short before), the right within 0.5 dB from 0.7 s, and the right's 0.05 to
    0.1 s within 0.5 dB (1.5 short before). The right is still 0.7 to 1.1 dB short from 0.15 to 0.5
    s, where the paths that end through the 1641-frame line and then C are weak (Known
    differences). Over the `reverb-impulse` set 889 of 1,084 energy windows are within 1 dB (788
    before) and 648 within 0.5 dB (447), the median error 0.38 dB (0.61). Cost: three filters and three short lines.

## Known differences

Measured against Live's renders of the same sets imported and rendered by `suhina-cli render` at
44.1 kHz: energy in five windows of the impulse response (0–0.15, 0.15–0.5, 0.5–1, 1–2, 2–4 s),
the decay time overall and in octave bands, and the correlation of the sides.

Of the 93 cases with the device on (98 less the five bypasses), 13 are left out: Live's export ignored Size while Smooth was Fast, so the Size,
Diffusion and Scale cases of `reverb-structure`, `reverb-decay` and `reverb-texture` ran at Size
100 (`reverb-size` repeats them with Smooth None, and those are compared). Over the other 80
(2026-10-29):

| Measure | Within | Median |
|---|---|---|
| Energy of the impulse response in seven windows, 0 to 4.8 s (402 within 60 dB of the whole response) | 286 within 0.5 dB, 353 within 1 dB, 375 within 2 dB (249, 350 and 390 before the changes of 2026-10-29; 296, 351 and 378 before the network of Decisions 2 and 3) | 0.29 dB |
| The tail alone: the five windows from 0.15 s, Spin's cases included (229) | 191 within 0.5 dB, 221 within 1 dB (161 and 220 before the changes of 2026-10-29) | 0.28 dB |
| The tail's decay time, fitted from −5 to −25 dB and 10 dB clear of the floor (55 cases) | 55 within 10 % | 1.2 % |
| The same per octave, 125 Hz to 8 kHz (379 fits) | 320 within 10 % (337 of 377 before the changes of 2026-10-29) | 2.7 % |
| The tail's spectral ripple from 0.15 to 1.5 s (Decision 2), Density Low, Size 100 | 2.6 dB against Live's 2.6 (5.0 before) | |
| The onset's kurtosis in 10 ms windows, 100 to 150 and 150 to 200 ms, Density Low, Size 100 (Gaussian noise: 3) | 26 and 5.8 against Live's 15 and 10.5 (15.4 and 6.0 with the shorter allpasses of Decision 2) | |
| The spread of the gaps between strong clicks against their mean, 50 to 250 ms, at the defaults, Shape 0 and Shape 1 (Decision 2; clicks at random: 1) | 0.93, 0.93 and 1.01 against Live's 0.78, 0.79 and 1.05 (1.07, 1.25 and 1.28 before) | |
| The same at Density Sparse, 150 to 200 and 300 to 400 ms (Decision 17) | 36 and 7.1 against Live's 27 and 6.2 (5.6 and 2.9 with 4 lines) | |
| The tail's ripple at Size 1.53 and at Density Sparse (Decisions 11 and 17) | 4.0 and 2.2 dB against Live's 4.9 and 2.0 (8.3 and 5.6 before) | |
| Tap times and levels, reflection-only cases (416 checks) | 317 within 0.5 ms and 0.5 dB | |
| `compare.py` checks over all 80 (4515) | 1925 within tolerance (2008 before the changes of 2026-10-29, which lost mostly tap checks of small rooms; 1232 before the measured taps, 1881 before the network of Decisions 2 and 3); RT60 73 of 85 | |
| Whole cases within every tolerance | 3 (`predelay-2.5ms`, `-10ms`, `-50ms`), and the bypasses | |

`compare.py` compares the 16 strongest taps one by one, the largest difference anywhere in the
1/6-octave responses, the gain of a 1 kHz tone and the sine's stereo levels. In every case where
the tail is heard these depend on the exact delays of Live's network: its first arrivals are as
strong as the reflections (they are 12 of the 16 strongest peaks of the default settings), and a
single-frequency gain follows the network's modes. They fail until the network is copied too.

What is further off, and why it is left:

- **Live's network** was identified only in part: at Density Low, Diffusion 0.1 % and Size 100,
  the six pairs of direct taps (buses) feed six paths through an orthogonal 6×6 matrix, and each
  path is heard again in fixed later copies (about 9, 57 and 71 ms after its first, on alternate
  sides). Its feedback, its diffusers at other Diffusion settings and the other Densities were not
  worked out, so this device keeps its own network, tuned to Live's level, onset, decay and
  texture (Decisions 2 and 3).
  Its tail's strongest repeat, at Diffusion 0.1 % (where an allpass is a plain delay) and
  across Scale, is one loop of `(62.9 + 22.1 × scale) ms × size factor`: 17.8 ms at Size 1.53,
  33.6 at 10.54, 70.6 at 100 (Scale 40 %), with the allpass's own 22.1 × Scale ms showing too
  (2.2, 4.1 and 8.6 ms). So Live's lines and allpasses shrink with the size factor itself, not
  less in small rooms as this device's lines do (Decision 11), and one loop dominates: its
  repeat correlates 0.25 to 0.45, this device's strongest about 0.1 to 0.2. That loop sets the
  pitch of the ringing at Size 1.53 (about 56 Hz apart; this device's strongest repeat there is
  36.8 ms).
- **Live's right side from 0.15 to 1 s** (Decisions 21 and 23): 0.1 to 0.4 dB louder than
  Suhina's over the `reverb-impulse` set (0.5 to 0.9 dB before Decision 23), and 0.6 to 0.8 dB in
  the sparse `reverb-network-taps` renders. The paths there that end through the 1641-frame line
  and then C are weak: the 1335 family's through A (77 % of Live's), the 3190 mix's through B (63
  %) and the 2982 mix's through F (69 %), while the 1639 mix's straight through the 1641-frame
  line is right (within 12 %). So that line takes too little of A, B and F in the fitted mix,
  which, kept orthogonal, cannot take more without taking less elsewhere (D, at 0.64, is the
  candidate); not changed yet. The return
  line's feed from the loop lines (`RETURN_LOOP`) is still fitted, now at 0.85 of its first fit.
  At Size 1.53 and 10.54 the tail is louder than Live's, most at Diffusion 0.1 % (0.4 to 1.6 dB
  from 0.15 s, 0.3 to 0.6 dB more since Decision 23): the families were read at Size 100 only.
- **Timing of the filtered reflections' copies:** Live's paths through them come 4 to 6 frames
  after Suhina's (the 2982 family's through B and C 6 frames, the 1639 family's through A 4),
  as with the network's other responses (below).
- **The reflections are brighter while Spin moves them:** at Hi Cut off (`reverb-network-spin2`)
  the first 93 ms are 1.4 dB above Live's from 8 to 12 kHz and 2.2 dB above 12 kHz. Live's moving
  reflections arrive as smooth, nearly symmetric bumps about 5 frames wide, which this device's
  cubic reads do not reproduce.
- **Timing of the reflections and the network at Hi Cut off:** solved per input group (Decision
  22), Live's reflections come about 4 frames before this device's and its network's responses 3
  to 4 frames after them, in every group. The Spin and network runs before had Hi Cut on, whose
  delay (1.45 frames below 1 kHz in Live) may be part of what the fitted constants absorbed; a
  static render with Hi Cut off would tell.
- **The tail's echoes thicken differently from Live's:** its kurtosis is 26 from 100 to 150 ms,
  5.8 from 150 to 200 and 3.0 (noise) from 200 to 400 ms, Live's 15, 10.5 and 4; from 50 to 250
  ms it has about four fifths of Live's strong clicks per second, and from 150 to 900 ms a third
  to a half of its distinct ones; Live stays sparse longest at Diffusion 0.1 % (35 and
  11). Weaker allpass gains or longer allpasses in the loop moved the onset's sparsity but not its later
  course, so the remaining difference is in Live's network itself.
- **Small rooms' first echoes:** at Size 1.53 the tail's first arrivals, 58 to 75 ms, are about
  5 dB spikier than Live's and its tail sets in at about 45 ms, Live's at 30 (the lines shrink
  less than the room, Decision 11); `compare.py`'s 16 strongest peaks then pick them over some
  reflections.
- **The tail's first 0.3 s is 0.4 to 0.8 dB loud against the rest** in most cases (2 dB from 80
  to 100 ms), where each tap's first echo arrives at full level through its line: 37 of the 77
  windows from 0.15 to 0.3 s are within 0.5 dB (66 before), and 12 of 79 from 0.06 to 0.15 s
  (22). A gain cannot fix the shape; other orders of the taps into the lines fit it better but
  made the echoes' texture worse (Decision 2).
- **Short decays (0.2 s):** at Density Low the tail is within 1.7 dB of Live's in every window
  from 60 to 500 ms and decays in the same time (Decision 20); the other Densities still use the
  short-decay cuts of Decision 5. With the input band 0.5 wide
  Low's was 1.4 dB loud before Decision 20 (not checked since).
- **Smooth None against Fast at Decay 3.46 s:** Live's renders of the same settings with only
  Smooth changed differ: with None (`reverb-size`) the tail decays in 3.12 s and is up to 2 dB
  louder by 2.4 s than with Fast (`reverb-decay`, 3.01 s). This device follows Fast, the preset's
  setting, and Smooth changes nothing while Size stands still; so the four `decay-3.46s-size-*`
  cases end 0.9 to 2.4 dB low. At Decay 1.2 s the two agree within 0.3 dB.
- **Octave decay times at short decays:** at Decay 0.83 to 1.2 s the 250 Hz octave decays 11 to
  16 % faster than Live's and 1 and 2 kHz up to 6 % slower, in almost every case. Few modes fall in
  those octaves within the fit's half second, and the figure moves by up to half with the fit's
  range, so it was not tuned.
- **The highs of long and small-room tails** (Decision 8): Live's tail with Smooth Fast loses its
  8 to 16 kHz octaves fastest in its first few seconds and more slowly after, which a fixed shelf
  per trip cannot do. At Decay 60 s and in frozen tails the 8 kHz octave ends up to 2.8 dB and 16
  kHz up to 5 dB bright (2 to 7 s; 4 and 7 dB with the power 0.33); at Size 1.53 the 8 kHz octave
  now decays within 0.4 dB/s of Live's but stays 3 dB bright from its first second (Live's darkens
  by then), and 16 kHz decays 2 dB/s too fast.
- **Single cases off in level:** Scale 100 % at Size 1.53 is 0.6 to 0.7 dB loud throughout (Scale
  5 % is within 0.3 dB), as measured once; Size 500 is 2 dB quiet up to 0.24 s (below).
- **Density Sparse's onset is spikier than Live's** from 100 to 200 ms (kurtosis 76 and 36 against
  63 and 27) and has fewer clicks before 150 ms; at less diffusion the late tail matches
  (Decision 17).
- **Low Cut at high settings:** with the band's lower edge above about 500 Hz the lowest octaves
  are 15 to 25 dB louder than Live's: the shelf stops at −14 dB while Live's Lo Cut goes on
  falling. Its law is not understood (Decisions 9).
- **Spin (Decision 10).** Tracked tap by tap the reflections match: the swing of each of the
  clearest taps is within 0.02 ms of Live's at every predelay from 0.5 to 250 ms and at Amount
  17.5 and 55, its phase within about 0.1 rad (within 0.06 rad in `reverb-network-spin`, with each LFO taken
  when its reflection is read, where Live's rates at Spin Rate 0.3 and 0.37 Hz were 0.5 to 0.7 %
  above `SPIN_VOICES`'s for nine of the ten taps and 0.1 % for the one at 7.37 ms). A 330 Hz tone's pitch swing, reflections only (semitones peak to peak,
  Predelay 0.5, 2.5, 10, 25, 50, 100, 250 ms): 0.37, 0.41, 0.27, 0.06, 0.04, 0.03, 0.02 against
  Live's 0.44, 0.47, 0.29, 0.06, 0.04, 0.03, 0.02 at Amount 17.5, and 1.61, 1.77, 0.74, 0.38,
  0.21, 0.15, 0.11 against 1.81, 1.75, 0.61, 0.40, 0.26, 0.26, 0.15 at 55 (1.24 and 3.03 at every
  predelay before). That measure (the extremes of one side's frequency) is noisy: with the
  tail it read 0.74, 0.70, 0.30 against Live's 0.99, 1.10, 0.80 at 0.5, 2.5 and 10 ms, but the
  rms deviation over both sides, in cents, is 12.6, 13.7, 5.8, 1.6, 1.1 against 14.9, 15.8, 5.4,
  1.4, 1.0 at 0.5 to 50 ms (reflections alone 11.4, 11.9, 4.2, 1.5, 1.0 against 12.9, 12.9, 4.3,
  1.5, 1.0; the tail alone 12.8, 12.9, 7.1, 1.3 against 12.7, 14.0, 4.4, 1.4 to 25 ms). And
  Live's tail input swings by the same law as the reflections: the tail's first echoes
  (Diffusion 0.1 %, Reflect off, `reverb-spin-tail`) swing as their direct taps do, on the same
  LFOs, 1.24 ms against the law's 1.25 for the first tap's echo at 25 ms, and within 0.1 ms of
  ours wherever they can be tracked; swinging the input by its delay into the network as well
  does not fit them. Left: the tail alone at 10 ms swings more than Live's (7.1 against 4.4
  cents), and at Spin Rate 1.3 Hz Live's LFOs run 0.6 % faster than at 0.3 Hz. Every filtered tap
  moves with its direct one as Live's do (Decision 10); a few faint filtered taps of High are
  about 1 dB quieter than Live's, a matter of the measured gains, not of Spin. Away from 44.1 kHz
  and the reference room the offsets round to whole frames on their own, so a filtered tap can
  land one frame from where its whole delay would round (Live at other rates and Sizes was not
  read tap by tap).
- **Size 500:** about 2 dB too little from 60 to 240 ms (the tail sets in later than Live's).
- **The later runs** (Lo Cut law, network frequencies, Freeze, Smooth, Spin grid) were used for the
  laws above but not rendered through this device.

## Cost

One instance at its defaults: 2.8 to 3.2 % of one core at 48 kHz (60 s rendered in 1.7 to 1.9 s,
release build, Apple M-series, 2026-10-30); 2.4 % with Spin off, 2.3 % at Density Sparse, 4.0 %
at Mid and 4.7 % at High, measured with noise in; see Decision 16. Run `cargo test --release -p
suhina-device-reverb cost -- --ignored --nocapture` to measure it.

## Tests

`devices/reverb/src/tests.rs`: the device contract (`device_api::conformance`); the mixing matrix
orthogonal and full for every Density; the decay time at 1 and 3 s within 10 % at 44.1 and 96 kHz;
Density Low's decay time at 0.2 s within 12 % at Diffusion 0.1, 60 and 96 %
alike; the same level at both rates; Stereo's correlation at 0, 100 and 120; Reflect and Diffuse
independent; the dry signal undelayed and the mix equal-power; the first reflection three frames
after the predelay; the reflections' fade with Shape and Decay; Spin's reach, swing, restart
and filtered taps following one direct tap each; Sparse and Low each playing their own
reflections; Freeze holding the tail and Cut keeping new input out; silence decaying to exact zeros;
every control moved from one end to the other without a click.

`devices/reverb/src/golden_tests.rs`: golden renders of nine scenarios (the defaults, no
modulation, the densities, damping, Chorus and Spin, Reflect and Diffuse, Freeze with and without
Cut, a size change), compared within 1e-6 with the files in `devices/reverb/tests/golden`; its
README lists them and how to make them again when a change to the sound is intended.
