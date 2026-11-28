// The page's rows, keyed by bullet, and the form that adds a bullet at the end.
import { useLayoutEffect, useRef } from "preact/hooks";
import { pendingFocus } from "../state/editing";
import { rows } from "../state/page";
import { addAtEnd } from "./actions";
import { focusRow } from "./caret";
import { Row } from "./Row";

export function Outline() {
  const list = rows.value;
  // The caret goes where the last action asked, once its rows are in the DOM.
  useLayoutEffect(() => {
    const id = pendingFocus.peek();
    if (!id) return;
    pendingFocus.value = undefined;
    focusRow(id);
  });
  return (
    <>
      <ul class="rows">
        {list.map((row, index) => (
          <Row key={row.id} row={row} index={index} />
        ))}
      </ul>
      <AddForm />
    </>
  );
}

function AddForm() {
  const input = useRef<HTMLInputElement>(null);
  const onSubmit = async (event: SubmitEvent) => {
    event.preventDefault();
    const value = input.current!.value.trim();
    if (!value) return;
    await addAtEnd(value);
    input.current!.value = "";
    input.current!.focus();
  };
  return (
    <form class="add" onSubmit={onSubmit}>
      <input ref={input} name="text" placeholder="New bullet" autocomplete="off" />
      <button>Add</button>
    </form>
  );
}
