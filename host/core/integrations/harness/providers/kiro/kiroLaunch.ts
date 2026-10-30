import { AcpClient, type AcpHandlers } from "../../core/acp";
import { spawnChild, watchChild } from "../../core/child";
import { KIRO_LAUNCH_ARGS } from "./kiroProtocol";

/** Kiro runs its own tools, so the client offers no filesystem or terminal. */
export const KIRO_INIT_PARAMS = {
  protocolVersion: 1,
  clientCapabilities: {
    fs: { readTextFile: false, writeTextFile: false },
    terminal: false,
  },
  clientInfo: { name: "gespan", version: "0.1.0" },
} as const;

export type KiroLaunchInput = {
  id: string;
  path: string;
  cwd: string;
  /** Built per attempt, because every attempt has its own client. */
  handlers: (acp: AcpClient) => AcpHandlers;
  initParams: unknown;
  initTimeoutMs: number;
  /** Called when the process exits after `initialize` succeeded. */
  onExit: (code: number | null) => void;
  onStderr?: (line: string) => void;
};

export type KiroLaunched = {
  acp: AcpClient;
  init: unknown;
  args: readonly string[];
};

/**
 * Spawn `kiro-cli acp` and complete `initialize`.
 *
 * A CLI that predates the V3 engine rejects `--agent-engine=v3` and exits, so
 * only a process that dies before `initialize` completes earns the fallback to
 * a bare `acp`. A live process that times out or refuses `initialize` fails
 * straight away: respawning would double the wait without changing the answer.
 * Each attempt gets its own client because a closed client refuses requests.
 */
export async function launchKiro(
  input: KiroLaunchInput,
): Promise<KiroLaunched> {
  let lastError = new Error("Kiro CLI did not start");
  for (const args of KIRO_LAUNCH_ARGS) {
    const handlers: AcpHandlers = {};
    const acp = new AcpClient(input.id, handlers);
    Object.assign(handlers, input.handlers(acp));
    let started = false;
    let exited = false;
    watchChild(
      input.id,
      (line) => acp.pushLine(line),
      (code) => {
        exited = true;
        acp.close(new Error("Kiro CLI exited"));
        if (started) input.onExit(code);
      },
      input.onStderr,
    );
    try {
      await spawnChild(
        input.id,
        input.path,
        [...args],
        input.cwd,
        undefined,
        "kiro",
      );
      const init = await acp.request<unknown>(
        "initialize",
        input.initParams,
        input.initTimeoutMs,
      );
      started = true;
      return { acp, init, args };
    } catch (error) {
      lastError = error instanceof Error ? error : new Error(String(error));
      acp.close(lastError);
      if (!exited) throw lastError;
      lastError = new Error(
        `Kiro CLI exited before it finished starting (${args.join(" ")})`,
      );
    }
  }
  throw lastError;
}
