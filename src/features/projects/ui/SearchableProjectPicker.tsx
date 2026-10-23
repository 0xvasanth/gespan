import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useTabGroupLogos } from "../hooks/useTabGroupLogos";
import { useProjectAppearance } from "../hooks/useProjectAppearance";
import { basename } from "../../../platform/tauri/fs";
import {
  prettyParent,
  projectKey,
  projectName,
} from "../../../shared/lib/paths";
import {
  looksLikeProject,
  projectRailItems,
  sameProjectPath,
  type RecentProject,
} from "../model/recents";
import {
  resolveTabGroupColor,
  resolveTabGroupLabel,
  resolveTabGroupLogo,
  resolveTabGroupMascot,
} from "../../workspace/model/tabGroups";
import { Check, ChevronDown, Plus, Search } from "../../../shared/ui/icons";
import { Popover, type PopoverAnchor } from "../../../shared/ui/Popover";
import { ProjectLogoIcon } from "./ProjectLogoIcon";
import { ProjectMascot } from "./ProjectMascot";
import { PixelMascot } from "./PixelMascot";
import { PilotRailMascot } from "../../pilots/ui/PilotRailMascot";
import { PILOT_STATUS_LABEL, type PilotStatus } from "../../pilots/model/pilot";

/** A Pilot the picker offers beside the projects. */
export type PickerPilot = {
  id: string;
  name: string;
  mascot: string;
  color: string;
  status?: PilotStatus;
};

/** Pilots listed above the projects; picking one opens it. */
export type PickerPilots = {
  items: readonly PickerPilot[];
  /** The Pilot open in the main area; then no project is the current one. */
  activeId?: string;
  onOpen: (pilotId: string) => void;
  onCreate?: () => void;
};

type Props = {
  cwd: string;
  recents: RecentProject[];
  /** Project whose rail supplies the choices when it differs from `cwd`. */
  railCwd?: string;
  busy?: boolean;
  mode?: "switch" | "move";
  className?: string;
  buttonClassName?: string;
  appearance?: "ghost" | "filled";
  compact?: boolean;
  onSelectProject: (path: string) => void;
  onOpenProject?: () => void;
  /** Opens the project's context menu; the search input receives focus back. */
  onProjectContextMenu?: (
    path: string,
    x: number,
    y: number,
    trigger: HTMLElement | null,
  ) => void;
  /** Keeps the dropdown open while the project's context menu or its dialogs show. */
  projectMenuActive?: boolean;
  /** Lists Pilots first, for the compact rail where they have no row of their own. */
  pilots?: PickerPilots;
};

