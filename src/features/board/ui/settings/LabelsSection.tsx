import { ask } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import {
  createLabel,
  deleteLabel,
  mergeLabels,
  moveLabel,
  updateLabel,
  type BoardLabel,
} from "../../model/board";
import { labelNameTaken } from "../../model/boardSettings";
import { errorText, nextLabelColor } from "../boardFormat";
import {
  CommitInput,
  FIELD,
  MoveButtons,
  SETTINGS_BUTTON,
  SettingsSection,
  Swatches,
} from "./settingsControls";

/** Create, rename, recolour, reorder, merge and delete a board's labels. */
export function LabelsSection({
  boardId,
  labels,
  onError,
}: {
  boardId: string;
  labels: readonly BoardLabel[];
  onError: (message: string) => void;
}) {
  const ordered = [...labels].sort((a, b) => a.position - b.position);
  const run = (what: string, work: () => Promise<unknown>) =>
    work().catch((cause) => onError(`Couldn't ${what}: ${errorText(cause)}`));
  const taken = (name: string, exceptId?: string) => {
    if (!labelNameTaken(ordered, name, exceptId)) return false;
    onError(`A label named “${name.trim()}” already exists`);
    return true;
  };

  return (
    <SettingsSection
      title="Labels"
      hint="Merging moves every card to the other label. Deleting removes the label from its cards."
    >
      <ul className="flex flex-col gap-2">
        {ordered.map((label, index) => (
          <LabelRow
            key={label.id}
            label={label}
            index={index}
            count={ordered.length}
            others={ordered.filter((other) => other.id !== label.id)}
            onRename={(name) => {
              const trimmed = name.trim();
              if (!trimmed) {
                onError("A label needs a name");
                return false;
              }
              if (trimmed === label.name || taken(trimmed, label.id)) {
                return trimmed === label.name;
              }
              void run("rename the label", () =>
                updateLabel(label.id, { name: trimmed }),
              );
            }}
            onRecolour={(color) => {
              if (color) {
                void run("recolour the label", () =>
                  updateLabel(label.id, { color }),
                );
              }
            }}
            onMove={(to) =>
              void run("move the label", () => moveLabel(label.id, to))
            }
            onMerge={async (intoId, intoName) => {
              const confirmed = await ask(
                `Merge “${label.name}” into “${intoName}”? Every card with “${label.name}” gets “${intoName}” instead.`,
                {
                  title: "Merge labels",
                  kind: "warning",
                  okLabel: "Merge",
                  cancelLabel: "Cancel",
                },
              ).catch(() => false);
              if (confirmed) {
                await run("merge the labels", () =>
                  mergeLabels(label.id, intoId),
                );
              }
            }}
            onDelete={async () => {
              const confirmed = await ask(
                `Delete the label “${label.name}”? It is removed from every card.`,
                {
                  title: "Delete label",
                  kind: "warning",
                  okLabel: "Delete label",
                  cancelLabel: "Cancel",
                },
              ).catch(() => false);
              if (confirmed) {
                await run("delete the label", () => deleteLabel(label.id));
              }
            }}
          />
        ))}
      </ul>
      <CommitInput
        label="New label"
        value=""
        placeholder="New label name"
        className="w-56"
        onCommit={(text) => {
          const name = text.trim();
          if (!name || taken(name)) return false;
          void run("create the label", () =>
            createLabel(boardId, name, nextLabelColor(ordered.length)),
          );
        }}
      />
    </SettingsSection>
  );
}

function LabelRow({
  label,
  index,
  count,
  others,
  onRename,
  onRecolour,
  onMove,
  onMerge,
  onDelete,
}: {
  label: BoardLabel;
  index: number;
  count: number;
  others: readonly BoardLabel[];
  onRename: (name: string) => boolean | void;
  onRecolour: (color: string | null) => void;
  onMove: (to: number) => void;
  onMerge: (intoId: string, intoName: string) => Promise<void>;
  onDelete: () => Promise<void>;
}) {
  const [into, setInto] = useState("");
  const target = others.find((other) => other.id === into);
  return (
    <li
      data-settings-label={label.id}
      className="flex flex-wrap items-center gap-2 rounded-lg border border-stroke p-2.5"
    >
      <CommitInput
        label="Label name"
        value={label.name}
        className="w-40"
        onCommit={onRename}
      />
      <Swatches
        label="Label colour"
        value={label.color}
        onChange={onRecolour}
      />
      <div className="ml-auto flex flex-wrap items-center gap-1">
        <MoveButtons index={index} count={count} onMove={onMove} />
        <select
          aria-label="Merge into…"
          value={into}
          onChange={(event) => setInto(event.target.value)}
          className={`${FIELD} w-36`}
        >
          <option value="">Merge into…</option>
          {others.map((other) => (
            <option key={other.id} value={other.id}>
              {other.name}
            </option>
          ))}
        </select>
        <button
          type="button"
          className={SETTINGS_BUTTON}
          disabled={!target}
          onClick={() => {
            if (!target) return;
            void onMerge(target.id, target.name).then(() => setInto(""));
          }}
        >
          Merge
        </button>
        <button
          type="button"
          className={`${SETTINGS_BUTTON} text-red-400`}
          onClick={() => void onDelete()}
        >
          Delete label
        </button>
      </div>
    </li>
  );
}
