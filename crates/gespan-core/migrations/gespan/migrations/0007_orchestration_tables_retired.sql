-- Adoption: an orchestration lead's workers join the session tree before
-- the table that named them goes. Hand-written: Toasty only drops tables.
INSERT INTO "session_links"
  ("child_session_id", "parent_session_id", "root_session_id", "depth", "origin", "wake", "created_at")
SELECT w."session_id", w."lead_id", w."lead_id", 1, 'orchestration', 'never', 0
FROM "orchestration_workers" AS w
WHERE EXISTS (SELECT 1 FROM "sessions" WHERE "id" = w."session_id")
  AND EXISTS (SELECT 1 FROM "sessions" WHERE "id" = w."lead_id")
  AND w."session_id" <> w."lead_id"
  AND NOT EXISTS (SELECT 1 FROM "session_links" WHERE "child_session_id" = w."session_id");
-- #[toasty::breakpoint]
DROP TABLE "orchestration_runs";
-- #[toasty::breakpoint]
DROP TABLE "orchestration_sidebar";
-- #[toasty::breakpoint]
DROP TABLE "orchestration_workers";
