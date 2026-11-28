// Keys for the whole page. Undo and redo for the outline, like the desktop: typing in a bullet
// is saved first (leaving it saves), so it is part of what undo takes back.
import { call } from "../core/rpc";
import { refresh } from "../state/page";

export function installShortcuts(): void {
  addEventListener("keydown", async (event) => {
    if (!(event.metaKey || event.ctrlKey) || (event.target instanceof HTMLElement && event.target.id === "search")) return;
    const key = event.key.toLowerCase();
    const redo = (key === "z" && event.shiftKey) || (key === "y" && event.ctrlKey);
    if (key !== "z" && !redo) return;
    event.preventDefault();
    const active = document.activeElement;
    if (active instanceof HTMLElement && active.isContentEditable) active.blur();
    if (await call(redo ? "redo" : "undo", {})) await refresh();
  });
}
