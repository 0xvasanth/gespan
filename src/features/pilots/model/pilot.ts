import { projectKey, projectName } from "../../../shared/lib/paths";
import {
  loadTabGroupColors,
  loadTabGroupCustomColors,
  loadTabGroupLabels,
  loadTabGroupMascots,
  resolveTabGroupColor,
  resolveTabGroupLabel,
  resolveTabGroupMascot,
  TAB_GROUP_COLORS,
} from "../../workspace/model/tabGroups";
import {
  hasPendingApproval,
  type Block,
  type Session,
} from "../../sessions/model/session";
import {
  PROJECT_MASCOTS,
  projectMascot,
} from "../../projects/model/projectMascots";
import { removePilotBackground } from "./pilotBackground";

/** The project palette's colors, plus indigo for a Pilot's ninth preset. */
export const PILOT_COLORS = [
  ...TAB_GROUP_COLORS.slice(1),
  "hsl(245 75% 65%)",
] as const;

/**
 * A Pilot is a long-lived agent of its own: a name, a mascot and a color, one
 * conversation, and the projects it works on. It is not tied to any one of
 * them; it lives on the rail beside the projects and can be given more.
 */
export type Pilot = {
  id: string;
  /** Its conversation; absent until it is first opened. */
  sessionId?: string;
  /** Replaces the mascot's own name. */
  name?: string;
  mascot: string;
  color: string;
  /** Project folders it works on, in the order they were added. */
  projects: string[];
  /** New sessions stay out of the project sidebar unless turned on. */
  showStartedSessionsInSidebar?: boolean;
  /** Files the sessions it starts into a sidebar folder named after it. */
  useSidebarFolders?: boolean;
  /** Superseded by SOUL.md; only read once, to seed it. */
  instructions?: string;
  /**
   * From when each project had its own Pilot: the project whose agent folder
   * it inherits. The folder moves under its id the first time it is read.
   */
  legacyProject?: string;
};

const ROSTER_KEY = "gespan:pilot-roster";
const PILOTS_CHANGED = "gespan:pilots-changed";
export const HANDED_KEY = "gespan:pilot-handed";

/** Storage from when each project had one Pilot, keyed by project. */
const LEGACY_AGENTS_KEY = "gespan:pilots";
const LEGACY_PROFILES_KEY = "gespan:pilot-profiles";
/** And from before Pilots had their name. */
const LEGACY_KEYS: [string, string][] = [
  ["gespan:project-agents", LEGACY_AGENTS_KEY],
  ["gespan:project-agent-profiles", LEGACY_PROFILES_KEY],
  ["gespan:project-agent-handed", HANDED_KEY],
];

function readJson(key: string): unknown {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as unknown) : undefined;
  } catch {
    return undefined;
  }
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

let migrated = false;

/**
 * Turns each project's Pilot into one of its own that works on that project,
 * keeping its conversation, name and look. One never claimed is dropped.
 */
export function migrateLegacyPilotStorage(): void {
  if (migrated) return;
  migrated = true;
  try {
    for (const [legacy, key] of LEGACY_KEYS) {
      const value = localStorage.getItem(legacy);
      if (value == null) continue;
      if (localStorage.getItem(key) == null) localStorage.setItem(key, value);
      localStorage.removeItem?.(legacy);
    }
    if (localStorage.getItem(ROSTER_KEY) != null) return;
    const agents = record(readJson(LEGACY_AGENTS_KEY));
    const profiles = record(readJson(LEGACY_PROFILES_KEY));
    const mascots = loadTabGroupMascots();
    const colors = loadTabGroupColors();
    const customColors = loadTabGroupCustomColors();
    const roster: Pilot[] = [];
    for (const project of new Set([
      ...Object.keys(agents),
      ...Object.keys(profiles),
    ])) {
      const sessionId = agents[project];
      const profile = record(profiles[project]);
      if (typeof sessionId !== "string" && profile.claim !== "claimed")
        continue;
      const seed = projectName(project);
      roster.push({
        id: newPilotId(),
        ...(typeof sessionId === "string" ? { sessionId } : {}),
        ...(typeof profile.name === "string" && profile.name.trim()
          ? { name: profile.name }
          : {}),
        mascot: projectMascot(seed, resolveTabGroupMascot(project, mascots))
          .name,
        color: resolveTabGroupColor(project, colors, customColors, seed),
        projects: [project],
        ...(typeof profile.instructions === "string" &&
        profile.instructions.trim()
          ? { instructions: profile.instructions }
          : {}),
        legacyProject: project,
      });
    }
    localStorage.setItem(ROSTER_KEY, JSON.stringify(roster));
    localStorage.removeItem?.(LEGACY_AGENTS_KEY);
    localStorage.removeItem?.(LEGACY_PROFILES_KEY);
  } catch {
    // Storage unavailable: there is nothing to move either.
  }
}

