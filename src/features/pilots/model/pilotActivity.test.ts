import { expect, it } from "vitest";
import type { Block } from "../../sessions/model/session";
import {
  resolvePilotActivity,
  type PilotActivitySelection,
} from "./pilotActivity";

const blocks: Block[] = [
  { id: "user", role: "user", text: "Inspect" },
  {
    id: "call",
    role: "tool",
    text: "Inspect file",
    tool: { status: "in_progress" },
  },
];
const selection: PilotActivitySelection = {
  sessionId: "pilot",
  turnId: "user",
  blocks,
};

it("follows the selected live turn, including new steps and settled tool state", () => {
  const updated: Block[] = [
    blocks[0],
    { ...blocks[1], tool: { status: "completed" } },
    { id: "reply", role: "assistant", text: "Done" },
  ];
  expect(
    resolvePilotActivity(selection, { id: "pilot", blocks: updated, busy: true }),
  ).toEqual({
    turnId: "user",
    blocks: updated,
    live: true,
  });
  expect(
    resolvePilotActivity(selection, { id: "pilot", blocks: updated, busy: false })
      ?.live,
  ).toBe(false);
  expect(
    resolvePilotActivity(selection, {
      id: "pilot",
      blocks: [
        ...updated,
        { id: "next", role: "user", text: "Another request" },
      ],
      busy: true,
    })?.live,
  ).toBe(false);
});

it("retains an archived turn without borrowing another turn or Pilot's activity", () => {
  expect(
    resolvePilotActivity(selection, {
      id: "pilot",
      blocks: [{ id: "new", role: "user", text: "Latest" }],
      busy: true,
    }),
  ).toEqual({
    turnId: "user",
    blocks,
    live: false,
  });
  expect(
    resolvePilotActivity(selection, { id: "another-pilot", blocks }),
  ).toBeNull();
  expect(resolvePilotActivity(selection, undefined)).toBeNull();
  expect(resolvePilotActivity(null, { id: "pilot", blocks })).toBeNull();
});

it("keeps completion activity separate from the previous reply before any output arrives", () => {
  const notification: Block = {
    id: "notification",
    role: "user",
    text: "Hidden completion prompt",
    internal: true,
    startedAt: 2_000,
    pilotSessionCompletion: {
      sessionId: "worker",
      title: "Review",
      status: "completed",
    },
  };
  const session = { id: "pilot", blocks: [...blocks, notification], busy: true };
  expect(resolvePilotActivity(selection, session)?.live).toBe(false);
  const completionSelection = {
    sessionId: "pilot",
    turnId: notification.id,
    blocks: [notification],
  };
  const step: Block = {
    id: "check-report",
    role: "tool",
    text: "Read the review session",
    tool: { status: "in_progress" },
  };
  const updated = { ...session, blocks: [...session.blocks, step] };
  expect(resolvePilotActivity(completionSelection, updated)).toEqual({
    turnId: notification.id,
    blocks: [notification, step],
    live: true,
  });
});
