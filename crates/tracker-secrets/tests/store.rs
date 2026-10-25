use gespan_tracker_secrets::testing::{fail_reads, fail_writes, stored, use_memory_keychain};
use gespan_tracker_secrets::{account, delete, read, write, Tracker};
use std::{fs, path::Path};

fn dir() -> tempfile::TempDir {
    use_memory_keychain();
    tempfile::tempdir().unwrap()
}

fn file(dir: &Path, name: &str) -> std::path::PathBuf {
    dir.join(name)
}

#[test]
fn migrates_the_old_file_once_and_deletes_it() {
    let dir = dir();
    fs::write(file(dir.path(), "linear-token"), "lin_api_x\n").unwrap();

    assert_eq!(
        read(Tracker::Linear, dir.path()).unwrap().as_deref(),
        Some("lin_api_x\n")
    );
    assert!(!file(dir.path(), "linear-token").exists());
    assert_eq!(
        stored(Tracker::Linear, dir.path()).as_deref(),
        Some("lin_api_x\n")
    );
    assert_eq!(
        read(Tracker::Linear, dir.path()).unwrap().as_deref(),
        Some("lin_api_x\n")
    );
}

#[test]
fn migrates_each_trackers_own_file() {
    let dir = dir();
    for (tracker, name) in [
        (Tracker::Jira, "jira-config.json"),
        (Tracker::Gitlab, "gitlab-config.json"),
        (Tracker::AzureDevOps, "azure-devops-config.json"),
    ] {
        fs::write(file(dir.path(), name), name).unwrap();
        assert_eq!(read(tracker, dir.path()).unwrap().as_deref(), Some(name));
        assert!(!file(dir.path(), name).exists(), "{name} was not removed");
    }
    assert_eq!(read(Tracker::Linear, dir.path()).unwrap(), None);
    assert_eq!(read(Tracker::Plane, dir.path()).unwrap(), None);
}

#[test]
fn keychain_value_wins_and_leftover_file_is_removed() {
    let dir = dir();
    write(Tracker::Jira, dir.path(), "new").unwrap();
    fs::write(file(dir.path(), "jira-config.json"), "old").unwrap();

    assert_eq!(
        read(Tracker::Jira, dir.path()).unwrap().as_deref(),
        Some("new")
    );
    assert!(!file(dir.path(), "jira-config.json").exists());
}

#[test]
fn failed_keychain_write_keeps_the_file() {
    let dir = dir();
    fs::write(file(dir.path(), "gitlab-config.json"), "cfg").unwrap();
    fail_writes(Tracker::Gitlab, dir.path(), true);

    assert_eq!(
        read(Tracker::Gitlab, dir.path()).unwrap().as_deref(),
        Some("cfg")
    );
    assert!(file(dir.path(), "gitlab-config.json").exists());
    assert_eq!(stored(Tracker::Gitlab, dir.path()), None);

    fail_writes(Tracker::Gitlab, dir.path(), false);
    assert_eq!(
        read(Tracker::Gitlab, dir.path()).unwrap().as_deref(),
        Some("cfg")
    );
    assert!(!file(dir.path(), "gitlab-config.json").exists());
    assert_eq!(stored(Tracker::Gitlab, dir.path()).as_deref(), Some("cfg"));
}

#[test]
fn write_replaces_the_old_file() {
    let dir = dir();
    fs::write(file(dir.path(), "linear-token"), "old").unwrap();

    write(Tracker::Linear, dir.path(), "v").unwrap();

    assert!(!file(dir.path(), "linear-token").exists());
    assert_eq!(stored(Tracker::Linear, dir.path()).as_deref(), Some("v"));
}

#[test]
fn delete_clears_keychain_and_file() {
    let dir = dir();
    write(Tracker::AzureDevOps, dir.path(), "new").unwrap();
    fs::write(file(dir.path(), "azure-devops-config.json"), "old").unwrap();

    delete(Tracker::AzureDevOps, dir.path()).unwrap();

    assert_eq!(read(Tracker::AzureDevOps, dir.path()).unwrap(), None);
    assert!(!file(dir.path(), "azure-devops-config.json").exists());
    delete(Tracker::AzureDevOps, dir.path()).unwrap();
}

