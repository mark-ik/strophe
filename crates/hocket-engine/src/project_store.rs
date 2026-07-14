//! Durable Hocket project storage over Muniment's host-supplied byte backend.
//!
//! The model's [`ProjectBundle`] is one mutable manifest. Captured media stays
//! immutable and content-addressed under its existing [`MediaRef`]. Hocket
//! deliberately does not use Muniment's `BlobStore` here: `MediaRef` hashes the
//! capture sample rate and samples together, which is Hocket's audio identity.

use std::collections::BTreeSet;

use muniment::{Backend, StoreError, WriteOp};
use hocket_model::{MediaRef, PersistenceError, ProjectBundle};

use crate::media::{InMemoryStore, MediaBuffer, MediaStore, hash_buffer};

/// The single mutable manifest key for one Hocket project backend.
pub const MANIFEST_KEY: &str = "hocket/manifest";
const MEDIA_PREFIX: &str = "hocket/media/";
// Renamed with the product (2026-07-14). Pre-rename bundles are not readable
// and no legacy path is kept: no saved project predates the rename.
const MEDIA_MAGIC: &[u8; 8] = b"HOCKMED\0";
const MEDIA_VERSION: u16 = 1;
const MEDIA_HEADER_LEN: usize = 8 + 2 + 4 + 8;

/// Project storage over a host-selected Muniment backend. A desktop host can
/// use Redb; a browser host can later provide OPFS through the same interface.
pub struct ProjectStore<B> {
    backend: B,
}

/// A manifest plus every media blob that was available at load time. Missing
/// blobs do not prevent opening the project: their layers remain in history but
/// stay silent until a peer, backup, or later import supplies the media.
#[derive(Clone, Debug)]
pub struct LoadedProject {
    pub bundle: ProjectBundle,
    pub media: InMemoryStore,
    pub missing_media: BTreeSet<MediaRef>,
}

#[derive(Debug)]
pub enum ProjectStoreError {
    Store(StoreError),
    Manifest(PersistenceError),
    MissingManifest,
    MissingMedia(BTreeSet<MediaRef>),
    InvalidMedia {
        reference: MediaRef,
        reason: &'static str,
    },
    MediaHashMismatch {
        expected: MediaRef,
        actual: MediaRef,
    },
}

impl std::fmt::Display for ProjectStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => write!(f, "storage failed: {error}"),
            Self::Manifest(error) => write!(f, "project manifest failed: {error}"),
            Self::MissingManifest => f.write_str("project manifest is missing"),
            Self::MissingMedia(references) => {
                write!(
                    f,
                    "project save is missing {} media blob(s)",
                    references.len()
                )
            }
            Self::InvalidMedia { reference, reason } => {
                write!(f, "media {reference} is invalid: {reason}")
            }
            Self::MediaHashMismatch { expected, actual } => {
                write!(
                    f,
                    "media hash mismatch: expected {expected}, found {actual}"
                )
            }
        }
    }
}

impl std::error::Error for ProjectStoreError {}

impl From<StoreError> for ProjectStoreError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<PersistenceError> for ProjectStoreError {
    fn from(error: PersistenceError) -> Self {
        Self::Manifest(error)
    }
}

