import { derived, get, writable } from "svelte/store";
import zhTW from "./locales/zh-TW.json";
import en from "./locales/en.json";
import ja from "./locales/ja.json";

export type Locale = "zh-TW" | "en" | "ja";

export interface LocaleOption {
  code: Locale;
  label: string;
}

export const LOCALES: readonly LocaleOption[] = [
  { code: "zh-TW", label: "繁體中文" },
  { code: "en", label: "English" },
  { code: "ja", label: "日本語" },
] as const;

const STORAGE_KEY = "vocalsync.locale";
const DEFAULT_LOCALE: Locale = "zh-TW";

type Dictionary = Record<string, string>;

const messages: Record<Locale, Dictionary> = {
  "zh-TW": zhTW as Dictionary,
  en: en as Dictionary,
  ja: ja as Dictionary,
};

function isLocale(value: unknown): value is Locale {
  return value === "zh-TW" || value === "en" || value === "ja";
}

function detectLocale(): Locale {
  if (typeof localStorage !== "undefined") {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (isLocale(saved)) return saved;
  }
  if (typeof navigator !== "undefined") {
    const lang = navigator.language?.toLowerCase() ?? "";
    if (lang.startsWith("ja")) return "ja";
    if (lang.startsWith("zh")) return "zh-TW";
    if (lang.startsWith("en")) return "en";
  }
  return DEFAULT_LOCALE;
}

export const locale = writable<Locale>(detectLocale());

locale.subscribe((value) => {
  if (typeof localStorage !== "undefined") {
    localStorage.setItem(STORAGE_KEY, value);
  }
  if (typeof document !== "undefined") {
    document.documentElement.lang = value;
  }
});

type Vars = Record<string, string | number>;

function format(template: string, vars?: Vars): string {
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (_, key) => {
    const v = vars[key];
    return v === undefined ? `{${key}}` : String(v);
  });
}

function resolve(loc: Locale, key: string): string {
  const dict = messages[loc];
  if (key in dict) return dict[key];
  const fallback = messages[DEFAULT_LOCALE];
  if (key in fallback) return fallback[key];
  return key;
}

/**
 * Reactive translator. In Svelte components use `$t('key', { ...vars })`.
 */
export const t = derived(locale, ($locale) => {
  return (key: string, vars?: Vars): string => format(resolve($locale, key), vars);
});

export function setLocale(value: Locale): void {
  locale.set(value);
}

export function getLocale(): Locale {
  return get(locale);
}

/**
 * Non-reactive translator for imperative contexts (toasts, errors emitted
 * outside component render). Reads the current locale at call time.
 */
export function tSync(key: string, vars?: Vars): string {
  return format(resolve(get(locale), key), vars);
}
