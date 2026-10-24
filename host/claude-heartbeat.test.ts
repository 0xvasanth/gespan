/**
 * The host parses Claude frames with the same adapter as the app, so a
 * heartbeat must leave its saved session free of phantom steps and ghost runs
 * there too. Fixtures are shared with the app and Gespan Native tests.
 */
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, expect, it, vi } from "vitest";

const sent: Array<{ id: string; line: string }> = [];
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

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function frames(name: string): string[] {
  return readFileSync(
    resolve(process.cwd(), `tests/fixtures/claude/${name}.jsonl`),
    "utf8",
  )
    .split("\n")
    .filter((line) => line.trim());
}

async function hostSession(name: string) {
  const directory = mkdtempSync(join(tmpdir(), "gespan-claude-heartbeat-"));
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
  engine.command({ type: "send", commandId: "turn", sessionId, text: "go" });
  const lines = frames(name);
  await vi.waitFor(() =>
    expect(
      sent.some((entry) => entry.id === sessionId && entry.line.includes('"initialize"')),
    ).toBe(true),
  );
  const onLine = watchers.get(sessionId)!;
  for (const line of lines.slice(0, 18)) onLine(line);
  await vi.waitFor(() =>
    expect(
      sent.some((entry) => entry.id === sessionId && entry.line.includes('"type":"user"')),
    ).toBe(true),
  );
  for (const line of lines.slice(18)) onLine(line);
  await vi.waitFor(() => expect(store.session(sessionId).status).toBe("idle"));
  return store.session(sessionId);
}

it.each(["heartbeat-agent", "heartbeat-bash"])(
  "keeps %s heartbeats out of the host's saved session",
  async (name) => {
    const saved = await hostSession(name);
    const blocks = saved.session.blocks;
    expect(blocks.length).toBeGreaterThan(1);
    expect(JSON.stringify(blocks)).not.toContain("-heartbeat-");
    expect(
      blocks.some((block) => block.tool?.callId?.startsWith("background:")),
    ).toBe(false);
  },
);
