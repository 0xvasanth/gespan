import { ask } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import {
  deleteTemplate,
  listTemplates,
  saveBoardAsTemplate,
  type TemplateInfo,
} from "../../model/board";
import { errorText } from "../boardFormat";
import { CommitInput, SETTINGS_BUTTON, SettingsSection } from "./settingsControls";

export const BUILT_IN_DELETE = "Built-in templates can't be deleted";

/** Built-ins first, then saved templates by name. */
export function orderTemplates(list: readonly TemplateInfo[]): TemplateInfo[] {
  return [...list].sort(
    (a, b) =>
      Number(b.builtIn) - Number(a.builtIn) ||
      (a.builtIn ? 0 : a.name.localeCompare(b.name)),
  );
}

/** Templates the board can start from, and "Save as template" for this board. */
export function TemplatesSection({
  boardId,
  onError,
}: {
  boardId: string;
  onError: (message: string) => void;
}) {
  const [templates, setTemplates] = useState<TemplateInfo[]>([]);
  // Template commands don't announce changes: the list is read again after each.
  const reload = useCallback(
    () =>
      listTemplates().then(
        (list) => setTemplates(orderTemplates(list)),
        (cause) => onError(`Couldn't load templates: ${errorText(cause)}`),
      ),
    [onError],
  );
  useEffect(() => {
    void reload();
  }, [reload]);

  return (
    <SettingsSection
      title="Templates"
      hint="A template keeps columns, labels, fields, views and settings, never cards."
    >
      <ul className="flex flex-col gap-1.5">
        {templates.map((template) => (
          <li
            key={template.id}
            data-template={template.id}
            className="flex items-center gap-2 rounded-lg border border-stroke px-2.5 py-1.5 text-[12px] text-content/85"
          >
            <span className="min-w-0 flex-1 truncate">{template.name}</span>
            {template.builtIn ? (
              <span
                data-template-built-in=""
                title={BUILT_IN_DELETE}
                className="rounded-md bg-content/8 px-2 py-0.5 text-[11px] text-content/55"
              >
                Built-in
              </span>
            ) : (
              <button
                type="button"
                aria-label={`Delete template ${template.name}`}
                className={`${SETTINGS_BUTTON} text-red-400`}
                onClick={async () => {
                  const confirmed = await ask(
                    `Delete the template “${template.name}”? Boards made from it are not affected.`,
                    {
                      title: "Delete template",
                      kind: "warning",
                      okLabel: "Delete template",
                      cancelLabel: "Cancel",
                    },
                  ).catch(() => false);
                  if (!confirmed) return;
                  await deleteTemplate(template.id).catch((cause) =>
                    onError(`Couldn't delete the template: ${errorText(cause)}`),
                  );
                  await reload();
                }}
              >
                Delete
              </button>
            )}
          </li>
        ))}
      </ul>
      <div className="flex items-center gap-2">
        <CommitInput
          label="Template name"
          value=""
          placeholder="Save this board as a template"
          className="w-64"
          onCommit={async (text) => {
            const name = text.trim();
            if (!name) return false;
            try {
              await saveBoardAsTemplate(boardId, name);
            } catch (cause) {
              onError(`Couldn't save the template: ${errorText(cause)}`);
              return false;
            }
            await reload();
          }}
        />
        <span className="text-[12px] text-content/45">Press Enter to save</span>
      </div>
    </SettingsSection>
  );
}
