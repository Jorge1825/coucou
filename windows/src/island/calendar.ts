// Calendar → the island. Rust fetches the iCal address and sends the next two
// days. Here: a Calendar pill in the overview (next meetings, Join buttons), a
// banner a few minutes before each meeting — kept up as a countdown if wanted —
// and one more when it starts. Pointing at it joins the call.

import { Bridge, onEvent } from "../core/bridge";
import { locale, t } from "../core/i18n";
import { CALENDAR_COLOR, CALENDAR_ID, State, type CalEvent } from "../core/state";
import type { Island } from "./island";

const TICK_MS = 30_000;
/** A meeting stays "starting now" this long. */
const STARTING_WINDOW_MS = 2 * 60_000;

let island: Island | null = null;
const reminded = new Set<string>();
const started = new Set<string>();

const keyOf = (e: CalEvent) => `${e.start}:${e.title}`;

/** "10:30", "Tomorrow 09:00", "Wed 14:00". */
export function whenLabel(ms: number): string {
  const d = new Date(ms);
  const time = d.toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" });
  const today = new Date();
  const tomorrow = new Date(today.getFullYear(), today.getMonth(), today.getDate() + 1);
  if (d.toDateString() === today.toDateString()) return time;
  if (d.toDateString() === tomorrow.toDateString()) return t("Tomorrow {time}", { time });
  return `${d.toLocaleDateString(locale(), { weekday: "short" })} ${time}`;
}

/** Join the call, or open the Calendar card when there is no link. */
export function openEvent(e: CalEvent) {
  if (e.link) {
    void Bridge.openUrl(e.link);
    return;
  }
  if (!island) return;
  State.setFocus(CALENDAR_ID);
  island.alert("overview");
}

/** The Calendar pill exists while the feature is on and an address is set. */
function syncPill() {
  const want = !!State.settings.calendar?.enabled && State.calendarConfigured;
  const has = State.tasks.some((x) => x.id === CALENDAR_ID);
  if (want && !has) {
    const at = State.tasks.findIndex((x) => x.id === "integration_claude") + 1;
    State.tasks.splice(at, 0, {
      id: CALENDAR_ID, name: t("Calendar"), color: CALENDAR_COLOR, state: "idle",
      stepIndex: 0, steps: [], source: "calendar", isIntegration: true,
    });
  } else if (!want && has) {
    State.removeTask(CALENDAR_ID);
  }
}

function visible(e: CalEvent): boolean {
  return !e.allDay || !!State.settings.calendar?.allDay;
}

/** Upcoming (or running) meetings worth showing. */
export function upcoming(): CalEvent[] {
  const now = Date.now();
  return State.calendar.filter((e) => visible(e) && e.end > now);
}

function tick() {
  const p = State.settings.calendar;
  if (!island || !p?.enabled || State.paused) return;
  const now = Date.now();
  for (const e of State.calendar) {
    if (e.allDay) continue; // no countdown to a whole day
    const key = keyOf(e);
    const mins = (e.start - now) / 60_000;
    const join = e.link ? ` · ${t("point here to join")}` : "";
    if (p.remindMinutes > 0 && mins > 0 && mins <= p.remindMinutes) {
      const first = !reminded.has(key);
      if (first || p.countdown) {
        reminded.add(key);
        island.showBanner({
          key: `cal:${key}`,
          title: e.title,
          text: t("in {n} min", { n: Math.max(1, Math.ceil(mins)) }) + join,
          open: () => openEvent(e),
          // With the countdown on, it stays until the next tick refreshes it.
          ms: p.countdown ? TICK_MS + 2000 : 8000,
          glance: first,
        });
        if (first) island.react("surprised");
      }
    } else if (mins <= 0 && now - e.start < STARTING_WINDOW_MS && !started.has(key)) {
      started.add(key);
      island.showBanner({
        key: `cal:${key}`,
        title: e.title,
        text: t("Starting now") + join,
        open: () => openEvent(e),
        ms: 15_000,
      });
    }
  }
  // Forget meetings that are over.
  for (const set of [reminded, started]) {
    for (const k of set) if (Number(k.split(":")[0]) < now - 24 * 3600_000) set.delete(k);
  }
}

export function registerCalendarHandlers(isl: Island) {
  island = isl;
  void onEvent<{ events: CalEvent[]; error: string | null; configured: boolean }>("calendar", (u) => {
    State.calendar = u.events;
    State.calendarError = u.error;
    State.calendarConfigured = u.configured;
    syncPill();
    tick();
    State.notify();
  });
  // Turning the feature on (or off) in Settings.
  let wasOn = !!State.settings.calendar?.enabled;
  State.subscribe(() => {
    const on = !!State.settings.calendar?.enabled;
    if (on !== wasOn) {
      wasOn = on;
      if (on) void Bridge.calendarRefresh();
      syncPill();
    }
  });
  if (wasOn) void Bridge.calendarRefresh();
  window.setInterval(tick, TICK_MS);
}
