---
description: The built-in algorithmic reverb - its parameters, signal flow (input band, early reflections, a damped feedback delay network, chorus, freeze, stereo), how Live's Reverb imports into it, and how close it comes to Live's renders
read_when: reverb, algorithmic reverb, room, decay, early reflections, predelay, feedback delay network, FDN, damping, freeze, diffusion, importing Live's Reverb, Reverb elements
type: spec
status: current
last_verified: 2026-10-28
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
                                                              +-> Shape delay -> direct taps -> 4 allpasses (Diffusion, Scale)
                                                                  -> feedback delay network (Size, Decay,        |
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
  reference room times `(size / 100)^(1/3)`, as for a room's volume. Smooth glides that factor
  with a time constant of 45 ms (Slow) or 15 ms (Fast), or not at all (None), which makes Size
  changes glitch as in Live.
- **Early reflections.** Live's own tap pattern for each Density (10 direct taps and 60 filtered
  ones over 129 ms at Low; 4, 10 and 10 direct taps at Sparse, Mid and High, each table read up
  to 67 ms), at whole frames of 44.1 kHz in the reference room, each on one side with its sign and
  level. The first tap is 3 frames (at 44.1 kHz) after the predelay, with a gain of 0.0942. The
  filtered taps pass, per side, a cookbook low-pass of Q 0.508 at `12 kHz − 818 Hz × decay^−0.886`
  (8.5 kHz at 0.2 s, 11.3 kHz at 1.2 s). Every tap fades by `(0.0328 + 0.0668 × shape) / decay`
  dB per ms of its delay (decay in seconds), and the whole wet signal, tail included, is
  0.0565 dB quieter per ms of predelay. Spin stretches every tap's delay, predelay included, by
  `1 + 0.0097 × amount × sin(2π × rate × t)`: all taps in step, the swing a fraction of each
  delay.
- **Network input.** The direct taps again, read after the Shape delay of `32 + (18.5 + 34.83 ×
  (shape − 0.5)) × size factor` ms, each at its level and fade (so Shape and Decay shape the tail's
  onset as they shape the reflections) and normalised by their number; then four Schroeder
  allpasses in series with the Diffusion as their gain and delays of 1.41, 2.19, 3.33 and 5.07 ms
  times Scale and the size factor. Its level is `1.194 × 10^(−(0.708 / decay + 0.12 / decay²) / 20)`,
  and the taps feeding it take their fade over 20 ms more than their own delay (so the tail falls
  with Shape twice as much as the reflections' first part does, as Live's does); in rooms
  larger than the reference it rises with the size factor to the power 0.75, and Density Sparse
  and High are trimmed by −4 and +1 dB. It then passes the loop's damping (below) once, at 0.21 of
  a trip. Freeze with Cut on fades it out (20 ms).
- **Network.** 4, 8, 12 or 16 lines (Density), 18 to 77 ms long in the reference room, mixed after
  every trip by an orthogonal matrix in which every line feeds every other one equally (a Hadamard
  matrix, for 12 lines a Hadamard matrix of 4 within three groups and a Householder reflection
  across them, followed by a shift of one line). The input goes into every line equally, so the
  tail sets in at its full level about 70 ms after the first reflection, as Live's does. Each line's gain per trip is
  `10^(−3 × length / (0.9 × decay))`, so every line decays in the same time whatever its length:
  the tail falls 60 dB in 0.9 of the Decay setting, as Live's mid bands do.
- **Damping.** In each line, after its gain: High Damping as a second-order high shelf (the
  cookbook shelf, Q 0.707, half way at the frequency) or a first-order low-pass at the frequency;
  Low Damping as a second-order low shelf (the same, below the frequency). Their gain is a linear factor per trip round a
  0.17 s loop, raised to the power of the line's length (in the room as it is) over 0.17 s, so
  the loss per second does not depend on Size. A fixed loss of the highs is
  always in the loop: a first-order high shelf half way at 8 kHz, 0.96 dB per 0.17 s trip, less for
  decays over 3.46 s (by the ratio to the power 0.27). Lines are whole frames long once Size and
  Decay have settled, so a line without chorus is not interpolated.
- **Chorus.** Swings every line's delay either way by up to `1.8 ms × amount` (at most half the
  line), each with its own phase of one LFO at the Chorus rate; lines are read with cubic
  interpolation.
- **Output.** The left and right tail are sums of the lines with two orthogonal sign patterns
  chosen so lines of similar length carry the same energy into each side: the two sides come out
  uncorrelated. The reflections (times Reflect) and the tail (times Diffuse) are added and rotated
  by `β = 45° × (1 − stereo / 120)`: `L' = cos β L + sin β R`, `R' = sin β L + cos β R`. So 120 keeps
  them apart, 100 gives a correlation of 0.26 and 0 makes them mono. Dry/Wet is an equal-power
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

