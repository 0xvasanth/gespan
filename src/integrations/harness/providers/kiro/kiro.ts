import { TurnNotReadyError } from "../../core/types";
import { nativeModelId } from "../../../../features/sessions/model/models";
import type { RuntimeMode } from "../../../../features/sessions/model/session";
import type { AcpClient } from "../../core/acp";
import {
  killChild,
  resolveKiroBinary,
  unwatchChild,
} from "../../core/child";
import { KIRO_INIT_PARAMS, launchKiro } from "./kiroLaunch";
import {
  kiroAutoPermission,
  kiroInitCapabilities,
  kiroIsReplay,
  kiroModelControl,
  kiroPermissionChoice,
  kiroPermissionOptions,
  kiroPromptBlocks,
  kiroSessionId,
  kiroStartupError,
  kiroStderrAuthError,
  kiroStopReason,
  kiroTurnErrorMessage,
  type KiroCapabilities,
  type KiroModelControl,
} from "./kiroProtocol";
import {
  eventsFromAcpUpdate,
  permissionRequestFromAcp,
} from "../grok/grokProtocol";
import type {
  ApprovalDecision,
  HarnessEvent,
  HarnessSessionInput,
  SendTurnInput,
  SteerTurnInput,
} from "../../core/types";

/** `cancelled` answers a permission whose turn was cancelled under it. */
type PendingApproval = (decision: ApprovalDecision | "cancelled") => void;

type Live = {
  acp: AcpClient;
  acpSessionId: string;
  cwd: string;
  caps: KiroCapabilities;
  modelControl: KiroModelControl;
  modelId: string;
  muteUpdates: boolean;
  cancelled: boolean;
  runtimeMode: RuntimeMode;
  planning: boolean;
  onEvent: (event: HarnessEvent) => void;
  approvals: Map<number, PendingApproval>;
  /** The `session/prompt` request currently awaiting Kiro, if any. */
  inflight: Promise<unknown> | null;
  turns: Promise<void>;
};

type Resume = { acpSessionId: string; cwd: string };

const INIT_TIMEOUT_MS = 20_000;
const SESSION_TIMEOUT_MS = 45_000;
const CONTROL_TIMEOUT_MS = 20_000;
const PROMPT_TIMEOUT_MS = 30 * 60_000;
/** How long a cancelled prompt may take to settle before it is abandoned. */
const CANCEL_SETTLE_MS = 8_000;

const liveByThread = new Map<string, Live>();
const resumeByThread = new Map<string, Resume>();
const cancelledThreads = new Set<string>();

/** Live Kiro CLI adapter. Spawns `kiro-cli acp` over standard ACP. */
export async function sendKiroTurn(input: SendTurnInput): Promise<void> {
  // A cancel that arrived while idle must not swallow this turn; only a
  // cancel landing during startup (after this point) should.
  cancelledThreads.delete(input.sessionId);
  let live: Live;
  try {
    live = await ensureLive(input);
  } catch (error) {
    cancelledThreads.delete(input.sessionId);
    throw error;
  }
  if (cancelledThreads.delete(input.sessionId)) return;

  live.onEvent = input.onEvent;
  live.runtimeMode = input.runtimeMode;
  live.planning = input.intent === "plan";
  live.turns = live.turns
    .catch(() => undefined)
    .then(async () => {
      live.cancelled = false;
      live.muteUpdates = false;
      try {
        await applyModelSelection(live, input);
        if (live.cancelled) return;
        await prompt(live, input);
      } catch (error) {
        if (live.cancelled) return;
        throw error;
      }
    });

  try {
    await live.turns;
  } catch (error) {
    if (liveByThread.get(input.sessionId) === live) {
      await stopKiroSession(input.sessionId);
    }
    throw error;
  }
}

/**
 * Kiro's V3 steering method has no documented parameters, so a follow-up sent
 * mid-turn is queued by the app (`canSteer: false`) instead of reaching here.
 */
export async function steerKiroTurn(_input: SteerTurnInput): Promise<void> {
  throw new TurnNotReadyError(
    "Kiro CLI takes a follow-up after the current turn finishes",
  );
}

export function respondKiroApproval(
  sessionId: string,
  requestId: number,
  decision: ApprovalDecision,
): void {
  liveByThread.get(sessionId)?.approvals.get(requestId)?.(decision);
}

/**
 * Kiro's cancel flow: answer pending permissions as cancelled, send
 * `session/cancel`, then wait for the original prompt to settle so the next
 * prompt never overlaps it. A prompt Kiro never answers is abandoned.
 */
export async function cancelKiroTurn(sessionId: string): Promise<void> {
  const live = liveByThread.get(sessionId);
  if (!live) {
    cancelledThreads.add(sessionId);
    return;
  }
  live.cancelled = true;
  live.muteUpdates = true;
  resolveApprovals(live, "cancelled");
  await live.acp
    .notify("session/cancel", { sessionId: live.acpSessionId })
    .catch(() => undefined);

  const inflight = live.inflight;
  if (!inflight) return;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const settled = await Promise.race([
    inflight.then(
      () => true,
      () => true,
    ),
    new Promise<false>((resolve) => {
      timer = setTimeout(() => resolve(false), CANCEL_SETTLE_MS);
    }),
  ]);
  clearTimeout(timer);
  if (!settled) live.acp.rejectPending(new Error("cancelled"));
}

