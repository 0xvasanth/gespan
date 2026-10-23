// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Block } from "../model/session";
import { AgentTranscript } from "./AgentTranscript";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
      unobserve() {}
    },
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function render(blocks: Block[], busy = false) {
  act(() => root.render(createElement(AgentTranscript, { blocks, busy })));
}

function run(
  name: string,
  agentRun: Partial<NonNullable<Block["agentRun"]>>,
  status = "completed",
  detail?: string,
): Block {
  return {
    id: "run",
    role: "tool",
    text: name,
    tool: { callId: "send-1", kind: "agent", status, ...(detail ? { detail } : {}) },
    agentRun: {
      name,
      steps: [{ id: "s1", kind: "tool", text: "Read a.ts", status: "completed" }],
      ...agentRun,
    },
  };
}

const settled = (block: Block): Block[] => [
  { id: "user", role: "user", text: "Go", durationMs: 4_000 },
  block,
  { id: "answer", role: "assistant", text: "Done." },
];

// A settled turn folds its work behind "Worked for"; a running one keeps the
// subagent rows on screen, which is where they are read.
const renderSettled = (block: Block) => render(settled(block), true);

const rowText = () =>
  [...container.querySelectorAll("button, [aria-label^='Subagent']")]
    .map((el) => el.textContent)
    .join("|");

const handback = (body: string, trailer: string) =>
  `[Subagent hand-back] The agent finished.\nThe report follows:\n\n    ${body}\n\n${trailer}`;

describe("AgentTranscript background agent runs", () => {
  it("shows the report without the hand-back framing and a metadata line", () => {
    renderSettled(
      run(
        "Review",
        {},
        "completed",
        handback(
          "All checks pass.",
          "agentId: abc123 (use SendMessage with to: 'abc123')\nsubagent_tokens: 12345 tool_uses: 1 duration_ms: 9949",
        ),
      ),
    );
    const open = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Show Review\'s work"]',
    )!;
    act(() => open.click());
    const text = container.textContent ?? "";
    expect(text).toContain("All checks pass.");
    expect(text).not.toContain("[Subagent hand-back]");
    expect(text).not.toContain("The report follows");
    expect(text).not.toContain("subagent_tokens");
    expect(text).toContain("1 tool use · 12,345 tokens · 9.9s");
  });

  it("titles a resumption and a continuation, never doubling the prefix", () => {
    renderSettled(run("Resumed · Review", { resumeOf: "agent-1" }));
    expect(rowText()).toContain("Resumed · Review");
    expect(rowText()).not.toContain("Resumed · Resumed");

    renderSettled(run("Review", { resumeOf: "agent-1" }));
    expect(rowText()).toContain("Resumed · Review");

    renderSettled(run("Resumed · Review", { resumeOf: "agent-1", continuation: true }));
    expect(rowText()).toContain("Continued · Review");
    expect(rowText()).not.toContain("Resumed");

    renderSettled(run("Continued · Review", { continuation: true }));
    expect(rowText()).toContain("Continued · Review");
    expect(rowText()).not.toContain("Continued · Continued");

    renderSettled(run("Review", {}));
    expect(rowText()).not.toMatch(/Resumed|Continued/);
  });

  it("shows the activity subtitle only while the run is running", () => {
    render(
      [
        { id: "user", role: "user", text: "Go", startedAt: 1_000 },
        run("Review", { activity: "Reading parser.ts" }, "in_progress"),
      ],
      true,
    );
    expect(container.textContent).toContain("Reading parser.ts");

    renderSettled(run("Review", { activity: "Reading parser.ts" }, "completed"));
    expect(container.textContent).not.toContain("Reading parser.ts");
  });

  it("marks a finished background agent as a muted system line, not a user message", () => {
    render([
      { id: "user", role: "user", text: "Go", durationMs: 1_000 },
      { id: "answer", role: "assistant", text: "Waiting." },
      {
        id: "wake",
        role: "system",
        text: "Review done",
        sentAt: 5,
        interjection: { customType: "task-notification" },
      },
      { id: "followup", role: "assistant", text: "The review passed." },
    ]);
    const marker = container.querySelector('[aria-label="Background agent finished"]');
    expect(marker?.textContent).toBe("Background agent finished · Review done");
    expect(container.textContent).not.toContain("task-notification");
    // Not a user bubble, and not the boxed interjection divider.
    expect(marker?.closest("[data-role='user']")).toBeNull();
    expect(marker?.className).toContain("text-content/45");
    expect(container.querySelector('[role="separator"]')).toBeNull();
  });

  it("falls back to a bare marker when the notification has no summary", () => {
    render([
      { id: "user", role: "user", text: "Go", durationMs: 1_000 },
      {
        id: "wake",
        role: "system",
        text: "  ",
        interjection: { customType: "task-notification" },
      },
      { id: "followup", role: "assistant", text: "Done." },
    ]);
    expect(
      container.querySelector('[aria-label="Background agent finished"]')?.textContent,
    ).toBe("Background agent finished");
  });
});