function newPilotId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

function parsePilot(value: unknown): Pilot | undefined {
  const entry = record(value);
  if (typeof entry.id !== "string" || !/^[A-Za-z0-9-]+$/.test(entry.id))
    return undefined;
  const text = (key: string) =>
    typeof entry[key] === "string" ? (entry[key] as string) : undefined;
  const sessionId = text("sessionId");
  const name = text("name");
  const instructions = text("instructions");
  const legacyProject = text("legacyProject");
  return {
    id: entry.id,
    ...(sessionId ? { sessionId } : {}),
    ...(name ? { name } : {}),
    mascot: text("mascot") ?? PROJECT_MASCOTS[0].name,
    color: text("color") ?? TAB_GROUP_COLORS[1],
    projects: Array.isArray(entry.projects)
      ? entry.projects.filter(
          (path): path is string => typeof path === "string" && !!path,
        )
      : [],
    ...(typeof entry.showStartedSessionsInSidebar === "boolean"
      ? { showStartedSessionsInSidebar: entry.showStartedSessionsInSidebar }
      : {}),
    ...(entry.useSidebarFolders === true ? { useSidebarFolders: true } : {}),
    ...(instructions ? { instructions } : {}),
    ...(legacyProject ? { legacyProject } : {}),
  };
}

/** Every Pilot, in the order the rail shows them. */
export function listPilots(): Pilot[] {
  migrateLegacyPilotStorage();
  const parsed = readJson(ROSTER_KEY);
  if (!Array.isArray(parsed)) return [];
  return parsed.flatMap((entry) => parsePilot(entry) ?? []);
}

function saveRoster(roster: readonly Pilot[]): void {
  try {
    localStorage.setItem(ROSTER_KEY, JSON.stringify(roster));
  } catch {
    return;
  }
  window.dispatchEvent(new CustomEvent(PILOTS_CHANGED));
}

export function findPilot(id: string): Pilot | undefined {
  return listPilots().find((pilot) => pilot.id === id);
}

/** The Pilot whose conversation this is. */
export function pilotForSession(sessionId: string): Pilot | undefined {
  return listPilots().find((pilot) => pilot.sessionId === sessionId);
}

export function isPilotSession(sessionId: string): boolean {
  return !!pilotForSession(sessionId);
}

/**
 * The mascot and color the next Pilot gets: ones the others are not using yet,
 * where any are left.
 */
export function nextPilotLook(
  roster: readonly Pick<Pilot, "mascot" | "color">[] = listPilots(),
): { mascot: string; color: string } {
  const mascots = new Set(roster.map((pilot) => pilot.mascot));
  const colors = new Set(roster.map((pilot) => pilot.color));
  const palette = PILOT_COLORS;
  return {
    mascot: (
      PROJECT_MASCOTS.find((mascot) => !mascots.has(mascot.name)) ??
      PROJECT_MASCOTS[roster.length % PROJECT_MASCOTS.length]
    ).name,
    color:
      palette.find((color) => !colors.has(color)) ??
      palette[roster.length % palette.length],
  };
}

/** A new Pilot with the next look, starting with the projects given, if any. */
export function createPilot(projects: readonly string[] = []): Pilot {
  const roster = listPilots();
  const pilot: Pilot = {
    id: newPilotId(),
    ...nextPilotLook(roster),
    projects: uniqueProjects(projects),
  };
  saveRoster([...roster, pilot]);
  return pilot;
}

const PILOT_CONVERSATION_PREFIX = "pilot:";

