// The top bar: title, search, today's journal page and sync's status.
import { openToday, search } from "../state/page";
import { t } from "../i18n";
import { openSettings } from "../settings/state";
import { phase, status, statusTitle } from "../state/session";

export function Header() {
  const ready = () => phase.value === "ready";
  return (
    <header>
      <h1>Flowie</h1>
      <input
        id="search"
        type="search"
        placeholder="Search"
        autocomplete="off"
        onInput={(event) => {
          if (ready()) search(event.currentTarget.value);
        }}
      />
      <button id="today" onClick={() => void (ready() && openToday())}>
        Today
      </button>
      <span id="status" title={statusTitle.value}>
        {status.value}
      </span>
      <button id="settings" title={t("settings.heading")} aria-label={t("settings.heading")} onClick={() => openSettings()}>
        ⚙
      </button>
    </header>
  );
}