export async function stopKiroSession(sessionId: string): Promise<void> {
  cancelledThreads.delete(sessionId);
  const live = liveByThread.get(sessionId);
  liveByThread.delete(sessionId);
  if (live) {
    live.muteUpdates = true;
    live.cancelled = true;
    resolveApprovals(live, "cancelled");
  }
  live?.acp.close();
  unwatchChild(sessionId);
  await killChild(sessionId).catch(() => undefined);
}

export async function forgetKiroSession(sessionId: string): Promise<void> {
  resumeByThread.delete(sessionId);
  await stopKiroSession(sessionId);
}

export function bindKiroSession(
  threadId: string,
  acpSessionId: string,
  cwd: string,
): void {
  const sessionId = acpSessionId.trim();
  if (!threadId || !sessionId || !cwd.trim()) return;
  resumeByThread.set(threadId, { acpSessionId: sessionId, cwd });
}

async function ensureLive(input: HarnessSessionInput): Promise<Live> {
  const existing = liveByThread.get(input.sessionId);
  if (existing && existing.cwd === input.cwd) {
    existing.onEvent = input.onEvent;
    existing.runtimeMode = input.runtimeMode;
    existing.planning = input.intent === "plan";
    return existing;
  }
  if (existing) {
    resumeByThread.delete(input.sessionId);
    await stopKiroSession(input.sessionId);
  }

  const resume = resumeByThread.get(input.sessionId);
  if (resume && resume.cwd !== input.cwd) resumeByThread.delete(input.sessionId);

  const { path } = await resolveKiroBinary();
  const liveRef: { current: Live | null } = { current: null };
  // Register-before-session: Kiro does not buffer updates, and replayed
  // history must be dropped while `session/load` is in flight.
  const muteGate = { current: false };
  const emit = (event: HarnessEvent) => {
    (liveRef.current?.onEvent ?? input.onEvent)(event);
  };

  let launched: Awaited<ReturnType<typeof launchKiro>>;
  try {
    launched = await launchKiro({
      id: input.sessionId,
      path,
      cwd: input.cwd,
      initParams: KIRO_INIT_PARAMS,
      initTimeoutMs: INIT_TIMEOUT_MS,
      handlers: (acp) => ({
        onNotification: (method, params) => {
          if (muteGate.current) return;
          const live = liveRef.current;
          if (!live || live.muteUpdates) return;
          handleNotification(live, method, params);
        },
        onRequest: (id, method, params) => {
          const live = liveRef.current;
          if (!live || method !== "session/request_permission") {
            void acp
              .respondError(id, {
                code: -32601,
                message: `Method not found: ${method}`,
              })
              .catch(() => undefined);
            return;
          }
          void handlePermission(live, id, params);
        },
      }),
      onExit: (code) => {
        const live = liveRef.current;
        if (live) {
          live.cancelled = true;
          resolveApprovals(live, "cancelled");
          if (liveByThread.get(input.sessionId) === live) {
            liveByThread.delete(input.sessionId);
          }
        }
        emit({ type: "session.ended", code });
      },
      onStderr: (line) => {
        console.debug("[gespan] kiro stderr", line);
        const message = kiroStderrAuthError(line);
        if (message) emit({ type: "session.error", message });
      },
    });
  } catch (error) {
    await stopKiroSession(input.sessionId);
    throw kiroStartupError(error);
  }

  const { acp } = launched;
  const caps = kiroInitCapabilities(launched.init);
  try {
    let setup: unknown;
    let acpSessionId: string | undefined;
    let didLoad = false;
    if (resume && resume.cwd === input.cwd && caps.loadSession) {
      muteGate.current = true;
      try {
        setup = await acp.request(
          "session/load",
          { sessionId: resume.acpSessionId, cwd: input.cwd, mcpServers: [] },
          SESSION_TIMEOUT_MS,
        );
        // A V3 load response need not repeat the id.
        acpSessionId = kiroSessionId(setup) ?? resume.acpSessionId;
        didLoad = true;
      } catch {
        setup = undefined;
        acpSessionId = undefined;
      } finally {
        muteGate.current = false;
      }
    }

    if (!acpSessionId) {
      setup = await acp.request(
        "session/new",
        { cwd: input.cwd, mcpServers: [] },
        SESSION_TIMEOUT_MS,
      );
      acpSessionId = kiroSessionId(setup);
    }
    if (!acpSessionId) throw new Error("Kiro CLI did not return a session id");

    const modelControl = kiroModelControl(setup);
    const live: Live = {
      acp,
      acpSessionId,
      cwd: input.cwd,
      caps,
      modelControl,
      modelId: modelControl.kind === "none" ? "" : (modelControl.current ?? ""),
      muteUpdates: didLoad,
      cancelled: false,
      runtimeMode: input.runtimeMode,
      planning: input.intent === "plan",
      onEvent: input.onEvent,
      approvals: new Map(),
      inflight: null,
      turns: Promise.resolve(),
    };
    liveRef.current = live;
    liveByThread.set(input.sessionId, live);
    resumeByThread.set(input.sessionId, { acpSessionId, cwd: input.cwd });
    live.onEvent({
      type: "session.providerBound",
      providerSessionId: acpSessionId,
    });
    live.onEvent({ type: "session.started" });
    return live;
  } catch (error) {
    acp.close(error instanceof Error ? error : new Error(String(error)));
    await stopKiroSession(input.sessionId);
    throw kiroStartupError(error);
  }
}

