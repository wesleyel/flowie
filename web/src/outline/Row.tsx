// One bullet: fold, dot (open as a page), checkbox, text (and picture), star, remove. Memoized
// on the row's fields, so a draw only touches rows that changed.
import { memo } from "preact/compat";
import type { Row as RowData } from "../core/protocol";
import { remove, setCompleted, toggleCollapsed, toggleStarred } from "./actions";
import { RowImage } from "./RowImage";
import { RowText } from "./RowText";

interface Props {
  row: RowData;
  index: number;
}

function RowView({ row, index }: Props) {
  const text = <RowText row={row} index={index} />;
  return (
    <li data-id={row.id} class={row.completed ? "row done" : "row"} style={{ "--depth": row.depth }}>
      <button class="fold" disabled={!row.has_children} title={row.collapsed ? "Expand" : "Collapse"} onClick={() => void toggleCollapsed(row)}>
        {row.has_children ? (row.collapsed ? "▸" : "▾") : ""}
      </button>
      <a class="dot" href={`#${row.id}`} title="Open as a page">
        •
      </a>
      <input type="checkbox" checked={row.completed} onChange={(event) => void setCompleted(row, event.currentTarget.checked)} />
      {row.image ? (
        <span class="body">
          {text}
          <RowImage name={row.image} text={row.text} />
        </span>
      ) : (
        text
      )}
      <button class={row.starred ? "star on" : "star"} title={row.starred ? "Unstar" : "Star"} onClick={() => void toggleStarred(row)}>
        {row.starred ? "★" : "☆"}
      </button>
      <button class="remove" title="Move to Trash" onClick={() => void remove(row)}>
        ×
      </button>
    </li>
  );
}

function sameRow(a: RowData, b: RowData): boolean {
  return (Object.keys(a) as (keyof RowData)[]).every((key) => a[key] === b[key]);
}

export const Row = memo(RowView, (prev, next) => prev.index === next.index && sameRow(prev.row, next.row));
