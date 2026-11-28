// The page: top bar, starred pages, breadcrumbs, then the token form or the outline.
import { Crumbs } from "../chrome/Crumbs";
import { Header } from "../chrome/Header";
import { Starred } from "../chrome/Starred";
import { Outline } from "../outline/Outline";
import { Settings } from "../settings/Settings";
import { Setup } from "../setup/Setup";
import { loaded } from "../state/page";
import { phase } from "../state/session";

export function App() {
  return (
    <>
      <Header />
      <Starred />
      <Crumbs />
      <Settings />
      <main id="main">{phase.value === "setup" ? <Setup /> : phase.value === "ready" && loaded.value ? <Outline /> : null}</main>
    </>
  );
}
