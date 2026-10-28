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
  renders (Decisions 12); the network's delay lengths and the other constants are this device's
  own, tuned to the measured laws below.

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
- **Early reflections.** Live's own tap pattern for each Density (10 direct taps and 60 filtered
  ones over 129 ms at Low; 4, 10 and 10 direct taps at Sparse, Mid and High, each table read up
  to 67 ms), at whole frames of 44.1 kHz in the reference room, each on one side with its sign and
  level. The first tap is 3 frames (at 44.1 kHz) after the predelay, with a gain of 0.0942. The
  filtered taps pass, per side, a cookbook low-pass of Q 0.508 at `12 kHz − 818 Hz × decay^−0.886`
  (8.5 kHz at 0.2 s, 11.3 kHz at 1.2 s). Every tap fades by `(0.0328 + 0.0668 × shape) / decay`
  dB per ms of its delay (decay in seconds), and the whole wet signal, tail included, is
  0.0565 dB quieter per ms of predelay. Spin stretches every tap's delay, predelay included, by
  `1 + 0.0097 × amount × sin(2π × rate × t + φ)`: the swing a fraction of each delay, each tap at
  its own golden-angle phase φ, and the network's input taps with their reflections.
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
- **Network.** 8, 8, 12 or 16 lines (Density Sparse, Low, Mid, High), 18 to 77 ms long in the
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
  decays over 3.46 s (by the ratio to the power 0.15). Lines and their allpasses are whole frames
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
- **Freeze.** The damping and the fixed loss of the highs are left out and every line's gain is
  set so the tail falls 0.25 dB per second with Flat on and 0.4 dB per second with Flat off, as
  Live's frozen tail drains. Cut decides whether
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
10. **Spin stretches every reflection** by 0.97 % of its delay per unit of Amount, as tracked tap
   by tap in Live's renders (34 % peak to peak at the default 17.5, the same at 0.3 and 0.6 Hz),
   **each at its own phase, and the tail's input with it.** In Live's impulse with Spin, against
   the one without, the clearest taps are stretched by 1.04 to 1.21 at the same moment, and left
   and right differently (1.18 and 1.08 at 20 ms), so the taps do not swing in step. And Live's
   tail with Spin is uncorrelated with its tail without (r −0.04 after the noise; ours was 0.99
   while only the reflections swung): Spin reaches the network's input. Swinging the input taps
   too brought the tail of the Spin cases from +6.2 to +0.5 dB at Amount 17.5 in 150 to 300 ms,
   and from +15.1 to +9.4 dB at 55. Cost: about 0.2 % of a core with Spin on. Chorus swings every line's delay
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
   reflections is a whole-frame delay of the input, either as it is or through one low-pass whose
   corner follows Decay; their times scale with the cube root of Size, and with the fade taken out
   the direct taps all have a level of exactly ±1. So the pattern was read tap by tap from the
   impulse responses (deconvolved from the input filters) and is used as measured, which puts
   every tap of the reflection-only cases within 0.5 ms and 0.5 dB of Live's. This reverses the
   earlier choice of an own pattern that only kept Live's laws: the per-tap checks of the
   comparison cannot pass without the real times, and the pattern is a measured behaviour of the
   device, read from its output alone. Only the part before Live's tail sets in (67 ms) could be
   read for Sparse, Mid and High. The network's line lengths and diffusers stay this device's own:
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
   Flat on and 0.4 with Flat off, and Cut keeps new input out entirely. The soft limit with Cut
   off is Suhina's, so a frozen tail fed for minutes cannot overflow.
