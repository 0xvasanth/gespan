import { ask } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import {
  createField,
  deleteField,
  moveField,
  renameFieldOption,
  updateField,
  type BoardField,
  type BoardFieldKind,
} from "../../model/board";
import { fieldOptions } from "../../model/cardProps";
import { errorText } from "../boardFormat";
import {
  CommitInput,
  FIELD,
  MoveButtons,
  SETTINGS_BUTTON,
  SettingsSection,
} from "./settingsControls";

export const FIELD_KINDS: readonly (readonly [BoardFieldKind, string])[] = [
  ["text", "Text"],
  ["number", "Number"],
  ["date", "Date"],
  ["select", "Select"],
  ["multi-select", "Multi-select"],
  ["checkbox", "Checkbox"],
  ["url", "URL"],
];

const hasOptions = (kind: BoardFieldKind) =>
  kind === "select" || kind === "multi-select";

/** Create, rename, reorder and delete custom fields; manage the options of select kinds. */
export function FieldsSection({
  boardId,
  fields,
  onError,
}: {
  boardId: string;
  fields: readonly BoardField[];
  onError: (message: string) => void;
}) {
  const ordered = [...fields].sort((a, b) => a.position - b.position);
  const [name, setName] = useState("");
  const [kind, setKind] = useState<BoardFieldKind>("text");
  const run = (what: string, work: () => Promise<unknown>) =>
    work().catch((cause) => onError(`Couldn't ${what}: ${errorText(cause)}`));

  const add = () => {
    const trimmed = name.trim();
    if (!trimmed) {
      onError("A field needs a name");
      return;
    }
    void run("add the field", () =>
      createField(boardId, trimmed, kind, hasOptions(kind) ? "[]" : null),
    ).then(() => setName(""));
  };

  return (
    <SettingsSection
      title="Fields"
      hint="A field's kind can't change. Deleting a field removes its values, its filter rows and its table column."
    >
      <ul className="flex flex-col gap-2">
        {ordered.map((field, index) => (
          <li
            key={field.id}
            data-settings-field={field.id}
            className="flex flex-col gap-2 rounded-lg border border-stroke p-2.5"
          >
            <div className="flex flex-wrap items-center gap-2">
              <CommitInput
                label="Field name"
                value={field.name}
                className="w-44"
                onCommit={(text) => {
                  const trimmed = text.trim();
                  if (!trimmed) {
                    onError("A field needs a name");
                    return false;
                  }
                  if (trimmed === field.name) return;
                  void run("rename the field", () =>
                    updateField(field.id, { name: trimmed }),
                  );
                }}
              />
              <span
                data-field-kind=""
                className="rounded-md bg-content/8 px-2 py-1 text-[11px] text-content/60"
              >
                {FIELD_KINDS.find(([known]) => known === field.kind)?.[1]}
              </span>
              <div className="ml-auto flex items-center gap-1">
                <MoveButtons
                  index={index}
                  count={ordered.length}
                  onMove={(to) =>
                    void run("move the field", () => moveField(field.id, to))
                  }
                />
                <button
                  type="button"
                  className={`${SETTINGS_BUTTON} text-red-400`}
                  onClick={async () => {
                    const confirmed = await ask(
                      `Delete the field “${field.name}”? Its values, filter rows and table column are removed.`,
                      {
                        title: "Delete field",
                        kind: "warning",
                        okLabel: "Delete field",
                        cancelLabel: "Cancel",
                      },
                    ).catch(() => false);
                    if (confirmed) {
                      await run("delete the field", () => deleteField(field.id));
                    }
                  }}
                >
                  Delete field
                </button>
              </div>
            </div>
            {hasOptions(field.kind) ? (
              <OptionsEditor field={field} onError={onError} run={run} />
            ) : null}
          </li>
        ))}
      </ul>
      <div className="flex flex-wrap items-center gap-2">
        <input
          aria-label="New field name"
          placeholder="New field name"
          value={name}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.nativeEvent.isComposing) {
              event.preventDefault();
              add();
            }
          }}
          className={`${FIELD} w-48`}
        />
        <select
          aria-label="New field kind"
          value={kind}
          onChange={(event) => setKind(event.target.value as BoardFieldKind)}
          className={`${FIELD} w-36`}
        >
          {FIELD_KINDS.map(([value, label]) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
        <button type="button" className={SETTINGS_BUTTON} onClick={add}>
          Add field
        </button>
      </div>
    </SettingsSection>
  );
}

function OptionsEditor({
  field,
  onError,
  run,
}: {
  field: BoardField;
  onError: (message: string) => void;
  run: (what: string, work: () => Promise<unknown>) => Promise<unknown>;
}) {
  const options = fieldOptions(field.optionsJson);
  const taken = (text: string, except?: string) =>
    options.some((option) => option !== except && option === text);
  return (
    <div role="group" aria-label={`${field.name} options`} className="flex flex-col gap-1.5">
      {options.map((option) => (
        <div key={option} data-field-option={option} className="flex items-center gap-2">
          <CommitInput
            label="Option name"
            value={option}
            className="w-40"
            onCommit={(text) => {
              const trimmed = text.trim();
              if (!trimmed) {
                onError("An option needs a name");
                return false;
              }
              if (trimmed === option) return;
              if (taken(trimmed, option)) {
                onError(`The option “${trimmed}” already exists`);
                return false;
              }
              // The dedicated rename keeps the stored values, even for a case-only change.
              void run("rename the option", () =>
                renameFieldOption(field.id, option, trimmed),
              );
            }}
          />
          <button
            type="button"
            aria-label={`Remove option ${option}`}
            className={SETTINGS_BUTTON}
            onClick={() =>
              void run("remove the option", () =>
                updateField(field.id, {
                  optionsJson: JSON.stringify(options.filter((o) => o !== option)),
                }),
              )
            }
          >
            Remove
          </button>
        </div>
      ))}
      <CommitInput
        label="New option"
        value=""
        placeholder="New option"
        className="w-40"
        onCommit={(text) => {
          const trimmed = text.trim();
          if (!trimmed) return false;
          if (taken(trimmed)) {
            onError(`The option “${trimmed}” already exists`);
            return false;
          }
          void run("add the option", () =>
            updateField(field.id, {
              optionsJson: JSON.stringify([...options, trimmed]),
            }),
          );
        }}
      />
    </div>
  );
}