1. **A feedback delay network of 8 lines (Density Low) with input allpasses and Live's
   early-reflection taps,** over a Schroeder reverb of combs: a denser, smoother tail with the
   same decay at every frequency the damping does not touch, in the same class as Live's (whose
   manual describes filters and modulated delay lines). Cost: see 16. Dragonfly Reverb (GPL-3.0),
   the ROADMAP's earlier candidate, was not ported: its algorithms and controls do not map onto
   Live's stages one to one, and an own network of textbook parts, tuned to the measurements, is
   smaller and needs no port.
2. **Hadamard mixing (with a shift) over a Householder reflection.** At 8 and more lines a
   Householder reflection sends most of each line back into itself (three quarters at 8 lines),
   so the lines keep unequal energy and the echo density builds slowly; a Hadamard matrix sends
   every line into every other one equally. Cost: N·log N additions instead of 2N.
3. **The two output sign patterns** are chosen so lines of similar length enter the sides once
   with the same and once with opposite signs. With plain alternating signs the sides came out
   correlated (−0.18 against Live's +0.22 at the default Stereo, which is Live's uncorrelated tail
   after its rotation).
4. **Cubic interpolation for the taps, the diffusers and the lines.** Linear interpolation
   would save about 0.8 % of a core but dulls and modulates the highs while Spin or Chorus moves.
   Without Spin, and with nothing moving, the taps are read at whole frames.
5. **The tail is tuned to 0.9 of the Decay setting** and its level follows
   `1.194 × 10^(−(0.708 / decay + 0.12 / decay²) / 20)` with the taps' fade over 20 ms more
   (Network input): the decay is what Live's renders show (0.87 to 0.9 of the setting from 0.8 to
   3.5 s), and the level law is fitted to the steady level of Live's tail from 0.3 s on in every
   case (0.2 s and later for Decay 0.2 s), where it leaves 65 of 68 within 0.5 dB. A level law
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
   line. The low-pass type is first-order, as measured.
7. **A fixed 90 Hz roll-off at the input,** first-order, the simplest that brings the lowest two
   octaves within about 2 dB of Live's in every case (they were 3 to 8 dB too loud without it).
8. **The fixed loss of the highs** (an 8 kHz shelf, 0.96 dB a trip, less above 3.46 s) is there
   because Live's tail is shorter above 2 kHz even with both filters off. Above 3.46 s it shrinks
   with the decay to the power 0.27: Live's own extra loss at 8 kHz goes nearly as the cube root,
   but the shelf scaled by the cube root left the 2 to 8 kHz octaves 10 % long at Decay 60 s;
   with 0.27 they are 2 to 7 % long.
9. **Low Cut is a 14 dB first-order shelf** half way at the band's lower edge. Live's Lo Cut fits
   no simple filter (3 to 8 dB/octave, flattening at the bottom); at 1 kHz, width 5.85 the shelf is
   within 0.6 dB rms of its measured response (1.3 dB at worst, at 57 Hz), and at 5 kHz it is
   0.6 dB above Live's −3 dB point and 1.3 dB above its −6 dB point.
10. **Spin stretches the whole reflection pattern** by 0.97 % of every delay per unit of Amount,
   all taps in step, as tracked tap by tap in Live's renders (34 % peak to peak at the default
   17.5, the same at 0.3 and 0.6 Hz, both sides together). Chorus swings every line's delay
   either way by 1.8 ms per unit of Amount, each line with its own phase; that figure comes from
   the FM of a tone through the tail, which overstated Spin's swing, and is kept until Chorus is
   tracked line by line.
11. **Level corrections for large rooms and for Density** (the size factor to the power 0.75
    above the reference room; −4 dB for Sparse, +1 dB for High) are fitted to Live's renders: a
    larger network spreads the same energy over longer lines and comes out quieter, which Live's
    does not.
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
   on, and the tail's broadband decay read 15 % short.
14. **Smooth's glide times (45 ms Slow, 15 ms Fast)** are what one point of Live's glide gives for
   an exponential glide (`reverb-smooth-glide`: 50 ms after a jump Fast has finished, Slow has 30
   to 40 % left); the curve itself was not measured.
15. **Freeze, Flat and Cut** follow Live's renders: a frozen tail drains 0.25 dB per second with
   Flat on and 0.4 with Flat off, and Cut keeps new input out entirely. The soft limit with Cut
   off is Suhina's, so a frozen tail fed for minutes cannot overflow.
