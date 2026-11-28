// The settings tabs, in order. Each feature adds its own tab as a file in settings/tabs/ (one
// file per tab, as on the desktop: general, appearance, shortcuts, sync, about) that exports
// `tab`; no list to edit.
import type { ComponentType } from "preact";

export interface SettingsTab {
  id: string;
  /** i18n key for the tab's name (the desktop's `settings.tab.*`). */
  title: string;
  /** Position in the list: general 10, appearance 20, shortcuts 30, sync 40, about 90. */
  order: number;
  component: ComponentType;
}

const modules = import.meta.glob<SettingsTab>("./tabs/*.tsx", { eager: true, import: "tab" });

export const tabs: SettingsTab[] = Object.values(modules).sort((a, b) => a.order - b.order);
