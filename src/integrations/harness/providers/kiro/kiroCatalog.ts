import { homeDir } from "../../../../platform/tauri/fs";
import {
  setHarnessModels,
  type AgentModel,
} from "../../../../features/sessions/model/models";
import { killChild, resolveKiroBinary, unwatchChild } from "../../core/child";
import { KIRO_INIT_PARAMS, launchKiro } from "./kiroLaunch";
import { modelsFromKiroSession } from "./kiroProtocol";

const PROBE_ID = "rebase-kiro-probe";
const DISCOVERY_TIMEOUT_MS = 30_000;
const REQUEST_TIMEOUT_MS = 20_000;

let inflight: Promise<void> | null = null;

export function refreshKiroCatalog(): Promise<void> {
  if (inflight) return inflight;
  inflight = discoverKiroModels()
    .then((models) => {
      if (models.length > 0) setHarnessModels("kiro", models);
    })
    .catch((error: unknown) => {
      console.debug("[rebase] kiro catalog", error);
    })
    .finally(() => {
      inflight = null;
    });
  return inflight;
}

/** Start a throwaway Kiro session and read the models it advertises. */
export async function discoverKiroModels(
  workingDirectory?: string,
): Promise<AgentModel[]> {
  const { path } = await resolveKiroBinary();
  const cwd = workingDirectory ?? (await homeDir());
  const probeId = `${PROBE_ID}-${crypto.randomUUID()}`;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const stop = async () => {
    unwatchChild(probeId);
    await killChild(probeId).catch(() => undefined);
  };

  const probe = (async () => {
    const { acp } = await launchKiro({
      id: probeId,
      path,
      cwd,
      handlers: (client) => ({
        // Kiro waits on an unanswered request, so refuse everything it asks.
        onRequest: (id, method) => {
          void client
            .respondError(id, {
              code: -32601,
              message: `Method not found: ${method}`,
            })
            .catch(() => undefined);
        },
      }),
      initParams: KIRO_INIT_PARAMS,
      initTimeoutMs: REQUEST_TIMEOUT_MS,
      onExit: () => undefined,
    });
    const created = await acp.request<unknown>(
      "session/new",
      { cwd, mcpServers: [] },
      REQUEST_TIMEOUT_MS,
    );
    return modelsFromKiroSession(created);
  })();

  try {
    return await Promise.race([
      probe,
      new Promise<never>((_, reject) => {
        timer = setTimeout(
          () => reject(new Error("Kiro model discovery timed out")),
          DISCOVERY_TIMEOUT_MS,
        );
      }),
    ]);
  } finally {
    clearTimeout(timer);
    // A losing probe still holds a process; settle it quietly after teardown.
    probe.catch(() => undefined);
    await stop();
  }
}
