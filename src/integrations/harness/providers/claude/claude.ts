import { TurnNotReadyError } from "../../core/types";
import { nativeModelId } from "../../../../features/sessions/model/models";
import { sameProviderAccountId } from "../../../../features/providers/model/providerAccounts";
import type {
  RuntimeMode,
  TaskListItem,
  TaskListMeta,
} from "../../../../features/sessions/model/session";
import { loadClaudeHooks } from "../../../../features/settings/model/settings";
import {
  killChild,
  resolveClaudeBinary,
  spawnChild,
  unwatchChild,
  watchChild,
  writeChild,
} from "../../core/child";
import {
  askUserQuestionAllowInput,
  asRecord,
  assistantMessageId,
  assistantTextBlocks,
  assistantThinkingBlocks,
  assistantToolUses,
  contextFromResult,
  contextUsedFromAssistant,
  turnMetricsFromResult,
  buildClaudeSpawnArgs,
  buildClaudeUserMessage,
  buildControlRequest,
  buildControlResponse,
  claudeSettingsKey,
  extractAskUserQuestionTitle,
  extractExitPlanModePlan,
  inputJsonDeltaFromEvent,
  isAgentTaskType,
  isClaudeUltracodeEffort,
  isReplayedTaskNotificationResult,
  isSubagentMessage,
  isTerminalAgentTaskStatus,
  isTodoTool,
  applyClaudeTaskTool,
  isUsageLimitResult,
  normalizeClaudeCliEffort,
  parseBackgroundTasks,
  parseControlCancelId,
  parseControlRequest,
  parseJsonLine,
  parseTaskNotification,
  parseTaskProgress,
  parseTaskStarted,
  parseTaskUpdated,
  parseToolProgress,
  taskListFromTodos,
  previewFromTool,
  resolveClaudeApiModelId,
  runtimeModeToPermission,
  sessionIdFromMessage,
  statusTextFromSystem,
  streamDeltaFromEvent,
  stringField,
  summarizeToolRequest,
  toClaudePermissionResult,
  toolKindFromName,
  toolResultsFromUserMessage,
  toolStartFromEvent,
  toolTitle,
  tryParseJsonRecord,
  turnStatusFromResult,
  usageLimitFromRateLimitEvent,
  type ClaudeAgentTaskNotification,
  type ClaudeCliSettings,
  type ClaudeControlRequest,
} from "./claudeProtocol";
import { isAgentToolName } from "../../core/preview";
import {
  clearAllToolElapsed,
  clearToolElapsed,
  setToolElapsed,
} from "../../core/toolElapsed";
import { heartbeatElapsed, isHeartbeatProgress } from "./claudeHeartbeat";
import type { ClaudeToolProgress } from "./claudeProtocol";
import { parseHandback } from "../../../../features/sessions/model/subagentHandback";
import type { Block } from "../../../../features/sessions/model/session";
import {
  AgentRegistry,
  CONTINUED_PREFIX,
  FALLBACK_PREFIX,
} from "./claudeAgentRegistry";
import { joinStreamText, snapshotRemainder } from "../../core/streamText";
import {
  questionPromptTitle,
  questionsFromUnknown,
  type UserQuestionReply,
} from "../../../../features/sessions/model/userQuestion";
import type {
  ApprovalDecision,
  CompactContextInput,
  HarnessEvent,
  HarnessSessionInput,
  SendTurnInput,
  SteerTurnInput,
} from "../../core/types";

/**
 * A PermissionRequest hook can decide before the user touches the prompt; Claude
 * then cancels the control request out from under us. That is not a rejection,
 * so it gets its own outcome instead of being folded into "deny".
 */
type ApprovalOutcome = ApprovalDecision | "cancelled";

type PendingApproval = {
  requestId: string;
  input: Record<string, unknown>;
  resolve: (decision: ApprovalOutcome) => void;
};

type PendingQuestion = {
  requestId: string;
  event: Extract<HarnessEvent, { type: "question.asked" }>;
  resolve: (reply: UserQuestionReply | "cancelled") => void;
};

type InFlightTool = {
  id: string;
  name: string;
  input: Record<string, unknown>;
  partialJson: string;
  title: string;
};

type LiveAgentTask = {
  taskId: string;
  toolUseId?: string;
  description: string;
  backgrounded: boolean;
};

type BackgroundTask = {
  description: string;
  toolUseId?: string;
  /** The block showing this task's work, for the waiting card. */
  toolId?: string;
  /** The task's latest progress description. */
  latest?: string;
  /** Tool steps on `toolId` as of the latest task event. */
  steps?: number;
};

type Live = {
  cwd: string;
  claudeSessionId: string;
  providerAccountId?: string;
  runtimeMode: RuntimeMode;
  planning: boolean;
  settingsKey: string;
  onEvent: (event: HarnessEvent) => void;
  approvals: Map<number, PendingApproval>;
  questions: Map<number, PendingQuestion>;
  visibleQuestionId: number | null;
  nextApprovalUiId: number;
  nextControlId: number;
  toolsByIndex: Map<number, InFlightTool>;
  toolsById: Map<string, InFlightTool>;
  agentTasks: Map<string, LiveAgentTask>;
  /**
   * Every task Claude still runs for this session, by id: subagents, shells it
   * backgrounded, monitors. Each one ends in a notification that wakes Claude
   * for another turn, so the Rebase turn stays open until they are done.
   */
  backgroundTasks: Map<string, BackgroundTask>;
  /** Rows shown for tasks still running when Claude yielded, by task id. */
  backgroundRows: Map<string, string>;
  /** A task finished after Claude yielded; its follow-up turn is on the way. */
  awaitingResume: ReturnType<typeof setTimeout> | null;
  /** Last background list sent to the UI, to skip repeats. */
  backgroundKey: string;
  /** Finished-subagent notes held until Claude picks the thread back up. */
  taskNotes: string[];
  /** TaskCreate/TaskUpdate items, keyed by Claude's task id. */
  claudeTasks: Map<string, TaskListItem>;
  turnResultSeen: boolean;
  /** Latest `rate_limit_event` refused requests; reported when the turn ends. */
  usageLimit: { resetsAt?: number } | null;
  cancelled: boolean;
  muteUpdates: boolean;
  turns: Promise<void>;
  turnDone: (() => void) | null;
  turnFailed: ((error: Error) => void) | null;
  turnEndPending: boolean;
  activeTurn: boolean;
  initDone: (() => void) | null;
  initialized: boolean;
  emittedAssistant: string;
  emittedReasoning: string;
  pendingAssistantBoundary: boolean;
  manualCompaction: boolean;
  compactionConfirmed: boolean;
  /** Which Agent call dispatched which sub-agent; outlives turns and processes. */
  registry: AgentRegistry;
  /** Main-thread calls that have their result (or finished, for agents); only a running agent is a live parent. */
  doneTools: Set<string>;
  /** Sub-agent step id -> the block it was attached to. */
  stepOwners: Map<string, string>;
  /** Block -> its tool step ids, for the waiting card's step count. */
  toolSteps: Map<string, Set<string>>;
  /** Synthetic continuation and fallback blocks this process opened. */
  synthetic: Set<string>;
  /** SendMessage blocks already turned into a resumed run. */
  resumedBlocks: Set<string>;
  /** Claude `system/init` frames seen; one per turn, user- or self-started. */
  turn: number;
  /** Unique per process, so synthetic block ids never repeat in a session. */
  scope: string;
  /** Task ids Claude reported as backgrounded. */
  backgrounded: Set<string>;
  /** Summaries of the background tasks whose notifications may wake Claude. */
  wakeSummaries: string[];
  /** The running turn already took its wake summary when it started. */
  wakeHandled: boolean;
  /** The user wrote into the running turn, so its reply may not be a wake. */
  steeredInTurn: boolean;
  /**
   * The user wrote a follow-up after Claude yielded: the next turn answers
   * them, so it must not lead with a background-wake marker.
   */
  userTurnPending: boolean;
};

type Resume = {
  sessionId: string;
  cwd: string;
  providerAccountId?: string;
};

const INIT_TIMEOUT_MS = 8_000;
/**
 * How long a finished background task may take to wake Claude before the turn
 * is let go anyway. The follow-up turn normally starts within a second or two.
 */
const RESUME_GRACE_MS = 15_000;

const liveByThread = new Map<string, Live>();
const resumeByThread = new Map<string, Resume>();
/**
 * Claude task lists outlive a Live: a restart that resumes the conversation
 * keeps its task ids, so later TaskUpdate calls must find earlier tasks. Task
 * ids belong to one Claude conversation, so each map records which one.
 */
const tasksByThread = new Map<
  string,
  { providerSessionId: string; tasks: Map<string, TaskListItem> }
>();
/** Task-list block key for TaskCreate/TaskUpdate items. */
const CLAUDE_TASKS_KEY = "claude-tasks";
/**
 * Agent registries outlive a Live like task lists do: a restarted process
 * resumes the same conversation, whose later turns refer to earlier agents.
 */
const registryByThread = new Map<
  string,
  { providerSessionId: string; registry: AgentRegistry }
>();
let scopeSequence = 0;
const cancelledThreads = new Set<string>();

let resolveClaudeBinaryImpl: () => Promise<{ path: string }> =
  resolveClaudeBinary;

/** Test seam. */
export function setClaudeBinaryResolver(
  fn: () => Promise<{ path: string }>,
): void {
  resolveClaudeBinaryImpl = fn;
}

export async function sendClaudeTurn(input: SendTurnInput): Promise<void> {
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
  live.turns = live.turns
    .catch(() => undefined)
    .then(async () => {
      live.cancelled = false;
      live.muteUpdates = false;
      try {
        await runTurn(live, input);
      } catch (error) {
        if (live.cancelled) return;
        throw error;
      }
    });
  await live.turns;
}

