// Links to the starred pages.
import { Fragment } from "preact";
import { starred } from "../state/page";

export function Starred() {
  return (
    <nav id="starred">
      {starred.value.map((page, i) => (
        <Fragment key={page.id}>
          {i ? " " : null}
          <a href={`#${page.id}`}>★ {page.text || "Untitled"}</a>
        </Fragment>
      ))}
    </nav>
  );
}
