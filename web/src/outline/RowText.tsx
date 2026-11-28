// A row's text, edited in place. The DOM text is the source while the caret is in it: a draw
// of the page never writes into the focused row, so typing and the caret are left alone.
import { useLayoutEffect, useRef } from "preact/hooks";
import type { Row } from "../core/protocol";
import { rows } from "../state/page";
import { indent, newAfter, removeEmpty, saveText, setEditing } from "./actions";
import { focusRow } from "./caret";

export function RowText({ row, index }: { row: Row; index: number }) {
  const ref = useRef<HTMLSpanElement>(null);
  /** The text the worker has, so leaving an unchanged row saves nothing. */
  const saved = useRef(row.text);
  saved.current = row.text;

  // Show the worker's text, unless the row is being typed in.
  useLayoutEffect(() => {
    const text = ref.current!;
    if (document.activeElement !== text && text.textContent !== row.text) text.textContent = row.text;
  });

  const current = () => ref.current!.textContent ?? "";
  const save = async () => {
    const text = current();
    if (text === saved.current) return;
    await saveText(row.id, text);
    saved.current = text;
  };

  const onKeyDown = async (event: KeyboardEvent) => {
    const move = async (to: Row | undefined) => {
      if (!to) return;
      await save();
      focusRow(to.id);
    };
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      await save();
      await newAfter(row);
    } else if (event.key === "Tab") {
      event.preventDefault();
      await save();
      await indent(row, event.shiftKey);
    } else if (event.key === "Backspace" && !current() && !row.has_children) {
      event.preventDefault();
      await removeEmpty(row, index);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      await move(rows.value[index - 1]);
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      await move(rows.value[index + 1]);
    }
  };

  return (
    <span
      ref={ref}
      class="text"
      contentEditable
      onFocus={() => void setEditing(row.id)}
      onBlur={async () => {
        await save();
        await setEditing(undefined);
      }}
      onKeyDown={onKeyDown}
    />
  );
}