export async function compactClaudeContext(
  input: CompactContextInput,
): Promise<void> {
  const settingsKey = settingsKeyFor(input);
  let live = liveByThread.get(input.sessionId);
  if (!live || live.cwd !== input.cwd || live.settingsKey !== settingsKey) {
    live = await ensureLive(input);
  }
  if (cancelledThreads.delete(input.sessionId)) return;

  live.onEvent = input.onEvent;
  live.runtimeMode = input.runtimeMode;
  live.turns = live.turns
    .catch(() => undefined)
    .then(async () => {
      live.cancelled = false;
      live.muteUpdates = false;
      live.manualCompaction = true;
      live.compactionConfirmed = false;
      try {
        await runTurn(live, {
          ...input,
          modelSettings: undefined,
          text: "/compact",
          attachments: [],
        });
        if (!live.compactionConfirmed) {
          throw new Error("Claude Code did not confirm context compaction");
        }
      } catch (error) {
        if (live.cancelled) return;
        throw error;
      } finally {
        live.manualCompaction = false;
      }
    });
  await live.turns;
}

export async function steerClaudeTurn(input: SteerTurnInput): Promise<void> {
  const live = liveByThread.get(input.sessionId);
  if (!live?.activeTurn) throw new TurnNotReadyError("No active turn to steer");

  const message = buildClaudeUserMessage({
    text: input.text,
    attachments: input.attachments,
    effort: input.modelSettings?.effort,
  });
  const content = (message.message as { content: unknown[] }).content;
  if (content.length === 0) return;

  // After Claude yielded, the next turn answers this message. Mid-turn,
  // Claude folds it into the current turn instead. Set before writing: the
  // answering turn's init can arrive before the write resolves.
  const pending = live.turnResultSeen && !live.userTurnPending;
  if (pending) live.userTurnPending = true;
  if (!live.turnResultSeen) live.steeredInTurn = true;
  try {
    await writeJson(input.sessionId, message);
  } catch (error) {
    if (pending) live.userTurnPending = false;
    throw error;
  }
}

export function respondClaudeApproval(
  sessionId: string,
  requestId: number,
  decision: ApprovalDecision,
): void {
  const live = liveByThread.get(sessionId);
  const pending = live?.approvals.get(requestId);
  if (!pending) return;
  pending.resolve(decision);
}

export function respondClaudeQuestion(
  sessionId: string,
  requestId: number,
  reply: UserQuestionReply,
): void {
  const live = liveByThread.get(sessionId);
  const pending = live?.questions.get(requestId);
  if (!pending) return;
  pending.resolve(reply);
}

export async function cancelClaudeTurn(sessionId: string): Promise<void> {
  const live = liveByThread.get(sessionId);
  if (!live) {
    cancelledThreads.add(sessionId);
    return;
  }
  live.cancelled = true;
  live.muteUpdates = true;
  for (const [, pending] of live.approvals) pending.resolve("deny");
  live.approvals.clear();
  for (const [, pending] of live.questions)
    pending.resolve({ kind: "skipped" });
  live.questions.clear();
  // Stop means the whole run, including what Claude left going in the
  // background. Otherwise it finishes later and wakes Claude up again.
  for (const taskId of live.backgroundTasks.keys()) {
    await writeJson(
      sessionId,
      buildControlRequest(nextControlId(live), {
        subtype: "stop_task",
        task_id: taskId,
      }),
    ).catch(() => undefined);
  }
  await writeJson(
    sessionId,
    buildControlRequest(nextControlId(live), { subtype: "interrupt" }),
  ).catch(() => undefined);
  finishActiveTurn(live, [
    { type: "message.completed" },
    { type: "reasoning.completed" },
  ]);
}

export async function stopClaudeSession(sessionId: string): Promise<void> {
  cancelledThreads.delete(sessionId);
  const live = liveByThread.get(sessionId);
  liveByThread.delete(sessionId);
  if (live) {
    live.muteUpdates = true;
    for (const [, pending] of live.approvals) pending.resolve("deny");
    live.approvals.clear();
    for (const [, pending] of live.questions)
      pending.resolve({ kind: "skipped" });
    live.questions.clear();
    live.activeTurn = false;
    live.turnDone?.();
    live.turnDone = null;
    live.turnFailed = null;
    live.initDone?.();
    live.initDone = null;
  }
  unwatchChild(sessionId);
  await killChild(sessionId).catch(() => undefined);
}

export async function forgetClaudeSession(sessionId: string): Promise<void> {
  resumeByThread.delete(sessionId);
  tasksByThread.delete(sessionId);
  registryByThread.delete(sessionId);
  await stopClaudeSession(sessionId);
}

export function bindClaudeSession(
  threadId: string,
  providerSessionId: string,
  cwd: string,
  providerAccountId?: string,
): void {
  const sessionId = providerSessionId.trim();
  if (!threadId || !sessionId || !cwd.trim()) return;
  resumeByThread.set(threadId, { sessionId, cwd, providerAccountId });
  // Task ids from another conversation mean nothing in this one.
  if (tasksByThread.get(threadId)?.providerSessionId !== sessionId) {
    tasksByThread.delete(threadId);
  }
  if (registryByThread.get(threadId)?.providerSessionId !== sessionId) {
    registryByThread.delete(threadId);
  }
}

/**
 * Seed the agent registry from a restored session's persisted blocks, written
 * by either app. A resumed conversation's later turns resume or continue
 * agents dispatched before the restart; without this their work has no home.
 * A registry this app already holds is fresher, so it wins.
 */
export function restoreClaudeAgentRuns(
  threadId: string,
  blocks: readonly Block[],
): void {
  if (!threadId) return;
  const providerSessionId = resumeByThread.get(threadId)?.sessionId;
  if (!providerSessionId) return;
  // Seeding is additive and marks nothing active, like Native's seed at every
  // process start, so it merges into a registry this app already holds.
  const retained = registryByThread.get(threadId);
  const registry =
    retained?.providerSessionId === providerSessionId
      ? retained.registry
      : new AgentRegistry();
  registry.seedFromBlocks(blocks);
  registryByThread.set(threadId, { providerSessionId, registry });
}

/**
 * Seed the task map from a restored session's persisted panel. After an app
 * restart only the transcript survives, and a resumed conversation still
 * refers to its earlier task ids.
 */
export function restoreClaudeTaskLists(
  threadId: string,
  lists: TaskListMeta[],
): void {
  if (!threadId || tasksByThread.has(threadId)) return;
  // Only a list produced by the conversation bound to this thread applies.
  const providerSessionId = resumeByThread.get(threadId)?.sessionId;
  if (!providerSessionId) return;
  let items: TaskListItem[] = [];
  for (const entry of lists) {
    if (entry.key !== CLAUDE_TASKS_KEY) continue;
    if (entry.providerSessionId !== providerSessionId) continue;
    items = entry.items.filter((item) => item.id);
  }
  if (items.length === 0) return;
  tasksByThread.set(threadId, {
    providerSessionId,
    tasks: new Map(items.map((item) => [item.id!, { ...item }])),
  });
}

