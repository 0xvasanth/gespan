import { useEffect, useRef, useState } from "react";
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
import { FIELD, SettingsSection, SettingsUnavailable } from "./settingsControls";

type Toggle = "runWith" | "writeBack" | "ticketStatusChanges";
type Values = {
  runWith: boolean;
  writeBack: boolean;
  ticketStatusChanges: boolean;
  autoMoveJson: string | null;
};
/** One queued edit: the new value, worked out from the value it is built on. */
type Op = {
  [K in keyof Values]: {
    key: K;
    compute: (base: Values[K]) => Values[K];
  };
}[keyof Values];

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

const valuesOf = (settings: BoardSettings): Values => ({
  runWith: settings.runWith,
  writeBack: settings.writeBack,
  ticketStatusChanges: settings.ticketStatusChanges,
  autoMoveJson: settings.autoMoveJson ?? null,
});

/**
 * The agent toggles and one auto-move rule per linked-work event. Each write
 * is a patch of one setting. Edits queue and run in order, each built on what
 * the earlier ones really saved, so a failed save is not carried into the
 * next. Without settings (they couldn't be read) nothing is written: an edit
 * would replace what is stored with defaults.
 */
export function AutomationSection({
  settings,
  columns,
  onPatch,
  onRetry,
}: {
  settings: BoardSettings | null;
  columns: readonly BoardColumn[];
  onPatch: (patch: Partial<BoardSettings>, what: string) => Promise<boolean>;
  /** Reads the settings again. */
  onRetry: () => void;
}) {
  const ordered = [...columns].sort((a, b) => a.position - b.position);
  const columnIds = new Set(ordered.map((column) => column.id));
  const stored = settings ? valuesOf(settings) : null;
  // What is saved, and the edits still to run on top of it.
  const confirmed = useRef<Values | null>(stored);
  const ops = useRef<Op[]>([]);
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const [, rerender] = useState(0);
  const storedKey = JSON.stringify(stored);
  useEffect(() => {
    // The board's read wins once nothing is in flight.
    if (ops.current.length === 0) {
      confirmed.current = storedKey === "null" ? null : JSON.parse(storedKey);
      rerender((n) => n + 1);
    }
  }, [storedKey]);

  const apply = (base: Values, list: readonly Op[]): Values =>
    list.reduce<Values>(
      (values, op) => ({
        ...values,
        [op.key]: (op.compute as (base: unknown) => unknown)(values[op.key]),
      }),
      base,
    );
  const base = confirmed.current ?? stored;
  const shown = base ? apply(base, ops.current) : null;

  const edit = (op: Op, what: string) => {
    ops.current = [...ops.current, op];
    rerender((n) => n + 1);
    queue.current = queue.current.then(async () => {
      const from = confirmed.current;
      let saved = false;
      if (from) {
        const value = (op.compute as (base: unknown) => unknown)(from[op.key]);
        saved = await onPatch({ [op.key]: value } as Partial<BoardSettings>, what);
        if (saved) confirmed.current = { ...from, [op.key]: value };
      }
      ops.current = ops.current.filter((held) => held !== op);
      rerender((n) => n + 1);
    });
  };

  const ready = shown !== null;
  return (
    <SettingsSection title="Automation">
      {ready ? null : <SettingsUnavailable onRetry={onRetry} />}
      <div className="flex flex-col gap-2">
        {TOGGLES.map(([key, label, hint]) => (
          <label key={key} className="flex items-start gap-2 text-[12px] text-content/85">
            <input
              type="checkbox"
              aria-label={label}
              disabled={!ready}
              checked={shown?.[key] ?? false}
              onChange={() => {
                if (!shown) return;
                const next = !shown[key];
                edit({ key, compute: () => next }, `change “${label}”`);
              }}
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
          const rule = shown
            ? shownRule(ruleFor(shown.autoMoveJson, event), columnIds)
            : "default";
          return (
            <label key={event} className="flex items-center gap-3 text-[12px] text-content/80">
              <span className="w-40 shrink-0">{label}</span>
              <select
                aria-label={`Auto-move: ${label}`}
                disabled={!ready}
                value={encode(rule)}
                onChange={(e) => {
                  const chosen = decode(e.target.value);
                  edit(
                    {
                      key: "autoMoveJson",
                      compute: (json) =>
                        withRule(json, event as LinkedWorkEvent, chosen),
                    },
                    "save the auto-move rule",
                  );
                }}
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
