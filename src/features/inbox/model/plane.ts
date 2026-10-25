import { invoke } from "@tauri-apps/api/core";
import { notifyTicketConnectionsChanged } from "../../sessions/model/ticketLinks";

export type PlaneStatus = {
  connected: boolean;
  baseUrl: string;
  workspace: string;
};

export const PLANE_CLOUD_URL = "https://api.plane.so";

/** Where to create an API key: Plane Cloud's app, or the self-hosted origin. */
export function planeApiKeysUrl(baseUrl: string): string {
  try {
    const url = new URL(baseUrl.trim());
    const host = url.hostname === "api.plane.so" ? "app.plane.so" : url.host;
    return `${url.protocol}//${host}/profile/api-tokens`;
  } catch {
    return "https://app.plane.so/profile/api-tokens";
  }
}

export function planeStatus(): Promise<PlaneStatus> {
  return invoke<PlaneStatus>("plane_status");
}

/** The key is checked against Plane before it is saved. */
export async function savePlaneConfig(config: {
  baseUrl: string;
  workspace: string;
  apiKey: string;
}): Promise<PlaneStatus> {
  const status = await invoke<PlaneStatus>("plane_set_config", {
    baseUrl: config.baseUrl.trim(),
    workspace: config.workspace.trim(),
    apiKey: config.apiKey.trim(),
  });
  notifyTicketConnectionsChanged();
  return status;
}

export async function disconnectPlane(): Promise<PlaneStatus> {
  const status = await invoke<PlaneStatus>("plane_set_config", {
    baseUrl: "",
    workspace: "",
    apiKey: "",
  });
  notifyTicketConnectionsChanged();
  return status;
}
