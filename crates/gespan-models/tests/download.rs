mod support;

use gespan_models::{
    CancelToken, DownloadError, ModelFile, ModelSpec, ModelStatus, ModelStore, Progress,
};
use sha2::{Digest, Sha256};
use std::fs;
use support::{dead_base, Reply, TestServer};

const CHUNK: usize = 16 * 1024;

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn body(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_add(seed)).collect()
}

fn leak<T>(v: T) -> &'static T {
    Box::leak(Box::new(v))
}

/// Files named `a.bin`, `b.bin` with the given contents, served at `/a.bin`, `/b.bin`.
fn spec(bases: Vec<String>, contents: &[&[u8]]) -> &'static ModelSpec {
    let files: Vec<ModelFile> = contents
        .iter()
        .enumerate()
        .map(|(i, c)| ModelFile {
            name: leak(format!("{}.bin", (b'a' + i as u8) as char)),
            url_path: leak(format!("/{}.bin", (b'a' + i as u8) as char)),
            size: c.len() as u64,
            sha256: leak(sha256(c)),
        })
        .collect();
    let bases: Vec<&'static str> = bases.into_iter().map(|b| &**leak(b)).collect();
    leak(ModelSpec {
        id: "test-model",
        label: "Test model",
        base_urls: leak(bases),
        files: leak(files),
    })
}

fn store(dir: &tempfile::TempDir) -> ModelStore {
    ModelStore::new(dir.path().to_path_buf()).with_chunk_size(CHUNK)
}

fn serve_files(files: Vec<Vec<u8>>) -> TestServer {
    TestServer::start(move |path, _| {
        let i = (path.as_bytes()[1] - b'a') as usize;
        Reply {
            body: files[i].clone(),
            cut_at: None,
            ignore_range: false,
        }
    })
}

fn part_files(store: &ModelStore, spec: &ModelSpec) -> Vec<String> {
    fs::read_dir(store.dir(spec))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".part"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn downloads_and_verifies() {
    let (a, b) = (body(100_000, 1), body(60_000, 2));
    let server = serve_files(vec![a.clone(), b.clone()]);
    let spec = spec(vec![server.base.clone()], &[&a, &b]);
    let tmp = tempfile::tempdir().unwrap();
    let store = store(&tmp);

    assert_eq!(store.status(spec), ModelStatus::Missing);
    let mut seen: Vec<Progress> = Vec::new();
    store
        .download(spec, &CancelToken::new(), |p| seen.push(p))
        .unwrap();

    assert_eq!(store.status(spec), ModelStatus::Ready);
    assert_eq!(fs::read(store.dir(spec).join("a.bin")).unwrap(), a);
    assert_eq!(fs::read(store.dir(spec).join("b.bin")).unwrap(), b);
    assert!(seen.windows(2).all(|w| w[0].downloaded <= w[1].downloaded));
    assert!(seen.iter().all(|p| p.total == 160_000));
    assert_eq!(seen.last().unwrap().downloaded, 160_000);
    assert!(part_files(&store, spec).is_empty());
}

#[test]
fn resumes_from_partial() {
    let a = body(100_000, 3);
    let served = a.clone();
    let server = TestServer::start(move |_, nth| Reply {
        body: served.clone(),
        cut_at: (nth == 0).then_some(40_000),
        ignore_range: false,
    });
    let spec = spec(vec![server.base.clone()], &[&a]);
    let tmp = tempfile::tempdir().unwrap();
    let store = store(&tmp);

    let first = store.download(spec, &CancelToken::new(), |_| {});
    assert!(matches!(first, Err(DownloadError::Network(_))), "{first:?}");
    assert_eq!(
        store.status(spec),
        ModelStatus::Partial {
            downloaded: 40_000,
            total: 100_000
        }
    );

    store.download(spec, &CancelToken::new(), |_| {}).unwrap();
    assert_eq!(store.status(spec), ModelStatus::Ready);
    assert_eq!(fs::read(store.dir(spec).join("a.bin")).unwrap(), a);

    let reqs = server.recorded();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].range, None);
    assert_eq!(reqs[1].range.as_deref(), Some("bytes=40000-"));
}

