import type {
  AgentField,
  BoardField,
  DisplayOptions,
} from "./board";

export const FACE_TOGGLES = [
  ["labels", "Labels"],
  ["priority", "Priority"],
  ["due", "Due date"],
  ["subtasks", "Subtasks"],
  ["comments", "Comments"],
  ["linkStatus", "Linked work"],
  ["number", "Card number"],
] as const;
export type FaceToggle = (typeof FACE_TOGGLES)[number][0];

export const AGENT_FIELDS: readonly (readonly [AgentField, string])[] = [
  ["provider", "Provider"],
  ["model", "Model"],
  ["cost", "Cost"],
  ["runTime", "Run time"],
];

export const DEFAULT_DISPLAY: DisplayOptions = {
  face: {
    labels: true,
    priority: true,
    due: true,
    subtasks: true,
    comments: true,
    linkStatus: true,
    number: true,
    fields: [],
    agentFields: [],
  },
  density: "comfortable",
  swimlane: "none",
};

const isObject = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === "object" && !Array.isArray(value);

const validSwimlane = (value: unknown): value is string =>
  value === "none" ||
  value === "priority" ||
  value === "label" ||
  value === "project" ||
  (typeof value === "string" && /^field:.+/.test(value));

/** The parts of a display object that read as valid; the rest is left out. */
function validParts(raw: unknown): {
  face: Partial<DisplayOptions["face"]>;
  density?: DisplayOptions["density"];
  swimlane?: string;
} {
  if (!isObject(raw)) return { face: {} };
  const face: Partial<DisplayOptions["face"]> = {};
  if (isObject(raw.face)) {
    for (const [key] of FACE_TOGGLES) {
      if (typeof raw.face[key] === "boolean") face[key] = raw.face[key];
    }
    if (Array.isArray(raw.face.fields)) {
      face.fields = [
        ...new Set(
          raw.face.fields.filter((id): id is string => typeof id === "string"),
        ),
      ];
    }
    if (Array.isArray(raw.face.agentFields)) {
      face.agentFields = [
        ...new Set(
          raw.face.agentFields.filter((id): id is AgentField =>
            AGENT_FIELDS.some(([known]) => known === id),
          ),
        ),
      ];
    }
  }
  return {
    face,
    density:
      raw.density === "compact" || raw.density === "comfortable"
        ? raw.density
        : undefined,
    swimlane: validSwimlane(raw.swimlane) ? raw.swimlane : undefined,
  };
}

function overlay(base: DisplayOptions, raw: unknown): DisplayOptions {
  const parts = validParts(raw);
  return {
    face: { ...base.face, ...parts.face },
    density: parts.density ?? base.density,
    swimlane: parts.swimlane ?? base.swimlane,
  };
}

function parseJson(json: string | null): unknown {
  if (!json) return null;
  try {
    return JSON.parse(json);
  } catch {
    return null;
  }
}

/** A board's stored display options; null, bad JSON and bad parts mean the defaults. */
export function parseDisplay(json: string | null): DisplayOptions {
  return overlay(DEFAULT_DISPLAY, parseJson(json));
}

export function serializeDisplay(display: DisplayOptions): string {
  return JSON.stringify(display);
}

/** The display a saved view carries in its layout, if any. */
export function layoutDisplay(layoutJson: string): unknown {
  const raw = parseJson(layoutJson);
  return isObject(raw) && isObject(raw.display) ? raw.display : null;
}

/**
 * What a board draws with: the board default, with a saved view's override
 * (`layout_json.display`) laid over it. "All cards" passes `null` and gets the
 * board default.
 */
export function resolveDisplay(
  boardJson: string | null,
  viewLayoutJson: string | null,
): DisplayOptions {
  const base = parseDisplay(boardJson);
  return viewLayoutJson === null
    ? base
    : overlay(base, layoutDisplay(viewLayoutJson));
}

/** `layoutJson` with its display override set, or removed with `null`; other keys stay. */
export function withLayoutDisplay(
  layoutJson: string,
  display: DisplayOptions | null,
): string {
  const raw = parseJson(layoutJson);
  const next: Record<string, unknown> = isObject(raw) ? { ...raw } : {};
  if (display) next.display = display;
  else delete next.display;
  if (!next.layout) next.layout = "board";
  return JSON.stringify(next);
}

export type LaneBy =
  | { kind: "priority" | "label" | "project" }
  | { kind: "field"; field: BoardField };

/**
 * The lanes a display asks for, or null for none. A lane on a field that is
 * gone, or isn't a select, is ignored: the board draws as it does without lanes.
 */
export function laneBy(
  display: DisplayOptions,
  fields: readonly BoardField[],
): LaneBy | null {
  const { swimlane } = display;
  if (swimlane === "priority" || swimlane === "label" || swimlane === "project") {
    return { kind: swimlane };
  }
  if (swimlane.startsWith("field:")) {
    const field = fields.find((found) => `field:${found.id}` === swimlane);
    return field && field.kind === "select" ? { kind: "field", field } : null;
  }
  return null;
}
