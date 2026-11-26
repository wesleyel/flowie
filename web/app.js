// Flowie in the browser: the basic outline (list, add, edit, tick off, delete) over the Rust
// core in wasm, which runs in a worker (worker.js) with the outline kept in OPFS, synced with
// the flowie-server this page comes from.

const main = document.getElementById("main");
const status = document.getElementById("status");

const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
const waiting = new Map();
let next = 0;
let render = () => {};

function call(op, args) {
  const id = next++;
  worker.postMessage({ id, op, args });
  return new Promise((resolve, reject) => waiting.set(id, { resolve, reject }));
}

worker.onmessage = ({ data }) => {
  if (data.event === "status") status.textContent = data.text;
  else if (data.event === "changed") {
    // Don't redraw under someone typing.
    if (!document.activeElement?.isContentEditable) render();
  } else {
    const { resolve, reject } = waiting.get(data.id);
    waiting.delete(data.id);
    if ("error" in data) reject(new Error(data.error));
    else resolve(data.result);
  }
};

function setup() {
  main.innerHTML = `<form class="setup"><label>Device token<br><input name="token" autocomplete="off" placeholder="from flowie-server token add"></label><button>Connect</button></form>`;
  main.querySelector("form").addEventListener("submit", (event) => {
    event.preventDefault();
    localStorage.setItem("flowie.token", event.target.token.value.trim());
    location.reload();
  });
  status.textContent = "not connected";
}

async function draw() {
  const rows = await call("page", {});
  main.innerHTML = "";
  const list = document.createElement("ul");
  for (const row of rows) {
    const item = document.createElement("li");
    item.style.paddingLeft = `${row.depth * 24}px`;
    item.className = row.completed ? "done" : "";
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = row.completed;
    box.addEventListener("change", async () => {
      await call("setCompleted", { id: row.id, completed: box.checked });
      render();
    });
    const text = document.createElement("span");
    text.className = "text";
    text.contentEditable = "true";
    text.textContent = row.text;
    text.addEventListener("focus", () => call("setEditing", { id: row.id }));
    text.addEventListener("blur", async () => {
      if (text.textContent !== row.text) await call("setText", { id: row.id, text: text.textContent });
      await call("setEditing", {});
      render();
    });
    const remove = document.createElement("button");
    remove.textContent = "×";
    remove.title = "Move to Trash";
    remove.addEventListener("click", async () => {
      await call("delete", { id: row.id });
      render();
    });
    item.append(box, text, remove);
    list.append(item);
  }
  const form = document.createElement("form");
  form.innerHTML = `<input name="text" placeholder="New bullet" autocomplete="off"><button>Add</button>`;
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const value = form.text.value.trim();
    if (!value) return;
    await call("create", { index: rows.filter((r) => r.depth === 0).length, text: value });
    await draw();
    main.querySelector("form input")?.focus();
  });
  main.append(list, form);
}

async function run() {
  const token = localStorage.getItem("flowie.token");
  if (!token) return setup();
  const { persistent } = await call("start", { origin: location.origin, token });
  if (!persistent) status.title = "This browser can't keep the outline: it is fetched again from the server each time.";
  render = () => draw().catch((error) => (status.textContent = `error: ${error.message}`));
  render();
}

run().catch((error) => {
  status.textContent = `error: ${error.message ?? error}`;
  console.error(error);
});