#[test]
fn corrupt_file_redownloads() {
    let a = body(80_000, 4);
    let (good, bad) = (a.clone(), body(80_000, 99));
    let server = TestServer::start(move |_, nth| Reply {
        body: if nth == 0 { bad.clone() } else { good.clone() },
        cut_at: None,
        ignore_range: false,
    });
    let spec = spec(vec![server.base.clone()], &[&a]);
    let tmp = tempfile::tempdir().unwrap();
    let store = store(&tmp);

    store.download(spec, &CancelToken::new(), |_| {}).unwrap();

    assert_eq!(store.status(spec), ModelStatus::Ready);
    let reqs = server.recorded();
    assert_eq!(reqs.len(), 2);
    assert!(
        reqs.iter().all(|r| r.range.is_none()),
        "retry must restart from zero"
    );
}

#[test]
fn persistent_checksum_mismatch_errors() {
    let a = body(50_000, 5);
    let bad = body(50_000, 77);
    let server = TestServer::start(move |_, _| Reply {
        body: bad.clone(),
        cut_at: None,
        ignore_range: false,
    });
    let spec = spec(vec![server.base.clone()], &[&a]);
    let tmp = tempfile::tempdir().unwrap();
    let store = store(&tmp);

    let err = store
        .download(spec, &CancelToken::new(), |_| {})
        .unwrap_err();
    assert!(
        matches!(&err, DownloadError::Checksum { file } if file == "a.bin"),
        "{err:?}"
    );
    assert_eq!(server.recorded().len(), 2, "exactly one retry");
    assert!(part_files(&store, spec).is_empty());
    assert!(!store.dir(spec).join("a.bin").exists());
    assert_ne!(store.status(spec), ModelStatus::Ready);
}

#[test]
fn cancel_leaves_no_partial_files() {
    let a = body(200_000, 6);
    let server = serve_files(vec![a.clone()]);
    let spec = spec(vec![server.base.clone()], &[&a]);
    let tmp = tempfile::tempdir().unwrap();
    let store = store(&tmp);

    let cancel = CancelToken::new();
    let mut calls = 0;
    let result = store.download(spec, &cancel, |_| {
        calls += 1;
        cancel.cancel();
    });

    assert!(
        matches!(result, Err(DownloadError::Cancelled)),
        "{result:?}"
    );
    assert_eq!(calls, 1, "stops right after the first chunk");
    assert!(part_files(&store, spec).is_empty());
    assert_eq!(store.status(spec), ModelStatus::Missing);
}

#[test]
fn falls_back_to_mirror() {
    let a = body(70_000, 7);
    let server = serve_files(vec![a.clone()]);
    let spec = spec(vec![dead_base(), server.base.clone()], &[&a]);
    let tmp = tempfile::tempdir().unwrap();
    let store = store(&tmp);

    store.download(spec, &CancelToken::new(), |_| {}).unwrap();
    assert_eq!(store.status(spec), ModelStatus::Ready);
}

#[test]
fn mirror_resumes_the_partial_left_by_the_primary() {
    let a = body(100_000, 8);
    let primary_body = a.clone();
    let primary = TestServer::start(move |_, _| Reply {
        body: primary_body.clone(),
        cut_at: Some(30_000),
        ignore_range: false,
    });
    let mirror = serve_files(vec![a.clone()]);
    let spec = spec(vec![primary.base.clone(), mirror.base.clone()], &[&a]);
    let tmp = tempfile::tempdir().unwrap();
    let store = store(&tmp);

    store.download(spec, &CancelToken::new(), |_| {}).unwrap();

    assert_eq!(store.status(spec), ModelStatus::Ready);
    assert_eq!(mirror.recorded()[0].range.as_deref(), Some("bytes=30000-"));
}

