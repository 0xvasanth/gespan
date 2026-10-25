import type { ReactNode } from "react";
import { Check, ListFilter } from "../../../shared/ui/icons";
import { PRIORITIES, type BoardLabel } from "../model/board";
import { filterCount, type BoardFilter } from "../model/boardFilter";
import {
  MenuButton,
  MenuItem,
  MenuLabel,
  MenuSeparator,
} from "./boardControls";

const TRIGGER =
  "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-content/15 px-2 text-[12px] text-content/80 hover:border-content/30 hover:bg-content/10 hover:text-content";

function toggled<T>(set: ReadonlySet<T>, value: T): Set<T> {
  const next = new Set(set);
  if (!next.delete(value)) next.add(value);
  return next;
}

function Option({
  checked,
  onSelect,
  children,
}: {
  checked: boolean;
  onSelect: () => void;
  children: ReactNode;
}) {
  return (
    <MenuItem checked={checked} onSelect={onSelect}>
      <span className="grid size-3.5 shrink-0 place-items-center">
        {checked ? <Check className="size-3.5" strokeWidth={2} /> : null}
      </span>
      <span className="min-w-0 truncate">{children}</span>
    </MenuItem>
  );
}

/** The header's Filter menu: projects, labels and priorities, with a count badge. */
export function BoardFilterMenu({
  filter,
  labels,
  projects,
  onChange,
}: {
  filter: BoardFilter;
  labels: readonly BoardLabel[];
  projects: readonly { path: string; name: string }[];
  onChange: (filter: BoardFilter) => void;
}) {
  const count = filterCount(filter);
  return (
    <MenuButton
      label="Filter"
      width={240}
      className={TRIGGER}
      trigger={
        <>
          <ListFilter className="size-3.5" strokeWidth={1.75} />
          Filter
          {count ? (
            <span
              data-filter-count=""
              className="rounded-full bg-accent/20 px-1.5 text-[11px] tabular-nums text-content"
            >
              {count}
            </span>
          ) : null}
        </>
      }
    >
      {() => (
        <div className="max-h-[60vh] overflow-y-auto">
          {projects.length ? (
            <>
              <MenuLabel>Projects</MenuLabel>
              {projects.map((project) => (
                <Option
                  key={project.path}
                  checked={filter.projects.has(project.path)}
                  onSelect={() =>
                    onChange({
                      ...filter,
                      projects: toggled(filter.projects, project.path),
                    })
                  }
                >
                  {project.name}
                </Option>
              ))}
              <MenuSeparator />
            </>
          ) : null}
          {labels.length ? (
            <>
              <MenuLabel>Labels</MenuLabel>
              {labels.map((label) => (
                <Option
                  key={label.id}
                  checked={filter.labels.has(label.id)}
                  onSelect={() =>
                    onChange({
                      ...filter,
                      labels: toggled(filter.labels, label.id),
                    })
                  }
                >
                  {label.name}
                </Option>
              ))}
              <MenuSeparator />
            </>
          ) : null}
          <MenuLabel>Priority</MenuLabel>
          {PRIORITIES.map((name, priority) => (
            <Option
              key={name}
              checked={filter.priorities.has(priority)}
              onSelect={() =>
                onChange({
                  ...filter,
                  priorities: toggled(filter.priorities, priority),
                })
              }
            >
              {name}
            </Option>
          ))}
          <MenuSeparator />
          <MenuItem
            disabled={count === 0}
            onSelect={() =>
              onChange({
                projects: new Set(),
                labels: new Set(),
                priorities: new Set(),
              })
            }
          >
            Clear
          </MenuItem>
        </div>
      )}
    </MenuButton>
  );
}
