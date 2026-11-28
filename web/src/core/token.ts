// The device token from flowie-server (`token add`), kept in this browser.
const KEY = "flowie.token";

export function token(): string | null {
  return localStorage.getItem(KEY);
}

export function setToken(value: string): void {
  localStorage.setItem(KEY, value);
}