#[test]
fn server_ignoring_range_restarts_cleanly() {
    let a = body(60_000, 9);
    let served = a.clone();
    let server = TestServer::start(move |_, _| Reply {
        body: served.clone(),
        cut_at: None,
        ignore_range: true,
    });
    let spec = spec(vec![server.base.clone()], &[&a]);
    let tmp = tempfile::tempdir().unwrap();
    let store = store(&tmp);
    fs::create_dir_all(store.dir(spec)).unwrap();
    fs::write(store.dir(spec).join("a.bin.part"), &a[..20_000]).unwrap();

    store.download(spec, &CancelToken::new(), |_| {}).unwrap();

    assert_eq!(server.recorded()[0].range.as_deref(), Some("bytes=20000-"));
    assert_eq!(store.status(spec), ModelStatus::Ready);
    assert_eq!(fs::read(store.dir(spec).join("a.bin")).unwrap(), a);
}

#[test]
fn truncated_final_file_is_not_ready() {
    let a = body(10_000, 10);
    let spec = spec(vec!["http://127.0.0.1:1".into()], &[&a]);
    let tmp = tempfile::tempdir().unwrap();
    let store = store(&tmp);
    fs::create_dir_all(store.dir(spec)).unwrap();
    fs::write(store.dir(spec).join("a.bin"), &a[..4_000]).unwrap();
    fs::write(store.dir(spec).join("a.bin.verified"), b"").unwrap();

    assert_eq!(
        store.status(spec),
        ModelStatus::Partial {
            downloaded: 4_000,
            total: 10_000
        }
    );
}

#[test]
fn full_size_file_without_marker_is_not_ready_until_verified() {
    let a = body(10_000, 11);
    let server = serve_files(vec![a.clone()]);
    let spec = spec(vec![server.base.clone()], &[&a]);
    let tmp = tempfile::tempdir().unwrap();
    let store = store(&tmp);
    fs::create_dir_all(store.dir(spec)).unwrap();
    fs::write(store.dir(spec).join("a.bin"), &a).unwrap();

    assert!(matches!(store.status(spec), ModelStatus::Partial { .. }));
    store.download(spec, &CancelToken::new(), |_| {}).unwrap();
    assert_eq!(store.status(spec), ModelStatus::Ready);
    assert!(
        server.recorded().is_empty(),
        "verified in place, not re-fetched"
    );
}

#[test]
fn verified_files_are_not_fetched_again() {
    let (a, b) = (body(20_000, 12), body(20_000, 13));
    let server = serve_files(vec![a.clone(), b.clone()]);
    let spec = spec(vec![server.base.clone()], &[&a, &b]);
    let tmp = tempfile::tempdir().unwrap();
    let store = store(&tmp);
    store.download(spec, &CancelToken::new(), |_| {}).unwrap();
    fs::remove_file(store.dir(spec).join("b.bin")).unwrap();
    fs::remove_file(store.dir(spec).join("b.bin.verified")).unwrap();
    let before = server.recorded().len();

    store.download(spec, &CancelToken::new(), |_| {}).unwrap();

    let after = server.recorded();
    assert_eq!(after.len(), before + 1);
    assert_eq!(after.last().unwrap().path, "/b.bin");
}

#[test]
fn delete_removes_model_dir() {
    let a = body(10_000, 14);
    let server = serve_files(vec![a.clone()]);
    let spec = spec(vec![server.base.clone()], &[&a]);
    let tmp = tempfile::tempdir().unwrap();
    let store = store(&tmp);
    store.download(spec, &CancelToken::new(), |_| {}).unwrap();

    store.delete(spec).unwrap();
    assert!(!store.dir(spec).exists());
    assert_eq!(store.status(spec), ModelStatus::Missing);
    store.delete(spec).unwrap(); // deleting a missing model is fine
}
