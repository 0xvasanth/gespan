CREATE TABLE "mailbox_messages" (
    "id" TEXT NOT NULL,
    "to_session_id" TEXT NOT NULL,
    "sender_session_id" TEXT,
    "sender_title" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "text" TEXT NOT NULL,
    "hops" BIGINT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "delivered_at" BIGINT,
    "state" TEXT NOT NULL,
    "held_reason" TEXT,
    "released" BOOLEAN NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "mailbox_messages_to_idx" ON "mailbox_messages" ("to_session_id", "state");
