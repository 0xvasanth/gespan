import { useEffect, useRef } from "react";
import type {
  AutoMoveRule,
  BoardColumn,
  BoardSettings,
  LinkedWorkEvent,
} from "../../model/board";
import {
  AUTO_MOVE_EVENTS,
  ruleFor,
  shownRule,
  withRule,
} from "../../model/boardSettings";
import { FIELD, SettingsSection } from "./settingsControls";

type Toggle = "runWith" | "writeBack" | "ticketStatusChanges";

const TOGGLES: readonly (readonly [Toggle, string, string])[] = [
  ["runWith", "Run with…", "Cards offer \"Run with…\" to start a session from them."],
  ["writeBack", "Agent write-back", "A finished turn comments on the cards linked to its session."],
  ["ticketStatusChanges", "Agent may change ticket status", "Sessions on this board's cards may move their ticket's status."],
];

const encode = (rule: AutoMoveRule) =>
  typeof rule === "object" ? `column:${rule.column}` : rule;
const decode = (value: string): AutoMoveRule =>
  value.startsWith("column:")
    ? { column: value.slice("column:".length) }
    : value === "stay"
      ? "stay"
      : "default";

/**
 * The agent toggles and one auto-move rule per linked-work event. Each write
 * is a patch of one setting. Quick edits build on the latest value, and a
 * failed save goes back to what is stored.
 */
export function AutomationSection({
  settings,
  columns,
  onPatch,
}: {
  settings: BoardSettings | null;
  columns: readonly BoardColumn[];
  onPatch: (patch: Partial<BoardSettings>, what: string) => Promise<boolean>;
}) {
  const ordered = [...columns].sort((a, b) => a.position - b.position);
  const columnIds = new Set(ordered.map((column) => column.id));
  // New boards start with all three on; before settings load they read on.
  const stored = {
    runWith: settings?.runWith ?? true,
    writeBack: settings?.writeBack ?? true,
    ticketStatusChanges: settings?.ticketStatusChanges ?? true,
    autoMoveJson: settings?.autoMoveJson ?? null,
  };
  const latest = useRef(stored);
  const storedRef = useRef(stored);
  storedRef.current = stored;
  const pending = useRef(0);
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const storedKey = JSON.stringify(stored);
  useEffect(() => {
    // The board's read wins once nothing is in flight.
    if (pending.current === 0) latest.current = JSON.parse(storedKey);
  }, [storedKey]);

  const write = (
    key: keyof typeof stored,
    value: boolean | string,
    what: string,
  ) => {
    latest.current = { ...latest.current, [key]: value };
    pending.current += 1;
    queue.current = queue.current
      .then(() => onPatch({ [key]: value } as Partial<BoardSettings>, what))
      .then((saved) => {
        pending.current -= 1;
        if (!saved && pending.current === 0) latest.current = storedRef.current;
      });
  };

  return (
    <SettingsSection title="Automation">
      <div className="flex flex-col gap-2">
        {TOGGLES.map(([key, label, hint]) => (
          <label key={key} className="flex items-start gap-2 text-[12px] text-content/85">
            <input
              type="checkbox"
              aria-label={label}
              checked={stored[key]}
              onChange={(event) =>
                write(key, event.target.checked, `change “${label}”`)
              }
              className="mt-0.5"
            />
            <span>
              {label}
              <span className="block text-content/50">{hint}</span>
            </span>
          </label>
        ))}
      </div>
      <div className="flex flex-col gap-1.5">
        <h3 className="text-[12px] font-medium text-content/70">
          When linked work reports, move the card
        </h3>
        {AUTO_MOVE_EVENTS.map(([event, label]) => {
          const rule = shownRule(ruleFor(stored.autoMoveJson, event), columnIds);
          return (
            <label key={event} className="flex items-center gap-3 text-[12px] text-content/80">
              <span className="w-40 shrink-0">{label}</span>
              <select
                aria-label={`Auto-move: ${label}`}
                value={encode(rule)}
                onChange={(e) =>
                  write(
                    "autoMoveJson",
                    withRule(
                      latest.current.autoMoveJson,
                      event as LinkedWorkEvent,
                      decode(e.target.value),
                    ),
                    "save the auto-move rule",
                  )
                }
                className={`${FIELD} w-52`}
              >
                <option value="default">Default (forward only)</option>
                <option value="stay">Stay</option>
                {ordered.map((column) => (
                  <option key={column.id} value={`column:${column.id}`}>
                    Move to {column.name}
                  </option>
                ))}
              </select>
            </label>
          );
        })}
      </div>
    </SettingsSection>
  );
}
