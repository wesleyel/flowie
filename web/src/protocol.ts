// What the page and the worker say to each other.

/** One row of a page (flowie-wasm's `RowView`). */
export interface Row {
  id: string;
  depth: number;
  text: string;
  note: string;
  kind: string;
  completed: boolean;
  collapsed: boolean;
  has_children: boolean;
  starred: boolean;
}

/** A page on the way down from Home, for breadcrumbs. */
export interface Crumb {
  id: string;
  text: string;
}

/** The calls the worker answers, with their arguments and results. */
export interface Calls {
  start: { args: { origin: string; token: string }; result: { persistent: boolean } };
  page: { args: { root?: string; query?: string }; result: Row[] };
  starred: { args: Record<string, never>; result: Crumb[] };
  setStarred: { args: { id: string; starred: boolean }; result: void };
  today: { args: Record<string, never>; result: string };
  path: { args: { id: string }; result: Crumb[] };
  create: { args: { parent?: string; index: number; text: string }; result: string };
  createAfter: { args: { sibling: string; text: string }; result: string };
  setText: { args: { id: string; text: string }; result: void };
  setCompleted: { args: { id: string; completed: boolean }; result: void };
  setCollapsed: { args: { id: string; collapsed: boolean }; result: void };
  indent: { args: { id: string }; result: boolean };
  outdent: { args: { id: string; root?: string }; result: boolean };
  delete: { args: { id: string }; result: void };
  setEditing: { args: { id?: string }; result: void };
  undo: { args: Record<string, never>; result: boolean };
  redo: { args: Record<string, never>; result: boolean };
}

export type Op = keyof Calls;

export interface Request<K extends Op = Op> {
  id: number;
  op: K;
  args: Calls[K]["args"];
}

export type Reply =
  | { id: number; result: unknown }
  | { id: number; error: string }
  | { event: "changed" }
  | { event: "status"; text: string };
