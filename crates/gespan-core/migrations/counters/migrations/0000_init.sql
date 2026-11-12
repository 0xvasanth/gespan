CREATE TABLE "diagnostic_counters" (
    "day" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "error_kind" TEXT NOT NULL,
    "count" BIGINT NOT NULL,
    "app_version" TEXT NOT NULL,
    PRIMARY KEY ("day", "name", "error_kind", "app_version")
);
