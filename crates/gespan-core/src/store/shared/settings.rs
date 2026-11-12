//! Native metadata lives separately from the desktop schema, in the Settings
//! database (`orm::Kind::Settings`). Updating one key must not clone,
//! serialize and fsync every cached app result under the Store mutex. SQLite
//! commits only the changed key and retains durable receipts.
//!
//! Before this file existed, the values lived in `<db>.gpui.json`. They are
//! imported once, with `user_version` 1 marking the import done; that import
//! is the Settings database's only legacy step.
use super::parse;
use crate::orm::{Database, Kind};
use crate::store::schema::Setting;
use std::{collections::BTreeMap, path::Path};

type Snapshot = BTreeMap<String, String>;

fn legacy(path: &Path) -> Result<Snapshot, String> {
    match std::fs::read_to_string(path) {
        Ok(raw) => parse(&raw),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error.to_string()),
    }
}

/// Saves one setting.
pub(super) fn save(database: &Database, key: &str, value: &str) -> Result<(), String> {
    let (key, value) = (key.to_owned(), value.to_owned());
    database.run(|mut db| async move {
        Setting::upsert_by_key(key)
            .value(value)
            .exec(&mut db)
            .await
            .map(drop)
    })
}

/// Removes the settings `keys`.
pub(super) fn remove(database: &Database, keys: Vec<String>) -> Result<(), String> {
    database.run(|mut db| async move {
        Setting::filter(Setting::fields().key().in_list(keys))
            .delete()
            .exec(&mut db)
            .await
    })
}

/// The `user_version` of the Settings database: 1 once the legacy JSON is in.
fn import_marker(database: &Database) -> Result<i64, String> {
    let rows = database.run(|mut db| async move {
        toasty::sql::query("PRAGMA user_version")
            .exec(&mut db)
            .await
    })?;
    match rows.first() {
        Some(toasty::stmt::Value::Record(record)) => match record.fields.first() {
            Some(toasty::stmt::Value::I64(version)) => Ok(*version),
            other => Err(format!("Unexpected user_version {other:?}")),
        },
        other => Err(format!("Unexpected user_version {other:?}")),
    }
}

pub(super) fn open(
    database: &Path,
    read_only: bool,
) -> Result<(Snapshot, Option<Database>), String> {
    if database == Path::new(":memory:") {
        return Ok((BTreeMap::new(), None));
    }
    let name = database.file_name().unwrap_or_default().to_string_lossy();
    let path = database.with_file_name(format!("{name}.gpui.sqlite3"));
    let old_path = database.with_file_name(format!("{name}.gpui.json"));
    if read_only {
        return read(&path, &old_path).map(|values| (values, None));
    }
    // Match the old metadata file's permissions before storing any values.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(_) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.to_string()),
    }
    let settings = crate::orm::open(Kind::Settings, &path)?;
    settings.run(|mut db| async move {
        toasty::sql::statement("PRAGMA synchronous=FULL")
            .exec(&mut db)
            .await
            .map(drop)
    })?;
    let version = import_marker(&settings)?;
    if version > 1 {
        return Err(format!("Unsupported native settings version: {version}"));
    }
    if version == 0 {
        let values = legacy(&old_path)?;
        let rows = values.clone();
        // Another native window may have finished the import while this one
        // waited for the write lock: the marker is read again inside.
        settings.transaction(|tx| {
            Box::pin(async move {
                let rows_marker = toasty::sql::query("PRAGMA user_version").exec(tx).await?;
                let done = !matches!(
                    rows_marker.first(),
                    Some(toasty::stmt::Value::Record(record))
                        if matches!(record.fields.first(), Some(toasty::stmt::Value::I64(0)))
                );
                if done {
                    return Ok(());
                }
                for (key, value) in rows {
                    Setting::upsert_by_key(key).value(value).exec(tx).await?;
                }
                // The marker and all imported keys commit together. Keep the old
                // file untouched as a backup, but never re-import stale values.
                toasty::sql::statement("PRAGMA user_version=1")
                    .exec(tx)
                    .await?;
                Ok(())
            })
        })?;
    }
    let values = settings.run(|mut db| async move { Setting::all().exec(&mut db).await })?;
    let values = values
        .into_iter()
        .map(|setting| (setting.key, setting.value))
        .collect();
    Ok((values, Some(settings)))
}

