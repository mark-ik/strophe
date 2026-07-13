# Muniment Project Store Plan

## Goal

Give Strophe one portable persistence path before adding save/open controls,
export, or peer transfer. Reuse Muniment's backend seam rather than create a
second filesystem protocol inside Strophe.

## Design

- `strophe-model::ProjectBundle` remains the versioned session/history manifest.
- `strophe-engine::ProjectStore<B>` stores that manifest at `strophe/manifest`
  and audio at `strophe/media/<MediaRef>` over a Muniment `Backend`.
- `MediaRef` remains Strophe's sample-rate-aware BLAKE3 identity. Muniment's
  generic `BlobStore` is intentionally not used directly because it hashes raw
  encoded bytes and would create a second identity for the same capture.
- Saving fails before writing when a manifest references unavailable media.
- Loading retains the session and reports missing media blobs. Those layers stay
  silent until their content arrives.

## Done Conditions

- Manifest schema version is explicit and rejects unknown versions.
- A generic store round-trips a manifest and captured audio through
  `MemoryBackend`.
- Missing media has distinct save and load behavior.
- Corrupt media is rejected rather than silently played.
- A future Genet host can choose Redb on desktop or OPFS in a browser without
  changing Strophe model or media semantics.

## Progress

- 2026-07-09: **LANDED.** Generic storage tests and Genet-host Redb save/open
  API pass. The local rail reports unavailable media after an open. Remaining
  host work is user-facing project selection and save/open controls.
