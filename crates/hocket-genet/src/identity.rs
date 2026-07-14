//! Host-local Hocket identity, persisted outside portable projects.

use std::path::{Path, PathBuf};

use personae::{
    DerivedKeyAttestation, Ed25519Keypair, Ed25519PublicKey, IdentityError, IdentityProvider,
    SealedIdentityProvider, SealedRecordStorage, load_or_create_auto_unlock_root,
};
use serde_json::Value;

const IDENTITY_RECORD: &str = "hocket/local-identity.json";

/// Pre-rename record name, when the product was Strophe (renamed 2026-07-14).
/// A sealed record's path is bound into its AEAD associated data, so the
/// identity cannot simply be moved to the new name: it has to be unsealed under
/// the old name and re-sealed under the new one. Without that, a musician who
/// already has an identity would silently become a new person with a new
/// fingerprint, and any hand-off envelope they had signed would no longer trace
/// back to them.
const LEGACY_IDENTITY_RECORD: &str = "strophe/local-identity.json";

/// A durable host identity whose secret is held in memory only while Hocket runs.
pub struct LocalIdentity {
    provider: SealedIdentityProvider,
}

impl LocalIdentity {
    /// Load or create the identity under Hocket's platform data directory.
    pub fn open_default() -> Result<Self, IdentityError> {
        let data_root = default_data_root()?;
        adopt_legacy_data_root(&data_root)?;
        let unlock_path = data_root.join("personae/auto-unlock-root.json");
        let root_key = load_or_create_auto_unlock_root(unlock_path)?.ok_or_else(|| {
            IdentityError::Backend(
                "OS-protected automatic identity unlock is unavailable on this platform"
                    .to_string(),
            )
        })?;
        Self::open_with_root(&data_root.join("personae/records"), root_key)
    }

    fn open_with_root(records_root: &Path, root_key: [u8; 32]) -> Result<Self, IdentityError> {
        let records = SealedRecordStorage::open_with_key(records_root, root_key);
        adopt_legacy_record(&records)?;
        let provider = SealedIdentityProvider::load_or_create(&records, IDENTITY_RECORD)?;
        Ok(Self { provider })
    }

    /// Short display fingerprint of the public key. This is not an address.
    pub fn fingerprint(&self) -> String {
        self.provider
            .master_public_key()
            .to_bytes()
            .iter()
            .take(6)
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

impl IdentityProvider for LocalIdentity {
    fn master_public_key(&self) -> Ed25519PublicKey {
        self.provider.master_public_key()
    }

    fn derive_keypair(&self, salt: &[u8]) -> Result<Ed25519Keypair, IdentityError> {
        self.provider.derive_keypair(salt)
    }

    fn attest_derived_key(&self, salt: &[u8]) -> Result<DerivedKeyAttestation, IdentityError> {
        self.provider.attest_derived_key(salt)
    }
}

fn default_data_root() -> Result<PathBuf, IdentityError> {
    if let Some(root) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(root).join("Hocket"));
    }
    if let Some(root) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(root).join("hocket"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home).join(".local/share/hocket"));
    }
    Err(IdentityError::Backend(
        "could not determine Hocket's local data directory".to_string(),
    ))
}

fn legacy_data_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("LOCALAPPDATA") {
        return Some(PathBuf::from(root).join("Strophe"));
    }
    if let Some(root) = std::env::var_os("XDG_DATA_HOME") {
        return Some(PathBuf::from(root).join("strophe"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/strophe"))
}

/// Move a pre-rename data directory to the Hocket one, once.
///
/// The auto-unlock root is DPAPI-wrapped against the Windows user, not against
/// its path, so it survives the move. The sealed record's associated data is its
/// path *relative to the records root*, which the move also leaves intact.
fn adopt_legacy_data_root(data_root: &Path) -> Result<(), IdentityError> {
    if data_root.exists() {
        return Ok(());
    }
    let Some(legacy) = legacy_data_root() else {
        return Ok(());
    };
    if !legacy.exists() {
        return Ok(());
    }
    std::fs::rename(&legacy, data_root).map_err(|err| {
        IdentityError::Backend(format!(
            "move pre-rename identity {legacy:?} -> {data_root:?}: {err}"
        ))
    })
}

/// Re-seal a pre-rename identity record under the current record name, once.
///
/// Unsealing yields the record as opaque JSON, which is re-sealed verbatim under
/// the new name's associated data. The key material is unchanged, so the
/// fingerprint shown in the circle stays the same across the rename.
fn adopt_legacy_record(records: &SealedRecordStorage) -> Result<(), IdentityError> {
    if records.load_record::<Value>(IDENTITY_RECORD)?.is_some() {
        return Ok(());
    }
    let Some(record) = records.load_record::<Value>(LEGACY_IDENTITY_RECORD)? else {
        return Ok(());
    };
    records.save_record(IDENTITY_RECORD, &record)?;
    records.delete_record(LEGACY_IDENTITY_RECORD)
}

#[cfg(test)]
mod tests {
    use personae::IdentityProvider;

    use super::*;

    #[test]
    fn sealed_identity_is_stable_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let first = LocalIdentity::open_with_root(dir.path(), [0x45; 32]).unwrap();
        let first_public = first.master_public_key();
        drop(first);

        let second = LocalIdentity::open_with_root(dir.path(), [0x45; 32]).unwrap();
        assert_eq!(second.master_public_key(), first_public);
    }

    #[test]
    fn a_pre_rename_identity_keeps_its_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let records = SealedRecordStorage::open_with_key(dir.path(), [0x51; 32]);
        let legacy = SealedIdentityProvider::load_or_create(&records, LEGACY_IDENTITY_RECORD)
            .expect("seal a pre-rename identity");
        let legacy_public = legacy.master_public_key();
        drop(legacy);

        let adopted = LocalIdentity::open_with_root(dir.path(), [0x51; 32]).unwrap();

        assert_eq!(
            adopted.master_public_key(),
            legacy_public,
            "the rename must not mint a new identity"
        );
        assert!(dir.path().join("hocket/local-identity.json").exists());
        assert!(!dir.path().join("strophe/local-identity.json").exists());
    }

    #[test]
    fn a_pre_rename_identity_does_not_displace_a_current_one() {
        let dir = tempfile::tempdir().unwrap();
        let records = SealedRecordStorage::open_with_key(dir.path(), [0x52; 32]);
        let current = SealedIdentityProvider::load_or_create(&records, IDENTITY_RECORD).unwrap();
        let current_public = current.master_public_key();
        drop(current);
        SealedIdentityProvider::load_or_create(&records, LEGACY_IDENTITY_RECORD).unwrap();

        let opened = LocalIdentity::open_with_root(dir.path(), [0x52; 32]).unwrap();

        assert_eq!(opened.master_public_key(), current_public);
    }

    #[test]
    fn wrong_record_root_cannot_open_identity() {
        let dir = tempfile::tempdir().unwrap();
        LocalIdentity::open_with_root(dir.path(), [0x45; 32]).unwrap();

        let error = LocalIdentity::open_with_root(dir.path(), [0x46; 32])
            .err()
            .expect("wrong root should fail");
        assert!(error.to_string().contains("decrypt sealed record"));
    }
}