async function ensureLive(input: HarnessSessionInput): Promise<Live> {
  const settingsKey = settingsKeyFor(input);
  const planning = input.intent === "plan";
  const existing = liveByThread.get(input.sessionId);
  if (
    existing &&
    existing.cwd === input.cwd &&
    existing.settingsKey === settingsKey &&
    existing.planning === planning
  ) {
    existing.onEvent = input.onEvent;
    existing.runtimeMode = input.runtimeMode;
    return existing;
  }
  if (existing) {
    // Model and launch-setting changes require a fresh Claude process, but
    // they must resume the same provider conversation. Only a cwd change
    // invalidates the stored session because Claude sessions are cwd-bound.
    if (existing.cwd !== input.cwd) resumeByThread.delete(input.sessionId);
    await stopClaudeSession(input.sessionId);
  }

  const resume = resumeByThread.get(input.sessionId);
  const canResume =
    resume != null &&
    resume.cwd === input.cwd &&
    sameProviderAccountId(resume.providerAccountId, input.providerAccountId);
  if (
    resume &&
    (resume.cwd !== input.cwd ||
      !sameProviderAccountId(resume.providerAccountId, input.providerAccountId))
  ) {
    resumeByThread.delete(input.sessionId);
  }
  const { path } = await resolveClaudeBinaryImpl();
  const liveRef: { current: Live | null } = { current: null };
  const claudeSessionId =
    canResume && resume ? resume.sessionId : crypto.randomUUID();
  const retained = tasksByThread.get(input.sessionId);
  const claudeTasks =
    retained?.providerSessionId === claudeSessionId
      ? retained.tasks
      : new Map<string, TaskListItem>();
  tasksByThread.set(input.sessionId, {
    providerSessionId: claudeSessionId,
    tasks: claudeTasks,
  });
  const retainedRegistry = registryByThread.get(input.sessionId);
  const registry =
    retainedRegistry?.providerSessionId === claudeSessionId
      ? retainedRegistry.registry
      : new AgentRegistry();
  // This process cannot continue the last one's resumed streams.
  registry.deactivateAll();
  registryByThread.set(input.sessionId, {
    providerSessionId: claudeSessionId,
    registry,
  });
  const launch = launchOptions(
    input,
    canResume ? resume?.sessionId : undefined,
    claudeSessionId,
  );

  const live: Live = {
    cwd: input.cwd,
    claudeSessionId,
    providerAccountId: input.providerAccountId,
    runtimeMode: input.runtimeMode,
    planning,
    settingsKey,
    onEvent: input.onEvent,
    approvals: new Map(),
    questions: new Map(),
    visibleQuestionId: null,
    nextApprovalUiId: 1,
    nextControlId: 1,
    toolsByIndex: new Map(),
    toolsById: new Map(),
    agentTasks: new Map(),
    backgroundTasks: new Map(),
    backgroundRows: new Map(),
    awaitingResume: null,
    backgroundKey: "",
    taskNotes: [],
    claudeTasks,
    turnResultSeen: false,
    usageLimit: null,
    cancelled: false,
    muteUpdates: false,
    turns: Promise.resolve(),
    turnDone: null,
    turnFailed: null,
    turnEndPending: false,
    activeTurn: false,
    initDone: null,
    initialized: false,
    emittedAssistant: "",
    emittedReasoning: "",
    pendingAssistantBoundary: false,
    manualCompaction: false,
    compactionConfirmed: false,
    registry,
    doneTools: new Set(),
    stepOwners: new Map(),
    toolSteps: new Map(),
    synthetic: new Set(),
    resumedBlocks: new Set(),
    turn: 0,
    scope: `${Date.now().toString(16)}${(scopeSequence++).toString(16)}`,
    backgrounded: new Set(),
    userTurnPending: false,
    wakeSummaries: [],
    wakeHandled: false,
    steeredInTurn: false,
  };
  liveRef.current = live;

  watchChild(
    input.sessionId,
    (line) => {
      const current = liveRef.current;
      if (!current) return;
      handleLine(input.sessionId, current, line);
    },
    (code) => {
      liveByThread.delete(input.sessionId);
      const current = liveRef.current;
      if (!current?.muteUpdates) {
        (current?.onEvent ?? input.onEvent)({ type: "session.ended", code });
      }
      current?.turnFailed?.(new Error("Claude Code exited"));
      current?.initDone?.();
      if (current) {
        current.turnDone = null;
        current.turnFailed = null;
        current.initDone = null;
      }
    },
  );

  await spawnChild(
    input.sessionId,
    path,
    buildClaudeSpawnArgs(launch),
    input.cwd,
    { provider: "claude", id: input.providerAccountId ?? "default" },
    "claude",
  );

  liveByThread.set(input.sessionId, live);
  resumeByThread.set(input.sessionId, {
    sessionId: claudeSessionId,
    cwd: input.cwd,
    providerAccountId: input.providerAccountId,
  });

  try {
    await writeJson(
      input.sessionId,
      buildControlRequest(nextControlId(live), { subtype: "initialize" }),
    );
    await waitForInit(live, INIT_TIMEOUT_MS);
    live.onEvent({
      type: "session.providerBound",
      providerSessionId: live.claudeSessionId,
    });
    live.onEvent({ type: "session.started" });
    return live;
  } catch (error) {
    await stopClaudeSession(input.sessionId);
    throw error;
  }
}

async function runTurn(live: Live, input: SendTurnInput): Promise<void> {
  const effort = input.modelSettings?.effort;
  const message = buildClaudeUserMessage({
    text: input.text,
    attachments: input.attachments,
    effort,
  });
  const content = (message.message as { content: unknown[] }).content;
  if (content.length === 0) return;

  live.emittedAssistant = "";
  live.emittedReasoning = "";
  live.pendingAssistantBoundary = false;
  live.toolsByIndex.clear();
  for (const id of live.toolsById.keys()) clearToolElapsed(id);
  live.toolsById.clear();
  live.agentTasks.clear();
  live.backgroundTasks.clear();
  live.backgroundRows.clear();
  clearAwaitingResume(live);
  live.backgroundKey = "";
  live.taskNotes = [];
  live.turnResultSeen = false;
  live.userTurnPending = false;
  live.wakeSummaries = [];
  live.wakeHandled = false;
  live.steeredInTurn = false;

  const turnPromise = new Promise<void>((resolve, reject) => {
    live.turnDone = resolve;
    live.turnFailed = reject;
  });
  live.activeTurn = true;
  settlePendingTurn(live);

  try {
    await writeJson(input.sessionId, message);
    input.onAccepted?.();
    settlePendingTurn(live);
    await turnPromise;
  } catch (error) {
    if (live.cancelled) return;
    live.onEvent({
      type: "session.error",
      message: error instanceof Error ? error.message : String(error),
    });
    throw error;
  } finally {
    live.turnDone = null;
    live.turnFailed = null;
  }
}

function handleLine(sessionId: string, live: Live, line: string): void {
  const rec = parseJsonLine(line);
  if (!rec) return;

  const type = stringField(rec, "type");
  if (type === "keep_alive") return;

  const cancelId = parseControlCancelId(rec);
  if (cancelId) {
    for (const [uiId, pending] of live.approvals) {
      if (pending.requestId === cancelId) {
        pending.resolve("cancelled");
        live.approvals.delete(uiId);
      }
    }
    for (const [uiId, pending] of live.questions) {
      if (pending.requestId === cancelId) {
        pending.resolve("cancelled");
        live.questions.delete(uiId);
      }
    }
    return;
  }

  const control = parseControlRequest(rec);
  if (control) {
    const turn = live.turnDone;
    void handleControlRequest(sessionId, live, control).catch(
      (error: unknown) => {
        if (live.muteUpdates || live.turnDone !== turn) return;
        const failure =
          error instanceof Error ? error : new Error(String(error));
        if (live.turnFailed) {
          live.turnFailed(failure);
        } else {
          live.onEvent({ type: "session.error", message: failure.message });
        }
      },
    );
    return;
  }

  if (live.muteUpdates) return;

  const sessionIdFromLine = sessionIdFromMessage(rec);
  if (sessionIdFromLine && sessionIdFromLine !== live.claudeSessionId) {
    live.claudeSessionId = sessionIdFromLine;
    // A different conversation starts with its own task ids.
    live.claudeTasks = new Map();
    tasksByThread.set(sessionId, {
      providerSessionId: sessionIdFromLine,
      tasks: live.claudeTasks,
    });
    // Agent and tool ids are unique, so the registry carries over.
    registryByThread.set(sessionId, {
      providerSessionId: sessionIdFromLine,
      registry: live.registry,
    });
    resumeByThread.set(sessionId, {
      sessionId: sessionIdFromLine,
      cwd: live.cwd,
      providerAccountId: live.providerAccountId,
    });
    live.onEvent({
      type: "session.providerBound",
      providerSessionId: sessionIdFromLine,
    });
  }

  if (
    type === "system" &&
    (stringField(rec, "subtype") === "init" ||
      stringField(rec, "subtype") === "initialized")
  ) {
    markInitialized(live);
    if (stringField(rec, "subtype") === "init") {
      live.turn += 1;
      noteClaudeTurnStarted(live);
    }
  }

  if (type === "control_response") {
    markInitialized(live);
    return;
  }

  if (live.manualCompaction && type !== "system" && type !== "result") {
    return;
  }

  if (handleAgentLifecycle(live, rec)) return;
  if (type === "tool_progress") {
    handleToolProgress(live, rec);
    return;
  }
  if (type === "stream_event") {
    if (!isSubagentMessage(rec)) noteClaudeTurnStarted(live);
    handleStreamEvent(live, rec);
    return;
  }
  if (type === "assistant") {
    if (!isSubagentMessage(rec)) noteClaudeTurnStarted(live);
    handleAssistant(live, rec);
    return;
  }
  if (type === "user") {
    handleUser(live, rec);
    return;
  }
  if (type === "result") {
    handleResult(live, rec);
    return;
  }
  if (type === "rate_limit_event") {
    live.usageLimit = usageLimitFromRateLimitEvent(rec);
    return;
  }
  if (type === "system") {
    const text = statusTextFromSystem(rec);
    if (text) {
      if ((stringField(rec, "subtype") ?? "").startsWith("compact")) {
        live.compactionConfirmed = true;
      }
      live.onEvent({ type: "status", text });
    }
  }
}

function handleStreamEvent(live: Live, rec: Record<string, unknown>): void {
  const subagent = isSubagentMessage(rec);
  const delta = streamDeltaFromEvent(rec);
  if (delta) {
    if (subagent) return;
    if (delta.kind === "assistant") {
      closePendingAssistantMessage(live);
      live.emittedAssistant = joinStreamText(live.emittedAssistant, delta.text);
      live.onEvent({ type: "message.delta", text: delta.text });
    } else {
      live.emittedReasoning = joinStreamText(live.emittedReasoning, delta.text);
      live.onEvent({ type: "reasoning.delta", text: delta.text });
    }
    return;
  }

  const started = toolStartFromEvent(rec);
  if (started) {
    if (subagent) {
      noteSubagentTool(live, rec, started.id, started.name, started.input);
      return;
    }
    const tool: InFlightTool = {
      id: started.id,
      name: started.name,
      input: started.input,
      partialJson: "",
      title: toolTitle(started.name, started.input),
    };
    if (started.index >= 0) live.toolsByIndex.set(started.index, tool);
    live.toolsById.set(started.id, tool);
    noteDispatch(live, tool);
    live.onEvent({
      type: "tool.started",
      callId: tool.id,
      title: tool.title,
      kind: toolKindFromName(tool.name),
      ...(isAgentToolName(tool.name) && stringField(tool.input, "model")
        ? { agentModel: stringField(tool.input, "model") }
        : {}),
      status: isAgentToolName(tool.name) ? "in_progress" : "pending",
      preview: previewFromTool(tool.name, tool.input),
    });
    emitTaskListIfNeeded(live, tool.name, tool.input);
    return;
  }

  const jsonDelta = inputJsonDeltaFromEvent(rec);
  if (jsonDelta) {
    if (subagent) return;
    const tool = live.toolsByIndex.get(jsonDelta.index);
    if (!tool) return;
    tool.partialJson += jsonDelta.partial;
    const parsed = tryParseJsonRecord(tool.partialJson);
    if (!parsed) return;
    tool.input = parsed;
    tool.title = toolTitle(tool.name, parsed);
    noteDispatch(live, tool);
    live.onEvent({
      type: "tool.updated",
      callId: tool.id,
      title: tool.title,
      kind: toolKindFromName(tool.name),
      ...(isAgentToolName(tool.name) && stringField(tool.input, "model")
        ? { agentModel: stringField(tool.input, "model") }
        : {}),
      status: "pending",
      detail: summarizeToolRequest(tool.name, parsed),
      preview: previewFromTool(tool.name, parsed),
    });
    emitTaskListIfNeeded(live, tool.name, parsed);
    return;
  }
}

