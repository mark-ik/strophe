# Open, Non-Lock-In Project Format

## Doctrine (maintainer direction, 2026-07-14)

A `.hock` file is a container, not a destination. The audio a musician makes in
Hocket is theirs, and the format must never be the thing that traps it. The
failure mode to avoid is the DAW-industry norm: a specialty project format that
is useless without the originating app or a bespoke converter. That is lock-in,
and Hocket rejects it as a matter of product identity, the same way it rejects
plugin-hosting gravity and DAW-shaped scope creep.

Two obligations follow:

1. **The container should be inspectable and un-lockable without Hocket.** A
   third party, a future web build, or the user with a zip tool should be able
   to open a `.hock` file and get at the material.
2. **The material inside should be standard, importable formats** — audio as
   ordinary WAV/FLAC, structure as a documented, non-proprietary serialization.
   "Export" should be a thin operation over what is already stored, not a
   lossy re-derivation.

This is a stated product value, not just an implementation preference. Surface
it into `PROJECT_DESCRIPTION.md` when the maintainer wants it at goal level.

## Where we are now (the starting point this plan moves away from)

A `.hock` file is currently a single [redb](../crates/hocket-engine/src/project_store.rs)
database written through Muniment's backend seam:

- One manifest key (`hocket/manifest`) holding the postcard-serialized
  `ProjectBundle` (session + history).
- Media blobs under `hocket/media/<blake3-hex>`, each a hand-rolled binary
  frame (`HOCKMED\0` magic + version + sample rate + count + raw f32 PCM).

Every part of that is opaque to the outside world: redb is a Rust-specific
embedded KV store, and the media is uncompressed f32 PCM in a private framing,
not even WAV. Nothing but Hocket can read a `.hock` file today. That is
acceptable for pre-alpha local durability, but it is exactly the state this
doctrine says we must not ship long-term.

## Target shape (leading candidate)

The genre-standard answer to "own extension, open guts" is a **zip archive with
a custom extension** — the pattern behind Renoise `.xrns`, `.docx`, `.odt`,
`.epub`, and Scratch `.sb3`. Applied here:

```text
mysession.hock            (a zip)
  manifest.cbor           session + history, CBOR (already the FT8 target)
  media/<blake3>.wav      or .flac; ordinary audio files, content-addressed
  meta.json               format version, app version, human-readable
```

Benefits: the OS still associates `.hock` with Hocket; any tool can unzip and
inspect; media travels as importable audio; nothing depends on a Rust KV store;
and export becomes "copy the media out" rather than a converter.

## Tensions to resolve (why this is a design decision, not a quick swap)

- **Muniment backend seam.** Persistence is deliberately KV-shaped so a browser
  host can swap redb for OPFS/IndexedDB behind the same interface. "The project
  *is* a zip file" is a different model. Options: (a) keep the KV seam as the
  live/working store and make zip an import/export envelope over it; (b) make
  the zip the canonical at-rest format and treat the KV store as a cache. (a)
  preserves the browser story with least disruption and is the likely answer.
- **CBOR is already planned.** `PROJECT_DESCRIPTION.md` sets postcard -> CBOR at
  FT8 to align with Moothold. The serialization half of "open format" lands with
  that move regardless; this plan adds the container half.
- **Content addressing.** `MediaRef` is BLAKE3 over samples + sample rate.
  Storing media as WAV/FLAC means the hash must be defined over decoded samples,
  not file bytes, or `MediaRef` identity changes. Keep hashing decoded samples;
  the file is a carrier.
- **Compression vs. exactness.** FLAC is lossless and importable; prefer it over
  raw or lossy. WAV is the safe floor.

## The CBOR half (structure serialization)

The manifest (`ProjectBundle` = session + history) is serialized today with
[postcard](../crates/hocket-model/src/persistence.rs). Postcard is compact but
Rust-only and *not self-describing*: you cannot decode a postcard blob without
the exact Rust types. That is itself a lock-in property, which is why the
already-planned move to CBOR (ciborium) serves this doctrine, not just Moothold
alignment.

Why CBOR specifically:

- **Self-describing and standard.** CBOR (RFC 8949) is an IETF standard with
  implementations in every language; a `.hock` manifest becomes inspectable by
  any CBOR tool, no Hocket required.
- **One format across storage and sync.** Moothold speaks CBOR (blobs,
  IndexCommits). If the at-rest manifest is already CBOR, the FT9 hand-off can
  put the same bytes on the wire with no transcode step.

The move is small because everything is serde-derive: swap
`postcard::to_allocvec` / `from_bytes` for ciborium's writer/reader calls and
the `PersistenceError` inner type. Two things are *not* mechanical and must be
handled:

- **Deterministic encoding is a hard requirement, not a nicety.** The bundle is
  meant to be content-addressable, so equal state must encode to equal bytes.
  Postcard gets this free from the `BTreeMap` collections. CBOR does *not* by
  default — it needs RFC 8949 §4.2 core-deterministic rules (sorted keys,
  shortest-form integers, definite lengths). Verify ciborium emits canonical
  CBOR, or add a canonicalization step, before relying on hashed bundles.
- **Encoding discriminator.** `format_version` is the first field but you cannot
  read it without already knowing the encoding. A clean CBOR-only cut sidesteps
  this; a read-both transition would need an out-of-band magic byte.

Because no `.hock` file exists on disk, FT8 can make a **clean break** to
CBOR-only with no read-both shim (DOC_POLICY section 3). Bump `FORMAT_VERSION`
and stop reading postcard.

## Sequencing

Fold into FT8 alongside the CBOR move rather than bolting a second format change
on later. Cost-free to defer until then: no `.hock` file exists on disk yet, so
there is no at-rest data to migrate when the container lands.

## Progress

- 2026-07-14: Doctrine captured from maintainer direction during the
  Strophe -> Hocket rename. Extension set to `.hock`. No code toward the zip
  container yet; current format remains the redb/postcard bundle above.
