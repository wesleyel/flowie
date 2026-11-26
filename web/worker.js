// The outline and its sync run here, in a dedicated worker: OPFS gives SQLite synchronous file
// access only in one, so the outline is kept across reloads. The page sends calls
// ({id, op, args}) and gets answers ({id, result} or {id, error}), and events: {event:
// "changed"} when records from the server changed the outline, {event: "status", text}.
import init, { Outline, install_opfs } from "./pkg/flowie_wasm.js";
import { Sync } from "./sync.js";

let outline;
let sync;

const ops = {
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
      onChange: () => self.postMessage({ event: "changed" }),
      onStatus: (text) => self.postMessage({ event: "status", text }),
    });
    sync.start();
    return { persistent };
  },
  page: ({ root }) => JSON.parse(outline.page(root)),
  create({ parent, index, text }) {
    const id = outline.create(parent, index, text);
    sync.push();
    return id;
  },
  setText({ id, text }) {
    outline.set_text(id, text);
    sync.push();
  },
  setCompleted({ id, completed }) {
    outline.set_completed(id, completed);
    sync.push();
  },
  delete({ id }) {
    outline.delete(id);
    sync.push();
  },
  setEditing: ({ id }) => outline.set_editing(id),
  undo() {
    const done = outline.undo();
    sync.push();
    return done;
  },
};

self.onmessage = async ({ data: { id, op, args } }) => {
  try {
    self.postMessage({ id, result: await ops[op](args ?? {}) });
  } catch (error) {
    self.postMessage({ id, error: String(error?.message ?? error) });
  }
};
