import { beforeEach, describe, expect, it, vi } from "vitest";

type Watch = {
  line: (line: string) => void;
  exit: (code: number | null) => void;
  stderr?: (line: string) => void;
};

const sent: string[] = [];
const spawns: string[][] = [];
let watch: Watch | undefined;
/** Decides what the fake process does after each spawn. */
let onSpawn: (attempt: number, args: string[]) => void = () => undefined;

vi.mock("../../core/child", () => ({
  spawnChild: async (_id: string, _path: string, args: string[]) => {
    spawns.push(args);
    const attempt = spawns.length;
    setTimeout(() => onSpawn(attempt, args), 0);
  },
  killChild: async () => undefined,
  unwatchChild: () => undefined,
  watchChild: (
    _id: string,
    line: (value: string) => void,
    exit: (code: number | null) => void,
    stderr?: (value: string) => void,
  ) => {
    watch = { line, exit, stderr };
  },
  writeChild: async (_id: string, line: string) => {
    sent.push(line);
  },
}));

const { launchKiro } = await import("./kiroLaunch");

function initialize() {
  return sent
    .map((line) => JSON.parse(line) as Record<string, unknown>)
    .filter((message) => message.method === "initialize")
    .at(-1);
}

function answerInitialize(result: unknown) {
  const request = initialize();
  watch!.line(JSON.stringify({ jsonrpc: "2.0", id: request!.id, result }));
}

const base = {
  id: "t1",
  path: "/fake/kiro-cli",
  cwd: "/repo",
  handlers: () => ({}),
  initParams: { protocolVersion: 1 },
  initTimeoutMs: 500,
  onExit: () => undefined,
};

beforeEach(() => {
  sent.length = 0;
  spawns.length = 0;
  watch = undefined;
  onSpawn = () => undefined;
});

describe("launchKiro", () => {
  it("starts the V3 engine and reports the initialize result", async () => {
    onSpawn = () => answerInitialize({ agentCapabilities: { loadSession: true } });
    const launched = await launchKiro(base);
    expect(spawns).toEqual([["acp", "--agent-engine=v3"]]);
    expect(launched.args).toEqual(["acp", "--agent-engine=v3"]);
    expect(launched.init).toEqual({ agentCapabilities: { loadSession: true } });
  });

  it("falls back to a bare acp once when the V3 launch exits before initialize", async () => {
    onSpawn = (attempt) => {
      if (attempt === 1) watch!.exit(2);
      else answerInitialize({});
    };
    const launched = await launchKiro(base);
    expect(spawns).toEqual([["acp", "--agent-engine=v3"], ["acp"]]);
    expect(launched.args).toEqual(["acp"]);
  });

  it("gives up after the second attempt and reports that Kiro exited", async () => {
    onSpawn = () => watch!.exit(2);
    await expect(launchKiro(base)).rejects.toThrow(/exited/);
    expect(spawns).toHaveLength(2);
  });

  it("does not respawn when initialize times out on a live process", async () => {
    await expect(launchKiro({ ...base, initTimeoutMs: 20 })).rejects.toThrow(
      /timed out/,
    );
    expect(spawns).toHaveLength(1);
  });

  it("does not respawn when initialize is rejected by a live process", async () => {
    onSpawn = () => {
      const request = initialize();
      watch!.line(
        JSON.stringify({
          jsonrpc: "2.0",
          id: request!.id,
          error: { code: -32000, message: "Not logged in" },
        }),
      );
    };
    await expect(launchKiro(base)).rejects.toThrow(/Not logged in/);
    expect(spawns).toHaveLength(1);
  });

  it("forwards stderr and reports a later exit through onExit only after startup", async () => {
    const stderr: string[] = [];
    const exits: (number | null)[] = [];
    onSpawn = () => {
      watch!.stderr?.("hello");
      answerInitialize({});
    };
    await launchKiro({
      ...base,
      onStderr: (line) => stderr.push(line),
      onExit: (code) => exits.push(code),
    });
    expect(stderr).toEqual(["hello"]);
    expect(exits).toEqual([]);
    watch!.exit(0);
    expect(exits).toEqual([0]);
  });
});