function handleAssistant(live: Live, rec: Record<string, unknown>): void {
  if (isSubagentMessage(rec)) {
    noteSubagentNarration(live, rec);
    for (const use of assistantToolUses(rec)) {
      noteSubagentTool(live, rec, use.id, use.name, use.input);
    }
    return;
  }

  const used = contextUsedFromAssistant(rec);
  if (used !== undefined) live.onEvent({ type: "context", used });

  const snapshot = assistantTextBlocks(rec).join("");
  if (snapshot) closePendingAssistantMessage(live);
  const extra = snapshotRemainder(live.emittedAssistant, snapshot);
  if (extra) {
    live.emittedAssistant = joinStreamText(live.emittedAssistant, extra);
    live.onEvent({ type: "message.delta", text: extra });
  }

  for (const use of assistantToolUses(rec)) {
    const streamed = live.toolsById.get(use.id);
    if (streamed) {
      // content_block_start often has an empty input. The input JSON delta may
      // never form a parseable object before the complete assistant snapshot.
      // Reconcile that snapshot instead of leaving the tool labelled "Shell".
      if (JSON.stringify(streamed.input) !== JSON.stringify(use.input)) {
        streamed.input = use.input;
        streamed.title = toolTitle(use.name, use.input);
        noteDispatch(live, streamed);
        live.onEvent({
          type: "tool.updated",
          callId: streamed.id,
          title: streamed.title,
          kind: toolKindFromName(streamed.name),
          ...(isAgentToolName(streamed.name) && stringField(use.input, "model")
            ? { agentModel: stringField(use.input, "model") }
            : {}),
          status: isAgentToolName(streamed.name) ? "in_progress" : "pending",
          preview: previewFromTool(streamed.name, use.input),
        });
        emitTaskListIfNeeded(live, streamed.name, use.input);
      }
      if (use.name === "ExitPlanMode") {
        const plan = extractExitPlanModePlan(use.input);
        if (plan) live.onEvent({ type: "plan", text: plan });
      }
      continue;
    }
    const tool: InFlightTool = {
      id: use.id,
      name: use.name,
      input: use.input,
      partialJson: "",
      title: toolTitle(use.name, use.input),
    };
    live.toolsById.set(use.id, tool);
    noteDispatch(live, tool);
    live.onEvent({
      type: "tool.started",
      callId: tool.id,
      title: tool.title,
      kind: toolKindFromName(tool.name),
      ...(isAgentToolName(tool.name) && stringField(tool.input, "model")
        ? { agentModel: stringField(tool.input, "model") }
        : {}),
      status: isAgentToolName(tool.name) ? "in_progress" : "pending",
      preview: previewFromTool(tool.name, tool.input),
    });
    if (use.name === "ExitPlanMode") {
      const plan = extractExitPlanModePlan(use.input);
      if (plan) live.onEvent({ type: "plan", text: plan });
    }
    emitTaskListIfNeeded(live, tool.name, tool.input);
  }

  // Each assistant record is one Claude message. Wait until the next message
  // begins to close its UI block, so a backgrounded turn stays visibly live.
  live.pendingAssistantBoundary = !!(snapshot || live.emittedAssistant);
  live.emittedAssistant = "";
  live.emittedReasoning = "";
}

function closePendingAssistantMessage(live: Live): void {
  if (!live.pendingAssistantBoundary) return;
  live.pendingAssistantBoundary = false;
  live.onEvent({ type: "message.completed" });
}

function handleUser(live: Live, rec: Record<string, unknown>): void {
  if (isSubagentMessage(rec)) {
    noteSubagentResults(live, rec);
    return;
  }
  const receipt = asRecord(rec.tool_use_result);
  for (const result of toolResultsFromUserMessage(rec)) {
    const tool = live.toolsById.get(result.toolUseId);
    if (!tool) continue;
    if (tool.name === "SendMessage") {
      const resumed =
        stringField(receipt, "resumedAgentId") ??
        stringField(tryParseJsonRecord(result.text), "resumedAgentId");
      if (resumed) claudeResume(live, tool.id, resumed);
      // A resumption's receipt is not the agent's report; its notification is.
      if (live.registry.isResume(tool.id) && !result.isError) continue;
    } else if (isAgentToolName(tool.name) && !live.registry.isResume(tool.id)) {
      const agentId =
        parseHandback(result.text).agentId ?? stringField(receipt, "agentId");
      if (agentId) noteAgentId(live, tool, agentId);
    }
    if (isAgentToolName(tool.name) && isBackgroundedAgentTool(live, tool.id)) {
      continue;
    }
    live.doneTools.add(tool.id);
    clearToolElapsed(tool.id);
    live.onEvent({
      type: "tool.updated",
      callId: tool.id,
      title: tool.title,
      kind: toolKindFromName(tool.name),
      status: result.isError ? "failed" : "completed",
      detail: result.text || undefined,
      preview: previewFromTool(tool.name, tool.input, result.text),
    });
    if (
      !result.isError &&
      applyClaudeTaskTool(live.claudeTasks, tool.name, tool.input, result.text)
    ) {
      live.onEvent({
        type: "tasks.updated",
        key: CLAUDE_TASKS_KEY,
        // The map is the source of truth, so a TaskUpdate subject is a rename.
        authoritative: true,
        providerSessionId: live.claudeSessionId,
        items: [...live.claudeTasks.values()],
      });
    }
    // What a subagent hands back is the last thing it said, so it closes out
    // that agent's own trail rather than sitting on the parent row as detail.
    if (isAgentToolName(tool.name) && result.text.trim() && !result.isError) {
      live.onEvent({
        type: "agent.step",
        callId: tool.id,
        stepId: `${tool.id}:report`,
        kind: "message",
        text: result.text,
      });
    }
    if (isAgentToolName(tool.name)) settleInlineAgentTask(live, tool.id);
  }
}

/**
 * A subagent that was never backgrounded reports back on the parent's own tool
 * result, and Claude sends no task record for one that ended inline. Without
 * this its task would keep the turn open for good: the reply reads as finished
 * while the composer and the plan's Build button stay disabled until a restart.
 */
function settleInlineAgentTask(live: Live, toolUseId: string): void {
  let settled = false;
  for (const [taskId, task] of [...live.agentTasks]) {
    if (task.toolUseId !== toolUseId || task.backgrounded) continue;
    live.agentTasks.delete(taskId);
    live.backgroundTasks.delete(taskId);
    settled = true;
  }
  if (!settled) return;
  maybeFinishTurn(live);
  syncBackgroundWait(live);
}

function handleResult(live: Live, rec: Record<string, unknown>): void {
  if (isSubagentMessage(rec)) return;
  if (isReplayedTaskNotificationResult(rec)) {
    // A limit reported for the replay must not carry over to the real turn.
    live.usageLimit = null;
    return;
  }
  // A self-started turn whose start went unseen still gets its marker. When a
  // user follow-up wins the race, the marker is emitted from the result, so it
  // trails the wake's text rather than leading it.
  const wokeAtStart = live.wakeHandled;
  if (
    stringField(asRecord(rec.origin), "kind") === "task-notification" &&
    !wokeAtStart
  ) {
    const text = live.wakeSummaries.shift();
    if (text !== undefined) emitWakeMarker(live, text);
  }
  live.wakeHandled = false;
  // A /compact result reports the summarizer call's usage, not the rebuilt
  // conversation level. The next real turn will provide the fresh reading.
  if (!live.manualCompaction) {
    const context = contextFromResult(rec);
    if (context) live.onEvent({ type: "context", ...context });
  }
  const metrics = turnMetricsFromResult(rec);
  if (metrics) live.onEvent({ type: "turn.metrics", ...metrics });

  const result = turnStatusFromResult(rec);
  if (result.status === "failed" && result.error && !live.cancelled) {
    live.onEvent({ type: "session.error", message: result.error });
  }
  // A refused window can still fall back to another model, so only a turn
  // that ended in error was stopped by it.
  const turnErrored = rec.is_error === true || result.status === "failed";
  const usageLimit = live.usageLimit ?? (isUsageLimitResult(rec) ? {} : null);
  live.usageLimit = null;
  if (usageLimit && turnErrored && !live.cancelled) {
    live.onEvent({ type: "usage.limited", ...usageLimit });
  }
  live.turnResultSeen = true;
  // Another finished task is queued: its own wake may follow this one. The
  // window opens at the wake's result, not its start, since a wake can run
  // longer than the grace. A user write into the wake leaves the next turn
  // theirs to answer, so no further wake is awaited.
  if (
    wokeAtStart &&
    live.wakeSummaries.length > 0 &&
    !live.steeredInTurn &&
    live.activeTurn
  ) {
    armAwaitingResume(live);
  }
  maybeFinishTurn(live);
  showBackgroundRows(live);
  syncBackgroundWait(live);
}

