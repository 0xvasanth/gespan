import {
  promptBlocks,
  type PromptContentBlock,
} from "../../../../features/sessions/model/attachments";
import type { AgentModel } from "../../../../features/sessions/model/models";
import type {
  Attachment,
  RuntimeMode,
} from "../../../../features/sessions/model/session";
import { pickAutoOption } from "../grok/grokProtocol";

export const KIRO_AUTH_HELP =
  "Run `kiro-cli login` (check with `kiro-cli whoami`).";

/**
 * Kiro's V3 engine is selected explicitly; the docs do not say which engine a
 * bare `acp` picks, so it is only the fallback for CLIs that reject the flag.
 */
export const KIRO_LAUNCH_ARGS: readonly (readonly string[])[] = [
  ["acp", "--agent-engine=v3"],
  ["acp"],
];

export type KiroCapabilities = {
  loadSession: boolean;
  image: boolean;
  embeddedContext: boolean;
};

export type KiroModelControl =
  | { kind: "config"; configId: string; current?: string }
  | { kind: "setModel"; current?: string }
  | { kind: "none" };

const AUTH_STDERR =
  /not logged in|login required|please (?:log ?in|sign ?in)|(?:authentication|auth) (?:required|failed|expired)|unauthori[sz]ed|(?:token|session|credentials?) (?:has )?expired|invalid (?:token|credentials?)/i;
const AUTH_DETAIL =
  /not logged in|log ?in|sign ?in|auth|credential|token|api key|unauthori[sz]ed/i;

/**
 * Only explicit sign-in failures reach the conversation. Kiro logs ordinary
 * progress to stderr, and matching on a bare word like `login` would turn a
 * successful turn into an apparent failure.
 */
export function kiroStderrAuthError(line: string): string | null {
  const detail = line.trim();
  if (!detail || /\[(?:debug|info|warn(?:ing)?)\]|^(?:debug|info|warn)\b/i.test(detail)) {
    return null;
  }
  if (!AUTH_STDERR.test(detail)) return null;
  return `${detail}\n\n${KIRO_AUTH_HELP}`;
}

export function kiroStartupError(error: unknown): Error {
  const detail = (error instanceof Error ? error.message : String(error)).trim();
  if (AUTH_DETAIL.test(detail)) {
    return new Error(`${detail}\n\n${KIRO_AUTH_HELP}`);
  }
  if (/timed out/i.test(detail)) {
    return new Error(`Kiro CLI did not start. ${KIRO_AUTH_HELP}`);
  }
  return new Error(`Kiro CLI did not start. ${detail}`);
}

/** A failed prompt keeps Kiro's wording, plus the login hint when it is an auth failure. */
export function kiroTurnErrorMessage(error: unknown): string {
  const detail = (error instanceof Error ? error.message : String(error)).trim();
  return AUTH_DETAIL.test(detail) ? `${detail}\n\n${KIRO_AUTH_HELP}` : detail;
}

export function kiroSessionId(result: unknown): string | undefined {
  const rec = asRecord(result);
  const id = rec?.sessionId ?? rec?.session_id;
  return typeof id === "string" && id.trim() ? id.trim() : undefined;
}

/** Capabilities come from `initialize`, never from the CLI version. */
export function kiroInitCapabilities(init: unknown): KiroCapabilities {
  const agent = asRecord(asRecord(init)?.agentCapabilities);
  const prompt = asRecord(agent?.promptCapabilities);
  return {
    loadSession: agent?.loadSession === true,
    image: prompt?.image === true,
    embeddedContext: prompt?.embeddedContext === true,
  };
}

/**
 * Standard ACP blocks, except images: an image Kiro did not advertise support
 * for travels as a path the agent's own tools can read.
 */
export function kiroPromptBlocks(
  text: string,
  attachments: Attachment[] = [],
  caps: Pick<KiroCapabilities, "image">,
): PromptContentBlock[] {
  const blocks = promptBlocks(text, attachments);
  if (caps.image) return blocks;
  // promptBlocks emits one block per attachment after an optional leading
  // text block, so the trailing blocks line up with `attachments` by index.
  const offset = blocks.length - attachments.length;
  return blocks.map((block, index) => {
    if (block.type !== "image") return block;
    const file = attachments[index - offset];
    if (!file?.path) {
      throw new Error(
        `Kiro CLI did not advertise image prompts and ${JSON.stringify(file?.name ?? "the image")} has no file path to attach instead. Save the image to disk and attach it again.`,
      );
    }
    return {
      type: "text",
      text: `Attached image (read from disk): ${JSON.stringify(file.path)}`,
    };
  });
}

export function kiroModelControl(session: unknown): KiroModelControl {
  const option = modelConfigOption(session);
  if (option) {
    const current = optionCurrent(option);
    return {
      kind: "config",
      configId: optionId(option) ?? "model",
      ...(current ? { current } : {}),
    };
  }
  const state = asRecord(asRecord(session)?.models);
  if (state) {
    const current = stringValue(state.currentModelId ?? state.current_model_id);
    const available = state.availableModels ?? state.available_models;
    if (current || (Array.isArray(available) && available.length > 0)) {
      return { kind: "setModel", ...(current ? { current } : {}) };
    }
  }
  return { kind: "none" };
}

