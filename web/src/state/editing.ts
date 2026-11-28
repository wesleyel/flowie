// The bullet being typed in, and the one to put the caret in once the page is drawn.
import { signal } from "@preact/signals";

/** The row to focus after the next draw of the outline (taken by outline/Outline.tsx). */
export const pendingFocus = signal<string | undefined>(undefined);

/** Whether the caret is in a bullet's text, so a change from the server must wait. */
export function isEditing(): boolean {
  return document.activeElement instanceof HTMLElement && document.activeElement.isContentEditable;
}
