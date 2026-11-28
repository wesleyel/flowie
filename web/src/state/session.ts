// Where the page is in starting up, and sync's status for the top bar.
import { signal } from "@preact/signals";

/** `setup` asks for a device token; `ready` once the worker has opened the outline. */
export type Phase = "starting" | "setup" | "ready";

export const phase = signal<Phase>("starting");
/** Sync's status line ("up to date", "syncing…", "offline: …"), or why the page stopped. */
export const status = signal("starting…");
/** More about the status, shown on hover. */
export const statusTitle = signal<string | undefined>(undefined);
