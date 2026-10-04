// Settings sections for the features that came together: Claude Code sessions,
// smart clipboard, system status, calendar, Mochi's day and outfits.
// Every switch and number lives in one nested block of Settings (see state.ts).

import { Bridge } from "../core/bridge";
import { t } from "../core/i18n";
import type { Settings } from "../core/state";
import { h, clear } from "../views/dom";
import { CLIP_ACTIONS, TRANSLATE_LANGUAGES, clipActionLabel } from "../island/clipboard";

export interface Ctx {
  settings: () => Settings;
  save: () => Promise<void>;
  toggle: (on: boolean, onChange: (v: boolean) => void) => HTMLElement;
  present: Record<string, boolean>;
}

/** A <select> from [value, label] pairs. */
function select(options: [string | number, string][], current: string | number, onChange: (v: string) => void) {
  const el = h("select", {}) as HTMLSelectElement;
  for (const [v, label] of options) el.append(h("option", { value: String(v), text: label }));
  el.value = String(current);
  if (el.selectedIndex < 0) el.selectedIndex = 0;
  el.addEventListener("change", () => onChange(el.value));
  return el;
}

const row = (label: string, ...children: (Node | null)[]) =>
  h("div", { class: "row" }, h("label", { text: label }), ...children);
const hint = (text: string) => h("span", { class: "hint", text });
const note = (text: string) => h("div", { class: "hint", text });
const hours = (): [number, string][] =>
  Array.from({ length: 24 }, (_, i) => [i, `${String(i).padStart(2, "0")}:00`]);
const percent = (values: number[]): [number, string][] => [
  [0, t("Never")], ...values.map((v): [number, string] => [v, `${v} %`]),
];
const minutes = (values: number[], never = true): [number, string][] => [
  ...(never ? [[0, t("Never")] as [number, string]] : []),
  ...values.map((v): [number, string] => [v, v < 60 ? t("{n} minutes", { n: v }) : v === 60 ? t("1 hour") : t("{n} hours", { n: v / 60 })]),
];

// ── Claude Code sessions ─────────────────────────────────────────────────────

export function sessionsSection(c: Ctx): HTMLElement {
  const s = () => c.settings().sessions;
  return h("section", {},
    h("h2", {}, h("span", { text: t("Claude Code sessions") })),
    note(t("Several Claude Code sessions at once each get their own pill, named after the project. When a run finishes, the card says what changed according to git.")),
    row(t("One pill per session"), c.toggle(s().separate, (v) => { s().separate = v; void c.save(); })),
    row(t("At most"),
      select([2, 3, 4].map((n) => [n, t("{n} pills", { n })]), s().max, (v) => { s().max = Number(v); void c.save(); })),
    row(t("Remove idle sessions after"),
      select(minutes([5, 15, 30, 60, 120], false), s().lingerMinutes, (v) => { s().lingerMinutes = Number(v); void c.save(); })),
    row(t("Changes summary (git)"), c.toggle(s().gitSummary, (v) => { s().gitSummary = v; void c.save(); }),
      hint(t("files changed and lines added/removed"))),
  );
}

// ── Smart clipboard ──────────────────────────────────────────────────────────

export function clipboardSection(c: Ctx): HTMLElement {
  const p = () => c.settings().clipboard;
  const chips = h("div", { class: "chips" });
  for (const a of CLIP_ACTIONS) {
    const b = h("button", {
      class: p().actions.includes(a) ? "chip on" : "chip",
      text: clipActionLabel(a),
      onclick: () => {
        const on = p().actions.includes(a);
        p().actions = on ? p().actions.filter((x) => x !== a) : [...p().actions, a];
        b.classList.toggle("on", !on);
        void c.save();
      },
    });
    chips.append(b);
  }
  const langs: [string, string][] = [["auto", t("Interface language")],
    ...Object.keys(TRANSLATE_LANGUAGES).map((k): [string, string] => [k, k.toUpperCase()])];
  return h("section", {},
    h("h2", {}, h("span", { text: t("Smart clipboard") })),
    note(t("When you copy text, Mochi offers to explain, summarize, translate or fix it with your AI provider. The text stays on this PC until you click an action; passwords and anything a password manager marks as private are ignored. Off by default.")),
    row(t("Enabled"), c.toggle(p().enabled, (v) => { p().enabled = v; void c.save(); })),
    row(t("Suggest when copying"), c.toggle(p().suggest, (v) => { p().suggest = v; void c.save(); }),
      hint(t("a small banner; otherwise only the history"))),
    row(t("Actions"), chips),
    row(t("Translate into"), select(langs, p().translateTo, (v) => { p().translateTo = v; void c.save(); })),
    row(t("Ignore copies shorter than"),
      select([5, 10, 20, 40, 80].map((n) => [n, t("{n} characters", { n })]), p().minChars, (v) => { p().minChars = Number(v); void c.save(); })),
    row(t("History"),
      select([5, 10, 15, 30].map((n) => [n, t("{n} items", { n })]), p().history, (v) => { p().history = Number(v); void c.save(); }),
      select(minutes([5, 15, 30, 60, 240], false), p().keepMinutes, (v) => { p().keepMinutes = Number(v); void c.save(); }),
      hint(t("then forgotten"))),
  );
}

