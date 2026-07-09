# Strophe

Strophe is a cross-platform loop recorder for building a session one turn at a
time. Its collaboration model is passing the mic around a circle: people add
parts asynchronously rather than trying to jam across a network.

It is a phrase sampler, not a DAW. A session is a small set of mono tracks;
recording appends a layer, and layers can be muted independently. The default
looper-pedal profile sums unmuted layers. The Deeler profile uses one selected
layer per track.

## Status

Pre-alpha. The Serval desktop host, framework-independent session model, and
Firewheel audio engine are working together for local record, playback, track
mute, solo, tempo, click, master-clock capture, and history-backed track
creation. Native Open and Save controls queue project work off the UI thread,
then persist or reopen a Redb-backed bundle through Muniment. New sessions begin
empty.

Not built yet: export, device selection, peer hand-off, and synchronization.
The pass-the-mic UI is deliberately local until those pieces exist.

Start with [design_docs/DOC_README.md](design_docs/DOC_README.md) for the
authoritative project and planning documents.

## Profiles

- Looper-pedal, the default: four tracks, layered overdub, optional master
  clock, and variable-length capture when the clock is off.
- Deeler: ten tracks, click-driven fixed-bar capture, and one selected layer
  per track.

Track count, phrase length, capture settings, and layer behavior are session
settings, not fixed product limits.

## Workspace

```
crates/
  strophe-engine/    Firewheel graph, capture, click, media-store abstraction
  strophe-model/     Framework-independent session, tracks, layers, history
  strophe-headless/  Scripted engine harness
  strophe-serval/    Serval/winit desktop host and one-screen recorder UI
design_docs/         Product reference and active implementation plans
```

`strophe-model` owns durable session truth. `strophe-engine` projects that
truth into a Firewheel runtime. `strophe-serval` owns host interaction and does
not become a second session model.

## Build

The workspace expects the sibling `woodshed` checkout because
`strophe-engine` uses its shared `audio-primitives` crate.

```text
cargo run -p strophe-serval
cargo run -p strophe-headless
cargo test -p strophe-model
cargo test -p strophe-engine
```

## License

Dual-licensed under Apache-2.0 or MIT, at your option.
