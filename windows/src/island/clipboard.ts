// Smart clipboard → the island. Rust sends copied text (only while the user has
// switched this on, and never what looks private). It goes into a short history
// that expires on its own, and a discreet banner offers what can be done with
// it: explain, summarize, translate, fix. An action asks the chat — nothing is
// sent anywhere until one is clicked.

import { onEvent, windowLabel } from "../core/bridge";
import { language, t } from "../core/i18n";
import { State, type ClipItem } from "../core/state";
import { askChat } from "../views/chat";
import type { Island } from "./island";

export type ClipAction = "explain" | "summarize" | "translate" | "fix";
export const CLIP_ACTIONS: ClipAction[] = ["explain", "summarize", "translate", "fix"];

/** Names for "translate into …" — what the model is told. */
export const TRANSLATE_LANGUAGES: Record<string, string> = {
  en: "English", es: "Spanish", ru: "Russian", zh: "Simplified Chinese", fr: "French",
  de: "German", pt: "Portuguese", it: "Italian", ja: "Japanese",
};

export type ClipKind = "error" | "code" | "link" | "text";

/** A rough idea of what was copied, for the banner and the explain prompt. */
export function clipKind(text: string): ClipKind {
  const s = text.trim();
  if (/^https?:\/\/\S+$/i.test(s)) return "link";
  // "TypeError", "NullPointerException"… end in the word; stack frames give it away too.
  if (
    /\w*(error|exception)\b|\b(traceback|panicked|fatal|failed|errno|stack trace|undefined is not)\b/i.test(s) ||
    /^\s+at .+:\d+(:\d+)?\)?$/m.test(s)
  ) {
    return "error";
  }
  const codeHints = (s.match(/[{};=<>]|=>|\bfunction\b|\bdef\b|\bclass\b|\bimport\b|\bconst\b|\blet\b|\breturn\b/g) ?? [])
    .length;
  if (codeHints >= 4 || /\n\s{2,}\S/.test(s)) return "code";
  return "text";
}

export function clipKindLabel(kind: ClipKind): string {
  switch (kind) {
    case "error":
      return t("Error copied");
    case "code":
      return t("Code copied");
    case "link":
      return t("Link copied");
    default:
      return t("Text copied");
  }
}

export function clipActionLabel(a: ClipAction): string {
  return { explain: t("Explain"), summarize: t("Summarize"), translate: t("Translate"), fix: t("Fix") }[a];
}

const preview = (s: string, n: number) => {
  const one = s.replace(/\s+/g, " ").trim();
  return one.length > n ? `${one.slice(0, n - 1)}…` : one;
};

let island: Island | null = null;
let nextId = 1;
let sweepTimer: number | null = null;

export function registerClipboardHandlers(isl: Island) {
  island = isl;
  void onEvent<Copied>("clipboard", (c) => handle(c));
}

interface Copied {
  text: string;
  at: number;
  /** The island on the display with the cursor at copy time. */
  cursorIsland?: string;
}

/** Does this island show the suggestion banner (Settings → "Show suggestions on")? */
function showsHere(c: Copied): boolean {
  const me = windowLabel();
  switch (State.settings.clipboard?.screens ?? "all") {
    case "cursor":
      return (c.cursorIsland ?? "island") === me;
    case "main":
      return me === "island";
    default:
      return true;
  }
}

function handle(c: Copied) {
  const p = State.settings.clipboard;
  if (!p?.enabled || State.paused) return;
  const keep = Math.max(1, p.history || 1);
  State.clips = [{ id: nextId++, text: c.text, at: c.at }, ...State.clips.filter((x) => x.text !== c.text)].slice(0, keep);
  State.clipIndex = 0;
  startSweeping();

  // Copying while the island is open (say, from the chat) must not interrupt it.
  if (p.suggest && (p.actions?.length ?? 0) > 0 && State.mode !== "expanded" && island && showsHere(c)) {
    island.showBanner({
      key: "clipboard",
      title: clipKindLabel(clipKind(c.text)),
      text: `“${preview(c.text, 48)}”`,
      open: "clipboard",
      glance: false,
    });
  }
  State.notify();
}

/** Copies older than the configured minutes leave the history. */
function sweep() {
  if (State.mode === "expanded" && State.view === "clipboard") return;
  const minutes = State.settings.clipboard?.keepMinutes ?? 30;
  const cutoff = Date.now() - minutes * 60_000;
  const kept = State.clips.filter((c) => c.at > cutoff);
  if (kept.length !== State.clips.length) {
    State.clips = kept;
    State.clipIndex = Math.min(State.clipIndex, Math.max(0, kept.length - 1));
    State.notify();
  }
  if (kept.length === 0 && sweepTimer != null) {
    window.clearInterval(sweepTimer);
    sweepTimer = null;
  }
}

function startSweeping() {
  if (sweepTimer == null) sweepTimer = window.setInterval(sweep, 60_000);
}

export function currentClip(): ClipItem | null {
  return State.clips[Math.min(State.clipIndex, State.clips.length - 1)] ?? null;
}

/** Removes the copy shown on the card (its OK button). */
export function dismissClip() {
  const c = currentClip();
  if (!c) return;
  State.clips = State.clips.filter((x) => x.id !== c.id);
  State.clipIndex = Math.min(State.clipIndex, Math.max(0, State.clips.length - 1));
  State.notify();
}

/** Builds the question for the chat and sends it. */
export function runClipAction(action: ClipAction) {
  const c = currentClip();
  if (!c || !island) return;
  const kind = clipKind(c.text);
  const target = State.settings.clipboard?.translateTo || "auto";
  const lang = TRANSLATE_LANGUAGES[target === "auto" ? language() : target] ?? "English";
  const ask = {
    explain:
      kind === "error"
        ? "Explain this error briefly: what it means and how to fix it."
        : kind === "code"
          ? "Explain briefly what this code does."
          : "Explain this briefly and clearly.",
    summarize: "Summarize this in a few short bullet points.",
    translate: `Translate this into ${lang}. Reply with the translation only.`,
    fix: "Fix the spelling, grammar and punctuation of this text, keeping its language and tone. Reply with the corrected text only.",
  }[action];
  const query = `${ask}\n\n"""\n${c.text}\n"""`;
  const display = `${clipActionLabel(action)}: “${preview(c.text, 60)}”`;
  island.setView("prompt");
  // Let the chat view become active before the question goes in.
  window.setTimeout(() => askChat(query, display), 60);
}
