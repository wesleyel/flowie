// Flowie in the browser: the outline as pages of bullets (zoom into any bullet, breadcrumbs
// back), with keyboard editing, over the Rust core in wasm. The core runs in a worker
// (worker.ts) with the outline kept in OPFS, synced with the flowie-server this page comes from.
import type { Calls, Op, Reply, Row } from "./protocol.js";

const main = document.getElementById("main")!;
const status = document.getElementById("status")!;
const crumbs = document.getElementById("crumbs")!;
const starredBar = document.getElementById("starred")!;
const search = document.getElementById("search") as HTMLInputElement;
const todayButton = document.getElementById("today") as HTMLButtonElement;

const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
const waiting = new Map<number, { resolve: (value: unknown) => void; reject: (error: Error) => void }>();
let next = 0;

function call<K extends Op>(op: K, args: Calls[K]["args"]): Promise<Calls[K]["result"]> {
  const id = next++;
  worker.postMessage({ id, op, args });
  return new Promise((resolve, reject) => waiting.set(id, { resolve: resolve as (value: unknown) => void, reject }));
}

worker.onmessage = ({ data }: MessageEvent<Reply>) => {
  if ("event" in data) {
    if (data.event === "status") status.textContent = data.text;
    // Don't redraw under someone typing.
    else if (!editing()) void draw();
    return;
  }
  const waiter = waiting.get(data.id);
  waiting.delete(data.id);
  if ("error" in data) waiter?.reject(new Error(data.error));
  else waiter?.resolve(data.result);
};

/** The page shown: the bullet zoomed into (from the address), or Home. */
function zoomed(): string | undefined {
  return location.hash.slice(1) || undefined;
}

function editing(): boolean {
  return document.activeElement instanceof HTMLElement && document.activeElement.isContentEditable;
}

/** Put the caret in row `id`'s text, at its end. */
function focusRow(id: string): void {
  const text = main.querySelector<HTMLElement>(`[data-id="${id}"] .text`);
  if (!text) return;
  text.focus();
  const range = document.createRange();
  range.selectNodeContents(text);
  range.collapse(false);
  const selection = getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
}

async function drawStarred(): Promise<void> {
  const starred = await call("starred", {});
  starredBar.replaceChildren(
    ...starred.flatMap((page, i) => {
      const link = document.createElement("a");
      link.href = `#${page.id}`;
      link.textContent = `★ ${page.text || "Untitled"}`;
      return i ? [" ", link] : [link];
    }),
  );
}

async function drawCrumbs(root: string | undefined): Promise<void> {
  crumbs.innerHTML = "";
  const home = document.createElement("a");
  home.href = "#";
  home.textContent = "Home";
  crumbs.append(home);
  if (!root) return;
  for (const crumb of await call("path", { id: root })) {
    const link = document.createElement("a");
    link.href = `#${crumb.id}`;
    link.textContent = crumb.text || "Untitled";
    crumbs.append(" › ", link);
  }
}

/** Image files, by name, as object URLs: fetched once with the device token. */
const images = new Map<string, Promise<string>>();

function imageUrl(name: string): Promise<string> {
  let url = images.get(name);
  if (!url) {
    const token = localStorage.getItem("flowie.token") ?? "";
    url = fetch(`/v1/blobs/${encodeURIComponent(name)}`, { headers: { Authorization: `Bearer ${token}` } }).then(async (response) => {
      if (!response.ok) throw new Error(`image ${name}: ${response.status}`);
      return URL.createObjectURL(await response.blob());
    });
    url.catch(() => images.delete(name));
    images.set(name, url);
  }
  return url;
}