16. **Cost: about 3.2 % of a core at the defaults,** over the 1 % budget, as the device notes allow
    for reverbs: up to 82 taps (70 at Low; cubic while Spin moves them) and 8 modulated lines
    with an allpass and three to four filters each run every frame. Spin off saves a fifth
    (2.2 %, measured on 2026-10-28); Density Sparse costs 1.6 %, High 4.1 %. Cutting further would mean fewer taps or
    linear interpolation (Decision 4).

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
- **Short decays (0.2 s):** the tail is within 2 dB of Live's up to 250 ms and 1 to 3 dB quiet
  from 300 to 600 ms (where Live's is already 50 to 150 dB down), and 1.4 dB loud with the input band 0.5
  wide.
- **Smooth None against Fast at Decay 3.46 s:** Live's renders of the same settings with only
  Smooth changed differ: with None (`reverb-size`) the tail decays in 3.12 s and is up to 2 dB
  louder by 2.4 s than with Fast (`reverb-decay`, 3.01 s). This device follows Fast, the preset's
  setting, and Smooth changes nothing while Size stands still; so the four `decay-3.46s-size-*`
  cases end 0.9 to 2.4 dB low. At Decay 1.2 s the two agree within 0.3 dB.
- **Octave decay times at short decays:** at Decay 0.83 to 1.2 s the 250 Hz octave decays 11 to
  16 % faster than Live's and 1 and 2 kHz up to 6 % slower, in almost every case. Few modes fall in
  those octaves within the fit's half second, and the figure moves by up to half with the fit's
  range, so it was not tuned.
- **Single cases off in level:** Scale 100 % at Size 1.53 is 0.6 to 0.7 dB loud throughout (Scale
  5 % is within 0.3 dB), as measured once; Size 500 is 2 dB quiet up to 0.24 s (below).
- **Density Sparse's onset is spikier than Live's** from 100 to 200 ms (kurtosis 76 and 36 against
  63 and 27) and has fewer clicks before 150 ms; at less diffusion the late tail matches
  (Decision 17).
- **Low Cut at high settings:** with the band's lower edge above about 500 Hz the lowest octaves
  are 15 to 25 dB louder than Live's: the shelf stops at −14 dB while Live's Lo Cut goes on
  falling. Its law is not understood (Decisions 9).
- **Spin at large amounts:** the stretched reflections end later than Live's: +9 dB at Amount
  55 in 150 to 300 ms with Diffuse at its minimum (+0.5 dB at 17.5); the LFO's phase at the
  impulse, and each tap's, are not known.
- **Size 500:** about 2 dB too little from 60 to 240 ms (the tail sets in later than Live's).
- **The later runs** (Lo Cut law, network frequencies, Freeze, Smooth, Spin grid) were used for the
  laws above but not rendered through this device.

## Cost

One instance at its defaults: 3.2 % of one core at 48 kHz (60 s rendered in 1.9 s, release build,
Apple M-series, 2026-10-29; 2.8 % on 2026-10-28); 2.2 % with Spin off (before them),
1.6 % at Density Sparse (1.2 % with 4 lines) and 4.1 % at High, measured the same way with noise
in; see Decision 16. (The earlier 3.3 % was measured on a busier machine.) Run `cargo test --release -p suhina-device-reverb cost
-- --ignored --nocapture` to measure it.

## Tests

`devices/reverb/src/tests.rs`: the device contract (`device_api::conformance`); the mixing matrix
orthogonal and full for every Density; the decay time at 1 and 3 s within 10 % at 44.1 and 96 kHz
alike; the same level at both rates; Stereo's correlation at 0, 100 and 120; Reflect and Diffuse
independent; the dry signal undelayed and the mix equal-power; the first reflection three frames
after the predelay; the reflections' fade with Shape and Decay; Freeze holding the tail and Cut keeping new input out; silence decaying to exact zeros;
every control moved from one end to the other without a click.

`devices/reverb/src/golden_tests.rs`: golden renders of nine scenarios (the defaults, no
modulation, the densities, damping, Chorus and Spin, Reflect and Diffuse, Freeze with and without
Cut, a size change), compared within 1e-6 with the files in `devices/reverb/tests/golden`; its
README lists them and how to make them again when a change to the sound is intended.
