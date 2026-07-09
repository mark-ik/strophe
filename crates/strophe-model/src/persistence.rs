//! Save / load support for sessions.
//!
//! Sessions serialize via [postcard](https://docs.rs/postcard) — a
//! compact serde-based binary format. The choice is documented in the
//! initial plan's Findings; postcard was picked over rkyv (zero-copy
//! complexity not warranted yet), bincode (less compact, less
//! portable), and serde_json (HashMap key-order is non-deterministic
//! and BTreeMap conversion would be needed anyway).
//!
//! **FT8 migration target:** ciborium (CBOR), to align with Moothold.
//! Postcard survives until the FT8 coordinated migration.
//!
//! Because `Session` and `History` use `BTreeMap` collections,
//! encoded byte output is deterministic — the same logical state
//! encodes to the same bytes every time. That's a prerequisite for
//! content-addressing the project bundle itself later.

use serde::{Deserialize, Serialize};

use crate::history::History;
use crate::session::Session;

/// On-disk representation of a saved project. Session + history.
///
/// Media buffers are *not* part of this bundle. They live in a
/// content-addressed store keyed by `MediaRef` and travel separately
/// (e.g. via Moothold blob storage).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectBundle {
    /// Schema version for the manifest payload. A future incompatible schema
    /// gets a new version rather than being decoded under false assumptions.
    pub format_version: u16,
    pub session: Session,
    pub history: History,
}

#[derive(Debug)]
pub enum PersistenceError {
    Encode(postcard::Error),
    Decode(postcard::Error),
    UnsupportedVersion(u16),
}

impl std::fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encode(e) => write!(f, "encode failed: {e}"),
            Self::Decode(e) => write!(f, "decode failed: {e}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported project format version {version}")
            }
        }
    }
}

impl std::error::Error for PersistenceError {}

impl ProjectBundle {
    /// Current serialized project-manifest schema.
    pub const FORMAT_VERSION: u16 = 1;

    pub fn new(session: Session, history: History) -> Self {
        Self {
            format_version: Self::FORMAT_VERSION,
            session,
            history,
        }
    }

    /// Encode to postcard bytes. Deterministic given equal input
    /// because all collections are `BTreeMap`.
    pub fn to_bytes(&self) -> Result<Vec<u8>, PersistenceError> {
        postcard::to_allocvec(self).map_err(PersistenceError::Encode)
    }

    /// Decode from postcard bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PersistenceError> {
        let bundle: Self = postcard::from_bytes(bytes).map_err(PersistenceError::Decode)?;
        if bundle.format_version != Self::FORMAT_VERSION {
            return Err(PersistenceError::UnsupportedVersion(bundle.format_version));
        }
        Ok(bundle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Edit;
    use crate::ids::MediaRef;
    use crate::phrase::{Layer, Phrase};

    /// FT2 validation criterion: build a session with multiple layers
    /// across multiple tracks, save, reload — bit-identical.
    #[test]
    fn session_with_layers_round_trips() {
        let mut session = Session::new_default();
        let mut history = History::new();

        // Append two layers to track 0, one layer to track 1.
        for (track_idx, layer_count) in &[(0_usize, 2_usize), (1, 1)] {
            let track_id = session.tracks[*track_idx].id;
            for i in 0..*layer_count {
                let phrase = Phrase::new(
                    MediaRef([(*track_idx * 4 + i) as u8; 32]),
                    4,
                    120.0,
                    1_000_000 + *track_idx as u64 * 1000 + i as u64,
                );
                let layer = Layer::new(phrase.id);
                history.commit(
                    Edit::AppendLayer {
                        track_id,
                        phrase,
                        layer,
                    },
                    &mut session,
                    0,
                );
            }
        }

        let bundle = ProjectBundle::new(session.clone(), history.clone());
        let bytes = bundle.to_bytes().unwrap();
        let restored = ProjectBundle::from_bytes(&bytes).unwrap();

        assert_eq!(bundle, restored, "round-trip should be bit-identical");
        assert_eq!(restored.session.phrases.len(), 3);
        assert_eq!(restored.session.tracks[0].layers.len(), 2);
        assert_eq!(restored.session.tracks[1].layers.len(), 1);
        assert_eq!(restored.history.nodes.len(), 4); // root + 3 appends
    }

    /// Bytes are deterministic — same input always produces same output.
    #[test]
    fn encoding_is_deterministic_for_equal_input() {
        let session = Session::new_default();
        let history = History::new();
        let b1 = ProjectBundle::new(session.clone(), history.clone());
        let b2 = ProjectBundle::new(session, history);
        assert_eq!(b1.to_bytes().unwrap(), b2.to_bytes().unwrap());
    }

    #[test]
    fn rejects_unknown_format_version() {
        let mut bundle = ProjectBundle::new(Session::new_default(), History::new());
        bundle.format_version += 1;
        let bytes = postcard::to_allocvec(&bundle).unwrap();
        assert_eq!(
            ProjectBundle::from_bytes(&bytes).unwrap_err().to_string(),
            "unsupported project format version 2"
        );
    }
}