function rowElement(row: Row, rows: Row[], index: number): HTMLLIElement {
  const root = zoomed();
  const item = document.createElement("li");
  item.dataset.id = row.id;
  item.style.paddingLeft = `${row.depth * 24}px`;
  item.className = row.completed ? "done" : "";

  const fold = document.createElement("button");
  fold.className = "fold";
  fold.textContent = row.has_children ? (row.collapsed ? "▸" : "▾") : "";
  fold.disabled = !row.has_children;
  fold.title = row.collapsed ? "Expand" : "Collapse";
  fold.addEventListener("click", async () => {
    await call("setCollapsed", { id: row.id, collapsed: !row.collapsed });
    await draw();
  });

  const dot = document.createElement("a");
  dot.className = "dot";
  dot.href = `#${row.id}`;
  dot.textContent = "•";
  dot.title = "Open as a page";

  const star = document.createElement("button");
  star.className = row.starred ? "star on" : "star";
  star.textContent = row.starred ? "★" : "☆";
  star.title = row.starred ? "Unstar" : "Star";
  star.addEventListener("click", async () => {
    await call("setStarred", { id: row.id, starred: !row.starred });
    await draw();
  });

  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = row.completed;
  box.addEventListener("change", async () => {
    await call("setCompleted", { id: row.id, completed: box.checked });
    await draw();
  });

  const text = document.createElement("span");
  text.className = "text";
  text.contentEditable = "true";
  text.textContent = row.text;
  const save = async () => {
    if (text.textContent !== row.text) {
      await call("setText", { id: row.id, text: text.textContent ?? "" });
      row.text = text.textContent ?? "";
    }
  };
  text.addEventListener("focus", () => void call("setEditing", { id: row.id }));
  text.addEventListener("blur", async () => {
    await save();
    await call("setEditing", {});
  });
  text.addEventListener("keydown", async (event) => {
    const move = async (to: Row | undefined) => {
      if (!to) return;
      await save();
      focusRow(to.id);
    };
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      await save();
      const id = await call("createAfter", { sibling: row.id, text: "" });
      await draw(id);
    } else if (event.key === "Tab") {
      event.preventDefault();
      await save();
      if (event.shiftKey) await call("outdent", { id: row.id, root });
      else await call("indent", { id: row.id });
      await draw(row.id);
    } else if (event.key === "Backspace" && !text.textContent && !row.has_children) {
      event.preventDefault();
      await call("delete", { id: row.id });
      await draw(rows[index - 1]?.id);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      await move(rows[index - 1]);
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      await move(rows[index + 1]);
    }
  });

  const remove = document.createElement("button");
  remove.className = "remove";
  remove.textContent = "×";
  remove.title = "Move to Trash";
  remove.addEventListener("click", async () => {
    await call("delete", { id: row.id });
    await draw();
  });

  if (row.image) {
    const picture = document.createElement("img");
    picture.className = "image";
    picture.alt = row.text;
    imageUrl(row.image).then(
      (url) => (picture.src = url),
      () => (picture.alt = `${row.text} (image not on the server)`),
    );
    const body = document.createElement("span");
    body.className = "body";
    body.append(text, picture);
    item.append(fold, dot, box, body, star, remove);
  } else {
    item.append(fold, dot, box, text, star, remove);
  }
  return item;
}

/** Draw the page shown, then put the caret in `focus` if given. */
async function draw(focus?: string): Promise<void> {
  const root = zoomed();
  let rows: Row[];
  try {
    rows = await call("page", { root, query: search.value || undefined });
  } catch {
    // The page's bullet is gone (deleted elsewhere): go Home.
    location.hash = "";
    return;
  }
  await Promise.all([drawCrumbs(root), drawStarred()]);
  const list = document.createElement("ul");
  rows.forEach((row, index) => list.append(rowElement(row, rows, index)));
  const form = document.createElement("form");
  form.innerHTML = `<input name="text" placeholder="New bullet" autocomplete="off"><button>Add</button>`;
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const input = form.elements.namedItem("text") as HTMLInputElement;
    const value = input.value.trim();
    if (!value) return;
    await call("create", { parent: root, index: rows.filter((r) => r.depth === 0).length, text: value });
    await draw();
    main.querySelector<HTMLInputElement>("form input")?.focus();
  });
  main.replaceChildren(list, form);
  if (focus) focusRow(focus);
}

function setup(): void {
  main.innerHTML = `<form class="setup"><label>Device token<br><input name="token" autocomplete="off" placeholder="from flowie-server token add"></label><button>Connect</button></form>`;
  main.querySelector("form")!.addEventListener("submit", (event) => {
    event.preventDefault();
    const input = (event.target as HTMLFormElement).elements.namedItem("token") as HTMLInputElement;
    localStorage.setItem("flowie.token", input.value.trim());
    location.reload();
  });
  status.textContent = "not connected";
}

async function run(): Promise<void> {
  const token = localStorage.getItem("flowie.token");
  if (!token) return setup();
  const { persistent } = await call("start", { origin: location.origin, token });
  if (!persistent) status.title = "This browser can't keep the outline: it is fetched again from the server each time.";
  addEventListener("hashchange", () => void draw());
  let typing: ReturnType<typeof setTimeout> | undefined;
  search.addEventListener("input", () => {
    clearTimeout(typing);
    typing = setTimeout(() => void draw(), 150);
  });
  todayButton.addEventListener("click", async () => {
    location.hash = await call("today", {});
  });
  // Undo and redo for the outline, like the desktop: typing in a bullet is saved first, so it
  // is part of what undo takes back.
  addEventListener("keydown", async (event) => {
    if (!(event.metaKey || event.ctrlKey) || event.target === search) return;
    const key = event.key.toLowerCase();
    const redo = (key === "z" && event.shiftKey) || (key === "y" && event.ctrlKey);
    if (key !== "z" && !redo) return;
    event.preventDefault();
    const active = document.activeElement;
    if (active instanceof HTMLElement && active.isContentEditable) active.blur();
    if (await call(redo ? "redo" : "undo", {})) await draw();
  });
  await draw();
}

// Keep the page's files for opening offline (sw.js); the outline is in OPFS already.
if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register("sw.js").catch((error: unknown) => console.warn("Flowie: no offline page:", error));
}

run().catch((error: unknown) => {
  status.textContent = `error: ${error instanceof Error ? error.message : String(error)}`;
  console.error(error);
});