/** The id a phone uses for a Pilot's conversation. */
export function pilotConversationId(pilotId: string): string {
  return PILOT_CONVERSATION_PREFIX + pilotId;
}

/** The Pilot id inside a phone conversation id, or null for any other id. */
export function pilotIdFromConversation(conversationId: string): string | null {
  return conversationId.startsWith(PILOT_CONVERSATION_PREFIX)
    ? conversationId.slice(PILOT_CONVERSATION_PREFIX.length)
    : null;
}

const OPEN_PILOT_KEY = "gespan:open-pilot";

/** The Pilot whose conversation was open, so the next launch reopens it. */
export function loadOpenPilot(): string | undefined {
  try {
    return localStorage.getItem(OPEN_PILOT_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}

export function saveOpenPilot(pilotId: string | undefined): void {
  try {
    if (pilotId) localStorage.setItem(OPEN_PILOT_KEY, pilotId);
    else localStorage.removeItem(OPEN_PILOT_KEY);
  } catch {
    // The next launch opens the workspace instead; harmless.
  }
}

function uniqueProjects(paths: readonly string[]): string[] {
  const seen = new Set<string>();
  return paths.filter((path) => {
    const key = projectKey(path);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

export function updatePilot(
  id: string,
  change: (pilot: Pilot) => Pilot,
): Pilot | undefined {
  const roster = listPilots();
  const index = roster.findIndex((pilot) => pilot.id === id);
  if (index < 0) return undefined;
  const next = change(roster[index]);
  if (!next.name?.trim()) delete next.name;
  if (!next.instructions?.trim()) delete next.instructions;
  next.projects = uniqueProjects(next.projects);
  roster[index] = next;
  saveRoster(roster);
  return next;
}

/** Forgets the Pilot and its background. Its folder of files stays on disk. */
export function removePilot(id: string): void {
  removePilotBackground(id);
  saveRoster(listPilots().filter((pilot) => pilot.id !== id));
}

/**
 * Puts the given Pilots in this order. Given only some, as a project's list
 * is, they swap among their own places and the rest stay where they are.
 */
export function reorderPilots(ids: readonly string[]): void {
  const roster = listPilots();
  const byId = new Map(roster.map((pilot) => [pilot.id, pilot]));
  const ordered = ids.flatMap((id) => byId.get(id) ?? []);
  const moved = new Set(ordered.map((pilot) => pilot.id));
  let next = 0;
  saveRoster(
    roster.map((pilot) => (moved.has(pilot.id) ? ordered[next++]! : pilot)),
  );
}

export function savePilotSessionId(pilotId: string, sessionId: string): void {
  updatePilot(pilotId, (pilot) => ({ ...pilot, sessionId }));
}

export function savePilotName(pilotId: string, name: string): void {
  updatePilot(pilotId, (pilot) => ({ ...pilot, name }));
}

export function savePilotMascot(pilotId: string, mascot: string): void {
  updatePilot(pilotId, (pilot) => ({ ...pilot, mascot }));
}

export function addPilotProject(pilotId: string, path: string): void {
  updatePilot(pilotId, (pilot) => ({
    ...pilot,
    projects: [...pilot.projects, path],
  }));
}

export function removePilotProject(pilotId: string, path: string): void {
  const key = projectKey(path);
  updatePilot(pilotId, (pilot) => ({
    ...pilot,
    projects: pilot.projects.filter((project) => projectKey(project) !== key),
  }));
}

/** Whether the Pilot has been given this project. */
export function pilotWorksOn(pilot: Pick<Pilot, "projects">, path: string) {
  const key = projectKey(path);
  return pilot.projects.some((project) => projectKey(project) === key);
}

export function subscribePilots(onChange: () => void): () => void {
  window.addEventListener(PILOTS_CHANGED, onChange);
  return () => window.removeEventListener(PILOTS_CHANGED, onChange);
}

/**
 * Snapshot for `useSyncExternalStore`; stable while storage is unchanged.
 * Project labels are part of it, since a Pilot's look names its projects.
 */
export function pilotsSnapshot(): string {
  migrateLegacyPilotStorage();
  try {
    return `${localStorage.getItem(ROSTER_KEY) ?? ""}|${JSON.stringify(
      loadTabGroupLabels(),
    )}`;
  } catch {
    return "";
  }
}

/** Mascots whose Pilot name is shorter than the mascot's. */
const PILOT_NAME: Record<string, string> = { mushroom: "Shroom" };

/** The name a Pilot goes by until the user names it: "PilotCrab", "PilotShroom". */
export function defaultPilotName(mascot: string): string {
  return `Pilot${PILOT_NAME[mascot] ?? mascot.charAt(0).toUpperCase() + mascot.slice(1)}`;
}

/** One project a Pilot works on, as the rail labels it. */
export type PilotProject = { path: string; name: string };

export type PilotLook = {
  /** The user's name for it, or its default: "PilotInvader", "PilotGhost". */
  name: string;
  mascot: string;
  color: string;
  projects: PilotProject[];
};

/** How a Pilot looks, and the projects it works on by their rail labels. */
export function pilotLook(pilot: Pilot): PilotLook {
  const labels = loadTabGroupLabels();
  return {
    name: pilot.name?.trim() || defaultPilotName(pilot.mascot),
    mascot: pilot.mascot,
    color: pilot.color,
    projects: pilot.projects.map((path) => ({
      path,
      name: resolveTabGroupLabel(projectKey(path), labels, projectName(path)),
    })),
  };
}

/** Its projects as a phrase: "app", "app and site", "app, site and api". */
export function pilotProjectsPhrase(projects: readonly PilotProject[]): string {
  const names = projects.map((project) => project.name);
  if (names.length <= 1) return names[0] ?? "";
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/** Plain one-line take on a step's title, for the Pilot's status. */
function plainLine(text: string): string {
  return text
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^\s*(?:#{1,6}|>)\s*/gm, "")
    .replace(/[`*_]/g, "")
    .replace(/^\s*[-\d.]+\s+/gm, "")
    .replace(/\s+/g, " ")
    .trim();
}

/** What the agent is doing right now, as the Pilots roster shows it. */
export type PilotStatus = "working" | "needs-you" | "idle";

export type PilotState = {
  status: PilotStatus;
  /** Short present-tense note while working or waiting. */
  activity?: string;
};

export const PILOT_STATUS_LABEL: Record<PilotStatus, string> = {
  working: "Working",
  "needs-you": "Needs you",
  idle: "Idle",
};

/** The agent's state, with the step it is on or the call it is waiting for. */
export function pilotState(
  session: Pick<
    Session,
    "blocks" | "busy" | "pendingQuestion" | "worktreeRemoved" | "usageLimit"
  >,
): PilotState {
  const { blocks, pendingQuestion: question } = session;
  if (!session.worktreeRemoved && question) {
    return {
      status: "needs-you",
      activity: question.title?.trim() || question.questions[0]?.prompt.trim(),
    };
  }
  if (!session.worktreeRemoved && hasPendingApproval(blocks)) {
    const approval = latest(
      blocks,
      (block) => !!block.approval && !block.approval.decided,
    );
    return {
      status: "needs-you",
      activity: approval?.tool?.title
        ? `Approve ${plainLine(approval.tool.title)}`
        : "Waiting for approval",
    };
  }
  if (!session.worktreeRemoved && session.usageLimit) {
    return { status: "needs-you", activity: "Usage limit reached" };
  }
  if (!session.busy) return { status: "idle" };
  const last = blocks[blocks.length - 1];
  if (last?.role === "assistant" && !last.tool && last.streaming) {
    return { status: "working", activity: "Writing a reply" };
  }
  // Only a step from the current turn says what it is doing now.
  for (let i = blocks.length - 1; i >= 0; i--) {
    const block = blocks[i];
    if (block.role === "user") break;
    if (block.tool?.title) {
      return { status: "working", activity: plainLine(block.tool.title) };
    }
  }
  return { status: "working", activity: "Thinking" };
}

function latest(
  blocks: readonly Block[],
  match: (block: Block) => boolean,
): Block | undefined {
  for (let i = blocks.length - 1; i >= 0; i--) {
    if (match(blocks[i])) return blocks[i];
  }
  return undefined;
}