// ── System at a glance ───────────────────────────────────────────────────────

export function systemSection(c: Ctx): HTMLElement {
  const p = () => c.settings().system;
  return h("section", {},
    h("h2", {}, h("span", { text: t("System at a glance") })),
    note(t("Mochi only speaks up when something is off: low battery, the CPU or memory working hard for a while, or no internet. Read from Windows, nothing leaves the PC.")),
    row(t("Enabled"), c.toggle(p().enabled, (v) => { p().enabled = v; void c.save(); })),
    row(t("Low battery below"),
      select(percent([10, 15, 20, 30]), p().batteryLow, (v) => { p().batteryLow = Number(v); void c.save(); })),
    row(t("Busy CPU above"),
      select(percent([80, 90, 95]), p().cpuHigh, (v) => { p().cpuHigh = Number(v); void c.save(); })),
    row(t("Memory above"),
      select(percent([80, 90, 95]), p().memoryHigh, (v) => { p().memoryHigh = Number(v); void c.save(); })),
    row(t("No internet"), c.toggle(p().offline, (v) => { p().offline = v; void c.save(); })),
    row(t("Mochi reacts"), c.toggle(p().react, (v) => { p().react = v; void c.save(); }),
      hint(t("tired, sweating… not just the banner"))),
  );
}

// ── Calendar ─────────────────────────────────────────────────────────────────

export function calendarSection(c: Ctx): HTMLElement {
  const p = () => c.settings().calendar;
  const KEY = "calendar-ics";
  const status = h("span", { class: "hint" });
  const input = h("input", {
    type: "password",
    placeholder: c.present[KEY] ? t("••••••••  (stored)") : "https://calendar.google.com/…/basic.ics",
    autocomplete: "off",
    spellcheck: "false",
    style: "flex:1 1 auto;min-width:0",
  }) as HTMLInputElement;
  const saveBtn = h("button", {
    class: "primary",
    text: t("Save"),
    onclick: async () => {
      const v = input.value.trim();
      if (!/^(https|webcal):\/\//i.test(v)) {
        status.textContent = t("It must be an https:// or webcal:// address.");
        return;
      }
      try {
        await Bridge.secretSet(KEY, v);
        c.present[KEY] = true;
        input.value = "";
        input.placeholder = t("••••••••  (stored)");
        if (!p().enabled) {
          p().enabled = true;
          await c.save();
        }
        void Bridge.calendarRefresh();
        status.textContent = t("Saved. Your meetings will appear in the Calendar pill.");
      } catch (err) {
        status.textContent = String(err).replace(/^Error:\s*/, "");
      }
    },
  });
  const removeBtn = h("button", {
    class: "danger",
    text: t("Remove"),
    onclick: async () => {
      await Bridge.secretClear(KEY);
      c.present[KEY] = false;
      input.placeholder = "https://calendar.google.com/…/basic.ics";
      void Bridge.calendarRefresh();
      status.textContent = t("Address removed.");
    },
  });
  return h("section", {},
    h("h2", {}, h("span", { text: t("Calendar") })),
    note(t("Free, no account needed: paste your calendar's private iCal (.ics) address. Google Calendar: Settings → your calendar → Integrate calendar → Secret address in iCal format. Outlook: Settings → Calendar → Shared calendars → Publish a calendar → ICS link. Stored in the Windows Credential Manager.")),
    row(t("Enabled"), c.toggle(p().enabled, (v) => { p().enabled = v; void c.save(); })),
    h("div", { class: "row" }, h("label", { text: t("iCal address") }), input, saveBtn, removeBtn),
    h("div", { class: "row" }, status),
    row(t("Remind me"),
      select([[0, t("Never")], [1, t("1 minute before")], [5, t("{n} minutes before", { n: 5 })],
        [10, t("{n} minutes before", { n: 10 })], [15, t("{n} minutes before", { n: 15 })]],
        p().remindMinutes, (v) => { p().remindMinutes = Number(v); void c.save(); })),
    row(t("Countdown"), c.toggle(p().countdown, (v) => { p().countdown = v; void c.save(); }),
      hint(t("keep the banner up until it starts"))),
    row(t("All-day events"), c.toggle(p().allDay, (v) => { p().allDay = v; void c.save(); })),
  );
}

