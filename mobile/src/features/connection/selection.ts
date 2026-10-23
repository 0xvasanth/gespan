export type MobileSelection = { projectIds: string[]; pilotIds: string[] };

/**
 * Whether pilots can be picked and shown. Off for the first version, as the
 * phone has no good way to talk to them yet; pilots picked before are kept in
 * the selection, so they come back when this is turned on.
 */
export const PILOTS_ENABLED = false;
export function normalizeSelection(value: unknown): MobileSelection | null {
  if (!value || typeof value !== "object") return null;
  const item = value as Record<string, unknown>;
  if (!Array.isArray(item.projectIds) || !Array.isArray(item.pilotIds))
    return null;
  const ids = (values: unknown[]) => [
    ...new Set(
      values.filter((value): value is string => typeof value === "string"),
    ),
  ];
  return { projectIds: ids(item.projectIds), pilotIds: ids(item.pilotIds) };
}
export function selectedWorkspace<
  T extends {
    projects: { id: string }[];
    pilots: { id: string }[];
    sessions: { projectId: string; pilotId?: string }[];
  },
>(catalog: T, selection: MobileSelection | null) {
  const projects = new Set(selection?.projectIds ?? []);
  const pilots = new Set(PILOTS_ENABLED ? (selection?.pilotIds ?? []) : []);
  return {
    ...catalog,
    projects: catalog.projects.filter((project) => projects.has(project.id)),
    pilots: catalog.pilots.filter((pilot) => pilots.has(pilot.id)),
    sessions: catalog.sessions.filter(
      (session) =>
        projects.has(session.projectId) ||
        (!!session.pilotId && pilots.has(session.pilotId)),
    ),
  };
}
