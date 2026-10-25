import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SecondaryButton } from "../../../shared/ui/SecondaryButton";
import {
  disconnectPlane,
  PLANE_CLOUD_URL,
  planeApiKeysUrl,
  planeStatus,
  savePlaneConfig,
  type PlaneStatus,
} from "../../inbox/model/plane";

export function PlaneSettings() {
  const [status, setStatus] = useState<PlaneStatus | null>(null);
  const [baseUrl, setBaseUrl] = useState(PLANE_CLOUD_URL);
  const [workspace, setWorkspace] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [checking, setChecking] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void planeStatus()
      .then((next) => {
        if (!cancelled) setStatus(next);
      })
      .catch((err: unknown) => {
        if (!cancelled)
          setError(String(err instanceof Error ? err.message : err));
      })
      .finally(() => {
        if (!cancelled) setChecking(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const ready = baseUrl.trim() && workspace.trim() && apiKey.trim();

  const run = async (work: () => Promise<PlaneStatus>) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await work());
      setApiKey("");
    } catch (err) {
      setError(String(err instanceof Error ? err.message : err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="px-4 py-3.5">
      {checking ? (
        <p className="text-[12px] text-content/45">
          Checking Plane connection…
        </p>
      ) : status?.connected ? (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="min-w-0 text-[12px] text-content/65">
            <p className="break-all">{status.baseUrl}</p>
            <p className="break-all">{status.workspace}</p>
          </div>
          <SecondaryButton
            onClick={() => void run(disconnectPlane)}
            disabled={busy}
          >
            {busy ? "Disconnecting" : "Disconnect"}
          </SecondaryButton>
        </div>
      ) : (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (busy || !ready) return;
            void run(() => savePlaneConfig({ baseUrl, workspace, apiKey }));
          }}
          className="flex flex-col gap-3"
        >
          <p className="text-[12px] leading-relaxed text-content/45">
            Connect Plane Cloud or a self-hosted Plane with a personal API key.
            Disconnect deletes the saved key.
          </p>
          {(
            [
              {
                label: "Plane base URL",
                value: baseUrl,
                set: setBaseUrl,
                type: "text",
                placeholder: PLANE_CLOUD_URL,
              },
              {
                label: "Plane workspace",
                value: workspace,
                set: setWorkspace,
                type: "text",
                placeholder: "workspace-slug",
              },
              {
                label: "Plane API key",
                value: apiKey,
                set: setApiKey,
                type: "password",
                placeholder: "API key",
              },
            ] as const
          ).map((field) => (
            <label
              key={field.label}
              className="flex flex-col gap-1 text-[12px] text-content/65"
            >
              {field.label}
              <input
                aria-label={field.label}
                type={field.type}
                value={field.value}
                onChange={(event) => field.set(event.target.value)}
                placeholder={field.placeholder}
                disabled={busy}
                required
                autoComplete="off"
                spellCheck={false}
                className="h-8 w-full rounded-md border border-content/10 bg-transparent px-2 text-content outline-none focus:border-content/20"
              />
            </label>
          ))}
          <div className="flex items-center gap-3">
            <SecondaryButton type="submit" disabled={busy || !ready}>
              {busy ? "Connecting" : "Connect"}
            </SecondaryButton>
            <button
              type="button"
              onClick={() => void openUrl(planeApiKeysUrl(baseUrl))}
              className="text-[12px] text-content/65 hover:text-content"
            >
              Create API key
            </button>
          </div>
        </form>
      )}
      {error ? (
        <p role="alert" className="mt-3 text-[12px] text-red-400/90">
          {error}
        </p>
      ) : null}
    </div>
  );
}