export function SearchableProjectPicker({
  cwd,
  recents,
  railCwd,
  busy = false,
  mode = "switch",
  className,
  buttonClassName,
  appearance = "ghost",
  compact = false,
  onSelectProject,
  onOpenProject,
  onProjectContextMenu,
  projectMenuActive = false,
  pilots,
}: Props) {
  const [open, setOpen] = useState(false);
  const pickerRef = useRef<HTMLDivElement>(null);
  const { groupLabels, groupColors, groupCustomColors, groupMascots } =
    useProjectAppearance();
  const groupLogos = useTabGroupLogos();
  const inProject = looksLikeProject(cwd);
  const seed = projectName(cwd);
  const key = projectKey(cwd);
  const label = inProject
    ? resolveTabGroupLabel(key, groupLabels, basename(cwd) || seed)
    : "Choose project";
  const logoPath = resolveTabGroupLogo(key, groupLogos);
  const color = resolveTabGroupColor(key, groupColors, groupCustomColors, seed);
  const orderedProjects = pickerProjects(recents, cwd, railCwd);
  const activePilot = pilots?.items.find((pilot) => pilot.id === pilots.activeId);
  const closePicker = () => setOpen(false);
  const openPicker = () => setOpen(true);

  const action = mode === "move" ? "Move note to project" : "Switch project";

  return (
    <div
      ref={pickerRef}
      className={`relative flex h-full min-w-0 items-center${
        className ? ` ${className}` : ""
      }`}
    >
      <button
        type="button"
        title={
          compact
            ? undefined
            : activePilot
              ? `${activePilot.name}\n${PILOT_STATUS_LABEL[activePilot.status ?? "idle"]}`
              : inProject
                ? cwd
                : undefined
        }
        // The compact rail draws its own instant tooltip from these.
        data-rail-tip={
          compact
            ? (activePilot?.name ?? (inProject ? label : action))
            : undefined
        }
        data-rail-tip-detail={
          compact && activePilot
            ? PILOT_STATUS_LABEL[activePilot.status ?? "idle"]
            : undefined
        }
        aria-label={
          activePilot
            ? `${action}, current pilot ${activePilot.name}, ${PILOT_STATUS_LABEL[activePilot.status ?? "idle"]}`
            : inProject
              ? `${action}, current project ${label}`
              : "Choose project for note"
        }
        aria-expanded={open}
        aria-haspopup="dialog"
        data-tauri-drag-region="false"
        onClick={() => (open ? closePicker() : openPicker())}
        onKeyDown={(event) => {
          if (open) return;
          if (event.key !== "ArrowDown") return;
          event.preventDefault();
          openPicker();
        }}
        className={`flex min-w-0 items-center rounded-md text-[12px] leading-none ${
          compact ? "size-8 justify-center p-0" : "h-6.5 gap-1.5 px-2"
        } ${
          open
            ? "bg-selection text-content"
            : appearance === "filled"
              ? "bg-content/10 text-content hover:bg-content/[0.14]"
              : "text-content/50 hover:bg-content/5 hover:text-content"
        }${buttonClassName ? ` ${buttonClassName}` : ""}`}
      >
        {activePilot ? (
          <PilotRailMascot
            name={activePilot.mascot}
            color={activePilot.color}
            status={activePilot.status}
            className={`${compact ? "size-4" : "size-3.5"} shrink-0`}
          />
        ) : !inProject ? null : logoPath ? (
          <ProjectLogoIcon
            path={logoPath}
            className={`${compact ? "size-4" : "size-3.5"} shrink-0 rounded-sm`}
            imageClassName={compact ? "size-4" : "size-3.5"}
          />
        ) : (
          <ProjectMascot
            project={seed}
            color={color}
            name={resolveTabGroupMascot(key, groupMascots)}
            className={`${compact ? "size-3.5" : "size-3"} shrink-0`}
            active={busy}
          />
        )}
        {compact ? null : (
          <>
            <span className="min-w-0 truncate font-medium text-content/90">
              {activePilot?.name ?? label}
            </span>
            <ChevronDown
              className={`size-3 shrink-0 text-content/45 transition-transform ${
                open ? "rotate-180" : ""
              }`}
              strokeWidth={1.75}
            />
          </>
        )}
      </button>
      {open ? (
        <ProjectPickerPopover
          anchor={pickerRef}
          projects={orderedProjects}
          currentProject={cwd}
          onDismiss={closePicker}
          onSelectProject={onSelectProject}
          onOpenProject={onOpenProject}
          onProjectContextMenu={onProjectContextMenu}
          projectMenuActive={projectMenuActive}
          pilots={pilots}
        />
      ) : null}
    </div>
  );
}

/** The rail's projects with the current one first, even when it is unlisted. */
export function pickerProjects(
  recents: RecentProject[],
  cwd: string,
  railCwd?: string,
): RecentProject[] {
  const railProjects = projectRailItems(recents, railCwd ?? cwd);
  const projects =
    looksLikeProject(cwd) &&
    !railProjects.some((item) => sameProjectPath(item.path, cwd))
      ? [{ path: cwd, openedAt: 0 }, ...railProjects]
      : railProjects;
  return [
    ...projects.filter((item) => sameProjectPath(item.path, cwd)),
    ...projects.filter((item) => !sameProjectPath(item.path, cwd)),
  ];
}

type ProjectPickerPopoverProps = Pick<
  Props,
  | "onSelectProject"
  | "onOpenProject"
  | "onProjectContextMenu"
  | "projectMenuActive"
  | "pilots"
> & {
  anchor: PopoverAnchor;
  projects: readonly RecentProject[];
  currentProject?: string;
  onDismiss: () => void;
  label?: string;
  emptyMessage?: string;
  /** A flyout off another menu opens beside it, and that menu owns outside
   * clicks for both. */
  flyout?: { layer: number; attributes?: Record<`data-${string}`, boolean> };
};

