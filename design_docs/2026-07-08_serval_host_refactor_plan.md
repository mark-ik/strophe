# Strophe onto the serval host + chisel leaves (Masonry retirement)

**Status (2026-07-08):** proposed; first design pass. Refactors Strophe's UI
off the `mark-ik/xilem` Masonry fork onto `xilem_serval` (serval's third
`xilem_core` backend, the same host woodshed migrated to) with the custom-paint
widgets reborn as `chisel` leaves. The audio spine does not move. Completing
this retires the fork family-wide.

Code samples are illustrative unless marked implementation-ready.

---

## Why

The `mark-ik/xilem` fork (`xilem` / `masonry` / `masonry_winit`,
`woodshed-theme` branch) is the last non-serval UI stack in the Strophos family.
Woodshed already proved the exit: it migrated its whole app onto `xilem_serval`
(the S0-S5 arc in
`woodshed/design_docs/2026-07-04_serval_host_cross_platform_plan.md`) and, at its
own S5 cut, deleted its Masonry app. But woodshed could not drop the fork,
because two Masonry-coupled shared crates live in its workspace and are still
consumed by **Strophe**:

- `audio-widgets` (`../woodshed/crates/audio-widgets`) — `meter` / `fader` /
  `knob` (+ `waveform`), Masonry `Widget`s with `paint()`.
- `xilem-components` (`../woodshed/crates/xilem-components`) — the domain-neutral
  combobox etc.

Because both are woodshed **workspace members**, their `xilem = { workspace = true }`
resolves against woodshed's `[workspace.dependencies]` even when Strophe builds
them, so the fork cannot leave woodshed while Strophe needs them. **Strophe is
the only live consumer.** Move Strophe off Masonry and both crates lose their
last consumer; then the fork retires from both workspaces. This plan is the
family-wide fork retirement, expressed as a Strophe refactor.

## What moves, and what does not

**Moves (the UI layer only):**

- `crates/strophe` (the Masonry app: `main.rs` + `view/{transport,tracks,combination,settings}.rs`).
- `crates/strophe-widgets` (re-exports + the custom-paint widgets).
- The theme (`strophe-widgets::theme`: `SP_*`, `TS_*`, `palette`, `mono_family`).

**Does not move (the whole audio spine + data):**

- `crates/strophe-engine` — the Firewheel audio graph, capture, the realtime
  thread. UI-agnostic.
- `crates/strophe-model` — pure session/track/layer data.
- `crates/strophe-headless` — the audio test-harness bin.
- `firewheel`, `cpal`, `rtrb`, `basedrop`, the sync/persistence layers.

This split is the whole reason the refactor is tractable and low-risk: **none of
the hard realtime work is touched.** `AppState` reads the model and drives the
engine through plain helper methods (`record`, `stop_all`, `arm`, `undo`,
`select_variation`, `toggle_layer_mute`, `nudge_layer_gain`, …); the views only
call those. Swapping the view engine leaves the audio behaviour a fixed
reference to compare against throughout.

## The surface today (what to map)

Read from the current app (2026-07-08):

- **App shell** (`view/mod.rs`): a persistent transport bar above one of three
  surfaces (`Tracks` / `Combination` / `Settings`), selected by `state.surface`;
  `flex_col((transport, surface))`.
- **Transport** (`transport.rs`): title label, surface-nav buttons (one per
  surface, active marked), status/meter/capture/loops readouts (mono labels), the
  **output meter** (two vertical `meter_view` bars), and controls
  (`Record`/`Stop all`/`Undo`/`Redo` `text_button`s wired to `AppState` methods).
- **Tracks** (`tracks.rs`): a strip per track, dispatched on `playback_mode`:
  - `Sum` (looper): arm `text_button` + summed **`waveform_view`** + expand
    toggle; when expanded, a per-layer row each with mute/gain buttons + the
    layer's own `waveform_view`.
  - `SelectOne` (Deeler): arm + a variation-slot button row (active slot marked).
- **Combination**: the Deeler combination grid (tracks x variation slots).
- **Settings**: session settings (profile, tempo).
- **Custom-paint widgets** (`audio-widgets` + `strophe-widgets`): `waveform`,
  `meter`, `fader`, `knob` — Masonry `Widget` + Xilem `View` pairs. The `knob`,
  for example, paints kurbo arcs + a circle in `paint()`, drives value on
  vertical-drag `on_pointer_event`, sizes 48px in `measure`, announces
  `Role::Slider` in `accessibility`.