async function handleControlRequest(
  sessionId: string,
  live: Live,
  control: ClaudeControlRequest,
): Promise<void> {
  if (control.subtype !== "can_use_tool" && control.subtype !== "permission") {
    await writeJson(sessionId, buildControlResponse(control.requestId, {}));
    return;
  }

  const toolName = control.toolName ?? "tool";
  const input = control.input ?? {};

  if (live.cancelled || live.muteUpdates) {
    await writeJson(
      sessionId,
      buildControlResponse(
        control.requestId,
        toClaudePermissionResult("deny", input),
      ),
    ).catch(() => undefined);
    return;
  }

  if (toolName === "AskUserQuestion") {
    const questions = questionsFromUnknown(input);
    const uiId = live.nextApprovalUiId++;
    const pending = waitQuestion(live, uiId, control.requestId, {
      type: "question.asked",
      requestId: uiId,
      title:
        questionPromptTitle(questions) || extractAskUserQuestionTitle(input),
      questions,
      callId: control.toolUseId,
    });
    showNextQuestion(live);
    const outcome = await pending;
    const decision =
      outcome === "cancelled"
        ? "cancelled"
        : outcome.kind === "answered"
          ? "answered"
          : "skipped";
    live.onEvent({ type: "question.resolved", requestId: uiId, decision });
    showNextQuestion(live);
    if (outcome === "cancelled") return;
    const response =
      outcome.kind === "answered"
        ? {
            behavior: "allow",
            updatedInput: askUserQuestionAllowInput(input, outcome),
          }
        : {
            behavior: "deny",
            message: "User cancelled tool execution.",
          };
    await writeJson(
      sessionId,
      buildControlResponse(control.requestId, response),
    );
    return;
  }

  if (toolName === "ExitPlanMode") {
    const plan = extractExitPlanModePlan(input);
    if (plan) live.onEvent({ type: "plan", text: plan });
    await writeJson(
      sessionId,
      buildControlResponse(control.requestId, {
        behavior: "deny",
        message:
          "The client captured your proposed plan. Stop here and wait for the user's feedback or implementation request in a later turn.",
      }),
    );
    return;
  }

  applyKnownToolInput(live, toolName, input, control.toolUseId);

  if (live.planning) {
    const kind = toolKindFromName(toolName);
    const decision = kind === "read" || kind === "search" ? "allow" : "deny";
    await writeJson(
      sessionId,
      buildControlResponse(
        control.requestId,
        toClaudePermissionResult(decision, input),
      ),
    );
    return;
  }

  if (live.runtimeMode === "full-access") {
    await writeJson(
      sessionId,
      buildControlResponse(
        control.requestId,
        toClaudePermissionResult("allow", input),
      ),
    );
    return;
  }

  const uiId = live.nextApprovalUiId++;
  const pending = waitApproval(live, uiId, control.requestId, input);
  live.onEvent({
    type: "approval.requested",
    requestId: uiId,
    title: toolTitle(toolName, input),
    kind: toolKindFromName(toolName),
    callId: control.toolUseId,
    preview: previewFromTool(toolName, input),
  });
  const decision = await pending;
  live.onEvent({ type: "approval.resolved", requestId: uiId, decision });
  if (decision === "cancelled") return;
  await writeJson(
    sessionId,
    buildControlResponse(
      control.requestId,
      toClaudePermissionResult(decision, input),
    ),
  );
}

function applyKnownToolInput(
  live: Live,
  toolName: string,
  input: Record<string, unknown>,
  callId?: string,
): void {
  if (!callId || Object.keys(input).length === 0) return;
  const existing = live.toolsById.get(callId);
  if (existing) {
    existing.input = input;
    existing.title = toolTitle(toolName, input);
  }
  live.onEvent({
    type: "tool.updated",
    callId,
    title: toolTitle(toolName, input),
    kind: toolKindFromName(toolName),
    status: "pending",
    preview: previewFromTool(toolName, input),
  });
}

function waitApproval(
  live: Live,
  uiId: number,
  requestId: string,
  input: Record<string, unknown>,
): Promise<ApprovalOutcome> {
  return new Promise<ApprovalOutcome>((resolve) => {
    live.approvals.set(uiId, { requestId, input, resolve });
  }).finally(() => {
    live.approvals.delete(uiId);
  });
}

function waitQuestion(
  live: Live,
  uiId: number,
  requestId: string,
  event: Extract<HarnessEvent, { type: "question.asked" }>,
): Promise<UserQuestionReply | "cancelled"> {
  return new Promise<UserQuestionReply | "cancelled">((resolve) => {
    live.questions.set(uiId, { requestId, event, resolve });
  }).finally(() => {
    live.questions.delete(uiId);
  });
}

function showNextQuestion(live: Live): void {
  if (live.muteUpdates || live.cancelled) return;
  if (
    live.visibleQuestionId !== null &&
    live.questions.has(live.visibleQuestionId)
  )
    return;
  const next = live.questions.entries().next().value;
  live.visibleQuestionId = next?.[0] ?? null;
  if (next) live.onEvent(next[1].event);
}

function emitTaskListIfNeeded(
  live: Live,
  toolName: string,
  input: Record<string, unknown>,
): void {
  if (!isTodoTool(toolName)) return;
  const items = taskListFromTodos(input);
  if (items) live.onEvent({ type: "tasks.updated", items });
}

function handleAgentLifecycle(
  live: Live,
  rec: Record<string, unknown>,
): boolean {
  const started = parseTaskStarted(rec);
  if (started) {
    if (started.ambient) return true;
    if (started.backgrounded) live.backgrounded.add(started.taskId);
    noteTaskStarted(live, started);
    const card = cardBlock(
      live,
      taskOwner(live, started.toolUseId, started.taskId),
    );
    const previous = live.backgroundTasks.get(started.taskId);
    live.backgroundTasks.set(started.taskId, {
      ...previous,
      description: started.description,
      toolUseId: started.toolUseId,
      ...(card ? { toolId: card, steps: stepCount(live, card) } : {}),
    });
    syncBackgroundWait(live);
    if (!isAgentTaskType(started.taskType)) return true;
    live.agentTasks.set(started.taskId, {
      taskId: started.taskId,
      toolUseId: started.toolUseId,
      description: started.description,
      backgrounded: started.backgrounded,
    });
    upsertAgentTool(
      live,
      started.toolUseId,
      started.description,
      "in_progress",
    );
    return true;
  }

  const progress = parseTaskProgress(rec);
  if (progress) {
    const task = live.agentTasks.get(progress.taskId);
    // Progress is what the agent is doing now, never its title.
    const title = task?.description || progress.description || "Subagent";
    const owner = taskOwner(live, progress.toolUseId, progress.taskId);
    const background = live.backgroundTasks.get(progress.taskId);
    const card = cardBlock(live, owner);
    if (background) {
      if (progress.description) background.latest = progress.description;
      if (card) {
        background.toolId = card;
        background.steps = stepCount(live, card);
      }
    }
    const detail =
      progress.summary ||
      progress.lastToolName ||
      (progress.subagentType
        ? `${progress.subagentType.replace(/[_-]+/g, " ")} subagent`
        : undefined);
    upsertAgentTool(
      live,
      progress.toolUseId ?? task?.toolUseId,
      title,
      "in_progress",
      detail,
    );
    const runId = owner ?? task?.toolUseId;
    if (runId && progress.description) {
      live.onEvent({
        type: "agent.run",
        callId: runId,
        activity: progress.description,
      });
    }
    syncBackgroundWait(live);
    return true;
  }

  const updated = parseTaskUpdated(rec);
  if (updated) {
    if (isEndedStatus(updated.status)) {
      const owner = taskOwner(live, undefined, updated.taskId);
      if (owner && live.registry.isResume(owner)) {
        live.registry.noteFinished(updated.taskId);
      }
    }
    const task = live.agentTasks.get(updated.taskId);
    if (task && updated.backgrounded !== undefined) {
      task.backgrounded = updated.backgrounded;
    }
    if (task && updated.description) task.description = updated.description;
    if (updated.backgrounded === true) live.backgrounded.add(updated.taskId);
    if (updated.backgrounded === false) live.backgrounded.delete(updated.taskId);
    const background = live.backgroundTasks.get(updated.taskId);
    if (background && updated.description) {
      background.description = updated.description;
    }
    if (isTerminalAgentTaskStatus(updated.status)) {
      settleBackgroundRow(
        live,
        updated.taskId,
        updated.status ?? "completed",
        updated.error,
      );
      finishBackgroundTask(live, updated.taskId);
      completeAgentTask(
        live,
        updated.taskId,
        updated.status === "completed" ? "completed" : "failed",
        updated.error,
      );
    }
    return true;
  }

  const notice = parseTaskNotification(rec);
  if (notice) {
    if (!notice.ambient) {
      noteResumedNotification(live, notice);
      noteTaskNotification(live, notice);
      finishBackgroundTask(live, notice.taskId);
      completeAgentTask(
        live,
        notice.taskId,
        notice.status === "completed" ? "completed" : "failed",
        notice.summary || undefined,
      );
    }
    return true;
  }

  const allTasks = parseBackgroundTasks(rec);
  if (!allTasks) return false;
  const next = new Set(allTasks.map((task) => task.taskId));
  for (const id of [...live.backgroundTasks.keys()]) {
    if (next.has(id)) continue;
    settleBackgroundRow(live, id, "completed");
    finishBackgroundTask(live, id);
  }
  for (const row of allTasks) {
    if (!live.backgroundTasks.has(row.taskId)) {
      live.backgroundTasks.set(row.taskId, { description: row.description });
    }
  }
  const liveTasks = allTasks.filter((task) => isAgentTaskType(task.taskType));
  for (const id of [...live.agentTasks.keys()]) {
    if (!next.has(id)) completeAgentTask(live, id, "completed");
  }
  for (const row of liveTasks) {
    if (live.agentTasks.has(row.taskId)) continue;
    // The list carries no tool_use_id and often lands before task_started, so
    // find the Agent call that spawned it rather than opening a second row.
    const toolUseId = unclaimedAgentCall(live, row.description);
    live.agentTasks.set(row.taskId, {
      taskId: row.taskId,
      toolUseId,
      description: row.description,
      backgrounded: true,
    });
    // An agent this session dispatched earlier is being resumed: its
    // task_started names the call that shows the work, and the old Agent
    // block stays as it was.
    if (!toolUseId && live.registry.dispatchOf(row.taskId)) continue;
    upsertAgentTool(live, toolUseId, row.description, "in_progress");
  }
  maybeFinishTurn(live);
  syncBackgroundWait(live);
  return true;
}

