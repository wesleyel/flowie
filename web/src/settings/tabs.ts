// The settings tabs, in order. Each feature adds its own tab component here (one file per tab,
// as on the desktop: general, appearance, shortcuts, sync, about).
import type { ComponentType } from "preact";

export interface SettingsTab {
  id: string;
  /** i18n key for the tab's name (the desktop's `settings.tab.*`). */
  title: string;
  component: ComponentType;
}

export const tabs: SettingsTab[] = [];