- **Reusable structure** (`xilem-components`): the combobox.

## The mapping (Masonry/Xilem -> serval)

| Today (Masonry / Xilem) | Target (serval) |
| --- | --- |
| `flex_col` / `flex_row` / `sized_box` | `el("div", ..)` + CSS flexbox (as woodshed) |
| `label` / `text_button` | native `xilem-serval` `text` / `clickable`/`button` views |
| `OneOf3` / `OneOf2` (surface / strip switch) | `match` -> boxed `AnyView` (as woodshed's tab/lens switch) |
| combobox (`xilem-components`) | native `xilem-serval` `select` view (dissolves) |
| `waveform_view` / `meter_view` / `fader` / `knob` | **`chisel` Path-A leaves** + `xilem-serval` view wrappers |
| `strophe-widgets::theme` (`SP_*`, `palette`, `mono`) | tinct-derived CSS (as woodshed's `theme.rs`) |
| `AppState` + helper methods | **unchanged** (UI-agnostic; action closures call `st.method()`) |
| `strophe-engine` / `strophe-model` / Firewheel | **unchanged** |

The custom-paint widgets are all Path-A (vector shapes the paint vocabulary can
say: bars, arcs, poly-lines), so they stay resolution-independent and tile-cached
and never touch the Path-B texture route. Per the chisel design
(`serval/docs/2026-07-07_chisel_widget_leaf_design.md`), each becomes a `Leaf`
(measure / paint / event / accessibility / paint_dirty) plus a thin `xilem-serval`
view over `chisel_leaf(key)` that diffs typed props (e.g. the knob's `value`) into
the host-owned `LeafRegistry`. The Masonry `paint()` bodies port almost verbatim:
`painter.stroke(arc, ..)` -> `cx.emit(PaintCmd::DrawPath { .. })`, `on_pointer_event`
drag math -> `Leaf::event`, `Role::Slider` -> `Leaf::accessibility`.

## Target crate structure (mirroring woodshed)

```text
strophe-model     (exists)  pure data + UI-agnostic AppState/helpers   [maybe lift AppState here]
strophe-engine    (exists)  Firewheel graph, realtime thread           UNCHANGED
strophe-headless  (exists)  audio test harness                          UNCHANGED
strophe-views     (NEW)     serval views over AppState                 mirrors woodshed-views
strophe-serval    (NEW)     serval winit host (SurfaceHost, redraw)     mirrors woodshed-serval
strophe-widgets   (rewrite) chisel leaves (waveform/meter/fader/knob)  Masonry code deleted
strophe           (app)     the old Masonry bin                         DELETED at the parity cut
```

Decisions / recommendations:

- **The audio chisel leaves start Strophe-owned** (in `strophe-widgets`, rewritten
  on chisel). Woodshed-serval currently uses none of them (it themes via tinct and
  needs no meter/waveform), so there is no second consumer to design for. Extract
  to a shared `audio-chisel` crate only if one appears.
- **`xilem-components` dissolves**, it does not port: its combobox is exactly what
  native `xilem-serval` `select` already provides (woodshed uses it).
- **Two bins coexist during the migration** (`strophe` Masonry + `strophe-serval`),
  the same way woodshed ran `woodshed-xilem` alongside `woodshed-serval` until
  parity. Delete the Masonry bin only at the cut.
- **Reuse woodshed's serval build setup verbatim**: the `[workspace.dependencies]`
  serval/netrender/tinct git deps, the `[patch.crates-io]` stylo mirror pointed at
  `mark-ik/stylo` (`mark-ik/servo-media-features`), and the gitignored
  `.cargo/config.toml` local-serval `[patch]`. **Build strophe-serval from the
  strophe cwd** so the local patch applies (see
  `memory/reference_mere_cargo_cwd_local_serval` — this bit woodshed for ~75 min).

## Dependencies and gates

- **chisel live runner wiring (serval-side).** The chisel leaf layer is scaffolded
  and its render path is proven in tests, but the on-screen path is gated on
  wiring `LeafRegistry` ownership into `xilem-serval/runner.rs` (a concurrent
  rewrite) plus a headed smoke (chisel doc "Next" 1-2). **The structural migration
  does not need chisel** (it uses only native views, already shipped). So sequence
  the structure first (unblocked now) and the widget->chisel conversion after the
  runner wiring lands; until then, render placeholder widgets (a CSS bar for the
  meter, a static peaks poly-line for the waveform).
- **tinct** for theming (shared crate, already woodshed's).
- **serval / netrender git deps** must eventually be pushed for reproducible /
  CI builds; local development rides the local checkouts via the `[patch]`.

## Plan (slices; keep an app runnable throughout)

Organized by feature target + done-condition, not calendar. Each slice keeps
either the Masonry `strophe` (until S6) or `strophe-serval` runnable.

- **S0 - scaffold `strophe-serval`.** New winit + `SurfaceHost` bin (copy
  woodshed-serval's host skeleton: boot, rasterize, acquire, compose, present;
  incremental layout; key/pointer routing; optional CSD chrome). *Done:* a themed
  placeholder serval window opens via `cargo run -p strophe-serval` from the
  strophe cwd; `cargo run -p strophe` (Masonry) still works.
- **S1 - AppState reachable + tinct theme.** Confirm/lift `AppState` to a
  UI-agnostic home (strophe-model or a new strophe-core) if it carries any Masonry
  coupling (e.g. a peniko-typed `palette`). Port `strophe-widgets::theme` to tinct
  seeds -> CSS. *Done:* strophe-serval renders the transport bar's static content
  (title, status, nav) from `AppState`; clicking nav switches `state.surface`.
- **S2 - Transport surface.** Full transport as native views: nav, the readout
  labels, and controls (`Record`/`Stop`/`Undo`/`Redo`) wired to `AppState`
  methods. Meter = placeholder CSS bar. *Done:* record/stop/undo/redo drive the
  engine identically to the Masonry app; nav switches surfaces.
- **S3 - Tracks surface.** Sum + SelectOne strips (arm, expand, per-layer
  mute/gain, variation slots) as native views. Waveform = placeholder poly-line.
  *Done:* arming / expanding / variation-select drive `AppState`; the strip layout
  matches the Masonry app.
- **S4 - Combination + Settings.** The Deeler grid (CSS grid; an arrangement leaf
  only if the grid needs paint the vocabulary cannot say, which it likely does
  not) + settings. *Done:* all three surfaces at structural parity with Masonry.
- **S5 - chisel leaves** (gated on chisel's runner wiring). Convert
  `waveform` / `meter` / `fader` / `knob` to chisel Path-A leaves + view wrappers;
  swap out the placeholders. *Done:* real meters / waveforms / knobs render and
  interact (knob/fader drag) via chisel; a headed smoke shows them live and a
  `paint_dirty` test shows an unchanged leaf produces zero repaints.
- **S6 - parity cut (fork retirement).** Delete the Masonry `strophe` bin and
  `strophe-widgets`' Masonry code; drop `audio-widgets`, `xilem-components`, and
  the `xilem`/`masonry`/`masonry_winit` deps from Strophe's `Cargo.toml`. With no
  consumer left, retire `audio-widgets` + `xilem-components` in the woodshed repo
  and drop the fork from woodshed's `[workspace.dependencies]` too. *Done:*
  `grep -ri masonry` is clean across the strophe + woodshed workspaces; both build
  serval-only, single wgpu tree. **The `mark-ik/xilem` fork is retired.**

## Validation

- The audio engine + model are untouched, so audio output is a fixed reference:
  A/B the Masonry `strophe` and `strophe-serval` for identical audio behaviour at
  each slice.
- Each slice ends with a runnable app and (from S2) a driven receipt of the ported
  surface, the same headed-verify discipline woodshed used.
- Build from the strophe cwd; keep the stylo mirror in sync with serval.

## Findings (verify during execution)

- **`AppState` home + coupling.** It lives in the `strophe` app crate today
  (`crate::AppState`); its helpers look pure but `state.palette` may be a
  Masonry/peniko palette. Confirm and lift the UI-agnostic part to a core crate;
  re-derive `palette` from tinct.
- **Widget inventory + homes.** `waveform_view` / `meter_view` are re-exported by
  `strophe-widgets`; `meter` / `fader` / `knob` live in `audio-widgets`;
  `waveform` home to confirm. Enumerate the full custom-paint set before S5.
- **chisel runner-wiring status.** The S5 gate; coordinate with the serval-side
  chisel work (its "Next" 1-2). Structure slices (S0-S4) run ahead of it.
- **Combination grid** paint needs (native CSS grid vs. an arrangement leaf).

## Open questions

1. Audio chisel leaves Strophe-owned (`strophe-widgets`) vs. a shared
   `audio-chisel` crate. Recommend Strophe-owned first; extract on a second
   consumer.
2. Coexisting bins during migration acceptable (woodshed precedent says yes).
3. CSD chrome (own title bar, as woodshed) vs. OS decorations for strophe-serval.
4. Does any Strophe widget need Path B (a `vello::Scene` / shader) rather than
   Path A? Current set (bars, arcs, poly-lines) is all Path A.

## Progress

- 2026-07-08: **S1 done — the approved loop-recorder UI renders in serval.**
  `view.rs` + `theme.rs` build the one-screen design from the 2026-07-08 concept:
  the pass-the-mic **rail** (the circle: You/Jonah/Mara/Eli + Hand off), the
  **loop table** (four owner-coloured lanes — Guitar/Bass/Drums/Keys — each an
  overdub stack of layer waveforms, muted layers dimmed, plus M/S/loop controls
  and an empty-state), and the **transport** (tempo/meter, click + master-clock
  toggles, the big Record, the output meter). Data is static this slice (S2 wires
  `AppState`); two interactions are live end-to-end (tap a lane dot to arm, tap
  Record to toggle capture). Waveforms are lightweight DOM bar stand-ins — S5
  swaps them for chisel leaves. Verified by client-area screenshot: faithful to
  the mockup, all four tracks + add-track on one screen, every edge clean.
  Three serval-CSS lessons banked (all worked around in `theme.rs`, none needed
  a host change):
  - **serval doesn't match `:root`.** Palette custom properties live on `.app`
    (the actual root element); on `:root` they never inherit. Inline `var()`
    (`--voice` per lane) and `.app`-scoped vars both cascade fine.
  - **Single-side border shorthands emit a phantom.** `border-bottom` /
    `border-right` / `border-top` paint a spurious 3px `currentColor` border on
    the *opposite* edge (a cream frame at the viewport rim here). The full
    four-side `border:` shorthand is clean, so dividers use
    `border: 1px solid transparent` + a `border-<side>-color` longhand. Worth a
    serval-layout paint fix later; the workaround is documented inline.
  - **Nested flex needs explicit `min-height: 0`** for the loop table to scroll
    internally instead of shoving the transport off-screen (`.body`/`.table`).
- 2026-07-08: **S0 done — `strophe-serval` scaffolded and building.** New winit +
  `SurfaceHost` bin (retained `IncrementalLayout` redraw + click dispatch),
  rendering a themed placeholder with a working counter (render + hit-test +
  dispatch + update + repaint proven). Built from the strophe cwd (9m cold,
  serval stack); the Masonry `strophe` bin still checks green, now riding the
  **git** xilem fork (`mark-ik/xilem@woodshed-theme`). Workspace wiring: serval /
  netrender / tinct git deps + a `[patch.crates-io]` stylo mirror to
  `mark-ik/stylo` (matching serval + woodshed). `.cargo/config.toml`
  (machine-local): dropped the old `paths = [woodshed, xilem-woodshed]` override
  (it would hijack serval's vendored `xilem_core` by name) for source-specific
  serval `[patch]` entries; the Masonry app rides git xilem instead of the
  worktree. **Both bins build.**
- 2026-07-08: **S5 unblocked.** The serval vector-widget / chisel-leaf path is now
  covered end to end, so S5 (converting `waveform`/`meter`/`fader`/`knob` to
  chisel leaves) no longer waits behind a runner-wiring gate; the real leaves can
  land when the slice arrives, and the S2-S4 placeholders are optional rather than
  necessary.
- 2026-07-08: Plan created. Grounded against the current Masonry app
  (`view/{mod,transport,tracks}.rs`, `audio-widgets/knob.rs`), the chisel design
  (`serval/docs/2026-07-07_chisel_widget_leaf_design.md`), and woodshed's shipped
  serval migration.