impl<B: Backend> ProjectStore<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Save newly referenced media blobs and the manifest in one backend batch.
    /// Existing blobs are validated and reused. Transactional backends make new
    /// blobs plus the manifest all-or-nothing. Simpler backends may leave only
    /// harmless content-addressed blobs after an interrupted write.
    pub async fn save(
        &self,
        bundle: &ProjectBundle,
        media: &impl MediaStore,
    ) -> Result<(), ProjectStoreError> {
        let references = referenced_media(bundle);
        let missing: BTreeSet<MediaRef> = references
            .iter()
            .filter(|reference| media.get(reference).is_none())
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(ProjectStoreError::MissingMedia(missing));
        }

        let mut writes = Vec::with_capacity(references.len() + 1);
        for reference in references {
            let buffer = media
                .get(&reference)
                .expect("missing references checked above");
            let actual = hash_buffer(&buffer.samples, buffer.sample_rate);
            if actual != reference {
                return Err(ProjectStoreError::MediaHashMismatch {
                    expected: reference,
                    actual,
                });
            }
            let key = media_key(reference);
            if let Some(existing) = self.backend.get(&key).await? {
                let stored = decode_media(reference, &existing)?;
                let actual = hash_buffer(&stored.samples, stored.sample_rate);
                if actual != reference {
                    return Err(ProjectStoreError::MediaHashMismatch {
                        expected: reference,
                        actual,
                    });
                }
                continue;
            }
            writes.push(WriteOp::Put {
                key,
                value: encode_media(buffer),
            });
        }
        writes.push(WriteOp::Put {
            key: MANIFEST_KEY.to_string(),
            value: bundle.to_bytes()?,
        });
        self.backend.apply(&writes).await?;
        Ok(())
    }

    /// Load the manifest and every available blob. Missing blob keys are
    /// reported in [`LoadedProject::missing_media`] rather than failing open.
    pub async fn load(&self) -> Result<LoadedProject, ProjectStoreError> {
        let bytes = self
            .backend
            .get(MANIFEST_KEY)
            .await?
            .ok_or(ProjectStoreError::MissingManifest)?;
        let bundle = ProjectBundle::from_bytes(&bytes)?;
        let mut media = InMemoryStore::new();
        let mut missing_media = BTreeSet::new();

        for reference in referenced_media(&bundle) {
            let Some(bytes) = self.backend.get(&media_key(reference)).await? else {
                missing_media.insert(reference);
                continue;
            };
            let buffer = decode_media(reference, &bytes)?;
            let actual = media.put(&buffer.samples, buffer.sample_rate);
            if actual != reference {
                return Err(ProjectStoreError::MediaHashMismatch {
                    expected: reference,
                    actual,
                });
            }
        }

        Ok(LoadedProject {
            bundle,
            media,
            missing_media,
        })
    }
}

fn referenced_media(bundle: &ProjectBundle) -> BTreeSet<MediaRef> {
    bundle
        .session
        .phrases
        .values()
        .map(|phrase| phrase.media)
        .collect()
}

fn media_key(reference: MediaRef) -> String {
    let mut key = String::with_capacity(MEDIA_PREFIX.len() + 64);
    key.push_str(MEDIA_PREFIX);
    for byte in reference.0 {
        use std::fmt::Write as _;
        write!(&mut key, "{byte:02x}").expect("writing to a string cannot fail");
    }
    key
}

fn encode_media(buffer: &MediaBuffer) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(MEDIA_HEADER_LEN + buffer.samples.len() * 4);
    bytes.extend_from_slice(MEDIA_MAGIC);
    bytes.extend_from_slice(&MEDIA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&buffer.sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(buffer.samples.len() as u64).to_le_bytes());
    for sample in &buffer.samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

