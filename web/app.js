// Flowie in the browser: the basic outline (list, add, edit, tick off, delete) over the Rust
// core in wasm, synced with the flowie-server this page comes from.
import init, { Outline } from "./pkg/flowie_wasm.js";
import { Sync } from "./sync.js";

const main = document.getElementById("main");
const status = document.getElementById("status");

function setup() {
  main.innerHTML = `<form class="setup"><label>Device token<br><input name="token" autocomplete="off" placeholder="from flowie-server token add"></label><button>Connect</button></form>`;
  main.querySelector("form").addEventListener("submit", (event) => {
    event.preventDefault();
    localStorage.setItem("flowie.token", event.target.token.value.trim());
    location.reload();
  });
  status.textContent = "not connected";
}

async function run() {
  await init();
  const token = localStorage.getItem("flowie.token");
  if (!token) return setup();
  const outline = new Outline("flowie.sqlite", location.origin);
  let sync;
  const render = () => {
    const rows = JSON.parse(outline.page(undefined));
    main.innerHTML = "";
    const list = document.createElement("ul");
    for (const row of rows) {
      const item = document.createElement("li");
      item.style.paddingLeft = `${row.depth * 24}px`;
      item.className = row.completed ? "done" : "";
      const box = document.createElement("input");
      box.type = "checkbox";
      box.checked = row.completed;
      box.addEventListener("change", () => {
        outline.set_completed(row.id, box.checked);
        render();
        sync.push();
      });
      const text = document.createElement("span");
      text.className = "text";
      text.contentEditable = "true";
      text.textContent = row.text;
      text.addEventListener("focus", () => outline.set_editing(row.id));
      text.addEventListener("blur", () => {
        if (text.textContent !== row.text) outline.set_text(row.id, text.textContent);
        outline.set_editing(undefined);
        render();
        sync.push();
      });
      const remove = document.createElement("button");
      remove.textContent = "×";
      remove.title = "Move to Trash";
      remove.addEventListener("click", () => {
        outline.delete(row.id);
        render();
        sync.push();
      });
      item.append(box, text, remove);
      list.append(item);
    }
    const form = document.createElement("form");
    form.innerHTML = `<input name="text" placeholder="New bullet" autocomplete="off"><button>Add</button>`;
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      const value = form.text.value.trim();
      if (!value) return;
      outline.create(undefined, rows.filter((r) => r.depth === 0).length, value);
      render();
      sync.push();
      document.querySelector("form input")?.focus();
    });
    main.append(list, form);
  };
  sync = new Sync(outline, {
    base: location.origin,
    token,
    onChange: () => {
      // Don't redraw under someone typing.
      if (!document.activeElement?.isContentEditable) render();
    },
    onStatus: (text) => (status.textContent = text),
  });
  render();
  sync.start();
}

run().catch((error) => {
  status.textContent = `error: ${error.message ?? error}`;
  console.error(error);
});
