CREATE TABLE "board_labels" (
    "id" TEXT NOT NULL,
    "board_id" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "color" TEXT NOT NULL,
    "position" BIGINT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "board_labels_board_idx" ON "board_labels" ("board_id", "position");
-- #[toasty::breakpoint]
CREATE TABLE "board_session_settings" (
    "session_key" TEXT NOT NULL,
    "may_change_status" BOOLEAN,
    PRIMARY KEY ("session_key")
);
-- #[toasty::breakpoint]
CREATE TABLE "board_cards" (
    "id" TEXT NOT NULL,
    "board_id" TEXT NOT NULL,
    "column_id" TEXT NOT NULL,
    "number" BIGINT NOT NULL,
    "title" TEXT NOT NULL,
    "body" TEXT NOT NULL,
    "position" BIGINT NOT NULL,
    "link_kind" TEXT,
    "link_id" TEXT,
    "created_at" BIGINT NOT NULL,
    "updated_at" BIGINT NOT NULL,
    "title_from_ticket" BOOLEAN NOT NULL,
    "body_from_ticket" BOOLEAN NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "board_cards_column_idx" ON "board_cards" ("column_id", "position");
-- #[toasty::breakpoint]
CREATE INDEX "board_cards_link_idx" ON "board_cards" ("link_kind", "link_id");
-- #[toasty::breakpoint]
CREATE TABLE "flow_runs" (
    "id" TEXT NOT NULL,
    "flow_id" TEXT NOT NULL,
    "status" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "run_json" TEXT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "flow_runs_history_idx" ON "flow_runs" ("flow_id", "created_at");
-- #[toasty::breakpoint]
CREATE INDEX "flow_runs_status_idx" ON "flow_runs" ("status");
-- #[toasty::breakpoint]
CREATE TABLE "board_card_projects" (
    "card_id" TEXT NOT NULL,
    "project" TEXT NOT NULL,
    PRIMARY KEY ("card_id", "project")
);
-- #[toasty::breakpoint]
CREATE TABLE "board_card_links" (
    "card_id" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "ref" TEXT NOT NULL,
    "role" TEXT,
    "created_at" BIGINT NOT NULL,
    PRIMARY KEY ("card_id", "kind", "ref")
);
-- #[toasty::breakpoint]
CREATE INDEX "board_card_links_ref_idx" ON "board_card_links" ("kind", "ref");
-- #[toasty::breakpoint]
CREATE TABLE "board_card_meta" (
    "card_id" TEXT NOT NULL,
    "priority" BIGINT NOT NULL,
    "due_at" BIGINT,
    "start_at" BIGINT,
    "estimate_minutes" BIGINT,
    "archived_at" BIGINT,
    "completed_at" BIGINT,
    PRIMARY KEY ("card_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "in_flight_sessions" (
    "session_id" TEXT NOT NULL,
    "cwd" TEXT NOT NULL,
    "sort_index" BIGINT NOT NULL,
    PRIMARY KEY ("session_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "task_decisions" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "project" TEXT NOT NULL,
    "statement" TEXT NOT NULL,
    "why" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "updated_at" BIGINT NOT NULL,
    "archived_at" BIGINT
);
-- #[toasty::breakpoint]
CREATE TABLE "orchestration_workers" (
    "session_id" TEXT NOT NULL,
    "lead_id" TEXT NOT NULL,
    PRIMARY KEY ("session_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "flow_visits" (
    "id" TEXT NOT NULL,
    "run_id" TEXT NOT NULL,
    "seq" BIGINT NOT NULL,
    "state_id" TEXT NOT NULL,
    "status" TEXT NOT NULL,
    "visit_json" TEXT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "flow_visits_order_idx" ON "flow_visits" ("run_id", "seq");
-- #[toasty::breakpoint]
CREATE TABLE "my_tasks" (
    "key" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "project" TEXT NOT NULL,
    "origin" TEXT NOT NULL,
    "source_label" TEXT NOT NULL,
    "headline" TEXT NOT NULL,
    "request_json" TEXT NOT NULL,
    "asked_at" BIGINT NOT NULL,
    "status" TEXT NOT NULL,
    "answer" TEXT,
    "answer_json" TEXT,
    "closed_at" BIGINT,
    "decision_id" BIGINT,
    PRIMARY KEY ("key")
);
-- #[toasty::breakpoint]
CREATE INDEX "my_tasks_closed_idx" ON "my_tasks" ("status", "closed_at");
-- #[toasty::breakpoint]
CREATE INDEX "my_tasks_decision_idx" ON "my_tasks" ("decision_id");
-- #[toasty::breakpoint]
CREATE TABLE "board_comments" (
    "id" TEXT NOT NULL,
    "card_id" TEXT NOT NULL,
    "body" TEXT NOT NULL,
    "author" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "edited_at" BIGINT,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "board_comments_card_idx" ON "board_comments" ("card_id", "created_at");
-- #[toasty::breakpoint]
CREATE TABLE "board_settings" (
    "board_id" TEXT NOT NULL,
    "run_with" BOOLEAN NOT NULL,
    "write_back" BOOLEAN NOT NULL,
    "ticket_status_changes" BOOLEAN NOT NULL,
    "auto_move_json" TEXT,
    "icon" TEXT,
    "color" TEXT,
    "default_view_id" TEXT,
    "display_json" TEXT,
    "priority_json" TEXT,
    "run_in_worktree" BOOLEAN,
    "run_worktree_choice" BOOLEAN,
    PRIMARY KEY ("board_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "automation_event_claims" (
    "automation_id" TEXT NOT NULL,
    "event_key" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    PRIMARY KEY ("automation_id", "event_key")
);
-- #[toasty::breakpoint]
CREATE TABLE "orchestration_sidebar" (
    "lead_id" TEXT NOT NULL,
    "summary" TEXT NOT NULL,
    PRIMARY KEY ("lead_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "workspace_snapshot" (
    "id" BIGINT NOT NULL,
    "snapshot_json" TEXT NOT NULL,
    "updated_at" BIGINT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE TABLE "pilot_transcripts" (
    "session_id" TEXT NOT NULL,
    PRIMARY KEY ("session_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "schema_migrations" (
    "version" BIGINT NOT NULL,
    "applied_at" BIGINT NOT NULL,
    PRIMARY KEY ("version")
);
-- #[toasty::breakpoint]
CREATE TABLE "board_activity" (
    "id" TEXT NOT NULL,
    "board_id" TEXT NOT NULL,
    "card_id" TEXT,
    "kind" TEXT NOT NULL,
    "actor" TEXT NOT NULL,
    "from_column" TEXT,
    "to_column" TEXT,
    "detail" TEXT,
    "created_at" BIGINT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "board_activity_board_idx" ON "board_activity" ("board_id", "created_at");
-- #[toasty::breakpoint]
CREATE INDEX "board_activity_card_idx" ON "board_activity" ("card_id", "created_at");
-- #[toasty::breakpoint]
CREATE TABLE "board_subtasks" (
    "id" TEXT NOT NULL,
    "card_id" TEXT NOT NULL,
    "title" TEXT NOT NULL,
    "done" BOOLEAN NOT NULL,
    "position" BIGINT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "board_subtasks_card_idx" ON "board_subtasks" ("card_id", "position");
-- #[toasty::breakpoint]
CREATE TABLE "boards" (
    "id" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "card_seq" BIGINT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "updated_at" BIGINT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE TABLE "board_fields" (
    "id" TEXT NOT NULL,
    "board_id" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "options_json" TEXT,
    "position" BIGINT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "board_fields_board_idx" ON "board_fields" ("board_id", "position");
-- #[toasty::breakpoint]
CREATE TABLE "review_drafts" (
    "run_id" TEXT NOT NULL,
    "automation_id" TEXT NOT NULL,
    "cwd" TEXT NOT NULL,
    "title" TEXT NOT NULL,
    "draft_json" TEXT NOT NULL,
    "status" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "updated_at" BIGINT NOT NULL,
    PRIMARY KEY ("run_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "board_views" (
    "id" TEXT NOT NULL,
    "board_id" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "position" BIGINT NOT NULL,
    "spec_json" TEXT NOT NULL,
    "layout_json" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "updated_at" BIGINT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "board_views_board_idx" ON "board_views" ("board_id", "position");
-- #[toasty::breakpoint]
CREATE TABLE "session_reminders" (
    "session_id" TEXT NOT NULL,
    "due_at" BIGINT NOT NULL,
    "fired_at" BIGINT,
    PRIMARY KEY ("session_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "automation_runs" (
    "id" TEXT NOT NULL,
    "automation_id" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "run_json" TEXT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "automation_runs_history_idx" ON "automation_runs" ("automation_id", "created_at");
-- #[toasty::breakpoint]
CREATE TABLE "orchestration_runs" (
    "lead_id" TEXT NOT NULL,
    "state" TEXT NOT NULL,
    PRIMARY KEY ("lead_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "pilot_turns" (
    "session_id" TEXT NOT NULL,
    "seq" BIGINT NOT NULL,
    PRIMARY KEY ("session_id", "seq")
);
-- #[toasty::breakpoint]
CREATE TABLE "sessions" (
    "id" TEXT NOT NULL,
    "cwd" TEXT NOT NULL,
    "harness" TEXT NOT NULL,
    "model" TEXT NOT NULL,
    "model_settings" TEXT NOT NULL,
    "runtime_mode" TEXT NOT NULL,
    "title" TEXT NOT NULL,
    "provider_session_id" TEXT,
    "blocks_json" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "updated_at" BIGINT NOT NULL,
    "branch" TEXT,
    "context_used" BIGINT,
    "context_window" BIGINT,
    "archived" BOOLEAN NOT NULL,
    "worktree_cwd" TEXT,
    "has_user_message" BOOLEAN NOT NULL,
    "pinned" BOOLEAN NOT NULL,
    "linked_work_item_json" TEXT,
    "provider_account_id" TEXT,
    "worktree_removed" BOOLEAN NOT NULL,
    "is_draft" BOOLEAN NOT NULL,
    "automation_id" TEXT,
    "queued_messages_json" TEXT NOT NULL,
    "queue_status" TEXT,
    "sidebar_hidden" BOOLEAN NOT NULL,
    "inbox_ask" TEXT,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE TABLE "board_card_labels" (
    "card_id" TEXT NOT NULL,
    "label_id" TEXT NOT NULL,
    PRIMARY KEY ("card_id", "label_id")
);
-- #[toasty::breakpoint]
CREATE INDEX "board_card_labels_label_idx" ON "board_card_labels" ("label_id");
-- #[toasty::breakpoint]
CREATE TABLE "board_card_relations" (
    "card_id" TEXT NOT NULL,
    "related_card_id" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    PRIMARY KEY ("card_id", "related_card_id", "kind")
);
-- #[toasty::breakpoint]
CREATE INDEX "board_card_relations_related_idx" ON "board_card_relations" ("related_card_id");
-- #[toasty::breakpoint]
CREATE TABLE "board_columns" (
    "id" TEXT NOT NULL,
    "board_id" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "position" BIGINT NOT NULL,
    "role" TEXT,
    "color" TEXT,
    "wip_limit" BIGINT,
    "collapsed" BOOLEAN NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "board_columns_board_idx" ON "board_columns" ("board_id", "position");
-- #[toasty::breakpoint]
CREATE TABLE "flows" (
    "id" TEXT NOT NULL,
    "definition_json" TEXT NOT NULL,
    "updated_at" BIGINT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE TABLE "board_templates" (
    "id" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "spec_json" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE TABLE "board_card_field_values" (
    "card_id" TEXT NOT NULL,
    "field_id" TEXT NOT NULL,
    "value_json" TEXT NOT NULL,
    PRIMARY KEY ("card_id", "field_id")
);
-- #[toasty::breakpoint]
CREATE INDEX "board_card_field_values_field_idx" ON "board_card_field_values" ("field_id");
-- #[toasty::breakpoint]
CREATE TABLE "notes" (
    "id" TEXT NOT NULL,
    "slug" TEXT NOT NULL,
    "title" TEXT NOT NULL,
    "body" TEXT NOT NULL,
    "tags_json" TEXT NOT NULL,
    "source_session_id" TEXT,
    "source_cwd" TEXT,
    "slug_pending" BOOLEAN NOT NULL,
    "created_at" BIGINT NOT NULL,
    "updated_at" BIGINT NOT NULL,
    "content_kind" TEXT NOT NULL,
    "artifact_kind" TEXT,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_notes_by_slug" ON "notes" ("slug");
-- #[toasty::breakpoint]
CREATE TABLE "automations" (
    "id" TEXT NOT NULL,
    "definition_json" TEXT NOT NULL,
    "enabled" BOOLEAN NOT NULL,
    "next_run_at" BIGINT NOT NULL,
    "updated_at" BIGINT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "automations_due_idx" ON "automations" ("enabled", "next_run_at");
-- #[toasty::breakpoint]
CREATE TABLE "pilot_blocks" (
    "session_id" TEXT NOT NULL,
    "seq" BIGINT NOT NULL,
    "block_id" TEXT NOT NULL,
    "block_json" TEXT NOT NULL,
    "search_text" TEXT NOT NULL,
    "is_user" BOOLEAN NOT NULL,
    "is_draft" BOOLEAN NOT NULL,
    PRIMARY KEY ("session_id", "seq")
);
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_pilot_blocks_by_session_id_and_block_id" ON "pilot_blocks" ("session_id", "block_id");
-- #[toasty::breakpoint]
CREATE INDEX "pilot_blocks_user" ON "pilot_blocks" ("session_id", "is_user", "is_draft");
