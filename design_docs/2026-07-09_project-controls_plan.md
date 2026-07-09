# Project Controls Plan

## Goal

Turn the local project store into a usable desktop workflow without blocking the
Serval kernel or the audio engine.

## Design

- `rfd` provides native open/save dialogs until Serval exposes a shared desktop
  dialog API.
- An Armillary actor owns Redb project I/O. The host sends cloned save snapshots
  or open paths and drains typed updates on its own event loop.
- The live `AppState` keeps engine authority. An open result replaces model and
  media state only after the worker returns.
- The header shows the actual project file stem, save/open status, and dirty
  state. Saving and opening are rejected during recording or another project
  operation.

## Done Conditions

- Open and Save are visible, accessible controls.
- New projects prompt for a `.strophe` path; later saves reuse that path.
- Redb I/O never runs on the Serval kernel thread.
- A worker result updates the view and preserves missing-media behavior.
- The worker has a Redb save/open round-trip test.

## Progress

- 2026-07-09: Implementing the project worker, controls, and host-event wake.