/// A read-only store's settings: never creates, imports or adopts anything.
fn read(path: &Path, old_path: &Path) -> Result<Snapshot, String> {
    if !path.exists() {
        return legacy(old_path);
    }
    let settings = crate::orm::open_read_only(Kind::Settings, path)?;
    let version = import_marker(&settings)?;
    if version > 1 {
        return Err(format!("Unsupported native settings version: {version}"));
    }
    if version == 0 {
        // An interrupted initial import has not replaced the legacy snapshot.
        return legacy(old_path);
    }
    let values = settings.run(|mut db| async move { Setting::all().exec(&mut db).await })?;
    Ok(values
        .into_iter()
        .map(|setting| (setting.key, setting.value))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn legacy_settings_and_large_receipts_import_once_without_changing_desktop_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        drop(Store::open_gespan(&path).unwrap());
        // Exercise a legacy installation, before the sidecar existed.
        crate::orm::remove_released(&dir.path().join("gespan.db.gpui.sqlite3"));
        let old = dir.path().join("gespan.db.gpui.json");
        let receipt = "large result".repeat(100_000);
        let values = BTreeMap::from([
            ("workspace", "saved tabs"),
            ("pilot-app-receipt:1:request", receipt.as_str()),
        ]);
        let original = serde_json::to_vec(&values).unwrap();
        std::fs::write(&old, &original).unwrap();
        let desktop = crate::orm::settled_bytes(&path);
        let store = Store::open_gespan(&path).unwrap();
        assert_eq!(
            store.setting("workspace").unwrap().as_deref(),
            Some("saved tabs")
        );
        assert_eq!(
            store.setting("pilot-app-receipt:1:request").unwrap(),
            Some(receipt.clone())
        );
        store.save_setting("workspace", "new tabs").unwrap();
        drop(store);
        assert_eq!(std::fs::read(&old).unwrap(), original);
        assert_eq!(crate::orm::settled_bytes(&path), desktop);
        let reopened = Store::open_gespan(&path).unwrap();
        assert_eq!(
            reopened.setting("workspace").unwrap().as_deref(),
            Some("new tabs")
        );
        assert_eq!(
            reopened.setting("pilot-app-receipt:1:request").unwrap(),
            Some(receipt)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.path().join("gespan.db.gpui.sqlite3"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn small_updates_do_not_rewrite_large_cached_results_and_unchanged_saves_do_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_gespan(&dir.path().join("gespan.db")).unwrap();
        store
            .save_setting("pilot-app-receipt:1:large", &"x".repeat(10_000_000))
            .unwrap();
        let database = store.shared.extras_db.as_ref().unwrap();
        let changes = || -> String {
            let rows = database
                .run(|mut db| async move {
                    toasty::sql::query("SELECT total_changes()")
                        .exec(&mut db)
                        .await
                })
                .unwrap();
            format!("{rows:?}")
        };
        database
            .run(|mut db| async move {
                toasty::sql::query("PRAGMA wal_checkpoint(TRUNCATE)")
                    .exec(&mut db)
                    .await
            })
            .unwrap();
        let wal = dir.path().join("gespan.db.gpui.sqlite3-wal");
        for i in 0..10 {
            store.save_setting("workspace", &i.to_string()).unwrap();
        }
        let bytes = std::fs::metadata(&wal).unwrap().len();
        assert!(bytes < 100_000, "ten tiny updates wrote {bytes} bytes");
        let before = changes();
        store.save_setting("workspace", "9").unwrap();
        assert_eq!(changes(), before);
        assert_eq!(std::fs::metadata(&wal).unwrap().len(), bytes);
    }

    #[test]
    fn failed_save_preserves_the_committed_cache_and_can_be_retried() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_gespan(&dir.path().join("gespan.db")).unwrap();
        store.save_setting("workspace", "old").unwrap();
        let database = store.shared.extras_db.as_ref().unwrap();
        let pragma = |sql: &'static str| {
            database
                .run(|mut db| async move { toasty::sql::statement(sql).exec(&mut db).await })
                .unwrap()
        };
        pragma("PRAGMA query_only=ON");
        assert!(store.save_setting("workspace", "new").is_err());
        assert_eq!(store.setting("workspace").unwrap().as_deref(), Some("old"));
        pragma("PRAGMA query_only=OFF");
        store.save_setting("workspace", "new").unwrap();
        assert_eq!(store.setting("workspace").unwrap().as_deref(), Some("new"));
    }

    #[test]
    fn read_only_legacy_access_does_not_create_or_import_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let old = dir.path().join("gespan.db.gpui.json");
        std::fs::write(&old, r#"{"workspace":"saved"}"#).unwrap();
        let (values, conn) = open(&path, true).unwrap();
        assert_eq!(values["workspace"], "saved");
        assert!(conn.is_none());
        assert!(!dir.path().join("gespan.db.gpui.sqlite3").exists());
        assert_eq!(
            std::fs::read_to_string(&old).unwrap(),
            r#"{"workspace":"saved"}"#
        );
    }

    #[test]
    fn read_only_reopen_sees_current_settings_and_refuses_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let store = Store::open_gespan(&path).unwrap();
        store.save_setting("workspace", "current").unwrap();
        let reader = Store::read_gespan(&path).unwrap();
        assert_eq!(
            reader.setting("workspace").unwrap().as_deref(),
            Some("current")
        );
        assert!(reader.save_setting("workspace", "wrong").is_err());
    }

    #[test]
    fn failed_import_is_retryable_without_losing_legacy_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let old = dir.path().join("gespan.db.gpui.json");
        std::fs::write(&old, "invalid JSON").unwrap();
        assert!(open(&path, false).is_err());
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "invalid JSON");
        std::fs::write(&old, r#"{"workspace":"recovered"}"#).unwrap();
        assert_eq!(open(&path, false).unwrap().0["workspace"], "recovered");
    }
}
