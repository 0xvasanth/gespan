//! Search by meaning: the `vec0` table, its partitions, the model sidecar,
//! atomic entry-and-vector writes and hybrid (keyword + vector) search.
//! Every test embeds with `HashEmbedder`; nothing downloads a model.
use crate::decisions::embed::{Embedder, HashEmbedder, DIMS};
use crate::decisions::entry::Error;
use crate::decisions::store::RRF_K;
use crate::decisions::vector::{self, fuse};
use crate::decisions::{Config, DecisionStore, Edit, Entry, Kind, NewEntry, Scope, Source, Status};
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

struct Log {
    dir: TempDir,
    store: DecisionStore,
}

impl Log {
    fn path(&self) -> PathBuf {
        self.dir.path().join("gespan.db")
    }

    /// A raw connection on the same file, to look at what was stored.
    fn conn(&self) -> Connection {
        Connection::open(self.path()).unwrap()
    }

    /// Each vector's entry id with its partition (scope) and fingerprint.
    fn vectors(&self) -> BTreeMap<i64, (String, String)> {
        let conn = self.conn();
        let mut statement = conn
            .prepare("SELECT rowid, scope, fingerprint FROM task_decisions_vec")
            .unwrap();
        statement
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn embedding(&self, id: i64) -> Vec<u8> {
        self.conn()
            .query_row(
                "SELECT embedding FROM task_decisions_vec WHERE rowid = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn rows(&self) -> i64 {
        self.conn()
            .query_row("SELECT COUNT(*) FROM task_decisions", [], |r| r.get(0))
            .unwrap()
    }
}

fn log() -> Log {
    let dir = TempDir::new().unwrap();
    let store = DecisionStore::open(&dir.path().join("gespan.db"), Config::app()).unwrap();
    Log { dir, store }
}

fn with_hash(log: &Log, model: &str) {
    log.store
        .set_embedder(Some(Arc::new(HashEmbedder::named(model))))
        .unwrap();
}

fn repo(name: &str) -> Scope {
    Scope::Repo(format!("github.com/o/{name}@0123456789ab"))
}

/// A pending proposal the user answered (so other sessions see it).
fn propose(store: &DecisionStore, scope: &Scope, kind: Kind, title: &str, text: &str) -> Entry {
    store
        .propose(
            NewEntry {
                kind,
                scope: scope.clone(),
                title: title.into(),
                text: text.into(),
                why: "the user said so".into(),
                user_answer: None,
                source: Source {
                    session: Some(3),
                    question_ref: Some("q".into()),
                    ..Source::default()
                },
            },
            10,
        )
        .unwrap()
}

fn active(store: &DecisionStore, scope: &Scope, title: &str, text: &str) -> Entry {
    let e = propose(store, scope, Kind::Decision, title, text);
    store.approve(e.id, None, false, &[], 20).unwrap()
}

fn ids(entries: &[Entry]) -> Vec<i64> {
    entries.iter().map(|e| e.id).collect()
}

fn query(text: &str) -> Vec<f32> {
    HashEmbedder::new().embed_query(text).unwrap()
}

#[test]
fn sqlite_vec_is_on_every_connection_and_the_table_matches_the_dims() {
    let log = log();
    // A plain connection opened after the store has the module too: it was
    // registered for every connection of the process, not just Toasty's.
    let version: String = log
        .conn()
        .query_row("SELECT vec_version()", [], |r| r.get(0))
        .unwrap();
    assert!(version.starts_with('v'), "{version}");
    assert!(vector::CONSTRAINTS[0].contains(&format!("float[{DIMS}]")));
    // The constraints run on every open: a second open of the file is fine.
    drop(DecisionStore::open(&log.path(), Config::app()).unwrap());
}

#[test]
fn vec_table_is_partitioned_by_scope_and_never_crosses() {
    let log = log();
    with_hash(&log, "hash-v1");
    let s = &log.store;
    let a = active(s, &repo("a"), "Tabs", "Indent with tabs");
    // The same words in another project: nearest of all by meaning.
    let b = active(s, &repo("b"), "Tabs", "Indent with tabs");
    let pref = active(s, &Scope::Global, "Terse", "Answer tersely");

    // Each vector sits in its entry's scope partition.
    let stored = log.vectors();
    assert_eq!(stored[&a.id].0, repo("a").to_string());
    assert_eq!(stored[&b.id].0, repo("b").to_string());
    assert_eq!(stored[&pref.id].0, "global");

    let near = s.related(&repo("a"), &query("tabs indent"), 10).unwrap();
    assert_eq!(ids(&near), [a.id, pref.id], "own scope, then global");
    let near = s.related(&repo("b"), &query("tabs indent"), 10).unwrap();
    assert_eq!(ids(&near), [b.id, pref.id]);
    // A scope with nothing of its own sees only the global partition.
    let near = s.related(&repo("c"), &query("tabs indent"), 10).unwrap();
    assert_eq!(ids(&near), [pref.id]);
    // The global scope reads its own partition only.
    let near = s
        .related(&Scope::Global, &query("tabs indent"), 10)
        .unwrap();
    assert_eq!(ids(&near), [pref.id]);
    // Hybrid search keeps to the scope as well.
    let found = s.search(&repo("a"), "tabs", 10).unwrap();
    assert!(found.iter().all(|e| e.id != b.id), "{found:?}");
    assert!(found.iter().any(|e| e.id == a.id));

    // Many entries in another scope that are nearer the query than repo a's
    // own: the nearest-neighbour query reads only repo a's partition, so
    // they never take the places of repo a's entries.
    let crowd = "Always indent every file with hard tabs";
    for _ in 0..80 {
        active(s, &repo("b"), "Tabs", crowd);
    }
    let doc = vector::document("Tabs", crowd, "the user said so");
    let near = s.related(&repo("a"), &query(&doc), 1).unwrap();
    assert_eq!(ids(&near), [a.id]);
}

#[test]
fn rejected_and_replaced_rows_never_returned() {
    let log = log();
    with_hash(&log, "hash-v1");
    let s = &log.store;
    let scope = repo("a");
    let rejected = propose(s, &scope, Kind::Decision, "Tabs", "Indent with tabs");
    s.reject(rejected.id, 30).unwrap();
    let old = active(s, &scope, "Tabs", "Indent with tabs, width 4");
    let new = propose(
        s,
        &scope,
        Kind::Decision,
        "Tabs",
        "Indent with tabs, width 8",
    );
    s.approve(new.id, None, false, &[old.id], 40).unwrap();
    let archived = active(s, &scope, "Tabs", "Indent with tabs in Go");
    s.archive(archived.id, 50).unwrap();
    // A rejected proposal replaced through a flag, too.
    let flagged = active(s, &scope, "Tab width", "Tabs are 2 wide");
    let flag = s.flag(flagged.id, 9, "too narrow", 60).unwrap();
    let successor = s
        .replace_from_flag(
            flagged.id,
            flag.id,
            Edit {
                title: "Tab width".into(),
                text: "Tabs are 4 wide".into(),
                why: "the user said so".into(),
            },
            70,
        )
        .unwrap();

    let near = s.related(&scope, &query("indent with tabs"), 10).unwrap();
    let mut got = ids(&near);
    got.sort();
    assert_eq!(got, [new.id, successor.id]);
    let found = s.search(&scope, "tabs indent", 10).unwrap();
    let mut got = ids(&found);
    got.sort();
    assert_eq!(got, [new.id, successor.id]);
    // Their vectors are gone, so they can't crowd the live ones out.
    let stored: Vec<i64> = log.vectors().into_keys().collect();
    assert_eq!(stored, [new.id, successor.id]);
}

#[test]
fn changing_model_reembeds_every_row() {
    let log = log();
    with_hash(&log, "hash-v1");
    let s = &log.store;
    let a = active(s, &repo("a"), "Tabs", "Indent with tabs");
    let b = propose(s, &repo("b"), Kind::Decision, "Rebase", "Prefer rebase");
    let p = active(s, &Scope::Global, "Terse", "Answer tersely");
    let fp = |model: &str, e: &Entry| vector::fingerprint(model, &e.title, &e.text, &e.why);
    let v1 = log.vectors();
    for e in [&a, &b, &p] {
        assert_eq!(v1[&e.id].1, fp("hash-v1", e));
    }
    let model = |log: &Log| -> (String, i64) {
        log.conn()
            .query_row(
                "SELECT model, dims FROM decision_vector_model WHERE vectors = 'task_decisions_vec'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    assert_eq!(model(&log), ("hash-v1".to_string(), 768));

    // The same model again embeds nothing.
    with_hash(&log, "hash-v1");
    assert_eq!(s.reembed().unwrap(), 0);

    with_hash(&log, "hash-v2");
    assert_eq!(model(&log), ("hash-v2".to_string(), 768));
    let v2 = log.vectors();
    assert_eq!(v2.len(), 3);
    let other = HashEmbedder::named("hash-v2");
    for e in [&a, &b, &p] {
        assert_eq!(v2[&e.id].1, fp("hash-v2", e));
        let doc = vector::document(&e.title, &e.text, &e.why);
        let expected = vector::blob(&other.embed_document(&doc).unwrap());
        assert_eq!(log.embedding(e.id), expected, "entry {}", e.id);
    }

    // An entry edited outside the log (My tasks) is the only one embedded
    // again on the next pass.
    log.conn()
        .execute(
            "UPDATE task_decisions SET statement = 'Indent with spaces' WHERE id = ?1",
            [a.id],
        )
        .unwrap();
    assert_eq!(s.reembed().unwrap(), 1);
    let edited = s.get(a.id).unwrap().unwrap().0;
    assert_eq!(log.vectors()[&a.id].1, fp("hash-v2", &edited));
}

#[test]
fn an_embedder_of_another_size_is_refused() {
    struct Small;
    impl Embedder for Small {
        fn model_id(&self) -> &str {
            "small"
        }
        fn dims(&self) -> usize {
            384
        }
        fn embed_document(&self, _: &str) -> Result<Vec<f32>, Error> {
            Ok(vec![1.0; 384])
        }
        fn embed_query(&self, _: &str) -> Result<Vec<f32>, Error> {
            Ok(vec![1.0; 384])
        }
    }
    let log = log();
    with_hash(&log, "hash-v1");
    let a = active(&log.store, &repo("a"), "Tabs", "Indent with tabs");
    assert!(matches!(
        log.store.set_embedder(Some(Arc::new(Small))),
        Err(Error::Embed(_))
    ));
    // Nothing changed: the old model and its vectors stay.
    assert_eq!(log.store.embedder().unwrap().model_id(), "hash-v1");
    assert!(log.vectors().contains_key(&a.id));
}

/// Says it makes 768 floats, but its document vectors are one short: the
/// `vec0` insert fails after the entry row was written, in the same
/// transaction, as a crash between the two writes would.
struct ShortDocuments;

impl Embedder for ShortDocuments {
    fn model_id(&self) -> &str {
        "short"
    }
    fn dims(&self) -> usize {
        DIMS
    }
    fn embed_document(&self, text: &str) -> Result<Vec<f32>, Error> {
        let mut v = HashEmbedder::new().embed_document(text)?;
        v.pop();
        Ok(v)
    }
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, Error> {
        HashEmbedder::new().embed_query(text)
    }
}

#[test]
fn crash_between_entry_and_vector_write_leaves_no_orphan() {
    let log = log();
    with_hash(&log, "hash-v1");
    let s = &log.store;
    let kept = active(s, &repo("a"), "Tabs", "Indent with tabs");
    let pending = propose(s, &repo("a"), Kind::Decision, "Rebase", "Prefer rebase");
    let rows = log.rows();
    s.set_embedder(Some(Arc::new(ShortDocuments))).unwrap_err();
    // `set_embedder` recorded the new model (the old vectors are gone) and
    // failed on its first re-embed: no entry has a vector now.
    assert!(log.vectors().is_empty());

    // A proposal whose vector write fails leaves no entry behind.
    let refused = s.propose(
        NewEntry {
            kind: Kind::Decision,
            scope: repo("a"),
            title: "Merge".into(),
            text: "Never merge".into(),
            why: "the user said so".into(),
            user_answer: None,
            source: Source::default(),
        },
        30,
    );
    assert!(refused.is_err());
    assert_eq!(log.rows(), rows, "no entry without its vector");

    // Approving with an edit: neither the approval nor the edit is kept.
    let edit = Edit {
        title: "Rebase".into(),
        text: "Always rebase".into(),
        why: "the user said so".into(),
    };
    assert!(s
        .approve(pending.id, Some(edit.clone()), false, &[], 40)
        .is_err());
    let still = s.get(pending.id).unwrap().unwrap().0;
    assert_eq!(still.status, Status::Pending);
    assert_eq!(still.text, "Prefer rebase");

    // Replacing from a flag: no successor, and the old entry stays active.
    let flag = s.flag(kept.id, 9, "spaces", 50).unwrap();
    assert!(s.replace_from_flag(kept.id, flag.id, edit, 60).is_err());
    assert_eq!(log.rows(), rows);
    assert_eq!(s.get(kept.id).unwrap().unwrap().0.status, Status::Active);
    assert!(log.vectors().is_empty(), "no vector without its entry");

    // A working embedder again: every live entry gets exactly one vector.
    with_hash(&log, "hash-v1");
    let stored: Vec<i64> = log.vectors().into_keys().collect();
    assert_eq!(stored, [kept.id, pending.id]);
}

#[test]
fn hybrid_search_merges_keyword_and_vector_by_rrf() {
    // The fusion itself, by hand: 1/(60 + rank) summed over the lists.
    // 3 is in both (1/63 + 1/61); 1 is first in one (1/61); 4 and 2 are
    // second in one each (1/62), the newer (larger) id first.
    assert_eq!(fuse(&[vec![1, 2, 3], vec![3, 4]], RRF_K), [3, 1, 4, 2]);
    assert_eq!(RRF_K, 60.0);

    let log = log();
    let s = &log.store;
    let scope = repo("a");
    let words = active(s, &scope, "Rebase", "Prefer rebase over merge");
    // No word in common with "rebase", only word parts.
    let meaning = active(s, &scope, "History", "Rebasing keeps history linear");
    let other = active(s, &scope, "Deploys", "Never deploy on Fridays");

    // Keyword only (search by meaning off): the stemmed form isn't found.
    let keyword = s.search(&scope, "rebase", 10).unwrap();
    assert_eq!(ids(&keyword), [words.id]);

    with_hash(&log, "hash-v1");
    let by_meaning = s.related(&scope, &query("rebase"), 50).unwrap();
    assert_eq!(by_meaning[0].id, words.id);
    assert_eq!(by_meaning[1].id, meaning.id);
    let hybrid = s.search(&scope, "rebase", 10).unwrap();
    assert_eq!(
        ids(&hybrid),
        fuse(&[ids(&keyword), ids(&by_meaning)], RRF_K),
        "keyword and vector ranks fused with k = 60"
    );
    assert_eq!(ids(&hybrid)[..2], [words.id, meaning.id]);
    assert!(ids(&hybrid).contains(&other.id));
    assert_eq!(s.search(&scope, "rebase", 1).unwrap()[0].id, words.id);
    assert!(s.search(&scope, "  *  ", 10).unwrap().is_empty());

    // Off again: exactly keyword search.
    s.set_embedder(None).unwrap();
    assert_eq!(ids(&s.search(&scope, "rebase", 10).unwrap()), [words.id]);
    assert!(s.related(&scope, &query("rebase"), 10).unwrap().is_empty());
}

#[test]
fn entries_written_while_off_are_embedded_when_it_comes_on() {
    let log = log();
    let s = &log.store;
    let a = active(s, &repo("a"), "Tabs", "Indent with tabs");
    assert!(log.vectors().is_empty(), "nothing is embedded while off");
    with_hash(&log, "hash-v1");
    assert_eq!(log.vectors().keys().copied().collect::<Vec<_>>(), [a.id]);
    // Moved to another scope behind the log's back: the next pass moves the
    // vector to that partition.
    log.conn()
        .execute(
            "UPDATE task_decisions SET scope = ?1 WHERE id = ?2",
            rusqlite::params![repo("b").to_string(), a.id],
        )
        .unwrap();
    assert!(s.related(&repo("b"), &query("tabs"), 5).unwrap().is_empty());
    assert_eq!(s.reembed().unwrap(), 1);
    assert_eq!(log.vectors()[&a.id].0, repo("b").to_string());
    assert_eq!(
        ids(&s.related(&repo("b"), &query("tabs"), 5).unwrap()),
        [a.id]
    );
}