#[test]
fn data_directories_are_isolated() {
    let (first, second) = (dir(), dir());
    write(Tracker::Linear, first.path(), "one").unwrap();
    write(Tracker::Linear, second.path(), "two").unwrap();

    assert_eq!(
        read(Tracker::Linear, first.path()).unwrap().as_deref(),
        Some("one")
    );
    assert_eq!(
        read(Tracker::Linear, second.path()).unwrap().as_deref(),
        Some("two")
    );
    delete(Tracker::Linear, first.path()).unwrap();
    assert_eq!(
        read(Tracker::Linear, second.path()).unwrap().as_deref(),
        Some("two")
    );
}

#[test]
fn errors_never_contain_the_secret() {
    let dir = dir();
    fail_writes(Tracker::Plane, dir.path(), true);

    let error = write(Tracker::Plane, dir.path(), "plane_api_secret").unwrap_err();

    assert!(!error.contains("plane_api_secret"), "{error}");
    assert!(error.contains("Plane"), "{error}");
}

#[test]
fn account_names_match_the_shared_contract() {
    assert_eq!(account(Tracker::Jira, Path::new("/d")), "jira:/d");
    assert_eq!(
        account(Tracker::AzureDevOps, Path::new("/d")),
        "azuredevops:/d"
    );
    assert_eq!(Tracker::Linear.legacy_file(), Some("linear-token"));
    assert_eq!(Tracker::Jira.legacy_file(), Some("jira-config.json"));
    assert_eq!(Tracker::Gitlab.legacy_file(), Some("gitlab-config.json"));
    assert_eq!(
        Tracker::AzureDevOps.legacy_file(),
        Some("azure-devops-config.json")
    );
    assert_eq!(Tracker::Plane.legacy_file(), None);
}

#[test]
fn unreadable_keychain_still_serves_and_keeps_the_file() {
    let dir = dir();
    fs::write(file(dir.path(), "jira-config.json"), "cfg").unwrap();
    fail_reads(Tracker::Jira, dir.path(), true);

    assert_eq!(
        read(Tracker::Jira, dir.path()).unwrap().as_deref(),
        Some("cfg")
    );
    assert!(file(dir.path(), "jira-config.json").exists());

    fail_reads(Tracker::Jira, dir.path(), false);
    assert_eq!(
        read(Tracker::Jira, dir.path()).unwrap().as_deref(),
        Some("cfg")
    );
    assert!(!file(dir.path(), "jira-config.json").exists());
}

#[test]
fn unreadable_keychain_with_no_old_file_reads_as_nothing_saved() {
    // Someone who never connected the tracker, on a machine whose keychain is
    // locked, denied or missing: "not connected", not an error.
    let dir = dir();
    for tracker in [
        Tracker::Linear,
        Tracker::Jira,
        Tracker::Gitlab,
        Tracker::AzureDevOps,
        Tracker::Plane,
    ] {
        fail_reads(tracker, dir.path(), true);
        assert_eq!(read(tracker, dir.path()).unwrap(), None);
    }
}

#[test]
fn two_spellings_of_one_data_directory_share_an_entry() {
    let dir = dir();
    fs::create_dir(dir.path().join("sub")).unwrap();
    let spelled = dir.path().join("sub").join("..");
    fs::write(file(dir.path(), "linear-token"), "lin_api_x").unwrap();

    // One app migrates through its spelling of the directory...
    assert_eq!(
        read(Tracker::Linear, &spelled).unwrap().as_deref(),
        Some("lin_api_x")
    );
    assert!(!file(dir.path(), "linear-token").exists());
    // ...and the other, spelling it plainly, still finds the secret.
    assert_eq!(
        read(Tracker::Linear, dir.path()).unwrap().as_deref(),
        Some("lin_api_x")
    );
    assert_eq!(
        account(Tracker::Linear, &spelled),
        account(Tracker::Linear, dir.path())
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_data_directory_shares_the_entry() {
    let dir = dir();
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    write(Tracker::Gitlab, &link, "cfg").unwrap();

    assert_eq!(
        read(Tracker::Gitlab, &real).unwrap().as_deref(),
        Some("cfg")
    );
}

#[test]
fn a_refused_save_says_how_to_fix_the_keychain() {
    let dir = dir();
    fail_writes(Tracker::Linear, dir.path(), true);

    let error = write(Tracker::Linear, dir.path(), "lin_api_x").unwrap_err();

    assert!(error.contains("unlocked"), "{error}");
    assert!(error.contains("Secret Service"), "{error}");
}
