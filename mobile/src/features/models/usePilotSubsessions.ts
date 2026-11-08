import { effortSetting } from "@/features/models/modelChoice";
import type { CatalogModel } from "@/features/models/types";
import { useModelCatalog } from "@/features/models/useModelCatalog";
import type { Pilot } from "@/features/workspace/types";
import { pilotConversationId } from "@/features/workspace/WorkspaceProvider";

/** A model id as the desktop keeps it (native_model in gespan-core's
 *  store): without its provider in front, and Claude's versioned ids spelled
 *  as the CLI takes them. */
const nativeModel = (id: string, harness: string) => {
  const bare = id.trim().replace(new RegExp(`^${harness}:`), "").split("[")[0];
  return harness === "claude" && !bare.startsWith("claude-") && /\d/.test(bare)
    ? `claude-${bare.replaceAll(".", "-")}`
    : bare;
};

/** Whether a catalog entry is the model the desktop saved. */
export const isModel = (entry: CatalogModel, harness: string | undefined, id: string | undefined) =>
  !!id &&
  !!harness &&
  entry.harness === harness &&
  (entry.id === id || nativeModel(entry.id, harness) === nativeModel(id, harness));

/**
 * What the sessions a Pilot starts run on when it names no model: the model
 * and effort set for them, or its own where they are left unset.
 */
export function usePilotSubsessions(pilot: Pilot | undefined) {
  const conversationId = pilot ? pilotConversationId(pilot.id) : undefined;
  const { catalog, error } = useModelCatalog(conversationId, conversationId);
  const set = pilot?.subsessions ?? null;
  const inherited = !set?.harness;
  const harness = inherited ? catalog?.current.harness : set?.harness;
  const models = catalog?.models.filter((entry) => entry.harness === harness) ?? [];
  const modelId = inherited ? catalog?.current.model : set?.model;
  // A provider picked without a model runs its first, as the desktop does.
  const model =
    models.find((entry) => isModel(entry, harness, modelId)) ?? (inherited ? undefined : models[0]);
  const effort = effortSetting(model);
  const name = pilot?.name ?? "";
  const unsetEffort = inherited ? `Same as ${name}` : "Model default";
  return {
    catalog,
    error,
    inherited,
    harness: inherited ? undefined : harness,
    model,
    modelLabel: inherited
      ? `Same as ${name}`
      : (model?.name ??
        set?.model ??
        catalog?.harnesses.find((entry) => entry.id === harness)?.label ??
        harness ??
        ""),
    effort,
    effortValue: set?.effort,
    unsetEffort,
    effortLabel: set?.effort
      ? (effort?.options.find((option) => option.value === set.effort)?.label ?? set.effort)
      : unsetEffort,
  };
}