/** Read models from a `model` config option (V3) or the models state (v2). */
export function modelsFromKiroSession(session: unknown): AgentModel[] {
  const option = modelConfigOption(session);
  const entries: { id: string; name: string }[] = [];
  let current: string | undefined;
  if (option) {
    current = optionCurrent(option);
    for (const item of flattenOptions(option.options)) {
      entries.push({ id: item.id, name: item.name });
    }
  } else {
    const state = asRecord(asRecord(session)?.models);
    current = stringValue(state?.currentModelId ?? state?.current_model_id);
    const raw = state?.availableModels ?? state?.available_models;
    for (const item of Array.isArray(raw) ? raw : []) {
      const model = asRecord(item);
      const id = stringValue(model?.modelId ?? model?.model_id ?? model?.id);
      if (!model || !id) continue;
      entries.push({ id, name: stringValue(model.name) ?? id });
    }
  }

  const seen = new Set<string>();
  const models: AgentModel[] = [];
  for (const entry of entries) {
    if (seen.has(entry.id)) continue;
    seen.add(entry.id);
    models.push({
      id: `kiro:${entry.id}`,
      harness: "kiro",
      name: entry.name,
      nativeId: entry.id,
    });
  }
  const index = current
    ? models.findIndex((model) => model.nativeId === current)
    : -1;
  if (index > 0) models.unshift(...models.splice(index, 1));
  return models;
}

/** Replayed history on `session/load` carries `_meta.kiro.replay`. */
export function kiroIsReplay(params: unknown): boolean {
  const update = updateOf(params);
  return kiroMeta(update)?.replay === true;
}

export type KiroPermissionOption = {
  optionId: string;
  name: string;
  kind: string;
};

/** Options exactly as the agent offered them, with their ACP kinds. */
export function kiroPermissionOptions(params: unknown): KiroPermissionOption[] {
  const raw = asRecord(params)?.options;
  if (!Array.isArray(raw)) return [];
  const options: KiroPermissionOption[] = [];
  for (const item of raw) {
    const option = asRecord(item);
    const optionId = stringValue(option?.optionId ?? option?.option_id);
    if (!option || !optionId) continue;
    options.push({
      optionId,
      name: stringValue(option.name) ?? optionId,
      kind: stringValue(option.kind) ?? "",
    });
  }
  return options;
}

const ALLOW_KINDS = ["allow_once", "allow_always"];
const DENY_KINDS = ["reject_once", "reject_always"];

/**
 * Pick an offered option for a decision, or undefined when the agent offered
 * none. Kiro's docs forbid synthesizing an option, so the caller answers
 * `cancelled` rather than guessing an id.
 */
export function kiroPermissionChoice(
  decision: "allow" | "deny",
  options: KiroPermissionOption[],
): string | undefined {
  for (const kind of decision === "allow" ? ALLOW_KINDS : DENY_KINDS) {
    const match = options.find((option) => option.kind === kind);
    if (match) return match.optionId;
  }
  return undefined;
}

/**
 * Auto-approval follows the app's access level. The shared policy ranks option
 * names, so it is asked about kinds and the result is mapped back to the
 * option the agent really offered.
 */
export function kiroAutoPermission(
  runtimeMode: RuntimeMode,
  toolKind: string | undefined,
  options: KiroPermissionOption[],
): string | undefined {
  const kinds = ALLOW_KINDS.filter((kind) =>
    options.some((option) => option.kind === kind),
  );
  const kind = pickAutoOption(runtimeMode, toolKind, kinds);
  return options.find((option) => option.kind === kind)?.optionId;
}

export function kiroStopReason(result: unknown): string | undefined {
  const rec = asRecord(result);
  return stringValue(rec?.stopReason ?? rec?.stop_reason);
}

function modelConfigOption(session: unknown): Record<string, unknown> | null {
  const raw = asRecord(session)?.configOptions ?? asRecord(session)?.config_options;
  if (!Array.isArray(raw)) return null;
  for (const item of raw) {
    const option = asRecord(item);
    if (!option) continue;
    if (optionId(option) === "model" || option.category === "model") {
      return option;
    }
  }
  return null;
}

function optionId(option: Record<string, unknown>): string | undefined {
  return stringValue(option.id ?? option.configId ?? option.config_id);
}

function optionCurrent(option: Record<string, unknown>): string | undefined {
  return stringValue(option.currentValue ?? option.current_value);
}

/** Options are a flat list or groups of options; flatten both. */
function flattenOptions(raw: unknown): { id: string; name: string }[] {
  if (!Array.isArray(raw)) return [];
  const out: { id: string; name: string }[] = [];
  for (const item of raw) {
    const option = asRecord(item);
    if (!option) continue;
    if (Array.isArray(option.options)) {
      out.push(...flattenOptions(option.options));
      continue;
    }
    const id = stringValue(option.value ?? option.id);
    if (id) out.push({ id, name: stringValue(option.name) ?? id });
  }
  return out;
}

function updateOf(params: unknown): Record<string, unknown> | null {
  const rec = asRecord(params);
  return asRecord(rec?.update) ?? rec;
}

function kiroMeta(update: Record<string, unknown> | null) {
  return asRecord(asRecord(update?._meta)?.kiro);
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

export function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}
