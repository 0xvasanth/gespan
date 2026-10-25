import { invoke } from "@tauri-apps/api/core";
import { notifyTicketConnectionsChanged } from "../../sessions/model/ticketLinks";

export type PlaneStatus = {
  connected: boolean;
  baseUrl: string;
  workspace: string;
};

export const PLANE_CLOUD_URL = "https://api.plane.so";

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