/**
 * V3 selects models through a config option, legacy servers through
 * `session/set_model`; the session result decides which, never the version.
 */
async function applyModelSelection(
  live: Live,
  input: HarnessSessionInput,
): Promise<void> {
  const modelId = nativeModelId(input.model).trim();
  if (!modelId || modelId === "default" || modelId === live.modelId) return;
  const control = live.modelControl;
  if (control.kind === "config") {
    await live.acp.request(
      "session/set_config_option",
      {
        sessionId: live.acpSessionId,
        configId: control.configId,
        value: modelId,
      },
      CONTROL_TIMEOUT_MS,
    );
  } else if (control.kind === "setModel") {
    await live.acp.request(
      "session/set_model",
      { sessionId: live.acpSessionId, modelId },
      CONTROL_TIMEOUT_MS,
    );
  } else {
    return;
  }
  live.modelId = modelId;
}

async function prompt(live: Live, input: SendTurnInput): Promise<void> {
  try {
    const blocks = kiroPromptBlocks(input.text, input.attachments, live.caps);
    if (blocks.length === 0) return;
    const pending = live.acp.request<unknown>(
      "session/prompt",
      { sessionId: live.acpSessionId, prompt: blocks },
      PROMPT_TIMEOUT_MS,
    );
    live.inflight = pending;
    input.onAccepted?.();
    let result: unknown;
    try {
      result = await pending;
    } finally {
      if (live.inflight === pending) live.inflight = null;
    }
    if (live.cancelled) return;
    // The prompt response, not a turn-end update, closes the turn.
    live.onEvent({ type: "message.completed" });
    live.onEvent({ type: "reasoning.completed" });
    if (kiroStopReason(result) === "refusal") {
      live.onEvent({
        type: "status",
        text: "Kiro declined this request.",
      });
    }
  } catch (error) {
    if (live.cancelled) return;
    live.onEvent({
      type: "session.error",
      message: kiroTurnErrorMessage(error),
    });
    throw error;
  }
}

function handleNotification(live: Live, method: string, params: unknown): void {
  if (method !== "session/update" || kiroIsReplay(params)) return;
  for (const event of eventsFromAcpUpdate(params)) live.onEvent(event);
}

async function handlePermission(
  live: Live,
  id: number,
  params: unknown,
): Promise<void> {
  const request = permissionRequestFromAcp(params);
  const options = kiroPermissionOptions(params);
  if (request.callId) {
    live.onEvent({
      type: "tool.updated",
      callId: request.callId,
      title: request.title,
      kind: request.kind,
      preview: request.preview,
    });
  }

  if (live.planning) {
    const readOnly = request.kind === "read" || request.kind === "search";
    await respondPermission(
      live,
      id,
      kiroPermissionChoice(readOnly ? "allow" : "deny", options),
    );
    return;
  }

  // Nothing can be allowed, so asking the user would be a question with no
  // answer; take the agent's own reject option, or cancel if it offered none.
  const canAllow = kiroPermissionChoice("allow", options) !== undefined;
  if (!canAllow) {
    await respondPermission(live, id, kiroPermissionChoice("deny", options));
    return;
  }

  const automatic = kiroAutoPermission(live.runtimeMode, request.kind, options);
  if (automatic) {
    await respondPermission(live, id, automatic);
    return;
  }

  live.onEvent({
    type: "approval.requested",
    requestId: id,
    title: request.title,
    kind: request.kind,
    callId: request.callId,
    preview: request.preview,
  });
  const decision = await new Promise<ApprovalDecision | "cancelled">(
    (resolve) => {
      live.approvals.set(id, resolve);
    },
  );
  live.approvals.delete(id);
  live.onEvent({
    type: "approval.resolved",
    requestId: id,
    decision: decision === "allow" ? "allow" : "deny",
  });
  await respondPermission(
    live,
    id,
    decision === "cancelled" ? undefined : kiroPermissionChoice(decision, options),
  );
}

/** `undefined` answers `cancelled` rather than guessing an option id. */
async function respondPermission(
  live: Live,
  id: number,
  optionId: string | undefined,
): Promise<void> {
  await live.acp
    .respond(id, {
      outcome: optionId
        ? { outcome: "selected", optionId }
        : { outcome: "cancelled" },
    })
    .catch(() => undefined);
}

function resolveApprovals(live: Live, decision: "cancelled"): void {
  for (const resolve of [...live.approvals.values()]) resolve(decision);
}
