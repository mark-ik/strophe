# Strophe

A cross-platform loop recorder with turn-based collaboration. The digital
equivalent of passing a mic around in a circle and building loops turn by turn.

Inspired by [Deeler](https://new.tapeop.com/interviews/47/menomena), the Max/MSP
patch the band Menomena built and used to write their albums. Strophe takes that
workflow (mono tracks, sequential overdubbing against an optional click) and
grounds it in modern infrastructure: peer-to-peer session sync, content-addressed
media, and nondestructive history.

The product is deliberately a phrase sampler, not an Ableton-shaped DAW. A
session is a small set of mono tracks; each track is an append-only stack of
captured layers that sum at playback (the looper-pedal / overdub model). Songs
emerge by muting layers in and out and exporting per-track or as a mix.

**Made with AI**

## Status

Pre-alpha. The workspace scaffold, session model, Firewheel-backed audio engine,
and a Xilem application shell are in place. Recent work has landed runtime tempo
control, count-in, free (unclocked) variable-length capture, per-layer looper
waveforms, and undo/redo over the nondestructive history graph. Collaboration
(peer-to-peer hand-off) is described in the plans but not yet implemented.

See [`design_docs/`](design_docs/) for the project description, documentation
policy, and the active plan. Start with
[`design_docs/DOC_README.md`](design_docs/DOC_README.md).

## Configuration philosophy

Track count, layer semantics, bar length, count-in behavior, and click
preference are session-level settings, not hardcoded limits. The model stores
counts explicitly so widening them is a config change, not a refactor.

Two named profiles frame the v1 target:

- **Looper-pedal profile** (the v1 default): 4 tracks, layered overdub,
  variable-length loops, optional master clock.
- **Deeler profile** (a named preset): 10 tracks, fixed bar length, click-driven,
  for users who want exactly the Menomena workflow.

## Tech stack

- **Language**: Rust, edition 2024, `rust-version` 1.92.
- **UI**: Xilem + Masonry, painted with Vello. Pinned to the `mark-ik/xilem` fork
  (`woodshed-theme` branch) so Strophe and its sibling apps share one local
  checkout and theme.
- **Audio engine**: [Firewheel](https://github.com/BillyDM/Firewheel) 0.10 from
  crates.io (features: `cpal`, `symphonium`, `peak_meter_node`, `stream_nodes`).
  The engine is a Firewheel graph: a click sampler, a pool of voice sampler
  nodes, a mic-input capture tap, and a post-mix peak meter.
- **Realtime-safe primitives**: provided by Firewheel internally (the
  workspace declares `rtrb`/`basedrop` for future direct use, but they are not
  currently consumed by any crate).
- **Content hashing**: `blake3` for content-addressed media (engine-side).
- **Persistence**: `postcard` (with `ciborium`/CBOR planned later to align with
  Moothold).
- **Async runtime**: `tokio` on the UI side only, never on the audio thread.

## Workspace layout

The repository is a Cargo workspace with five member crates under `crates/`.

```
crates/
  strophe/           Xilem + Masonry application shell (the binary users run)
  strophe-engine/    Audio engine: Firewheel graph, click, capture, media store
  strophe-model/     Session data model: tracks, layers, phrases, history graph
  strophe-widgets/   Strophe-specific Masonry widgets + re-exports of shared ones
  strophe-headless/  Headless audio-engine test harness (scripted demo, no UI)
design_docs/         Project description, doc policy, active plans
```

Crate roles:

- **`strophe`** — the application binary. `AppState` owns the authoritative
  `Session` + `History` + a content-addressed media store alongside the audio
  `Engine`. The UI is organized into surfaces (`view/`): a persistent transport
  bar plus one of Tracks / Combination / Settings.
- **`strophe-engine`** — the audio runtime. `strophe-model` is the authority for
  session truth; the engine plays/captures/mixes the projected state through a
  Firewheel graph. Modules: `capture`, `click`, `media`.
- **`strophe-model`** — framework-agnostic session model. No cpal, xilem, masonry,
  or winit dependencies. Modules: `ids`, `phrase`, `track`, `session`, `history`,
  `persistence`. Defaults: 4 tracks, variable-length layers, 4 bars per phrase,
  120 BPM, 4/4.
- **`strophe-widgets`** — Strophe-specific Masonry/Vello widgets. The waveform,
  peak, and meter widgets were extracted to the shared `audio-widgets` crate and
  are re-exported here; the `combobox` from `xilem-components` is re-exported too,
  so call sites keep using `strophe_widgets::...` unchanged.
- **`strophe-headless`** — a scripted, UI-free demo of the engine for validating
  bar-aligned capture and playback end-to-end.

## Relationship to sibling repos

Strophe is the **pressure vessel for the reusable audio layer** in the Strophos
family. Hard audio engineering happens here first; when a piece stabilizes it is
extracted into a shared crate that other apps consume.

Strophe currently consumes three shared crates from the sibling
[Woodshed](../woodshed/) repo via cross-repo path-deps:

- `audio-primitives` — pure-DSP cores (click synth, onset/tempo, latency
  estimation), `strophe-engine`'s click loop calls the shared synth.
- `audio-widgets` — shared audio-domain widgets (waveform + peaks + meter),
  re-exported by `strophe-widgets`.
- `xilem-components` — domain-neutral UI components (combobox, ...) shared across
  the Xilem apps.

Note: Strophe no longer depends on Woodshed's `woodshed-audio` umbrella crate.
The earlier `SequencerEngine` wrap was retired when the engine moved to a
Firewheel graph. Dependency direction is one-way: Strophe and the other apps
depend on the shared audio crates, never on each other.

P2P session sync is planned over Moothold + Murm (other Strophos-family
projects); it is not yet wired up.

## Build and run

This is a workspace, so it expects the sibling `woodshed` repo checked out
alongside it (the shared audio crates are referenced by relative path
`../woodshed/...`).

```
cargo build

# Launch the application
cargo run -p strophe

# Run the scripted, UI-free engine demo (turn speakers down: feedback risk)
cargo run -p strophe-headless

# Tests
cargo test
```

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
