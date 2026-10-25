//! An in-memory keychain for tests, so they never touch the real one.
//!
//! Unlike `keyring::mock`, values outlive the `Entry` that wrote them, which is
//! what a migration test needs. Accounts embed the data directory, so tests that
//! use their own temporary directories never see each other's entries.
use crate::{account, Tracker, SERVICE};
use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi};
use std::{
    any::Any,
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Mutex, Once},
};

type Key = (String, String);

static VALUES: Mutex<Option<HashMap<Key, Vec<u8>>>> = Mutex::new(None);
static FAILING: Mutex<Option<HashSet<Key>>> = Mutex::new(None);
static UNREADABLE: Mutex<Option<HashSet<Key>>> = Mutex::new(None);
static INSTALL: Once = Once::new();

/// Routes every `keyring::Entry` in this process to the in-memory store.
pub fn use_memory_keychain() {
    INSTALL.call_once(|| keyring::set_default_credential_builder(Box::new(MemoryBuilder)));
}

/// What the keychain holds for this tracker and data directory.
pub fn stored(tracker: Tracker, data_dir: &Path) -> Option<String> {
    let values = VALUES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let bytes = values.as_ref()?.get(&key(tracker, data_dir))?;
    Some(String::from_utf8(bytes.clone()).unwrap())
}

/// Makes keychain writes (saves and deletes) for this tracker and data
/// directory fail, as a locked or missing keychain does.
pub fn fail_writes(tracker: Tracker, data_dir: &Path, fail: bool) {
    let mut failing = FAILING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let failing = failing.get_or_insert_with(HashSet::new);
    if fail {
        failing.insert(key(tracker, data_dir));
    } else {
        failing.remove(&key(tracker, data_dir));
    }
}

/// Makes keychain reads for this tracker and data directory fail, as a locked
/// keychain, a denied access prompt or a missing Secret Service does.
pub fn fail_reads(tracker: Tracker, data_dir: &Path, fail: bool) {
    let mut unreadable = UNREADABLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let unreadable = unreadable.get_or_insert_with(HashSet::new);
    if fail {
        unreadable.insert(key(tracker, data_dir));
    } else {
        unreadable.remove(&key(tracker, data_dir));
    }
}

fn key(tracker: Tracker, data_dir: &Path) -> Key {
    (SERVICE.to_string(), account(tracker, data_dir))
}

struct MemoryBuilder;
struct MemoryCredential(Key);

impl CredentialBuilderApi for MemoryBuilder {
    fn build(
        &self,
        _: Option<&str>,
        service: &str,
        user: &str,
    ) -> keyring::Result<Box<Credential>> {
        Ok(Box::new(MemoryCredential((service.into(), user.into()))))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl CredentialApi for MemoryCredential {
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        let failing = FAILING
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failing.as_ref().is_some_and(|set| set.contains(&self.0)) {
            return Err(keyring::Error::NoStorageAccess(
                format!(
                    "keychain is locked; refused {}",
                    String::from_utf8_lossy(secret)
                )
                .into(),
            ));
        }
        let mut values = VALUES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        values
            .get_or_insert_with(HashMap::new)
            .insert(self.0.clone(), secret.to_vec());
        Ok(())
    }
    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        let unreadable = UNREADABLE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if unreadable.as_ref().is_some_and(|set| set.contains(&self.0)) {
            return Err(keyring::Error::NoStorageAccess("keychain is locked".into()));
        }
        let values = VALUES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        values
            .as_ref()
            .and_then(|values| values.get(&self.0).cloned())
            .ok_or(keyring::Error::NoEntry)
    }
    fn delete_credential(&self) -> keyring::Result<()> {
        let failing = FAILING
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failing.as_ref().is_some_and(|set| set.contains(&self.0)) {
            let held = self.get_secret().unwrap_or_default();
            return Err(keyring::Error::NoStorageAccess(
                format!(
                    "keychain is locked; kept {}",
                    String::from_utf8_lossy(&held)
                )
                .into(),
            ));
        }
        let mut values = VALUES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        values
            .as_mut()
            .and_then(|values| values.remove(&self.0))
            .map(|_| ())
            .ok_or(keyring::Error::NoEntry)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl std::fmt::Debug for MemoryCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemoryCredential")
    }
}
