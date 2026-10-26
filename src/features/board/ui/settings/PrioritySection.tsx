import type { BoardSettings, PriorityScheme } from "../../model/board";
import {
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
  onPatch: (patch: Partial<BoardSettings>, what: string) => void;
  onError: (message: string) => void;
}) {
  const save = (next: PriorityScheme) =>
    onPatch(
      { priorityJson: serializePriorityScheme(next) },
      "save the priorities",
    );
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
                if (text.trim() !== scheme.names[level]) {
                  save(withName(scheme, level, text));
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
                  save(withHidden(scheme, level, !event.target.checked))
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
          onClick={() =>
            onPatch({ priorityJson: null }, "reset the priorities")
          }
        >
          Reset to defaults
        </button>
      </div>
    </SettingsSection>
  );
}
