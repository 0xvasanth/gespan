import { useEffect, useState } from "react";
import { offeredPriorities, priorityName } from "../model/boardSettings";
import { DATE_PICKS, datePickValue } from "../model/cardProps";
import { dueInputValue, parseDueInput } from "./boardFormat";
import { MenuItem, MenuSeparator } from "./boardControls";
import { usePriorityScheme } from "./priorityScheme";

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
  const scheme = usePriorityScheme();
  // Hidden levels aren't offered, though a card that holds one still shows it.
  return (
    <>
      {offeredPriorities(scheme).map((level) => (
        <MenuItem
          key={level}
          checked={level === priority}
          onSelect={() => {
            close();
            onChoose(level);
          }}
        >
          {priorityName(scheme, level)}
        </MenuItem>
      ))}
    </>
  );
}

/** The date presets of a Start or Due menu, then Clear. */
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
      {DATE_PICKS.map(([name, days]) => (
        <MenuItem
          key={name}
          onSelect={() => {
            close();
            onChoose(datePickValue(days));
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

/** A date field for a start, due or field date: a half-typed date waits, an emptied one clears. */
export function DueDateInput({
  dueAt,
  onChoose,
  className,
  label = "Due date",
}: {
  dueAt: number | null;
  onChoose: (dueAt: number | null) => void;
  className?: string;
  label?: string;
}) {
  const [value, setValue] = useState(dueInputValue(dueAt));
  useEffect(() => setValue(dueInputValue(dueAt)), [dueAt]);
  return (
    <input
      type="date"
      aria-label={label}
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
