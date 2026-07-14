# Hocket

Hocket is a cross-platform loop recorder for building a session one turn at a
time. Its collaboration model is passing the mic around a circle: people add
parts asynchronously rather than trying to jam across a network.

It is a phrase sampler, not a DAW. A session is a small set of mono tracks;
recording appends a layer, and layers can be muted independently. The default
looper-pedal profile sums unmuted layers. The Deeler profile uses one selected
layer per track.

## Status

Pre-alpha. The Genet desktop host, framework-independent session model, and
Firewheel audio engine are working together for local record, playback, track
mute, solo, tempo, click, master-clock capture, and history-backed track
creation. Native Open and Save controls queue project work off the UI thread,
then persist or reopen a Redb-backed bundle through Muniment. New sessions begin
empty.

Summed and per-layer Chisel waveforms now project real stored samples through a
content-addressed cache, and unavailable media is labeled instead of replaced by
a generated silhouette. Output meters use configurable shared attack, release,
peak-hold, and peak-decay ballistics.

Not built yet: peer hand-off and synchronization.
The pass-the-mic UI is deliberately local until those pieces exist.

The engine now has a signed, recipient-addressed hand-off envelope for a
complete project snapshot and its media, plus a transactional same-root
branch-acceptance rule. Envelope v2 identifies the durable sender that
authorized its session key. It is transport-neutral groundwork only: the
desktop host cannot send, receive, review, or accept one yet, and raw envelope
bytes are not encrypted.

On Windows, the desktop host now restores one durable local identity from a
`personae` sealed record protected by DPAPI and shows its short public
fingerprint in the circle. The identity remains outside project files. Other
platforms report identity unavailable until their OS unlock backend exists.

History now retains divergent branches and can integrate a same-root remote
graph without replacing local work. It does not yet reconcile conflicting edits
into a new merged head.

Audio input and output can be selected per launch from the transport. Those are
host settings rather than project state; device preference persistence and
hot-plug refresh remain follow-ups.

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
  hocket-engine/    Firewheel graph, capture, click, media-store abstraction
  hocket-model/     Framework-independent session, tracks, layers, history
  hocket-headless/  Scripted engine harness
  hocket-genet/    Genet/winit desktop host and one-screen recorder UI
design_docs/         Product reference and active implementation plans
```

`hocket-model` owns durable session truth. `hocket-engine` projects that
truth into a Firewheel runtime. `hocket-genet` owns host interaction and does
not become a second session model.

## Build

The workspace expects the sibling `woodshed` checkout because
`hocket-engine` uses its shared `audio-primitives` crate.

```text
cargo run -p hocket-genet
cargo run -p hocket-headless
cargo test -p hocket-model
cargo test -p hocket-engine
```

## License

Dual-licensed under Apache-2.0 or MIT, at your option.
