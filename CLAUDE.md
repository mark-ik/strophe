# CLAUDE.md — Strophe Repository Role

This file defines how Claude Code should behave in this repository. Read
it first when starting any session.

---

## Project Identity

**Strophe** is a cross-platform loop recorder with turn-based
collaboration, inspired by Menomena's Deeler. Sibling to
[Woodshed](../woodshed/); shares audio infrastructure via path-dep on
woodshed's `woodshed-audio` crate.

The north-star metaphor (from `PROJECT_DESCRIPTION.md`): *the digital
equivalent of passing a mic around in a circle and building loops
turn by turn.* Asynchronous, sequential, no shared clock, no real-time
jam pressure.

The product is not an Ableton-shaped DAW. It's a Tracks-of-Phrases
phrase sampler — **ten tracks × four variations as defaults, both
configurable** — with sequential async overdubbing, content-addressed
media, nondestructive history, and peer-to-peer session sync as the
collaboration model. Configurability is in the model from day one;
parity-with-Deeler is the initial target, with broader configuration
following. See `design_docs/PROJECT_DESCRIPTION.md` for the product
description and `design_docs/DOC_README.md` for the doc index.

## Document Structure

All authoritative design material lives in `design_docs/`. Read
`design_docs/DOC_README.md` first.

| Path | What's there |
|------|-------------|
| `design_docs/DOC_README.md` | Index and AI working principles |
| `design_docs/DOC_POLICY.md` | Documentation governance |
| `design_docs/PROJECT_DESCRIPTION.md` | Product goals, features (maintainer-owned) |
| `design_docs/<date>_<keyword>_plan.md` | Active feature plans |
| `design_docs/archive_docs/<date>/` | Retired plans |

## Workspace Layout

```
crates/
  strophe/           The Xilem application (the binary users run)
  strophe-engine/    Audio engine (Firewheel)
  strophe-model/     Session data model (nondestructive)
  strophe-widgets/   Masonry custom widgets
  strophe-headless/  Headless audio-engine test harness (scripted demos)
```

`cargo run -p strophe` launches the app; `cargo run -p strophe-headless`
runs the scripted audio demo without a UI.

Sibling: `../woodshed/crates/woodshed-audio` (path-dep). Eventually
`woodshed-audio` may be pulled apart into per-module crates; today it's
consumed as the umbrella.

## General Guidelines

- Rust: standard idioms. No `unsafe` without documented justification.
- The model crate (`strophe-model`) must remain framework-agnostic — no
  cpal, no xilem, no masonry. The audio engine and the UI both consume
  it.
- Plans go in `design_docs/` per the date-keyword-plan convention. Do
  not store project plans in `.claude/plans/`.
- Follow `DOC_POLICY.md` for documentation changes.

## Important Don'ts

- Do not build an Ableton-shaped feature set. The constraint set is
  deliberate; widening it would erase the project's identity.
- Do not couple `strophe-model` to any UI or audio framework.
- Do not add real-time multiplayer ("everyone jams together over the
  network") as a primary mode. The collaboration model is *asynchronous*
  — sequential turn-taking with explicit hand-offs. Real-time presence
  is at most a side-channel awareness feature.
- Do not extract `woodshed-audio` into per-module crates speculatively.
  Wait for actual consumer pain across both projects.
