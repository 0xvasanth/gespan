CREATE TABLE "graph_edges" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "src" BIGINT NOT NULL,
    "dst" BIGINT NOT NULL,
    "kind" TEXT NOT NULL,
    "props_json" TEXT NOT NULL,
    "valid_from" BIGINT NOT NULL,
    "valid_from_basis" TEXT NOT NULL,
    "valid_to" BIGINT,
    "source" TEXT NOT NULL,
    "scope" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "last_seen_run" BIGINT
);
-- #[toasty::breakpoint]
CREATE INDEX "graph_edges_scope" ON "graph_edges" ("scope");
-- #[toasty::breakpoint]
CREATE TABLE "graph_runs" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "started_at" BIGINT NOT NULL,
    "finished_at" BIGINT,
    "observed_at" BIGINT NOT NULL,
    "status" TEXT NOT NULL,
    "report_json" TEXT
);
-- #[toasty::breakpoint]
CREATE TABLE "graph_nodes" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "kind" TEXT NOT NULL,
    "key" TEXT NOT NULL,
    "label" TEXT NOT NULL,
    "props_json" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "updated_at" BIGINT NOT NULL,
    "last_seen_run" BIGINT
);
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "graph_nodes_kind_key" ON "graph_nodes" ("kind", "key");
-- #[toasty::breakpoint]
CREATE TABLE "graph_scopes" (
    "source" TEXT NOT NULL,
    "scope" TEXT NOT NULL,
    "observed_updated_at" BIGINT NOT NULL,
    "observed_at" BIGINT NOT NULL,
    PRIMARY KEY ("source", "scope")
);
-- #[toasty::breakpoint]
CREATE TABLE "graph_run_scopes" (
    "run" BIGINT NOT NULL,
    "source" TEXT NOT NULL,
    "scope" TEXT NOT NULL,
    "close_at" BIGINT NOT NULL,
    PRIMARY KEY ("run", "source", "scope")
);
