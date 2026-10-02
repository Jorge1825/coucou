// Interface language. English is the source: every string in the code is the
// English text, wrapped in t(); the other languages map that text to theirs.
// A string missing from a dictionary simply shows in English.
//
// The language is fixed when a window loads. Changing it in Settings reloads
// the islands and the settings window (see main.ts and settings/main.ts).

import { ES } from "./i18n/es";
import { RU } from "./i18n/ru";
import { ZH } from "./i18n/zh";

export type Lang = "en" | "es" | "ru" | "zh";

/** What the language select offers. "auto" follows Windows' display language. */
export const LANGUAGES: { id: "auto" | Lang; label: string }[] = [
  { id: "auto", label: "Auto" },
  { id: "en", label: "English" },
  { id: "es", label: "Español" },
  { id: "ru", label: "Русский" },
  { id: "zh", label: "中文" },
];

const DICTS: Record<Lang, Record<string, string> | null> = { en: null, es: ES, ru: RU, zh: ZH };

let current: Lang = "en";

/** "auto", "es", "zh-CN"… → one of the supported languages. */
export function resolveLanguage(pref: string | undefined): Lang {
  const pick = (code: string): Lang | null => {
    const base = code.toLowerCase().split(/[-_]/)[0];
    return base === "en" || base === "es" || base === "ru" || base === "zh" ? base : null;
  };
  if (pref && pref !== "auto") return pick(pref) ?? "en";
  for (const code of navigator.languages ?? [navigator.language]) {
    const lang = pick(code);
    if (lang) return lang;
  }
  return "en";
}

export function setLanguage(pref: string | undefined) {
  current = resolveLanguage(pref);
  document.documentElement.lang = current;
}

export function language(): Lang {
  return current;
}

/** Translates an English string; `{name}` placeholders are filled from `vars`. */
export function t(text: string, vars?: Record<string, string | number>): string {
  const out = DICTS[current]?.[text] ?? text;
  if (!vars) return out;
  return out.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m));
}

/** Locale for dates and numbers. */
export function locale(): string {
  return { en: "en-US", es: "es-ES", ru: "ru-RU", zh: "zh-CN" }[current];
}
