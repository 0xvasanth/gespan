import {
  bindKiroSession,
  cancelKiroTurn,
  forgetKiroSession,
  respondKiroApproval,
  sendKiroTurn,
  steerKiroTurn,
  stopKiroSession,
} from "./kiro";
import { refreshKiroCatalog } from "./kiroCatalog";
import { registerHarness, type HarnessAdapter } from "../../core/registry";

export const kiroAdapter: HarnessAdapter = {
  id: "kiro",
  live: true,
  canSteer: false,
  sendTurn: sendKiroTurn,
  steerTurn: steerKiroTurn,
  cancelTurn: cancelKiroTurn,
  respondApproval: respondKiroApproval,
  stopSession: stopKiroSession,
  forgetSession: forgetKiroSession,
  bindSession: bindKiroSession,
  refreshCatalog: refreshKiroCatalog,
};

let registered = false;

export function ensureKiroRegistered(): void {
  if (registered) return;
  registerHarness(kiroAdapter);
  registered = true;
}
