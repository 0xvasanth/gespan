ALTER TABLE "task_decisions" ADD COLUMN "import_src" TEXT;
-- #[toasty::breakpoint]
ALTER TABLE "task_decisions" ADD COLUMN "title" TEXT NOT NULL DEFAULT '';
-- #[toasty::breakpoint]
ALTER TABLE "task_decisions" ADD COLUMN "decided_at" BIGINT;
-- #[toasty::breakpoint]
ALTER TABLE "task_decisions" ADD COLUMN "session" BIGINT;
-- #[toasty::breakpoint]
ALTER TABLE "task_decisions" ADD COLUMN "kind" TEXT NOT NULL DEFAULT '';
-- #[toasty::breakpoint]
ALTER TABLE "task_decisions" ADD COLUMN "question_ref" TEXT;
-- #[toasty::breakpoint]
ALTER TABLE "task_decisions" ADD COLUMN "status" TEXT NOT NULL DEFAULT '';
-- #[toasty::breakpoint]
ALTER TABLE "task_decisions" ADD COLUMN "pinned" BOOLEAN NOT NULL DEFAULT 0;
-- #[toasty::breakpoint]
ALTER TABLE "task_decisions" ADD COLUMN "user_answer" TEXT;
-- #[toasty::breakpoint]
ALTER TABLE "task_decisions" ADD COLUMN "replaced_by" BIGINT;
-- #[toasty::breakpoint]
ALTER TABLE "task_decisions" ADD COLUMN "supersedes" BIGINT;
-- #[toasty::breakpoint]
ALTER TABLE "task_decisions" ADD COLUMN "user_turn" TEXT;
-- #[toasty::breakpoint]
ALTER TABLE "task_decisions" ADD COLUMN "scope" TEXT NOT NULL DEFAULT '';
-- #[toasty::breakpoint]
CREATE INDEX "task_decisions_scope_status_idx" ON "task_decisions" ("scope", "status");
-- #[toasty::breakpoint]
CREATE INDEX "task_decisions_replaced_by_idx" ON "task_decisions" ("replaced_by");
-- #[toasty::breakpoint]
CREATE TABLE "decision_flags" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "entry" BIGINT NOT NULL,
    "session" BIGINT NOT NULL,
    "reason" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "dismissed" BOOLEAN NOT NULL
);
-- #[toasty::breakpoint]
CREATE INDEX "decision_flags_entry_idx" ON "decision_flags" ("entry", "dismissed");
-- #[toasty::breakpoint]
UPDATE "task_decisions" SET
    "kind" = 'decision',
    "status" = CASE WHEN "archived_at" IS NULL THEN 'active' ELSE 'archived' END,
    "scope" = 'project:' || "project",
    "title" = substr(trim(substr("statement", 1, instr("statement" || char(10), char(10)) - 1), ' ' || char(9) || char(13)), 1, 80),
    "pinned" = 0;
