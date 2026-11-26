// Sync for the browser: moves the bodies the Rust engine asks for to and from flowie-server
// with fetch (per-node-sync.md §5.1). Saves go out as soon as there are any; one fetch waits
// on the server for new records (long polling). A failed call is tried again after a wait
// that doubles from a second up to a minute.
import type { Outline } from "../pkg/flowie_wasm.js";

const LONG_POLL = 30;

export interface SyncOptions {
  base: string;
  zone?: string;
  token: string;
  onChange?: () => void;
  onStatus?: (text: string) => void;
}

export class Sync {
  private saving = false;
  private caughtUp = false;
  private failures = 0;
  private stopped = false;
  private readonly headers: Record<string, string>;

  constructor(
    private readonly outline: Outline,
    private readonly options: SyncOptions,
  ) {
    this.headers = { Authorization: `Bearer ${options.token}` };
  }

  private url(path: string): string {
    return `${this.options.base.replace(/\/$/, "")}/v1/zones/${this.options.zone ?? "outline"}/${path}`;
  }

  start(): void {
    void this.fetchLoop();
    void this.push();
  }

  stop(): void {
    this.stopped = true;
  }

  private async backoff(): Promise<void> {
    const wait = Math.min(1000 * 2 ** Math.max(this.failures - 1, 0), 60000);
    await new Promise((resolve) => setTimeout(resolve, this.failures ? wait : 0));
  }

  private fail(error: unknown): void {
    this.failures += 1;
    this.options.onStatus?.(`offline: ${error instanceof Error ? error.message : String(error)}`);
  }

  /** Send what waits; call after each edit. */
  async push(): Promise<void> {
    if (this.saving || this.stopped) return;
    this.saving = true;
    try {
      let body: string | undefined;
      while ((body = this.outline.next_saves(200))) {
        try {
          const response = await fetch(this.url("modify"), {
            method: "POST",
            headers: { ...this.headers, "Content-Type": "application/json" },
            body,
          });
          if (!response.ok) throw new Error(`the server answered ${response.status}: ${await response.text()}`);
          this.outline.on_saved(await response.text());
          this.failures = 0;
        } catch (error) {
          this.outline.on_save_failed(body, String(error));
          this.fail(error);
          await this.backoff();
        }
      }
    } finally {
      this.saving = false;
    }
    this.report();
  }

  private async fetchLoop(): Promise<void> {
    while (!this.stopped) {
      const params = new URLSearchParams({ limit: "500", wait: this.caughtUp ? String(LONG_POLL) : "0" });
      const cursor = this.outline.cursor();
      if (cursor) params.set("since", cursor);
      try {
        const response = await fetch(`${this.url("changes")}?${params}`, { headers: this.headers });
        if (!response.ok) throw new Error(`the server answered ${response.status}: ${await response.text()}`);
        const body = await response.text();
        this.caughtUp = !(JSON.parse(body) as { more: boolean }).more;
        if (this.outline.on_fetched(body)) this.options.onChange?.();
        this.failures = 0;
        this.report();
        // A merged record may have something newer here to send back.
        void this.push();
      } catch (error) {
        this.fail(error);
        await this.backoff();
      }
    }
  }

  private report(): void {
    if (this.failures) return;
    this.options.onStatus?.(this.caughtUp && !this.outline.has_unsent() ? "up to date" : "syncing…");
  }
}
