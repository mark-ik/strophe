# design_docs Index

Canonical first-reference document for project documentation. Read this
before any other doc in this directory.

## Project Reference Docs

- [PROJECT_DESCRIPTION.md](PROJECT_DESCRIPTION.md) — Product goals,
  major features, scope. Maintainer-owned (passed by Mark 2026-05-18).
- [DOC_POLICY.md](DOC_POLICY.md) — Documentation governance.

## Active Plans

- [2026-05-18_initial_plan.md](2026-05-18_initial_plan.md) — Initial
  scaffold, workspace skeleton, and feature-target ladder from
  click-track engine through P2P session hand-off.

## Archive

- `archive_docs/` — retired plans and superseded notes (created on
  first archive).

## Working Principles for AI Assistants

These principles apply to AI-assisted work on this project. Update this
section whenever a durable working insight emerges from a session.

- **The model is framework-agnostic.** `strophe-model` does not depend
  on cpal, xilem, masonry, or any UI/audio framework. The audio
  engine, the UI, and the sync layer all consume it as a peer.
- **Async-first collaboration**, never real-time multiplayer jamming.
  The product's identity is sequential turn-taking over Moothold; that
  shape must not erode into "everyone plays together over the net" as
  a primary mode.
- **Hand-on-instrument flow.** Borrowed from Deeler's design north
  star. UI decisions are evaluated against whether they let the
  musician keep their hands on the instrument. Big buttons, minimal
  modes, count-in countdowns, no clicking-around-to-arm-a-track
  ceremonies.
- **Defaults, not limits.** Ten tracks, four variations, four-bar
  phrase length, click-driven recording — these are *defaults* per
  `PROJECT_DESCRIPTION.md`'s configurability framing. Initial target
  is parity with Deeler; the model should be shaped so widening the
  parameters later (12 tracks, 8 variations, different phrase
  lengths) is a session-config change, not a refactor. Use `Vec`
  and runtime-known counts in the model — never `[T; 10]`.
- **"Pass the mic" is the north-star metaphor.** The collaboration
  model is the digital equivalent of passing a mic around in a circle
  and building loops turn by turn. UI and protocol decisions should
  honor that — sequential, polite, asynchronous. Latency is not our
  friend; we don't fight it, we route around it.
- **Strict cap doctrine — loop-first, not DAW.** Every proposed
  feature must answer: *does this serve the passing-the-mic
  workflow?* If yes, consider taking it. If it serves traditional
  DAW workflows instead, defer it or skip it. The arrange view is
  the single feature that could turn Strophe into something a person
  uses *instead of* a DAW — add it deliberately and only after the
  loop-recorder identity is solid. The failure mode to avoid: "loop
  recorder that grew DAW features and now is worse than both."
- **Firewheel is the audio substrate.** Not Dropseed (back-burner),
  not a SequencerEngine wrap (placeholder from FT1). At FT3b-prime
  the engine moves to a Firewheel graph: loop tracks as sample-player
  nodes, mix bus, input capture node, optional click node. **On
  crates.io** at `firewheel-graph` 0.10.x — pin a version, only
  path-dep if unreleased fixes are needed.
- **strophe-model is the authority; Firewheel is the runtime.**
  Session truth lives in the model; the audio graph plays the
  *projected state*. Don't let runtime concerns leak back into the
  model (no transport-dominance, no graph topology bleed-through).
- **No plugin hosting for v1.** No CLAP, no VST3, no LV2, no AU, no
  plugin GUI hosting. Curated first-party Rust devices on Firewheel
  nodes with schema-driven UI generated from parameter metadata.
  Drags DAW-shaped product gravity if we say yes; preserves
  loop-recorder identity if we say no. v1 says no.
- **Layered tracks, not variation slots.** A track is a stack of
  layers (each layer = one captured sample). Variations as a separate
  concept do not exist; multiple takes are muteable layers.
- **Local-only transport for v1.** Each peer has its own playback
  head. Recording lock prevents two peers from capturing into the
  same track on the same turn; CRDTs apply to session structure
  (track existence, turn ownership, mute/solo, locked BPM), never to
  audio data.

## Reference reading

Hand-picked technical references that inform Strophe's architecture:

**Realtime audio engineering:**
- Bencina, ["Real-time audio programming 101: time waits for nothing"](http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing)
- Bencina, ["Interfacing Real-Time Audio and File I/O"](http://www.rossbencina.com/code/interfacing-real-time-audio-and-file-io)
- Renn-Giles + Rowland, "Real-Time 101 Parts I & II" — ADC19 (YouTube)
- Doumler, "Using Locks in Real-Time Audio Processing, Safely" — ADC20
- Doumler, "What is Low Latency C++?" Parts 1 & 2 — CppNow 2023
- Doumler, "Thread synchronisation in real-time audio processing with RCU" — ADC
- Renn-Giles, "Real-Time Confessions in C++" — ADC23

**BillyDM blog (read in order for the Dropseed→Firewheel arc):**
- [DAW Frontend Development Struggles (2023-02)](https://billydm.github.io/blog/daw-frontend-development-struggles/)
- [Why I'm Taking a Break from Meadowlark (2023-04)](https://billydm.github.io/blog/why-im-taking-a-break-from-meadowlark/)
- [Clarifying Some Things (2023-05)](https://billydm.github.io/blog/clarifying-some-things/)
- [Rust vs C++ (2023-06)](https://billydm.github.io/blog/)
- [Accurate Timekeeping in a DAW (2022-11)](https://billydm.github.io/blog/) — directly relevant to optional-master-clock design

**Architecture references:**
- [Firewheel design doc](https://github.com/BillyDM/Firewheel/blob/main/DESIGN_DOC.md)
- [CLAP spec](https://github.com/free-audio/clap) — headers themselves are the spec
- **Share with Woodshed, don't fork.** The `woodshed-audio` crate is
  consumed as-is via path-dep. Extraction into per-module crates is
  deferred until consumer pain across both projects justifies it.
- **Strophos family naming.** Crates use `strophe-*`. The bare
  `strophe` crate name on crates.io is squatted by a dead 2016 redirect;
  this is acceptable because distribution is via itch.io/Gumroad and
  the workspace doesn't need to publish the umbrella name.