/** Shared searchable project menu for switching projects or adding them to a Pilot. */
export function ProjectPickerPopover({
  anchor,
  projects,
  currentProject,
  onDismiss,
  onSelectProject,
  onOpenProject,
  onProjectContextMenu,
  projectMenuActive = false,
  pilots,
  label = "Project picker",
  emptyMessage = "No projects found",
  flyout,
}: ProjectPickerPopoverProps) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const { groupLabels, groupColors, groupCustomColors, groupMascots } =
    useProjectAppearance();
  const groupLogos = useTabGroupLogos();
  const activePilot = pilots?.items.find((pilot) => pilot.id === pilots.activeId);
  // With a Pilot open, no project is current: each one is a way back.
  const isCurrent = (path: string) =>
    !activePilot &&
    currentProject != null &&
    sameProjectPath(path, currentProject);
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const filteredPilots = (pilots?.items ?? []).filter((pilot) =>
    pilot.name.toLocaleLowerCase().includes(normalizedQuery),
  );
  const filteredProjects = normalizedQuery
    ? projects.filter((item) => {
        const itemKey = projectKey(item.path);
        const itemLabel = resolveTabGroupLabel(
          itemKey,
          groupLabels,
          basename(item.path) || projectName(item.path),
        );
        return `${itemLabel}\n${item.path}`
          .toLocaleLowerCase()
          .includes(normalizedQuery);
      })
    : projects;

  useEffect(() => {
    searchRef.current?.focus();
    const frame = window.requestAnimationFrame(() => {
      searchRef.current?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, []);

  const pickProject = (path: string) => {
    onDismiss();
    if (!isCurrent(path)) onSelectProject(path);
  };

  const pickPilot = (pilotId: string) => {
    onDismiss();
    pilots?.onOpen(pilotId);
  };

  /** Keyboard rows: the Pilots, then the projects. */
  const rowCount = filteredPilots.length + filteredProjects.length;
  const pickRow = (index: number) => {
    if (index < filteredPilots.length) pickPilot(filteredPilots[index].id);
    else {
      const project = filteredProjects[index - filteredPilots.length];
      if (project) pickProject(project.path);
    }
  };

  const onPickerKeyDown = (event: ReactKeyboardEvent<HTMLElement>) => {
    const target = event.target;
    const rows = listRef.current?.children;
    if (
      onProjectContextMenu &&
      rows &&
      (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10"))
    ) {
      // A Tab-focused row opens its own menu; the search input opens the highlighted one.
      const index =
        target instanceof HTMLInputElement
          ? active
          : Array.prototype.indexOf.call(rows, target);
      const project = filteredProjects[index - filteredPilots.length];
      const row = rows[index];
      if (!project || !row) return;
      event.preventDefault();
      const rect = row.getBoundingClientRect();
      onProjectContextMenu(
        project.path,
        rect.left,
        rect.bottom,
        searchRef.current,
      );
      return;
    }
    if (!(target instanceof HTMLInputElement)) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      if (rowCount === 0) return;
      setActive((index) => Math.min(rowCount - 1, index + 1));
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive((index) => Math.max(0, index - 1));
      return;
    }
    if (event.key === "Enter") {
      if (active >= rowCount) return;
      event.preventDefault();
      pickRow(active);
    }
  };

  return (
    <Popover
      anchor={anchor}
      side={flyout ? "right" : "bottom"}
      align="start"
      gap={flyout ? -4 : 4}
      width={286}
      maxHeight={380}
      layer={flyout?.layer}
      {...flyout?.attributes}
      role="dialog"
      aria-label={label}
      onDismiss={projectMenuActive || flyout ? undefined : onDismiss}
      onKeyDown={onPickerKeyDown}
      className="flex flex-col overflow-hidden"
    >
      <label className="flex h-11 shrink-0 items-center gap-2.5 border-b border-stroke px-3 text-content/45 focus-within:text-content/70">
        <Search className="size-4 shrink-0" strokeWidth={1.75} />
        <span className="sr-only">Search projects</span>
        <input
          ref={searchRef}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setActive(0);
          }}
          placeholder={
            pilots?.items.length
              ? "Search pilots and projects..."
              : "Search projects..."
          }
          className="min-w-0 flex-1 bg-transparent text-[13px] text-content outline-none placeholder:text-content/35"
        />
      </label>
      <div
        ref={listRef}
        className="min-h-0 flex-1 overflow-y-auto overscroll-none p-1.5"
      >
        {filteredPilots.map((pilot, index) => (
          <button
            key={`pilot:${pilot.id}`}
            type="button"
            title={pilot.name}
            data-picker-pilot={pilot.id}
            onMouseEnter={() => setActive(index)}
            onClick={() => pickPilot(pilot.id)}
            className={`flex h-9 w-full items-center gap-2.5 rounded-lg px-2.5 text-left ${
              active === index
                ? "bg-selection text-content"
                : "text-content/75 hover:bg-content/5 hover:text-content"
            }`}
          >
            <span className="grid size-4 shrink-0 place-items-center">
              {pilot.id === pilots?.activeId ? (
                <Check className="size-3.5" strokeWidth={2} />
              ) : (
                <PixelMascot
                  name={pilot.mascot}
                  color={pilot.color}
                  still
                  className="size-4"
                />
              )}
            </span>
            <span className="min-w-0 flex-1 truncate text-[13px] font-medium">
              {pilot.name}
            </span>
            <span className="shrink-0 text-[11px] text-content/40">Pilot</span>
          </button>
        ))}
        {filteredProjects.length > 0 ? (
          filteredProjects.map((item, projectIndex) => {
            const index = filteredPilots.length + projectIndex;
            const current = isCurrent(item.path);
            const itemKey = projectKey(item.path);
            const itemSeed = projectName(item.path);
            const itemLabel = resolveTabGroupLabel(
              itemKey,
              groupLabels,
              basename(item.path) || itemSeed,
            );
            const itemLogo = resolveTabGroupLogo(itemKey, groupLogos);
            const itemColor = resolveTabGroupColor(
              itemKey,
              groupColors,
              groupCustomColors,
              itemSeed,
            );
            return (
              <button
                key={item.path}
                type="button"
                title={item.path}
                onMouseEnter={() => setActive(index)}
                onClick={() => pickProject(item.path)}
                onContextMenu={
                  onProjectContextMenu
                    ? (event) => {
                        event.preventDefault();
                        event.stopPropagation();
                        onProjectContextMenu(
                          item.path,
                          event.clientX,
                          event.clientY,
                          searchRef.current,
                        );
                      }
                    : undefined
                }
                className={`flex h-9 w-full items-center gap-2.5 rounded-lg px-2.5 text-left ${
                  active === index
                    ? "bg-selection text-content"
                    : "text-content/75 hover:bg-content/5 hover:text-content"
                }`}
              >
                <span className="grid size-4 shrink-0 place-items-center">
                  {current ? (
                    <Check className="size-3.5" strokeWidth={2} />
                  ) : itemLogo ? (
                    <ProjectLogoIcon
                      path={itemLogo}
                      className="size-4 rounded-sm"
                      imageClassName="size-4"
                    />
                  ) : (
                    <ProjectMascot
                      project={itemSeed}
                      color={itemColor}
                      name={resolveTabGroupMascot(itemKey, groupMascots)}
                      className="size-3.5"
                    />
                  )}
                </span>
                <span className="min-w-0 max-w-[calc(100%_-_36px)] shrink-0 truncate text-[13px] font-medium">
                  {itemLabel}
                </span>
                <span className="min-w-0 max-w-28 flex-1 truncate font-mono text-[11px] text-content/40">
                  {prettyParent(item.path)}
                </span>
              </button>
            );
          })
        ) : filteredPilots.length ? null : (
          <p className="px-2.5 py-5 text-center text-[12px] text-content/45">
            {emptyMessage}
          </p>
        )}
      </div>
      {onOpenProject || pilots?.onCreate ? (
        <div className="shrink-0 border-t border-stroke p-1.5">
          {pilots?.onCreate ? (
            <button
              type="button"
              onClick={() => {
                onDismiss();
                pilots.onCreate?.();
              }}
              className="flex h-9 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] text-content/75 hover:bg-content/8 hover:text-content"
            >
              <Plus className="size-4 shrink-0" strokeWidth={1.75} />
              <span>New pilot</span>
            </button>
          ) : null}
          {onOpenProject ? (
            <button
              type="button"
              onClick={() => {
                onDismiss();
                onOpenProject();
              }}
              className="flex h-9 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] text-content/75 hover:bg-content/8 hover:text-content"
            >
              <Plus className="size-4 shrink-0" strokeWidth={1.75} />
              <span>New project</span>
            </button>
          ) : null}
        </div>
      ) : null}
    </Popover>
  );
}
