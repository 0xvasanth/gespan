import { useState } from "react";
import { Modal } from "../../../shared/ui/Modal";
import {
  HARNESS_LABEL,
  HARNESSES,
  type HarnessId,
} from "../../sessions/model/session";
import { defaultSessionChoice, modelsFor, resolveModel } from "../../sessions/model/models";
import type { RunRequest } from "./boardFormat";
import { FIELD, SETTINGS_BUTTON } from "./settings/settingsControls";

/**
 * "Run with…": pick a provider, a model and where to work, then start a
 * session on the card. The prompt is built by the store, not here.
 */
export function RunWithDialog({
  cardId,
  projects,
  onRun,
  onClose,
}: {
  cardId: string;
  projects: readonly { path: string; name: string }[];
  onRun: (request: RunRequest) => void;
  onClose: () => void;
}) {
  // The provider and model the user starts sessions with by default.
  const [choice] = useState(() => defaultSessionChoice(projects[0]?.path));
  const [harness, setHarness] = useState<HarnessId>(choice.harness);
  const [model, setModel] = useState(() => resolveModel(choice.harness, choice.model).id);
  const [cwd, setCwd] = useState(projects[0]?.path ?? "");
  const [worktree, setWorktree] = useState(false);
  const models = modelsFor(harness);

  return (
    <Modal title="Run with…" size="sm" hideClose onClose={onClose}>
      <form
        className="flex flex-col gap-3 px-4 pb-4 pt-2 text-[12px] text-content/80"
        onSubmit={(event) => {
          event.preventDefault();
          if (cwd) onRun({ cardId, harness, model, cwd, worktree });
        }}
      >
        <label className="flex items-center gap-3">
          <span className="w-20 shrink-0">Provider</span>
          <select
            aria-label="Provider"
            value={harness}
            onChange={(event) => {
              const next = event.target.value as HarnessId;
              setHarness(next);
              setModel(resolveModel(next).id);
            }}
            className={`${FIELD} w-52`}
          >
            {HARNESSES.map((id) => (
              <option key={id} value={id}>
                {HARNESS_LABEL[id]}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-3">
          <span className="w-20 shrink-0">Model</span>
          <select
            aria-label="Model"
            value={model}
            onChange={(event) => setModel(event.target.value)}
            className={`${FIELD} w-52`}
          >
            {models.map((entry) => (
              <option key={entry.id} value={entry.id}>
                {entry.name}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-3">
          <span className="w-20 shrink-0">Project</span>
          <select
            aria-label="Project"
            value={cwd}
            onChange={(event) => setCwd(event.target.value)}
            className={`${FIELD} w-52`}
          >
            {projects.map((project) => (
              <option key={project.path} value={project.path}>
                {project.name}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-2">
          <span className="w-20 shrink-0" />
          <input
            type="checkbox"
            aria-label="New worktree"
            checked={worktree}
            onChange={(event) => setWorktree(event.target.checked)}
          />
          Work in a new worktree
        </label>
        {projects.length === 0 ? (
          <span role="note" className="text-content/50">
            Open a project first.
          </span>
        ) : null}
        <div className="flex justify-end gap-2">
          <button type="button" className={SETTINGS_BUTTON} onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className={SETTINGS_BUTTON} disabled={!cwd}>
            Run
          </button>
        </div>
      </form>
    </Modal>
  );
}