function handleToolProgress(live: Live, rec: Record<string, unknown>): void {
  const progress = parseToolProgress(rec);
  if (!progress) return;
  // A heartbeat is the call itself still running, never a step: dropped before
  // routing, so it cannot become a never-ending child or a "Background agent".
  if (isHeartbeatProgress(progress)) {
    noteHeartbeat(live, progress);
    return;
  }
  const own = live.toolsById.get(progress.toolUseId);
  if (!(own && isAgentToolName(own.name)) && progress.parentToolUseId) {
    // A sub-agent's call in flight: a step wherever that agent's work goes.
    const parentId = progress.parentToolUseId;
    // Nothing to show: route nothing, so no empty block opens.
    if (!progress.toolName) {
      if (liveParent(live, parentId)) touchLiveParent(live, parentId);
      return;
    }
    const target =
      live.stepOwners.get(progress.toolUseId) ??
      routeChild(live, parentId, stringField(rec, "agent_id"));
    if (target === parentId) touchLiveParent(live, parentId);
    live.onEvent({
      type: "agent.step",
      callId: target,
      stepId: progress.toolUseId,
      kind: "tool",
      text: progress.toolName,
      status: "in_progress",
      ...(progress.subagentType ? { agentType: progress.subagentType } : {}),
    });
    noteStep(live, target, progress.toolUseId, "tool");
    return;
  }
  const tool = own;
  if (!tool || !isAgentToolName(tool.name)) return;
  live.onEvent({
    type: "tool.updated",
    callId: tool.id,
    title: tool.title,
    kind: "agent",
    status: "in_progress",
  });
  // Progress names the call in flight. That is a step in the run, not the
  // result of it, so it goes to the panel rather than onto the Agent row.
  if (progress.toolName) {
    live.onEvent({
      type: "agent.step",
      callId: tool.id,
      stepId: progress.toolUseId,
      kind: "tool",
      text: progress.toolName,
      status: "in_progress",
      ...(progress.subagentType ? { agentType: progress.subagentType } : {}),
    });
  }
}

/** Live elapsed for the running parent call. In memory only, never saved. */
function noteHeartbeat(live: Live, progress: ClaudeToolProgress): void {
  const beat = heartbeatElapsed(progress);
  if (!beat) return;
  if (
    !live.activeTurn ||
    !live.toolsById.has(beat.callId) ||
    live.doneTools.has(beat.callId)
  ) {
    return;
  }
  setToolElapsed(beat.callId, beat.seconds);
}

/**
 * The live Agent call a subagent frame belongs to: routing rule 1. A call that
 * already finished can only receive resumed or stray work, which rules 2-4
 * place.
 */
function liveParent(live: Live, parentId: string): InFlightTool | undefined {
  const parent = live.toolsById.get(parentId);
  if (!parent) return undefined;
  // An agent call, or a SendMessage that became a resumed agent run.
  const agent =
    isAgentToolName(parent.name) || live.resumedBlocks.has(parentId);
  if (!agent || live.doneTools.has(parentId)) return undefined;
  return parent;
}

/** `<process>-<turn>`: one synthetic block per parent and turn. */
function syntheticScope(live: Live): string {
  return `${live.scope}-${live.turn}`;
}

/** A block this process can still write to. */
function knownBlock(live: Live, id: string): boolean {
  return live.toolsById.has(id) || live.synthetic.has(id);
}

/** Rules 1 and 2 only: an existing block, never a new one. */
function placedChild(
  live: Live,
  parentId: string,
  agentHint: string | undefined,
): string | undefined {
  if (liveParent(live, parentId)) return parentId;
  const target =
    live.registry.targetForParent(parentId) ??
    (agentHint ? live.registry.targetForAgent(agentHint) : undefined);
  if (!target || !knownBlock(live, target)) return undefined;
  // A continuation belongs to one (parent, turn); a later turn gets its own.
  if (
    target.startsWith(CONTINUED_PREFIX) &&
    !target.endsWith(`:${syntheticScope(live)}`)
  ) {
    return undefined;
  }
  return target;
}

/**
 * Where a sub-agent frame whose parent is `parentId` goes, in the order Rebase
 * Native uses:
 * 1. `parentId` is a running agent call in this turn;
 * 2. the active resumption of the agent `parentId` dispatched;
 * 3. a per-turn continuation block for a known but finished dispatch;
 * 4. a visible "Background agent" block. Never nowhere.
 */
function routeChild(
  live: Live,
  parentId: string,
  agentHint: string | undefined,
): string {
  const placed = placedChild(live, parentId, agentHint);
  if (placed) return placed;
  const scope = syntheticScope(live);
  const name = live.registry.dispatchName(parentId);
  if (name !== undefined) {
    const id = `${CONTINUED_PREFIX}${parentId}:${scope}`;
    if (!live.synthetic.has(id)) {
      live.synthetic.add(id);
      const agentId = live.registry.dispatchAgent(parentId) ?? agentHint;
      if (agentId) live.registry.noteResume(id, agentId);
      openSyntheticRun(live, id, `Continued · ${name || "agent"}`, {
        agentId,
        resumeOf: parentId,
        continuation: true,
      });
    }
    return id;
  }
  const id = `${FALLBACK_PREFIX}${parentId}:${scope}`;
  if (!live.synthetic.has(id)) {
    live.synthetic.add(id);
    openSyntheticRun(live, id, "Background agent", { agentId: agentHint });
  }
  return id;
}

/**
 * A container for work no live call owns. Its steps carry their own state and
 * nothing reports when such work ends, so the block is settled from the start
 * rather than spinning forever.
 */
function openSyntheticRun(
  live: Live,
  id: string,
  name: string,
  meta: { agentId?: string; resumeOf?: string; continuation?: boolean },
): void {
  live.onEvent({
    type: "tool.started",
    callId: id,
    title: name,
    kind: "agent",
    status: "completed",
  });
  live.onEvent({
    type: "agent.run",
    callId: id,
    name,
    ...(meta.agentId ? { agentId: meta.agentId } : {}),
    ...(meta.resumeOf ? { resumeOf: meta.resumeOf } : {}),
    ...(meta.continuation ? { continuation: true } : {}),
  });
  live.onEvent({
    type: "tool.updated",
    callId: id,
    title: name,
    kind: "agent",
    status: "completed",
  });
}

/** The live parent keeps its own "still running" status while its trail grows. */
function touchLiveParent(live: Live, parentId: string): void {
  const parent = live.toolsById.get(parentId);
  if (!parent) return;
  live.onEvent({
    type: "tool.updated",
    callId: parent.id,
    title: parent.title,
    kind: "agent",
    status: "in_progress",
  });
}

function noteStep(
  live: Live,
  block: string,
  stepId: string,
  kind: "tool" | "message" | "reasoning",
): void {
  live.stepOwners.set(stepId, block);
  if (kind !== "tool") return;
  let steps = live.toolSteps.get(block);
  if (!steps) {
    steps = new Set();
    live.toolSteps.set(block, steps);
  }
  steps.add(stepId);
}

/** Rebase Native bounds a child result to 16 KiB before storing it. */
const MAX_CHILD_DETAIL_BYTES = 16 * 1024;

function boundDetail(text: string): string {
  const bytes = new TextEncoder().encode(text);
  if (bytes.length <= MAX_CHILD_DETAIL_BYTES) return text;
  // Cut on a character boundary at or below the limit.
  return new TextDecoder()
    .decode(bytes.slice(0, MAX_CHILD_DETAIL_BYTES))
    .replace(/\uFFFD$/, "");
}

/**
 * Rebase Native's terminal set: any status it can read other than running.
 * Unknown statuses are not terminal.
 */
function isEndedStatus(status: string | undefined): boolean {
  switch ((status ?? "").toLowerCase()) {
    case "completed":
    case "complete":
    case "done":
    case "finished":
    case "closed":
    case "stopped":
    case "failed":
    case "error":
    case "errored":
    case "notfound":
    case "not_found":
    case "interrupted":
    case "cancelled":
    case "canceled":
    case "killed":
    case "aborted":
      return true;
    default:
      return false;
  }
}

function stepCount(live: Live, block: string): number {
  return live.toolSteps.get(block)?.size ?? 0;
}

/**
 * A call a subagent made, mirrored onto the run its frame routes to. No
 * detail on the row: an Agent row's detail is the report the run hands back.
 */
function noteSubagentTool(
  live: Live,
  rec: Record<string, unknown>,
  id: string,
  name: string,
  input: Record<string, unknown>,
): void {
  const parentId = stringField(rec, "parent_tool_use_id");
  if (!parentId) return;
  if (!id) {
    if (liveParent(live, parentId)) touchLiveParent(live, parentId);
    return;
  }
  const target = routeChild(live, parentId, stringField(rec, "agent_id"));
  if (target === parentId) touchLiveParent(live, parentId);
  const preview = previewFromTool(name, input);
  live.onEvent({
    type: "agent.step",
    callId: target,
    stepId: id,
    kind: "tool",
    text: toolTitle(name, input),
    toolKind: toolKindFromName(name),
    status: "in_progress",
    ...(preview ? { preview } : {}),
  });
  noteStep(live, target, id, "tool");
}

