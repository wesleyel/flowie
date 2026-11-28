// The outline and its sync run here, in a dedicated worker: OPFS gives SQLite synchronous file
// access only in one, so the outline is kept across reloads. The page sends calls and gets
// answers, and events when the outline changed from the server or sync's status did.
import init, { Outline, install_opfs } from "../../pkg/flowie_wasm.js";
import type { Calls, Op, Reply, Request } from "../core/protocol";
import { Sync } from "./sync";

const scope = self as unknown as { postMessage(message: Reply): void; onmessage: ((event: MessageEvent<Request>) => void) | null };

let outline: Outline;
let sync: Sync;

/** Run an edit, then send it. */
function edit<T>(f: () => T): T {
  const result = f();
  void sync.push();
  return result;
}

type Handlers = { [K in Op]: (args: Calls[K]["args"]) => Calls[K]["result"] | Promise<Calls[K]["result"]> };

const handlers: Handlers = {
  async start({ origin, token }) {
    await init();
    let persistent = true;
    try {
      await install_opfs();
    } catch (error) {
      // No OPFS (an old browser, a private window): keep the outline in memory.
      persistent = false;
      console.warn("Flowie: OPFS unavailable, keeping the outline in memory:", error);
    }
    outline = new Outline("flowie.sqlite", origin);
    sync = new Sync(outline, {
      base: origin,
      token,
      onChange: () => scope.postMessage({ event: "changed" }),
      onStatus: (text) => scope.postMessage({ event: "status", text }),
    });
    sync.start();
    return { persistent };
  },
  page: ({ root, query }) => JSON.parse(outline.page(root, query)),
  starred: () => JSON.parse(outline.starred()),
  setStarred: ({ id, starred }) => edit(() => outline.set_starred(id, starred)),
  today: () => edit(() => outline.today()),
  path: ({ id }) => JSON.parse(outline.path(id)),
  create: ({ parent, index, text }) => edit(() => outline.create(parent, index, text)),
  createAfter: ({ sibling, text }) => edit(() => outline.create_after(sibling, text)),
  setText: ({ id, text }) => edit(() => outline.set_text(id, text)),
  setCompleted: ({ id, completed }) => edit(() => outline.set_completed(id, completed)),
  setCollapsed: ({ id, collapsed }) => edit(() => outline.set_collapsed(id, collapsed)),
  indent: ({ id }) => edit(() => outline.indent(id)),
  outdent: ({ id, root }) => edit(() => outline.outdent(id, root)),
  delete: ({ id }) => edit(() => outline.delete(id)),
  setEditing: ({ id }) => outline.set_editing(id),
  undo: () => edit(() => outline.undo()),
  redo: () => edit(() => outline.redo()),
};

scope.onmessage = async ({ data: { id, op, args } }) => {
  try {
    const handler = handlers[op] as (args: unknown) => unknown;
    scope.postMessage({ id, result: await handler(args) });
  } catch (error) {
    scope.postMessage({ id, error: error instanceof Error ? error.message : String(error) });
  }
};
