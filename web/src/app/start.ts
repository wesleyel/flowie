// Starting up: without a device token, ask for one; with one, open the outline in the worker
// (OPFS, synced with the server this page comes from) and draw it.
import { call, on } from "../core/rpc";
import { token } from "../core/token";
import { isEditing } from "../state/editing";
import { refresh } from "../state/page";
import { phase, status, statusTitle } from "../state/session";
import { installShortcuts } from "./shortcuts";

export async function start(): Promise<void> {
  const device = token();
  if (!device) {
    phase.value = "setup";
    status.value = "not connected";
    return;
  }
  on("status", ({ text }) => (status.value = text));
  // Don't redraw under someone typing.
  on("changed", () => {
    if (!isEditing()) void refresh();
  });
  const { persistent } = await call("start", { origin: location.origin, token: device });
  if (!persistent) statusTitle.value = "This browser can't keep the outline: it is fetched again from the server each time.";
  phase.value = "ready";
  addEventListener("hashchange", () => void refresh());
  installShortcuts();
  await refresh();
}
