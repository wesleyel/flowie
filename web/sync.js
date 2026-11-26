// Sync for the browser: moves the bodies the Rust engine asks for to and from flowie-server
// with fetch (per-node-sync.md §5.1). Saves go out as soon as there are any; one fetch waits
// on the server for new records (long polling). A failed call is tried again after a wait
// that doubles from a second up to a minute.

const LONG_POLL = 30;

export class Sync {
  constructor(outline, { base, zone = "outline", token, onChange, onStatus }) {
    this.outline = outline;
    this.url = (path) => `${base.replace(/\/$/, "")}/v1/zones/${zone}/${path}`;
    this.headers = { Authorization: `Bearer ${token}` };
    this.onChange = onChange;
    this.onStatus = onStatus;
    this.saving = false;
    this.caughtUp = false;
    this.failures = 0;
    this.stopped = false;
  }

  start() {
    this.fetchLoop();
    this.push();
  }

  stop() {
    this.stopped = true;
  }

  status(text) {
    this.onStatus?.(text);
  }

  async backoff() {
    const wait = Math.min(1000 * 2 ** Math.max(this.failures - 1, 0), 60000);
    await new Promise((resolve) => setTimeout(resolve, this.failures ? wait : 0));
  }

  // Send what waits; call after each edit.
  async push() {
    if (this.saving || this.stopped) return;
    this.saving = true;
    try {
      let body;
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
          this.failures += 1;
          this.status(`offline: ${error.message ?? error}`);
          await this.backoff();
        }
      }
    } finally {
      this.saving = false;
    }
    this.report();
  }

  async fetchLoop() {
    while (!this.stopped) {
      const params = new URLSearchParams({ limit: "500", wait: this.caughtUp ? String(LONG_POLL) : "0" });
      const cursor = this.outline.cursor();
      if (cursor) params.set("since", cursor);
      try {
        const response = await fetch(`${this.url("changes")}?${params}`, { headers: this.headers });
        if (!response.ok) throw new Error(`the server answered ${response.status}: ${await response.text()}`);
        const body = await response.text();
        this.caughtUp = !JSON.parse(body).more;
        if (this.outline.on_fetched(body)) this.onChange?.();
        this.failures = 0;
        this.report();
        // A merged record may have something newer here to send back.
        this.push();
      } catch (error) {
        this.failures += 1;
        this.status(`offline: ${error.message ?? error}`);
        await this.backoff();
      }
    }
  }

  report() {
    if (this.failures) return;
    this.status(this.caughtUp && !this.outline.has_unsent() ? "up to date" : "syncing…");
  }
}
