// UI text: `t("group.key", { name: value })`. Keys are looked up in the browser's own catalogs
// first, then in the desktop app's
// (crates/flowie-app/locales/*.json), which the browser reuses but never adds keys to: the
// desktop's checks fail on keys its code doesn't use. The browser's text is one pair of files per
// feature, `i18n/locales/<feature>.en.json` and `<feature>.zh-CN.json`, whose keys sit under
// `web.<feature>.` (so `sync.en.json` holding `{"title": …}` is `t("web.sync.title")`). Same format as the desktop: nested groups,
// `{name}` placeholders, and `{ "one": …, "other": … }` plurals chosen by `count`.
import { computed, signal } from "@preact/signals";
import desktopEn from "../../../crates/flowie-app/locales/en.json";
import desktopZh from "../../../crates/flowie-app/locales/zh-CN.json";

type Plural = { one?: string; other: string };
type Message = string | Plural;
type Group = { [key: string]: Message | Group };

function flatten(group: Group, prefix = "", out = new Map<string, Message>()): Map<string, Message> {
  for (const [key, value] of Object.entries(group)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (typeof value === "string") out.set(path, value);
    else if ("other" in value && Object.keys(value).every((k) => k === "one" || k === "other")) out.set(path, value as Plural);
    else flatten(value as Group, path, out);
  }
  return out;
}

const features = import.meta.glob<Group>("./locales/*.json", { eager: true, import: "default" });

/** The browser's own text in `code`: every `<feature>.<code>.json`, under `web.<feature>`. */
function web(code: string): Group {
  const group: Group = {};
  for (const [path, messages] of Object.entries(features)) {
    const match = /\/([\w-]+)\.([\w-]+)\.json$/.exec(path);
    if (match?.[2] === code) group[match[1]] = messages;
  }
  return { web: group };
}

const catalogs = {
  en: new Map([...flatten(desktopEn as Group), ...flatten(web("en"))]),
  "zh-CN": new Map([...flatten(desktopZh as Group), ...flatten(web("zh-CN"))]),
};

export type Locale = keyof typeof catalogs;
export const locales: Locale[] = ["en", "zh-CN"];

const STORAGE = "flowie.locale";

function stored(): Locale | "" {
  try {
    const value = localStorage.getItem(STORAGE) ?? "";
    return value in catalogs ? (value as Locale) : "";
  } catch {
    return "";
  }
}

function system(): Locale {
  return navigator.languages.some((l) => l.toLowerCase().startsWith("zh")) ? "zh-CN" : "en";
}

/** The chosen language, or "" to follow the browser. */
export const chosenLocale = signal<Locale | "">(stored());
/** The language in use; components that call `t` re-render when it changes. */
export const locale = computed<Locale>(() => chosenLocale.value || system());

export function setLocale(code: Locale | ""): void {
  chosenLocale.value = code;
  try {
    if (code) localStorage.setItem(STORAGE, code);
    else localStorage.removeItem(STORAGE);
  } catch {
    // Private windows may refuse storage: the choice lasts until the page closes.
  }
  document.documentElement.lang = locale.value;
}

/** The language's own name (`meta.name`), for the language picker. */
export function languageName(code: Locale): string {
  return String(catalogs[code].get("meta.name") ?? code);
}

/** The text for `key` in the current language (English, then the key itself, when missing). */
export function t(key: string, vars: Record<string, string | number> = {}): string {
  const message = catalogs[locale.value].get(key) ?? catalogs.en.get(key);
  if (message === undefined) {
    if (import.meta.env.DEV) console.warn(`Flowie: no text for ${key}`);
    return key;
  }
  const text = typeof message === "string" ? message : Number(vars.count) === 1 && message.one !== undefined ? message.one : message.other;
  return text.replace(/\{(\w+)\}/g, (whole, name: string) => (name in vars ? String(vars[name]) : whole));
}

document.documentElement.lang = locale.value;
