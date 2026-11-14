CREATE TABLE "flow_versions" (
    "id" TEXT NOT NULL,
    "flow_id" TEXT NOT NULL,
    "version" BIGINT NOT NULL,
    "status" TEXT NOT NULL,
    "definition_json" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "flow_versions_number_idx" ON "flow_versions" ("flow_id", "version");
