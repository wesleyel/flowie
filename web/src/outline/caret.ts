// Putting the caret in a row's text.

/** Put the caret in row `id`'s text, at its end. */
export function focusRow(id: string): void {
  const text = document.querySelector<HTMLElement>(`.rows [data-id="${CSS.escape(id)}"] .text`);
  if (!text) return;
  text.focus();
  const range = document.createRange();
  range.selectNodeContents(text);
  range.collapse(false);
  const selection = getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
}
