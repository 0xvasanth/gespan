//! The decision log's vectors, for finding related entries by meaning.
//!
//! Each embeddable entry (active or pending) has one row in the sqlite-vec
//! table `task_decisions_vec`, keyed by the entry's id (`rowid`) and
//! partitioned by its scope string, so a nearest-neighbour query reads one
//! scope's vectors and never another's. The row also keeps a fingerprint of
//! the model and text it was embedded from, so a vector whose entry was
//! edited elsewhere (My tasks) is noticed and embedded again.
//!
//! Which model made the vectors is recorded in `decision_vector_model`, a
//! Toasty model with one row per vector table. Changing the model empties
//! the vector table in the same transaction that records the new model, so
//! vectors of two models are never compared.
//!
//! `vec0` is a virtual table, which Toasty cannot model, so every statement
//! on `task_decisions_vec` here is raw SQL. Nothing else in the log is.
use std::collections::HashMap;

use crate::store::db::{self, args, Sql};
use sha2::{Digest, Sha256};

/// The vector table's name, as the SQL below spells it.
pub(crate) const TABLE: &str = "task_decisions_vec";

/// Run with the gespan kind's constraints on every open: the `vec0` table,
/// partitioned by scope, with the spike's smaller `chunk_size` (vec0
/// allocates storage a chunk per partition, and most scopes hold few
/// entries). Cosine distance, so vectors need not be normalized.
/// `float[768]` is [`super::embed::DIMS`].
pub(crate) const CONSTRAINTS: &[&str] = &[
    "CREATE VIRTUAL TABLE IF NOT EXISTS task_decisions_vec USING vec0(
       scope text partition key,
       embedding float[768] distance_metric=cosine,
       +fingerprint text,
       chunk_size=64
     )",
];

/// Which model made the vectors in a vector table.
///
/// One row per vector table, not per vector: a per-row model would have to
/// live in the `vec0` table itself, keyed by its rowids, which Toasty cannot
/// model. With one row it is an ordinary Toasty model and migration.
#[derive(Debug, toasty::Model)]
#[table = "decision_vector_model"]
pub(crate) struct DecisionVectorModel {
    /// The vector table this describes ([`TABLE`]).
    #[key]
    pub(crate) vectors: String,
    pub(crate) model: String,
    pub(crate) dims: i64,
}

/// An entry's vector and the fingerprint of what it was embedded from.
#[derive(Clone, Debug)]
pub(crate) struct Vector {
    pub(crate) embedding: Vec<f32>,
    pub(crate) fingerprint: String,
}

/// What a write does to an entry's vector.
#[derive(Clone, Debug)]
pub(crate) enum Embedding {
    /// Search by meaning is off: the vector is left alone (the next pass
    /// after it comes on checks it).
    Off,
    /// The embedder failed on the entry's new text: any old vector, of other
    /// text, is removed, so it never stands for words the entry no longer has.
    Failed,
    Vector(Vector),
}

impl Embedding {
    /// Applies this to entry `id` in partition `scope`, inside the
    /// transaction that writes the entry.
    pub(crate) async fn apply(
        &self,
        tx: &mut toasty::Transaction<'_>,
        id: i64,
        scope: &str,
    ) -> toasty::Result<()> {
        match self {
            Embedding::Off => Ok(()),
            Embedding::Failed => remove(tx, &[id]).await,
            Embedding::Vector(v) => put(tx, id, scope, v).await,
        }
    }
}

/// The text an entry is embedded from.
pub(crate) fn document(title: &str, text: &str, why: &str) -> String {
    format!("{title}\n{text}\n{why}")
}

/// Identifies `model`'s embedding of an entry with this title, text and why.
pub(crate) fn fingerprint(model: &str, title: &str, text: &str, why: &str) -> String {
    let mut hash = Sha256::new();
    for part in [model, title, text, why] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// How sqlite-vec takes a float vector: little-endian `f32`s.
pub(crate) fn blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Every stored vector's entry id, with its scope and fingerprint.
pub(crate) fn stored(db: &crate::orm::Database) -> Result<HashMap<i64, (String, String)>, String> {
    // A vec0 virtual table, which Toasty cannot model: raw SQL.
    Sql::Orm(db)
        .query(
            "SELECT rowid, scope, fingerprint FROM task_decisions_vec",
            args![],
        )?
        .iter()
        .map(|r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))
        .collect()
}