/**
 * What a subagent said and thought on its way through the work. Its prose
 * never joins the parent transcript — that would read as the main agent
 * talking — but it is the most legible thing in the panel for its own row.
 */
function noteSubagentNarration(live: Live, rec: Record<string, unknown>): void {
  const parentId = stringField(rec, "parent_tool_use_id");
  if (!parentId) return;
  const thinking = assistantThinkingBlocks(rec).join("").trim();
  const text = assistantTextBlocks(rec).join("").trim();
  if (!thinking && !text) {
    // Only a live parent's model is worth noting; nothing else to place.
    const model = stringField(asRecord(rec.message), "model");
    if (model && liveParent(live, parentId))
      live.onEvent({
        type: "tool.updated",
        callId: parentId,
        kind: "agent",
        agentModel: model,
      });
    return;
  }
  const target = routeChild(live, parentId, stringField(rec, "agent_id"));
  const model = stringField(asRecord(rec.message), "model");
  if (model && target === parentId)
    live.onEvent({
      type: "tool.updated",
      callId: parentId,
      kind: "agent",
      agentModel: model,
    });
  const messageId = assistantMessageId(rec) ?? crypto.randomUUID();
  if (thinking) {
    live.onEvent({
      type: "agent.step",
      callId: target,
      stepId: `${messageId}:thinking`,
      kind: "reasoning",
      text: thinking,
    });
    noteStep(live, target, `${messageId}:thinking`, "reasoning");
  }
  if (text) {
    live.onEvent({
      type: "agent.step",
      callId: target,
      stepId: `${messageId}:text`,
      kind: "message",
      text,
    });
    noteStep(live, target, `${messageId}:text`, "message");
  }
}

/**
 * Settles a subagent's own tool rows once their results come back, wherever
 * those rows live. A result for a call this session never saw still shows up.
 */
function noteSubagentResults(live: Live, rec: Record<string, unknown>): void {
  const parentId = stringField(rec, "parent_tool_use_id");
  if (!parentId) return;
  for (const result of toolResultsFromUserMessage(rec)) {
    const status = result.isError ? "failed" : "completed";
    // The output is the step's detail whatever the outcome, as in Native.
    const detail = boundDetail(result.text);
    const owner = live.stepOwners.get(result.toolUseId);
    if (owner) {
      live.onEvent({
        type: "agent.step",
        callId: owner,
        stepId: result.toolUseId,
        kind: "tool",
        text: "",
        status,
        ...(detail ? { detail } : {}),
      });
      continue;
    }
    const target = routeChild(live, parentId, stringField(rec, "agent_id"));
    live.onEvent({
      type: "agent.step",
      callId: target,
      stepId: result.toolUseId,
      kind: "tool",
      text: "Tool result",
      status,
      ...(detail ? { detail } : {}),
    });
    noteStep(live, target, result.toolUseId, "tool");
  }
}

/** A main-thread Agent call: remember it so later turns can place its agent. */
function noteDispatch(live: Live, tool: InFlightTool): void {
  if (!isAgentToolName(tool.name) || live.registry.isResume(tool.id)) return;
  live.registry.noteDispatch(tool.id, undefined, agentName(tool));
}

/** The name a dispatch's run shows, the same one its block is titled with. */
function agentName(tool: InFlightTool): string {
  return stringField(tool.input, "description") ?? tool.title;
}

/** An Agent call learned its agent id; its block records it once. */
function noteAgentId(live: Live, tool: InFlightTool, agentId: string): void {
  const known = live.registry.dispatchAgent(tool.id);
  live.registry.noteDispatch(tool.id, agentId, agentName(tool));
  if (known) return;
  live.onEvent({ type: "agent.run", callId: tool.id, agentId });
}

/**
 * A task_started names the call behind an agent task: an Agent dispatch learns
 * its agent id; a SendMessage becomes that agent's resumption.
 */
function noteTaskStarted(
  live: Live,
  started: { taskId: string; toolUseId?: string; taskType: string },
): void {
  if (!started.toolUseId) return;
  const tool = live.toolsById.get(started.toolUseId);
  if (!tool) return;
  const agentTask = isAgentTaskType(started.taskType);
  if (tool.name === "SendMessage") {
    if (agentTask || live.registry.dispatchOf(started.taskId)) {
      claudeResume(live, tool.id, started.taskId);
    }
  } else if (
    agentTask &&
    isAgentToolName(tool.name) &&
    !live.registry.isResume(tool.id)
  ) {
    noteAgentId(live, tool, started.taskId);
  }
}

/** The SendMessage block `send` now shows `agentId`'s resumed work. */
function claudeResume(live: Live, send: string, agentId: string): void {
  const known = live.registry.resumedAgent(send) === agentId;
  live.registry.noteResume(send, agentId);
  const origin = live.registry.dispatchOf(agentId);
  const tool = live.toolsById.get(send);
  if (!tool) return;
  if (known && live.resumedBlocks.has(send)) return;
  live.resumedBlocks.add(send);
  const name =
    origin?.name || stringField(tool.input, "to") || agentId;
  live.onEvent({
    type: "agent.run",
    callId: send,
    name: `Resumed · ${name}`,
    agentId,
    ...(origin ? { resumeOf: origin.toolId } : {}),
  });
  live.onEvent({
    type: "tool.updated",
    callId: send,
    title: tool.title,
    kind: "agent",
    status: "in_progress",
  });
}

/**
 * The block a task event belongs to: the call that started the task (an Agent
 * or a resuming SendMessage), else the agent's active resumption or dispatch.
 */
function taskOwner(
  live: Live,
  toolUseId: string | undefined,
  taskId: string | undefined,
): string | undefined {
  const target = live.registry.targetForTask(toolUseId, taskId);
  if (target && knownBlock(live, target)) return target;
  return toolUseId;
}

/** `owner`, when it is an agent-run block the waiting card can point at. */
function cardBlock(live: Live, owner: string | undefined): string | undefined {
  if (!owner) return undefined;
  if (live.registry.isResume(owner) || live.synthetic.has(owner)) return owner;
  const tool = live.toolsById.get(owner);
  return tool && isAgentToolName(tool.name) ? owner : undefined;
}

/**
 * A resumed (or continued) run's notification: its summary is the report and
 * its status the run's. It also opens the window in which Claude may wake
 * itself to answer the result.
 */
function noteResumedNotification(
  live: Live,
  notice: ClaudeAgentTaskNotification,
): void {
  if (live.backgrounded.delete(notice.taskId) && live.turnResultSeen) {
    const summary = notice.summary || notice.description;
    if (summary) live.wakeSummaries.push(summary);
  }
  const owner = taskOwner(live, notice.toolUseId, notice.taskId);
  if (!owner || !live.registry.isResume(owner)) return;
  live.registry.noteFinished(notice.taskId);
  live.doneTools.add(owner);
  clearToolElapsed(owner);
  live.onEvent({
    type: "tool.updated",
    callId: owner,
    kind: "agent",
    status: notice.status === "completed" ? "completed" : "failed",
    ...(notice.summary ? { detail: notice.summary } : {}),
  });
}

/**
 * Persisted as an interjection row (`customType: "task-notification"`), the
 * same row Rebase Native writes, so Claude's self-started follow-up is not
 * read as a reply to the user.
 */
function emitWakeMarker(live: Live, text: string): void {
  if (live.muteUpdates) return;
  live.onEvent({
    type: "interjection",
    text,
    customType: "task-notification",
    sentAt: Date.now(),
  });
}

function isBackgroundedAgentTool(live: Live, toolUseId: string): boolean {
  for (const task of live.agentTasks.values()) {
    if (task.toolUseId === toolUseId && task.backgrounded) return true;
  }
  return false;
}

/** The latest Agent call with this description that no task has claimed yet. */
function unclaimedAgentCall(
  live: Live,
  description: string,
): string | undefined {
  const claimed = new Set(
    [...live.agentTasks.values()].map((task) => task.toolUseId),
  );
  let match: string | undefined;
  for (const tool of live.toolsById.values()) {
    if (!isAgentToolName(tool.name) || claimed.has(tool.id)) continue;
    if (stringField(tool.input, "description") === description) match = tool.id;
  }
  return match;
}

function upsertAgentTool(
  live: Live,
  callId: string | undefined,
  title: string,
  status: string,
  detail?: string,
): void {
  const id = callId ?? `agent:${title}`;
  const existing = live.toolsById.get(id);
  if (!existing) {
    live.toolsById.set(id, {
      id,
      name: "Agent",
      input: {},
      partialJson: "",
      title,
    });
    live.onEvent({
      type: "tool.started",
      callId: id,
      title,
      kind: "agent",
      status,
    });
    if (
      status !== "in_progress" &&
      status !== "pending" &&
      status !== "running"
    ) {
      live.onEvent({
        type: "tool.updated",
        callId: id,
        title,
        kind: "agent",
        status,
        ...(detail ? { detail } : {}),
      });
    }
    return;
  }
  if (title) existing.title = title;
  live.onEvent({
    type: "tool.updated",
    callId: id,
    title: existing.title,
    kind: "agent",
    status,
    ...(detail ? { detail } : {}),
  });
}

function completeAgentTask(
  live: Live,
  taskId: string,
  status: string,
  detail?: string,
): void {
  const task = live.agentTasks.get(taskId);
  live.agentTasks.delete(taskId);
  if (task?.toolUseId) {
    live.doneTools.add(task.toolUseId);
    clearToolElapsed(task.toolUseId);
  }
  if (task && !task.toolUseId && live.registry.dispatchOf(taskId)) {
    // A resumed agent seen only in the task list never had a row of its own.
  } else if (task) {
    upsertAgentTool(
      live,
      task.toolUseId,
      task.description,
      status,
      detail ?? (status === "failed" ? "Subagent failed." : undefined),
    );
  }
  maybeFinishTurn(live);
}