// ── Mochi's day ──────────────────────────────────────────────────────────────

export function daySection(c: Ctx): HTMLElement {
  const p = () => c.settings().day;
  const [bm, bd] = (p().birthday || "-").split("-");
  const month = select([["", "—"], ...Array.from({ length: 12 }, (_, i): [string, string] =>
    [String(i + 1).padStart(2, "0"), new Date(2000, i, 1).toLocaleDateString(undefined, { month: "long" })])], bm ?? "", () => setBirthday());
  const dayOf = select([["", "—"], ...Array.from({ length: 31 }, (_, i): [string, string] =>
    [String(i + 1).padStart(2, "0"), String(i + 1)])], bd ?? "", () => setBirthday());
  function setBirthday() {
    const m = (month as HTMLSelectElement).value;
    const d = (dayOf as HTMLSelectElement).value;
    p().birthday = m && d ? `${m}-${d}` : "";
    void c.save();
  }
  return h("section", {},
    h("h2", {}, h("span", { text: t("Mochi's day") })),
    note(t("Mochi notices your day: suggests a break after a long stretch, celebrates finished tasks, and dresses up on special days.")),
    row(t("Enabled"), c.toggle(p().enabled, (v) => { p().enabled = v; void c.save(); })),
    row(t("Suggest a break after"),
      select(minutes([45, 60, 90, 120]), p().breakMinutes, (v) => { p().breakMinutes = Number(v); void c.save(); }),
      hint(t("of use without a pause"))),
    row(t("Celebrate finished tasks"), c.toggle(p().celebrate, (v) => { p().celebrate = v; void c.save(); })),
    row(t("Your birthday"), month, dayOf),
    row(t("Seasonal outfits"), c.toggle(p().seasonal, (v) => { p().seasonal = v; void c.save(); }),
      hint(t("scarf in winter, Halloween, Christmas, New Year"))),
    row(t("Hemisphere"),
      select([["north", t("North")], ["south", t("South")]], p().hemisphere, (v) => { p().hemisphere = v; void c.save(); })),
  );
}

// ── Outfits ──────────────────────────────────────────────────────────────────

export function outfitsSection(c: Ctx): HTMLElement {
  const p = () => c.settings().outfits;
  const found = h("span", { class: "hint", text: p().city ? `${t("Using")}: ${p().city}` : "" });
  const city = h("input", {
    type: "text", placeholder: t("City"), spellcheck: "false", style: "flex:1 1 auto;min-width:0",
  }) as HTMLInputElement;
  const find = h("button", {
    text: t("Find"),
    onclick: async () => {
      found.textContent = t("Looking…");
      try {
        const place = await Bridge.weatherFindCity(city.value);
        const o = p();
        o.city = place.country ? `${place.name}, ${place.country}` : place.name;
        o.latitude = place.latitude;
        o.longitude = place.longitude;
        o.weather = true;
        await c.save();
        found.textContent = `${t("Using")}: ${o.city}`;
        city.value = "";
      } catch (err) {
        found.textContent = String(err).replace(/^Error:\s*/, "");
      }
    },
  });
  return h("section", {},
    h("h2", {}, h("span", { text: t("Outfits") })),
    note(t("Mochi can change outfit by itself — the hat you picked comes back when nothing applies.")),
    row(t("Automatic outfits"), c.toggle(p().auto, (v) => { p().auto = v; void c.save(); })),
    row(t("Nightcap at night"), c.toggle(p().night, (v) => { p().night = v; void c.save(); }),
      select(hours(), p().nightFrom, (v) => { p().nightFrom = Number(v); void c.save(); }),
      hint("→"),
      select(hours(), p().nightTo, (v) => { p().nightTo = Number(v); void c.save(); })),
    row(t("Umbrella when it rains"), c.toggle(p().weather, (v) => { p().weather = v; void c.save(); })),
    h("div", { class: "row" }, h("label", { text: t("Weather for") }), city, find),
    h("div", { class: "row" }, found),
    note(t("Weather from Open-Meteo, free and without an account; only the city's coordinates are sent, every 30 minutes.")),
    row(t("Gamer headset"), c.toggle(p().gamer, (v) => { p().gamer = v; void c.save(); }),
      hint(t("while a game runs full screen"))),
  );
}

export { clear };
