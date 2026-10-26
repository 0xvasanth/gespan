import { useState, type ReactNode } from "react";
import { Check, ListFilter, X } from "../../../shared/ui/icons";
import type { FilterRow, SortSpec } from "../model/board";
import {
  FILTER_FIELDS,
  SORT_KEYS,
  chipText,
  customFieldKey,
  fieldChoices,
  fieldTitle,
  sortLabel,
  type FilterSources,
} from "../model/views";
import {
  MenuButton,
  MenuItem,
  MenuLabel,
  MenuSeparator,
} from "./boardControls";

const TRIGGER =
  "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-content/15 px-2 text-[12px] text-content/80 hover:border-content/30 hover:bg-content/10 hover:text-content";

const SMALL =
  "h-6 shrink-0 rounded-md px-1.5 text-[12px] text-content/70 hover:bg-content/10 hover:text-content";

function toggled(values: readonly string[], value: string): string[] {
  return values.includes(value)
    ? values.filter((held) => held !== value)
    : [...values, value];
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
    <MenuItem kind="checkbox" checked={checked} onSelect={onSelect}>
      <span className="grid size-3.5 shrink-0 place-items-center">
        {checked ? <Check className="size-3.5" strokeWidth={2} /> : null}
      </span>
      <span className="min-w-0 truncate">{children}</span>
    </MenuItem>
  );
}

/** "Where" on the first row, "and" on the rest. */
export const leadLabel = (row: number) => (row === 0 ? "Where" : "and");

function FilterBuilder({
  rows,
  sources,
  onChange,
}: {
  rows: readonly FilterRow[];
  sources: FilterSources;
  onChange: (rows: FilterRow[]) => void;
}) {
  // The rows whose values are listed: one just added, or picked.
  const [open, setOpen] = useState<ReadonlySet<number>>(new Set());
  const edit = (index: number, patch: Partial<FilterRow>) =>
    onChange(
      rows.map((row, at) => (at === index ? { ...row, ...patch } : row)),
    );
  const add = (field: string) => {
    onChange([...rows, { field, op: "is", values: [] }]);
    setOpen((held) => new Set(held).add(rows.length));
  };
  return (
    <div className="max-h-[60vh] overflow-y-auto">
      {rows.map((row, index) => {
        const choices = fieldChoices(row.field, sources, row.values);
        const expanded = open.has(index);
        return (
          <div key={index} data-filter-row={index}>
            <div className="flex items-center gap-1 px-1">
              <span
                data-filter-lead=""
                className="w-9 shrink-0 text-[11px] text-content/45"
              >
                {leadLabel(index)}
              </span>
              <button
                type="button"
                aria-label={`Pick ${fieldTitle(row.field, sources)} values`}
                aria-expanded={expanded}
                className={`${SMALL} min-w-0 flex-1 truncate text-left`}
                onClick={() =>
                  setOpen((held) => {
                    const next = new Set(held);
                    if (!next.delete(index)) next.add(index);
                    return next;
                  })
                }
              >
                {fieldTitle(row.field, sources)}
              </button>
              <button
                type="button"
                aria-label="Match or exclude"
                className={SMALL}
                onClick={() =>
                  edit(index, { op: row.op === "is" ? "is_not" : "is" })
                }
              >
                {row.op === "is" ? "is" : "is not"}
              </button>
              <button
                type="button"
                aria-label="Remove filter"
                className={SMALL}
                onClick={() => {
                  onChange(rows.filter((_, at) => at !== index));
                  setOpen(new Set());
                }}
              >
                <X className="size-3" strokeWidth={2} />
              </button>
            </div>
            {expanded ? (
              choices.length ? (
                choices.map(([value, label]) => (
                  <Option
                    key={value}
                    checked={row.values.includes(value)}
                    onSelect={() =>
                      edit(index, { values: toggled(row.values, value) })
                    }
                  >
                    {label}
                  </Option>
                ))
              ) : (
                <div className="px-2 py-1.5 text-[12px] text-content/45">
                  Nothing to pick
                </div>
              )
            ) : null}
          </div>
        );
      })}
      {rows.length ? <MenuSeparator /> : null}
      <MenuLabel>Add filter</MenuLabel>
      {FILTER_FIELDS.map(([field, label]) => (
        <MenuItem key={field} onSelect={() => add(field)}>
          {label}
        </MenuItem>
      ))}
      {sources.fields.map((field) => (
        <MenuItem key={field.id} onSelect={() => add(customFieldKey(field.id))}>
          {field.name}
        </MenuItem>
      ))}
      <MenuSeparator />
      <MenuItem
        disabled={rows.length === 0}
        onSelect={() => {
          onChange([]);
          setOpen(new Set());
        }}
      >
        Clear all
      </MenuItem>
    </div>
  );
}

/** The header's Filter menu: the builder, with a count badge. */
export function BoardFilterMenu({
  rows,
  count,
  sources,
  onChange,
}: {
  rows: readonly FilterRow[];
  /** Rows that filter something. */
  count: number;
  sources: FilterSources;
  onChange: (rows: FilterRow[]) => void;
}) {
  return (
    <MenuButton
      label="Filter"
      width={280}
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
        <FilterBuilder rows={rows} sources={sources} onChange={onChange} />
      )}
    </MenuButton>
  );
}

/** The header's Sort menu: a key and, for any but Manual, a direction. */
export function BoardSortMenu({
  sort,
  onChange,
}: {
  sort: SortSpec;
  onChange: (sort: SortSpec) => void;
}) {
  return (
    <MenuButton
      label="Sort"
      width={200}
      className={TRIGGER}
      trigger={<>Sort</>}
    >
      {() => (
        <>
          {SORT_KEYS.map((key) => (
            <MenuItem
              key={key}
              checked={sort.key === key}
              onSelect={() => onChange({ key, dir: sort.dir })}
            >
              {sortLabel(key)}
            </MenuItem>
          ))}
          <MenuSeparator />
          {(["asc", "desc"] as const).map((dir) => (
            <MenuItem
              key={dir}
              disabled={sort.key === "manual"}
              checked={sort.key !== "manual" && sort.dir === dir}
              onSelect={() => onChange({ key: sort.key, dir })}
            >
              {dir === "asc" ? "Ascending" : "Descending"}
            </MenuItem>
          ))}
        </>
      )}
    </MenuButton>
  );
}

/** One chip per filter row, with Clear all. */
export function FilterChips({
  rows,
  sources,
  onRemove,
  onClear,
}: {
  rows: readonly FilterRow[];
  sources: FilterSources;
  onRemove: (index: number) => void;
  onClear: () => void;
}) {
  if (!rows.length) return null;
  return (
    <div
      data-filter-chips=""
      className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-stroke px-4 py-1.5"
    >
      {rows.map((row, index) => (
        <span
          key={index}
          data-filter-chip={index}
          className="inline-flex h-[22px] items-center gap-1 rounded-full bg-accent/12 pl-2 pr-1 text-[11px] text-content/85"
        >
          {chipText(row, sources)}
          <button
            type="button"
            aria-label="Remove filter"
            className="grid size-3.5 place-items-center rounded-full hover:bg-content/12"
            onClick={() => onRemove(index)}
          >
            <X className="size-2.5" strokeWidth={2} />
          </button>
        </span>
      ))}
      <button
        type="button"
        aria-label="Clear all"
        className="px-1.5 text-[11px] text-content/55 hover:text-content/90"
        onClick={onClear}
      >
        Clear all
      </button>
    </div>
  );
}
