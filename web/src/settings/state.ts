// Which settings tab is open, or none.
import { signal } from "@preact/signals";

export const settingsTab = signal<string | null>(null);

export function openSettings(tab?: string): void {
  settingsTab.value = tab ?? settingsTab.value ?? "general";
}

export function closeSettings(): void {
  settingsTab.value = null;
}
