// What the rows' buttons and keys do: a call to the worker, then the page drawn again.
import { call } from "../core/rpc";
import type { Row } from "../core/protocol";
import { refresh, root, rows, textSaved } from "../state/page";

export async function toggleCollapsed(row: Row): Promise<void> {
  await call("setCollapsed", { id: row.id, collapsed: !row.collapsed });
  await refresh();
}

export async function toggleStarred(row: Row): Promise<void> {
  await call("setStarred", { id: row.id, starred: !row.starred });
  await refresh();
}

export async function setCompleted(row: Row, completed: boolean): Promise<void> {
  await call("setCompleted", { id: row.id, completed });
  await refresh();
}

export async function remove(row: Row): Promise<void> {
  await call("delete", { id: row.id });
  await refresh();
}

/** Save row `id`'s typed text. */
export async function saveText(id: string, text: string): Promise<void> {
  await call("setText", { id, text });
  textSaved(id, text);
}

export function setEditing(id: string | undefined): Promise<void> {
  return call("setEditing", { id });
}

/** Enter: a new bullet after `row`, with the caret in it. */
export async function newAfter(row: Row): Promise<void> {
  const id = await call("createAfter", { sibling: row.id, text: "" });
  await refresh(id);
}

/** Tab and Shift-Tab. */
export async function indent(row: Row, outdent: boolean): Promise<void> {
  if (outdent) await call("outdent", { id: row.id, root: root.value });
  else await call("indent", { id: row.id });
  await refresh(row.id);
}

/** Backspace in an empty bullet: delete it, caret to the row above. */
export async function removeEmpty(row: Row, index: number): Promise<void> {
  const above = rows.value[index - 1]?.id;
  await call("delete", { id: row.id });
  await refresh(above);
}

/** A new top-level bullet at the end of the page (the form under the outline). */
export async function addAtEnd(text: string): Promise<void> {
  await call("create", { parent: root.value, index: rows.value.filter((row) => row.depth === 0).length, text });
  await refresh();
}