/// How many vectors partition `scope` holds.
pub(crate) fn partition_size(db: &crate::orm::Database, scope: &str) -> Result<i64, String> {
    // A vec0 virtual table, which Toasty cannot model: raw SQL.
    Sql::Orm(db).value(
        "SELECT COUNT(*) FROM task_decisions_vec WHERE scope = ?1",
        args![scope],
    )
}

/// The `k` vectors in partition `scope` nearest `query` (a [`blob`]), as
/// entry ids with their cosine distances, nearest first.
pub(crate) fn knn(
    db: &crate::orm::Database,
    scope: &str,
    query: &[u8],
    k: usize,
) -> Result<Vec<(i64, f64)>, String> {
    // sqlite-vec's nearest-neighbour MATCH on a vec0 virtual table, which
    // Toasty cannot model: raw SQL.
    Sql::Orm(db)
        .query(
            "SELECT rowid, distance FROM task_decisions_vec \
             WHERE embedding MATCH ?1 AND k = ?2 AND scope = ?3 ORDER BY distance",
            args![query.to_vec(), i64::try_from(k).unwrap_or(i64::MAX), scope],
        )?
        .iter()
        .map(|r| Ok((r.get(0)?, r.get(1)?)))
        .collect()
}

/// Writes entry `id`'s vector in partition `scope`, replacing any it had.
/// vec0 has no upsert and cannot move a row between partitions, so it is a
/// delete and an insert; run it inside the transaction that wrote the entry.
pub(crate) async fn put(
    tx: &mut toasty::Transaction<'_>,
    id: i64,
    scope: &str,
    v: &Vector,
) -> toasty::Result<()> {
    remove(tx, &[id]).await?;
    // vec0 is a virtual table, which Toasty cannot model: raw SQL.
    db::statement(
        "INSERT INTO task_decisions_vec(rowid, scope, embedding, fingerprint) \
         VALUES (?1, ?2, ?3, ?4)",
        args![id, scope, blob(&v.embedding), v.fingerprint.clone()],
    )
    .exec(&mut *tx)
    .await?;
    Ok(())
}

/// Removes the vectors of `ids` (entries that can no longer be shown).
pub(crate) async fn remove(tx: &mut toasty::Transaction<'_>, ids: &[i64]) -> toasty::Result<()> {
    for &id in ids {
        // vec0 is a virtual table, which Toasty cannot model: raw SQL.
        db::statement("DELETE FROM task_decisions_vec WHERE rowid = ?1", args![id])
            .exec(&mut *tx)
            .await?;
    }
    Ok(())
}

/// Records `model` as what made the vectors. When it differs from what was
/// recorded (or nothing was), every vector is deleted in the same
/// transaction: they belong to another model. True when that happened.
pub(crate) async fn adopt_model(
    tx: &mut toasty::Transaction<'_>,
    model: &str,
    dims: usize,
) -> toasty::Result<bool> {
    let dims = i64::try_from(dims).unwrap_or(i64::MAX);
    let recorded = DecisionVectorModel::filter_by_vectors(TABLE)
        .first()
        .exec(&mut *tx)
        .await?;
    match recorded {
        Some(row) if row.model == model && row.dims == dims => Ok(false),
        recorded => {
            if recorded.is_some() {
                DecisionVectorModel::filter_by_vectors(TABLE)
                    .delete()
                    .exec(&mut *tx)
                    .await?;
            }
            // vec0 is a virtual table, which Toasty cannot model: raw SQL.
            db::statement("DELETE FROM task_decisions_vec", args![])
                .exec(&mut *tx)
                .await?;
            DecisionVectorModel::create()
                .vectors(TABLE)
                .model(model)
                .dims(dims)
                .exec(&mut *tx)
                .await?;
            Ok(true)
        }
    }
}

/// Reciprocal rank fusion: each list ranks ids best first, and an id scores
/// `1 / (k + rank)` (rank from 1) in every list it is in. Highest total
/// first; ties go to the id ranked best in any list, then the larger id
/// (the newer entry).
pub(crate) fn fuse(lists: &[Vec<i64>], k: f64) -> Vec<i64> {
    use std::collections::HashMap;
    let mut scores: HashMap<i64, (f64, usize)> = HashMap::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            let entry = scores.entry(*id).or_insert((0.0, usize::MAX));
            entry.0 += 1.0 / (k + rank as f64 + 1.0);
            entry.1 = entry.1.min(rank);
        }
    }
    let mut ids: Vec<(i64, f64, usize)> = scores
        .into_iter()
        .map(|(id, (score, best))| (id, score, best))
        .collect();
    ids.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.2.cmp(&b.2)).then(b.0.cmp(&a.0)));
    ids.into_iter().map(|(id, ..)| id).collect()
}