16. **Cost: about 3.3 % of a core at the defaults,** over the 1 % budget, as the device notes allow
    for reverbs: up to 82 taps (70 at Low; cubic while Spin moves them) and 8 modulated lines
    with three to four filters each run every frame. Spin off saves a third (2.1 %); Density Sparse costs 2.8 %, High 4.5 %.
    Cutting further would mean fewer taps or linear interpolation (Decision 4).

## Known differences

Measured against Live's renders of the same sets imported and rendered by `suhina-cli render` at
44.1 kHz: energy in five windows of the impulse response (0–0.15, 0.15–0.5, 0.5–1, 1–2, 2–4 s),
the decay time overall and in octave bands, and the correlation of the sides.

Of the 93 cases with the device on (98 less the five bypasses), 13 are left out: Live's export ignored Size while Smooth was Fast, so the Size,
Diffusion and Scale cases of `reverb-structure`, `reverb-decay` and `reverb-texture` ran at Size
100 (`reverb-size` repeats them with Smooth None, and those are compared). Over the other 80
(2026-10-28):

| Measure | Within | Median |
|---|---|---|
| Energy of the impulse response in seven windows, 0 to 4.8 s (402 within 60 dB of the whole response) | 296 within 0.5 dB, 351 within 1 dB, 378 within 2 dB (197, 309 and 370 before the tail's level was refitted) | 0.28 dB |
| The tail alone: the five windows from 0.15 s, Spin's cases included (229) | 197 within 0.5 dB, 219 within 1 dB (75 and 162 before) | 0.28 dB |
| The tail's decay time, fitted from −5 to −25 dB and 10 dB clear of the floor (55 cases) | 55 within 10 % | 1.8 % |
| The same per octave, 125 Hz to 8 kHz (368) | 291 within 10 % | 3.5 % |
| Tap times and levels, reflection-only cases (416 checks) | 317 within 0.5 ms and 0.5 dB | |
| `compare.py` checks over all 80 (4512) | 1881 within tolerance (1232 before the measured taps, 1860 before the tail's level was refitted); RT60 72 of 85 | |
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
  worked out, so this device keeps its own network, tuned only to Live's level, onset and decay.
- **Short decays (0.2 s):** Live's tail sets in at once with a strong first burst; this device's
  builds up over 15 ms, so it is 4 to 5 dB quieter from 60 to 90 ms and 1.3 to 1.9 dB louder from
  90 to 150 ms. From 150 ms on it is within 0.5 dB, apart from the case with the input band 0.5
  wide (+2 dB).
- **Smooth None against Fast at Decay 3.46 s:** Live's renders of the same settings with only
  Smooth changed differ: with None (`reverb-size`) the tail decays in 3.12 s and is up to 2 dB
  louder by 2.4 s than with Fast (`reverb-decay`, 3.01 s). This device follows Fast, the preset's
  setting, and Smooth changes nothing while Size stands still; so the four `decay-3.46s-size-*`
  cases end 0.9 to 2.4 dB low. At Decay 1.2 s the two agree within 0.3 dB.
- **Octave decay times at short decays:** at Decay 0.83 to 1.2 s the 250 Hz octave decays 11 to
  16 % faster than Live's and 1 and 2 kHz up to 6 % slower, in almost every case. Few modes fall in
  those octaves within the fit's half second, and the figure moves by up to half with the fit's
  range, so it was not tuned.
- **Single cases off in level:** Density Sparse is 1.1 dB loud from 0.15 to 0.3 s (its onset);
  Scale 100 % at Size 1.53 is 0.8 dB loud throughout (Scale 5 % is within 0.4 dB), as
  measured once; Size 500 is 2 dB quiet up to 0.24 s (below).
- **Low Cut at high settings:** with the band's lower edge above about 500 Hz the lowest octaves
  are 15 to 25 dB louder than Live's: the shelf stops at −14 dB while Live's Lo Cut goes on
  falling. Its law is not understood (Decisions 9).
- **Spin at large amounts:** the stretched reflections end later than Live's: +6 dB at Amount
  17.5 and +15 dB at 55 in 150 to 300 ms with Diffuse at its minimum; the LFO's phase at the
  impulse is not known.
- **Size 500:** about 2 dB too little from 60 to 240 ms (the tail sets in later than Live's).
- **The later runs** (Lo Cut law, network frequencies, Freeze, Smooth, Spin grid) were used for the
  laws above but not rendered through this device.

## Cost

One instance at its defaults: 3.3 % of one core at 48 kHz (60 s rendered in 2.0 s, release build,
Apple M-series, 2026-10-28); before Live's tap tables, 2.1 % with Spin off, 2.8 % at Density
Sparse and 4.5 % at High (not measured again); see Decision 16. Run `cargo test --release -p suhina-device-reverb cost
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
