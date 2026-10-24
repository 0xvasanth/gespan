/**
 * A remote host stops (forgets) its Claude process after every turn and binds
 * the conversation again for the next one. A sub-agent resumed in a later turn
 * must still land on its resumption block, so the host reseeds the agent
 * registry from the session it holds.
 */
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, expect, it, vi } from "vitest";

const sent: Array<{ id: string; line: string }> = [];
/** Stdout callbacks by child id; title generation watches children of its own. */
const watchers = new Map<string, (line: string) => void>();

vi.mock("../src/integrations/harness/core/child", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  resolveClaudeBinary: async () => ({ path: "/fake/claude" }),
  spawnChild: async () => undefined,
  killChild: async () => undefined,
  unwatchChild: () => undefined,
  watchChild: (id: string, line: (l: string) => void) => {
    watchers.set(id, line);
  },
  writeChild: async (id: string, line: string) => {
    sent.push({ id, line });
  },
}));

const { HostEngine } = await import("./engine");
const { HostStore } = await import("./store");
const { hostProviders } = await import("./providers");

const FRAMES = readFileSync(
  resolve(process.cwd(), "tests/fixtures/claude/resumed-background-agent.jsonl"),
  "utf8",
)
  .split("\n")
  .filter((line) => line.trim());
const AGENT = "toolu_019x6aJjknXrg5CSy9dvntEn";
const SEND = "toolu_01LfGgNGSbW1AVPVpfLsAYkh";
const AGENT_ID = "aba491c492e906740";
const TURN_ONE = 125;

const count = (id: string, pred: (m: Record<string, unknown>) => boolean) =>
  sent
    .filter((entry) => entry.id === id)
    .map((entry) => JSON.parse(entry.line) as Record<string, unknown>)
    .filter(pred).length;
const initializes = (id: string) =>
  count(
    id,
    (m) =>
      (m.request as Record<string, unknown> | undefined)?.subtype ===
      "initialize",
  );
const prompts = (id: string) => count(id, (m) => m.type === "user");

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/** Sends a host turn and plays `frames` once its fresh process asks to start. */
async function hostTurn(
  engine: InstanceType<typeof HostEngine>,
  sessionId: string,
  commandId: string,
  frames: string[],
) {
  const inits = initializes(sessionId);
  const users = prompts(sessionId);
  engine.command({ type: "send", commandId, sessionId, text: "go" });
  await vi.waitFor(() => expect(initializes(sessionId)).toBeGreaterThan(inits));
  const onLine = watchers.get(sessionId)!;
  onLine(
    JSON.stringify({
      type: "control_response",
      response: { subtype: "success", request_id: "gespan_2" },
    }),
  );
  await vi.waitFor(() => expect(prompts(sessionId)).toBeGreaterThan(users));
  for (const frame of frames) onLine(frame);
}

it("routes a resumed sub-agent to its resumption block after the host forgot the process", async () => {
  const directory = mkdtempSync(join(tmpdir(), "gespan-claude-resume-"));
  const store = new HostStore(join(directory, "host.db"));
  const project = store.addProject(directory, "Test");
  const engine = new HostEngine(store, { claude: hostProviders.claude });
  cleanups.push(async () => {
    await engine.close();
    store.close();
    rmSync(directory, { recursive: true, force: true });
  });
  const { sessionId } = engine.command({
    type: "create",
    commandId: "create",
    projectId: project.id,
    harness: "claude",
    model: "claude:test",
    runtimeMode: "supervised",
  });

  await hostTurn(engine, sessionId, "turn-1", FRAMES.slice(0, TURN_ONE));
  await vi.waitFor(() => expect(store.session(sessionId).status).toBe("idle"));
  const agent = store
    .session(sessionId)
    .session.blocks.find((block) => block.tool?.callId === AGENT);
  expect(agent?.agentRun?.agentId).toBe(AGENT_ID);

  // Turn 2 starts a new process after the host's stop/forget and rebind.
  await hostTurn(engine, sessionId, "turn-2", FRAMES.slice(TURN_ONE));
  await vi.waitFor(() => expect(store.session(sessionId).status).toBe("idle"));
  const blocks = store.session(sessionId).session.blocks;
  const resumed = blocks.find((block) => block.tool?.callId === SEND)?.agentRun;
  expect(resumed).toMatchObject({ resumeOf: AGENT, agentId: AGENT_ID });
  expect(
    resumed?.steps.filter((step) => step.kind === "tool").map((s) => s.id),
  ).toEqual([
    "toolu_01HWq7foDHczTjtHuoNvz4M7",
    "toolu_013E6GSRiGhqgcQ3vbtT7XNg",
    "toolu_01QVuG3c6ZordypvcUmhhfD6",
    "toolu_01CZhBxnaocVpUQuvCrXoa9C",
    "toolu_01LEVibA1bj4U6XFyozZ5QFk",
  ]);
  // Nothing fell back to a "Background agent" or continuation block.
  expect(
    blocks.some((block) =>
      /^(background|continued):/.test(block.tool?.callId ?? ""),
    ),
  ).toBe(false);
});