/**
 * A task is done. If Claude had already yielded, the notification about it
 * starts a follow-up turn, so hold the Rebase turn open for that too rather
 * than settling in the gap between the two.
 */
function finishBackgroundTask(live: Live, taskId: string): void {
  if (!live.backgroundTasks.delete(taskId)) return;
  if (live.turnResultSeen && live.activeTurn) armAwaitingResume(live);
  maybeFinishTurn(live);
  syncBackgroundWait(live);
}

/**
 * Claude began another turn inside this Rebase turn: woken by a finished
 * task, or by a follow-up written in while it waited. Its own result, not the
 * earlier one, decides when the Rebase turn ends.
 */
function noteClaudeTurnStarted(live: Live): void {
  if (!live.activeTurn || !live.turnResultSeen) return;
  // Claude woke itself after a background task's notification: lead its
  // follow-up with that notification. A turn the user started gets no marker;
  // the summary stays for the result-origin fallback should Claude still wake.
  let marker: string | undefined;
  const userTurn = live.userTurnPending;
  live.userTurnPending = false;
  let woke = false;
  // Without the window the summary stays queued: the wake's result then
  // shows it, so it is never dropped unseen.
  if (!userTurn && live.awaitingResume && live.wakeSummaries.length > 0) {
    marker = live.wakeSummaries.shift();
    woke = true;
  }
  live.steeredInTurn = false;
  live.turnResultSeen = false;
  // A new message, not more of the last one: its snapshot must not be
  // compared against what the earlier turn streamed.
  live.emittedAssistant = "";
  live.emittedReasoning = "";
  live.pendingAssistantBoundary = false;
  // Close the message Claude left off with, so the reply starts its own and
  // the fold puts the earlier one away, the same as prose between tool calls.
  // A background command's row already sits between the two; a subagent's
  // report is noted in the trail to do the same.
  live.onEvent({ type: "message.completed" });
  live.onEvent({ type: "reasoning.completed" });
  if (marker) emitWakeMarker(live, marker);
  for (const text of live.taskNotes) {
    if (text !== marker) live.onEvent({ type: "status", text });
  }
  live.taskNotes = [];
  clearAwaitingResume(live);
  live.wakeHandled = woke;
  syncBackgroundWait(live);
}

/**
 * Claude yielded with commands still running. Each gets a live row under the
 * message it left off with, like any call in flight, until it finishes.
 * Subagents already have a row of their own.
 */
function showBackgroundRows(live: Live): void {
  if (!live.activeTurn || live.cancelled) return;
  for (const [taskId, task] of live.backgroundTasks) {
    if (live.backgroundRows.has(taskId) || live.agentTasks.has(taskId))
      continue;
    const source = task.toolUseId
      ? live.toolsById.get(task.toolUseId)
      : undefined;
    if (source && isAgentToolName(source.name)) continue;
    const callId = `background:${taskId}`;
    live.backgroundRows.set(taskId, callId);
    live.onEvent({
      type: "tool.started",
      callId,
      title: source ? toolTitle(source.name, source.input) : task.description,
      kind: source ? toolKindFromName(source.name) : "execute",
      status: "in_progress",
      background: true,
      ...(source
        ? { preview: previewFromTool(source.name, source.input) }
        : {}),
    });
  }
}

function settleBackgroundRow(
  live: Live,
  taskId: string,
  status: string,
  detail?: string,
): void {
  const callId = live.backgroundRows.get(taskId);
  if (!callId || live.muteUpdates) return;
  live.onEvent({
    type: "tool.updated",
    callId,
    status: status === "completed" ? "completed" : "failed",
    ...(detail ? { detail } : {}),
  });
}

/**
 * What Claude was told when a task finished. A command's row takes the
 * summary; a subagent's is kept for the trail until Claude picks back up.
 */
function noteTaskNotification(
  live: Live,
  notice: ClaudeAgentTaskNotification,
): void {
  if (!live.activeTurn) return;
  if (live.backgroundRows.has(notice.taskId)) {
    settleBackgroundRow(
      live,
      notice.taskId,
      notice.status,
      notice.summary || undefined,
    );
    return;
  }
  const tool = notice.toolUseId ? live.toolsById.get(notice.toolUseId) : null;
  const agent =
    (tool && isAgentToolName(tool.name)) || live.agentTasks.has(notice.taskId);
  if (!agent || !live.turnResultSeen) return;
  live.taskNotes.push(notice.summary || "Subagent finished.");
}

function armAwaitingResume(live: Live): void {
  if (live.awaitingResume) return;
  live.awaitingResume = setTimeout(() => {
    live.awaitingResume = null;
    maybeFinishTurn(live);
    syncBackgroundWait(live);
  }, RESUME_GRACE_MS);
}

function clearAwaitingResume(live: Live): void {
  if (!live.awaitingResume) return;
  clearTimeout(live.awaitingResume);
  live.awaitingResume = null;
}

/**
 * Tells the UI what the turn is waiting on once Claude has yielded with work
 * still running, and clears it when Claude picks the thread back up.
 */
function syncBackgroundWait(live: Live): void {
  const open = live.activeTurn && live.turnResultSeen && !live.cancelled;
  const details = open
    ? [...live.backgroundTasks].map(([id, task]) => ({
        id,
        description: task.description,
        ...(task.toolId ? { toolId: task.toolId } : {}),
        ...(task.latest ? { latest: task.latest } : {}),
        steps: task.steps ?? 0,
      }))
    : [];
  const waiting = details.map((task) => task.description);
  const key = details.length ? JSON.stringify(details) : "";
  if (key === live.backgroundKey) return;
  live.backgroundKey = key;
  if (live.muteUpdates) return;
  live.onEvent({ type: "background.updated", tasks: waiting, details });
}

function maybeFinishTurn(live: Live): void {
  if (!live.turnResultSeen) return;
  if (live.agentTasks.size > 0 || live.backgroundTasks.size > 0) return;
  if (live.awaitingResume) return;
  if (!live.activeTurn && !live.turnDone) return;
  finishActiveTurn(live, [
    { type: "message.completed" },
    { type: "reasoning.completed" },
  ]);
}

function finishActiveTurn(live: Live, extraEvents: HarnessEvent[] = []): void {
  clearAwaitingResume(live);
  live.turnEndPending = false;
  live.activeTurn = false;
  // A call still running when the turn ends or is cancelled gets no result.
  for (const id of live.toolsById.keys()) clearToolElapsed(id);
  for (const event of extraEvents) live.onEvent(event);
  const done = live.turnDone;
  const failed = live.turnFailed;
  live.turnDone = null;
  live.turnFailed = null;
  if (done) {
    done();
    return;
  }
  if (!failed) live.turnEndPending = true;
}

function settlePendingTurn(live: Live): void {
  if (!live.turnEndPending || !live.turnDone) return;
  finishActiveTurn(live);
}

function markInitialized(live: Live): void {
  if (live.initialized) return;
  live.initialized = true;
  live.initDone?.();
  live.initDone = null;
}

function waitForInit(live: Live, timeoutMs: number): Promise<void> {
  if (live.initialized) return Promise.resolve();
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      live.initDone = null;
      resolve();
    }, timeoutMs);
    live.initDone = () => {
      clearTimeout(timer);
      resolve();
    };
  });
}

function nextControlId(live: Live): string {
  live.nextControlId += 1;
  return `rebase_${live.nextControlId}`;
}

function writeJson(
  sessionId: string,
  payload: Record<string, unknown>,
): Promise<void> {
  return writeChild(sessionId, JSON.stringify(payload));
}

function settingsKeyFor(input: HarnessSessionInput): string {
  return `${input.providerAccountId ?? "default"}:${claudeSettingsKey({
    model: nativeModelId(input.model),
    effort: input.modelSettings?.effort,
    fast: input.modelSettings?.fast,
    thinking: input.modelSettings?.thinking,
    context: input.modelSettings?.context,
    runtimeMode: input.runtimeMode,
    hooks: loadClaudeHooks(),
  })}`;
}

function launchOptions(
  input: HarnessSessionInput,
  resume: string | undefined,
  sessionId: string,
): {
  model?: string;
  effort?: string;
  permissionMode?: ReturnType<typeof runtimeModeToPermission>;
  resume?: string;
  sessionId?: string;
  settings?: ClaudeCliSettings;
} {
  const native = nativeModelId(input.model);
  const effortRaw = input.modelSettings?.effort;
  const context = input.modelSettings?.context;
  const settings: ClaudeCliSettings = {};
  if (input.modelSettings?.thinking === "true") {
    settings.alwaysThinkingEnabled = true;
  }
  if (input.modelSettings?.fast === "true") {
    settings.fastMode = true;
  }
  if (isClaudeUltracodeEffort(effortRaw)) {
    settings.ultracode = true;
  }
  if (!loadClaudeHooks()) {
    settings.disableAllHooks = true;
  }
  return {
    model: resolveClaudeApiModelId(native, context),
    effort: normalizeClaudeCliEffort(effortRaw, native),
    permissionMode:
      input.intent === "plan"
        ? "plan"
        : runtimeModeToPermission(input.runtimeMode),
    resume,
    sessionId: resume ? undefined : sessionId,
    settings: Object.keys(settings).length > 0 ? settings : undefined,
  };
}

/** Exported for tests. */
export function __claudeTestReset(): void {
  liveByThread.clear();
  resumeByThread.clear();
  tasksByThread.clear();
  registryByThread.clear();
  cancelledThreads.clear();
  clearAllToolElapsed();
}
