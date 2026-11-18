CREATE TABLE "orch_commands" (
    "command_id" TEXT NOT NULL,
    "caller_session_id" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "result_json" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    PRIMARY KEY ("command_id")
);
-- #[toasty::breakpoint]
CREATE INDEX "orch_commands_caller_idx" ON "orch_commands" ("caller_session_id");
-- #[toasty::breakpoint]
CREATE TABLE "session_links" (
    "child_session_id" TEXT NOT NULL,
    "parent_session_id" TEXT NOT NULL,
    "root_session_id" TEXT NOT NULL,
    "depth" BIGINT NOT NULL,
    "origin" TEXT NOT NULL,
    "wake" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    PRIMARY KEY ("child_session_id")
);
-- #[toasty::breakpoint]
CREATE INDEX "session_links_parent_idx" ON "session_links" ("parent_session_id");
-- #[toasty::breakpoint]
CREATE TABLE "orch_effects" (
    "effect_id" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "session_id" TEXT NOT NULL,
    "payload_json" TEXT NOT NULL,
    "state" TEXT NOT NULL,
    "attempts" BIGINT NOT NULL,
    "last_error" TEXT,
    "created_at" BIGINT NOT NULL,
    PRIMARY KEY ("effect_id")
);
-- #[toasty::breakpoint]
CREATE INDEX "orch_effects_state_idx" ON "orch_effects" ("state");
-- #[toasty::breakpoint]
CREATE TABLE "orch_events" (
    "seq" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "at" BIGINT NOT NULL,
    "kind" TEXT NOT NULL,
    "session_id" TEXT NOT NULL,
    "payload_json" TEXT NOT NULL,
    "command_id" TEXT
);
-- #[toasty::breakpoint]
CREATE INDEX "orch_events_session_idx" ON "orch_events" ("session_id");
