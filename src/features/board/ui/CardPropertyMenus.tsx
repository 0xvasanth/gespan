import { useEffect, useState } from "react";
import { PRIORITIES } from "../model/board";
import { dayStart, dueInputValue, parseDueInput } from "./boardFormat";
import { MenuItem, MenuSeparator } from "./boardControls";

/** The priority choices of a Priority menu, current one checked. */
export function PriorityItems({
  priority,
  onChoose,
  close,
}: {
  priority: number;
  onChoose: (priority: number) => void;
  close: () => void;
}) {
  return (
    <>
      {PRIORITIES.map((name, index) => (
        <MenuItem
          key={name}
          checked={index === priority}
          onSelect={() => {
            close();
            onChoose(index);
          }}
        >
          {name}
        </MenuItem>
      ))}
    </>
  );
}

/** The due-date presets of a Due menu, then Clear. */
export function DueItems({
  dueAt,
  onChoose,
  close,
}: {
  dueAt: number | null;
  onChoose: (dueAt: number | null) => void;
  close: () => void;
}) {
  return (
    <>
      {(
        [
          ["Today", 0],
          ["Tomorrow", 1],
          ["Next week", 7],
        ] as const
      ).map(([name, days]) => (
        <MenuItem
          key={name}
          onSelect={() => {
            close();
            onChoose(dayStart(days));
          }}
        >
          {name}
        </MenuItem>
      ))}
      <MenuSeparator />
      <MenuItem
        disabled={dueAt === null}
        onSelect={() => {
          close();
          onChoose(null);
        }}
      >
        Clear
      </MenuItem>
    </>
  );
}

/** A date field for a due date: a half-typed date waits, an emptied one clears. */
export function DueDateInput({
  dueAt,
  onChoose,
  className,
}: {
  dueAt: number | null;
  onChoose: (dueAt: number | null) => void;
  className?: string;
}) {
  const [value, setValue] = useState(dueInputValue(dueAt));
  useEffect(() => setValue(dueInputValue(dueAt)), [dueAt]);
  return (
    <input
      type="date"
      aria-label="Due date"
      value={value}
      onChange={(event) => {
        setValue(event.target.value);
        const next = parseDueInput(event.target.value);
        if (next !== null) onChoose(next);
        else if (event.target.value === "") onChoose(null);
      }}
      className={
        className ??
        "h-7 rounded-md border border-stroke bg-background-base px-2 text-[12px] text-content/80 outline-none focus:border-accent/60"
      }
    />
  );
}
