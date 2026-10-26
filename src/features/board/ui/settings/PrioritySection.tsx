import { useEffect, useRef, useState } from "react";
import type { BoardSettings, PriorityScheme } from "../../model/board";
import {
  DEFAULT_PRIORITY_SCHEME,
  PRIORITY_NAME_MAX,
  priorityName,
  serializePriorityScheme,
  validPriorityName,
  withHidden,
  withName,
} from "../../model/boardSettings";
import {
  CommitInput,
  SETTINGS_BUTTON,
  SettingsSection,
} from "./settingsControls";

/** The five priority names, and which levels pickers and filters offer. */
export function PrioritySection({
  scheme,
  stored,
  onPatch,
  onError,
}: {
  scheme: PriorityScheme;
  /** Whether the board holds its own scheme (so Reset has something to clear). */
  stored: boolean;
  onPatch: (patch: Partial<BoardSettings>, what: string) => Promise<boolean>;
  onError: (message: string) => void;
}) {
  // Every edit builds on the latest scheme, saved or not yet: two quick edits
  // can't undo each other. Writes go one at a time, in order.
  const latest = useRef(scheme);
  const storedScheme = useRef(scheme);
  storedScheme.current = scheme;
  const pending = useRef(0);
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  // Per level, bumped when a rename of it fails: that name field starts again
  // from the saved scheme. Other fields (one being typed in, say) keep going.
  const [resets, setResets] = useState<Record<number, number>>({});
  const failedLevels = useRef(new Set<number>());
  useEffect(() => {
    // The board's read wins once nothing is in flight.
    if (pending.current === 0) latest.current = scheme;
  }, [scheme]);
  /** `level` is the name field a rename came from, if it was one. */
  const write = (
    patch: Partial<BoardSettings>,
    what: string,
    level?: number,
  ) => {
    pending.current += 1;
    queue.current = queue.current
      .then(() => onPatch(patch, what))
      .then((saved) => {
        pending.current -= 1;
        if (!saved && level !== undefined) failedLevels.current.add(level);
        // A save that failed leaves what is stored; edits stop building on it.
        if (!saved && pending.current === 0) {
          latest.current = storedScheme.current;
          const failed = [...failedLevels.current];
          failedLevels.current.clear();
          if (failed.length) {
            setResets((current) => {
              const next = { ...current };
              for (const at of failed) next[at] = (next[at] ?? 0) + 1;
              return next;
            });
          }
        }
      });
  };
  const save = (next: PriorityScheme, level?: number) => {
    latest.current = next;
    write(
      { priorityJson: serializePriorityScheme(next) },
      "save the priorities",
      level,
    );
  };
  return (
    <SettingsSection
      title="Priority"
      hint="A hidden level leaves the pickers and filters; a card that already has it still shows it."
    >
      <ul className="flex flex-col gap-1.5">
        {scheme.names.map((_, level) => (
          <li
            key={level}
            data-settings-priority={level}
            className="flex items-center gap-3"
          >
            <CommitInput
              key={resets[level] ?? 0}
              label="Priority name"
              value={scheme.names[level]}
              className="w-44"
              onCommit={(text) => {
                if (!validPriorityName(text)) {
                  onError(
                    `A priority name is 1 to ${PRIORITY_NAME_MAX} characters`,
                  );
                  return false;
                }
                if (text.trim() !== latest.current.names[level]) {
                  save(withName(latest.current, level, text), level);
                }
              }}
            />
            <label className="flex items-center gap-1.5 text-[12px] text-content/70">
              <input
                type="checkbox"
                aria-label={`Show ${priorityName(scheme, level)} in pickers`}
                checked={!scheme.hidden.includes(level)}
                disabled={level === 0}
                onChange={(event) =>
                  save(withHidden(latest.current, level, !event.target.checked))
                }
              />
              Show in pickers
            </label>
          </li>
        ))}
      </ul>
      <div>
        <button
          type="button"
          className={SETTINGS_BUTTON}
          disabled={!stored}
          onClick={() => {
            latest.current = DEFAULT_PRIORITY_SCHEME;
            write({ priorityJson: null }, "reset the priorities");
          }}
        >
          Reset to defaults
        </button>
      </div>
    </SettingsSection>
  );
}
