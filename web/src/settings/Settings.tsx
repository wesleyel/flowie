// Settings, as a dialog over the page: a tab list and the open tab (⌘, opens it, Esc closes it).
import { useEffect, useRef } from "preact/hooks";
import { t } from "../i18n";
import { closeSettings, settingsTab } from "./state";
import { tabs } from "./tabs";

export function Settings() {
  const dialog = useRef<HTMLDialogElement>(null);
  const open = settingsTab.value;
  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (open && !element.open) element.showModal();
    else if (!open && element.open) element.close();
  }, [open]);
  const current = tabs.find((tab) => tab.id === open) ?? tabs[0];
  return (
    <dialog ref={dialog} class="settings" onClose={closeSettings} aria-label={t("settings.heading")}>
      {open && (
        <>
          <nav class="settings-tabs" role="tablist">
            {tabs.map((tab) => (
              <button
                key={tab.id}
                role="tab"
                aria-selected={tab === current}
                class={tab === current ? "on" : ""}
                onClick={() => (settingsTab.value = tab.id)}
              >
                {t(tab.title)}
              </button>
            ))}
          </nav>
          <section class="settings-body" role="tabpanel">
            {current ? <current.component /> : null}
          </section>
        </>
      )}
    </dialog>
  );
}
