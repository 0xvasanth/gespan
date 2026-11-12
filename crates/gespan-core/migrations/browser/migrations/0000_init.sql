CREATE TABLE "browser_settings" (
    "key" TEXT NOT NULL,
    "value" TEXT NOT NULL,
    PRIMARY KEY ("key")
);
-- #[toasty::breakpoint]
CREATE TABLE "browser_audit" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "ts" BIGINT NOT NULL,
    "session_id" TEXT NOT NULL,
    "request_id" TEXT NOT NULL,
    "global_epoch" TEXT NOT NULL,
    "session_epoch" TEXT NOT NULL,
    "action" TEXT NOT NULL,
    "tab_id" TEXT,
    "url" TEXT,
    "params" TEXT NOT NULL,
    "outcome" TEXT NOT NULL,
    "reason" TEXT,
    "duration_ms" BIGINT NOT NULL,
    "commands_sent" BIGINT NOT NULL
);
-- #[toasty::breakpoint]
CREATE INDEX "browser_audit_ts" ON "browser_audit" ("ts");
-- #[toasty::breakpoint]
CREATE TABLE "browser_leases" (
    "tab_id" TEXT NOT NULL,
    "run_id" TEXT NOT NULL,
    "target_id" TEXT NOT NULL,
    "session_id" TEXT NOT NULL,
    "opened_by" TEXT,
    "url" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "closed_at" BIGINT,
    PRIMARY KEY ("tab_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "browser_events" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "ts" BIGINT NOT NULL,
    "kind" TEXT NOT NULL,
    "actor" TEXT NOT NULL,
    "session_id" TEXT,
    "detail" TEXT
);
-- #[toasty::breakpoint]
CREATE INDEX "browser_events_ts" ON "browser_events" ("ts");
