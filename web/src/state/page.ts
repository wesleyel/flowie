// The page shown: the bullet zoomed into (from the address, `#<id>`), or Home, with its rows,
// the way down to it, the starred pages and the search. `refresh` reads all of it again from the
// worker; the components draw from these signals.
import { batch, signal } from "@preact/signals";
import { call } from "../core/rpc";
import type { Crumb, Row } from "../core/protocol";
import { pendingFocus } from "./editing";

/** The bullet the address zooms into, or undefined for Home. */
export function zoomed(): string | undefined {
  return location.hash.slice(1) || undefined;
}

/** The page's bullet as of the last refresh (undefined: Home). */
export const root = signal<string | undefined>(undefined);
export const rows = signal<Row[]>([]);
/** Pages from Home down to `root`, for breadcrumbs. */
export const crumbs = signal<Crumb[]>([]);
export const starred = signal<Crumb[]>([]);
/** The search typed in the top bar (desktop syntax), or empty. */
export const query = signal("");
/** Whether the page has been read once, so there is something to draw. */
export const loaded = signal(false);

/** Read the page shown again, then put the caret in `focus` if given. */
export async function refresh(focus?: string): Promise<void> {
  const at = zoomed();
  let page: Row[];
  try {
    page = await call("page", { root: at, query: query.value || undefined });
  } catch {
    // The page's bullet is gone (deleted elsewhere): go Home.
    location.hash = "";
    return;
  }
  const [path, stars] = await Promise.all([at ? call("path", { id: at }) : Promise.resolve([]), call("starred", {})]);
  batch(() => {
    root.value = at;
    rows.value = page;
    crumbs.value = path;
    starred.value = stars;
    loaded.value = true;
    if (focus) pendingFocus.value = focus;
  });
}

/** Note that row `id` now has `text` in the worker (saved from its contenteditable). */
export function textSaved(id: string, text: string): void {
  rows.value = rows.value.map((row) => (row.id === id ? { ...row, text } : row));
}

let typing: ReturnType<typeof setTimeout> | undefined;

/** Search for `text`, once typing pauses. */
export function search(text: string): void {
  query.value = text;
  clearTimeout(typing);
  typing = setTimeout(() => void refresh(), 150);
}

/** Open today's journal page. */
export async function openToday(): Promise<void> {
  location.hash = await call("today", {});
}
