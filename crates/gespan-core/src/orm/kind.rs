//! The databases Gespan keeps, each with its models, migrations and the
//! legacy step that adoption runs first.
use toasty::migration::MigrationSet;
use toasty::ModelSet;

/// Which database a file is. Each kind has its own models and migrations
/// in `migrations/<name>/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// The shared `gespan.db`: sessions, notes, My tasks and the rest.
    Gespan,
    Settings,
    Counters,
    Browser,
    Graph,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Gespan,
        Kind::Settings,
        Kind::Counters,
        Kind::Browser,
        Kind::Graph,
    ];

    /// The name the migration tool takes and `migrations/<name>/` uses.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Gespan => "gespan",
            Kind::Settings => "settings",
            Kind::Counters => "counters",
            Kind::Browser => "browser",
            Kind::Graph => "graph",
        }
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// Registration point: every model stored in this kind's database. A
    /// model added here needs a migration (`toasty-migrate <kind> <name>`);
    /// the drift test fails until it has one.
    pub fn models(self) -> ModelSet {
        use crate::automations::{AutomationRow, AutomationRunRow, EventClaimRow, ReviewDraftRow};
        use crate::decisions::store::DecisionFlag;
        use crate::diagnostics::metrics::DiagnosticCounter;
        use crate::flow::store::{FlowRow, FlowRunRow, FlowVisitRow};
        use crate::store::my_tasks::{MyTask, TaskDecision};
        use crate::store::schema::*;
        match self {
            Kind::Gespan => {
                let mut models = toasty::models!(
                    MyTask,
                    TaskDecision,
                    DecisionFlag,
                    SessionRecord,
                    TauriMigration,
                    WorkspaceSnapshot,
                    InFlightSession,
                    NoteRecord,
                    SessionReminder,
                    PilotTranscript,
                    PilotTurn,
                    PilotBlock,
                    OrchestrationRun,
                    OrchestrationSidebar,
                    OrchestrationWorker,
                    AutomationRow,
                    AutomationRunRow,
                    EventClaimRow,
                    ReviewDraftRow,
                    FlowRow,
                    FlowRunRow,
                    FlowVisitRow
                );
                gespan_board_store::register_models(&mut models);
                models
            }
            Kind::Settings => toasty::models!(Setting),
            Kind::Counters => toasty::models!(DiagnosticCounter),
            Kind::Browser => gespan_browser::audit::models(),
            Kind::Graph => gespan_graph::schema::models(),
        }
    }

    /// This kind's migrations, embedded from `migrations/<name>/`.
    pub fn migrations(self) -> MigrationSet {
        match self {
            Kind::Gespan => toasty::embed_migrations!("migrations/gespan"),
            Kind::Settings => toasty::embed_migrations!("migrations/settings"),
            Kind::Counters => toasty::embed_migrations!("migrations/counters"),
            Kind::Browser => toasty::embed_migrations!("migrations/browser"),
            Kind::Graph => toasty::embed_migrations!("migrations/graph"),
        }
    }

    /// Registration point: SQL that Toasty's generated DDL cannot express,
    /// run on every open after the migrations, on fresh and adopted files
    /// alike. Use it for what the hand-made tables had and Toasty drops:
    /// foreign-key cascades and CHECK rules (as triggers), partial and
    /// expression indexes, descending index columns. Each statement must be
    /// idempotent (`CREATE TRIGGER IF NOT EXISTS`, `CREATE INDEX IF NOT
    /// EXISTS`). To change one, add it under a new name and drop the old one
    /// here (`DROP TRIGGER IF EXISTS`).
    pub(crate) fn constraints(self) -> &'static [&'static str] {
        match self {
            Kind::Gespan => {
                static GESPAN: std::sync::LazyLock<Vec<&str>> = std::sync::LazyLock::new(|| {
                    [
                        crate::store::schema::CONSTRAINTS,
                        gespan_board_store::CONSTRAINTS,
                        crate::automations::CONSTRAINTS,
                        crate::flow::store::CONSTRAINTS,
                        crate::decisions::store::CONSTRAINTS,
                    ]
                    .concat()
                });
                &GESPAN
            }
            Kind::Browser => gespan_browser::audit::CONSTRAINTS,
            Kind::Graph => gespan_graph::schema::constraints(),
            Kind::Settings | Kind::Counters => &[],
        }
    }

    /// Registration point: brings a file the hand-written stores made up to
    /// their newest hand-made shape, before adoption rebuilds its tables into
    /// the first migration's DDL. Runs inside the adoption transaction with
    /// foreign keys off. Move a store's `ALTER TABLE` / `pragma_table_info`
    /// upgrades here when its tables move to Toasty; a table the file lacks
    /// needs nothing, adoption creates it.
    pub(crate) fn legacy_upgrade(self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        match self {
            // `my_tasks` and `task_decisions` only ever had one shape.
            Kind::Gespan => {
                crate::store::my_tasks::legacy_upgrade(conn)?;
                crate::store::schema::legacy_upgrade(conn)?;
                crate::board::legacy_upgrade(conn)?;
                crate::automations::legacy_upgrade(conn)
            }
            // Graph schema v1 is the only hand-made shape. Its version table
            // has no model; a fresh install has none.
            Kind::Graph => conn.execute_batch("DROP TABLE IF EXISTS graph_schema"),
            // The settings, counters and browser tables only ever had one shape.
            Kind::Settings | Kind::Counters | Kind::Browser => Ok(()),
        }
    }
}
