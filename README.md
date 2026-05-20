# Strophe

A cross-platform loop recorder with turn-based collaboration. The
digital equivalent of passing a mic around in a circle and building
loops turn by turn.

Inspired by [Deeler](https://new.tapeop.com/interviews/47/menomena), the
Max/MSP patch the band Menomena built and used to write their albums.
Strophe takes that workflow — a configurable number of mono tracks
(ten by default), each holding a configurable number of phrase
variations (four by default, A/B/C/D), sequential overdubbing against
a click — and grounds it in modern infrastructure: peer-to-peer session
sync, content-addressed media, nondestructive history.

## Status

Pre-alpha scaffold. See [`design_docs/`](design_docs/) for the project
description, policy, and active plan.

## Layout

```
crates/
  strophe/           The Xilem application — the binary users run
  strophe-engine/    Audio engine: Firewheel graph, click, capture, layers
  strophe-model/     Session model: tracks, layers, history graph
  strophe-widgets/   Masonry custom widgets: waveform, track strip, transport
  strophe-headless/  Headless audio-engine test harness (scripted demos)
design_docs/         Plans, policy, project description
```

## Sibling project

Strophe shares audio infrastructure with [Woodshed](../woodshed/), the
guitarist's practice toolkit. Woodshed's `woodshed-audio` crate
provides the looper, click engine, calibration, MIDI, and onset
detection that Strophe consumes via path-dep.

## Build

```
cargo build
cargo run -p strophe
```

## License

Dual-licensed under either of:

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