fn decode_media(reference: MediaRef, bytes: &[u8]) -> Result<MediaBuffer, ProjectStoreError> {
    if bytes.len() < MEDIA_HEADER_LEN {
        return Err(ProjectStoreError::InvalidMedia {
            reference,
            reason: "truncated header",
        });
    }
    if &bytes[..8] != MEDIA_MAGIC {
        return Err(ProjectStoreError::InvalidMedia {
            reference,
            reason: "unknown media format",
        });
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != MEDIA_VERSION {
        return Err(ProjectStoreError::InvalidMedia {
            reference,
            reason: "unsupported media version",
        });
    }
    let sample_rate = u32::from_le_bytes(bytes[10..14].try_into().expect("fixed header slice"));
    let sample_count = u64::from_le_bytes(bytes[14..22].try_into().expect("fixed header slice"));
    let Ok(sample_count) = usize::try_from(sample_count) else {
        return Err(ProjectStoreError::InvalidMedia {
            reference,
            reason: "sample count is too large",
        });
    };
    let Some(expected_len) = sample_count
        .checked_mul(4)
        .and_then(|sample_bytes| MEDIA_HEADER_LEN.checked_add(sample_bytes))
    else {
        return Err(ProjectStoreError::InvalidMedia {
            reference,
            reason: "sample payload is too large",
        });
    };
    if bytes.len() != expected_len {
        return Err(ProjectStoreError::InvalidMedia {
            reference,
            reason: "sample payload length does not match header",
        });
    }
    let samples = bytes[MEDIA_HEADER_LEN..]
        .chunks_exact(4)
        .map(|sample| f32::from_le_bytes(sample.try_into().expect("four-byte sample")))
        .collect();
    Ok(MediaBuffer {
        samples,
        sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use muniment::{Backend, MemoryBackend};
    use pollster::block_on;
    use hocket_model::{Edit, History, Layer, Phrase, Session};

    use super::*;

    fn bundle_with_one_layer(store: &mut InMemoryStore) -> ProjectBundle {
        let mut session = Session::new_default();
        let mut history = History::new();
        let media = store.put(&[0.25, -0.5, 0.75], 48_000);
        let phrase = Phrase::new(media, session.bars_per_phrase, session.bpm, 1);
        let layer = Layer::new(phrase.id);
        let track_id = session.tracks[0].id;
        history.commit(
            Edit::AppendLayer {
                track_id,
                phrase,
                layer,
            },
            &mut session,
            1,
        );
        ProjectBundle::new(session, history)
    }

    #[test]
    fn save_and_load_round_trip_manifest_and_media() {
        block_on(async {
            let backend = MemoryBackend::new();
            let project = ProjectStore::new(backend.clone());
            let mut media = InMemoryStore::new();
            let bundle = bundle_with_one_layer(&mut media);

            project.save(&bundle, &media).await.unwrap();
            let loaded = project.load().await.unwrap();

            assert_eq!(loaded.bundle, bundle);
            assert!(loaded.missing_media.is_empty());
            let reference = loaded.bundle.session.phrases.values().next().unwrap().media;
            assert_eq!(
                loaded.media.get(&reference).unwrap().samples,
                vec![0.25, -0.5, 0.75]
            );
            assert_eq!(backend.get(MANIFEST_KEY).await.unwrap().is_some(), true);
        });
    }

    #[test]
    fn save_rejects_manifest_that_references_missing_media() {
        block_on(async {
            let backend = MemoryBackend::new();
            let project = ProjectStore::new(backend);
            let mut populated = InMemoryStore::new();
            let bundle = bundle_with_one_layer(&mut populated);
            let empty = InMemoryStore::new();

            let error = project.save(&bundle, &empty).await.unwrap_err();
            assert!(matches!(error, ProjectStoreError::MissingMedia(_)));
        });
    }

    #[test]
    fn load_keeps_project_when_a_media_blob_is_missing() {
        block_on(async {
            let backend = MemoryBackend::new();
            let project = ProjectStore::new(backend.clone());
            let mut media = InMemoryStore::new();
            let bundle = bundle_with_one_layer(&mut media);
            backend
                .put(MANIFEST_KEY, &bundle.to_bytes().unwrap())
                .await
                .unwrap();

            let loaded = project.load().await.unwrap();
            assert_eq!(loaded.bundle, bundle);
            assert_eq!(loaded.missing_media.len(), 1);
            assert!(loaded.media.is_empty());
        });
    }

    #[test]
    fn corrupt_media_is_rejected() {
        block_on(async {
            let backend = MemoryBackend::new();
            let project = ProjectStore::new(backend.clone());
            let mut media = InMemoryStore::new();
            let bundle = bundle_with_one_layer(&mut media);
            let reference = bundle.session.phrases.values().next().unwrap().media;
            backend
                .put(MANIFEST_KEY, &bundle.to_bytes().unwrap())
                .await
                .unwrap();
            backend
                .put(&media_key(reference), b"not audio")
                .await
                .unwrap();

            let error = project.load().await.unwrap_err();
            assert!(matches!(error, ProjectStoreError::InvalidMedia { .. }));
        });
    }

    #[test]
    fn save_rejects_a_corrupt_existing_media_blob() {
        block_on(async {
            let backend = MemoryBackend::new();
            let project = ProjectStore::new(backend.clone());
            let mut media = InMemoryStore::new();
            let bundle = bundle_with_one_layer(&mut media);
            let reference = bundle.session.phrases.values().next().unwrap().media;
            backend
                .put(&media_key(reference), b"not audio")
                .await
                .unwrap();

            let error = project.save(&bundle, &media).await.unwrap_err();
            assert!(matches!(error, ProjectStoreError::InvalidMedia { .. }));
        });
    }
}
