// The way from Home down to the page shown.
import { Fragment } from "preact";
import { crumbs, loaded } from "../state/page";

export function Crumbs() {
  return (
    <nav id="crumbs">
      {loaded.value && <a href="#">Home</a>}
      {crumbs.value.map((crumb) => (
        <Fragment key={crumb.id}>
          {" › "}
          <a href={`#${crumb.id}`}>{crumb.text || "Untitled"}</a>
        </Fragment>
      ))}
    </nav>
  );
}
