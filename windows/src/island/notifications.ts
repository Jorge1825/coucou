// Windows notifications → the island. Rust sends each new toast as
// `os-notification` (apps muted one by one never get this far). Here we decide
// how much to show — by default as little as possible: a one-line banner on the
// compact island for a few seconds, then just a dot. Nothing pops up while the
// island is open, while silenced, in quiet hours, or when Windows itself says
// not now (Do not disturb, an app full screen).

import { Bridge, onEvent } from "../core/bridge";
import { Sound } from "../core/sound";
import { State, type OsNotification } from "../core/state";
import type { Island } from "./island";

/** How many recent notifications the bell keeps… */
const KEEP = 20;
/** …and for how long: after this they are dropped, read or not. */
const TTL_MS = 5 * 60_000;
/** How often expired ones are swept. The timer only runs while the list has any. */
const SWEEP_MS = 30_000;

let sweepTimer: number | null = null;

/** Drops what is older than TTL_MS; stops the timer once nothing is left. */
function sweep() {
  // Never pull a notification from under the user while they are reading it.
  if (State.mode === "expanded" && State.view === "notification") return;
  const cutoff = Date.now() - TTL_MS;
  const kept = State.osNotifications.filter((n) => (n.receivedAt ?? n.time) > cutoff);
  if (kept.length !== State.osNotifications.length) {
    State.osNotifications = kept;
    State.osIndex = Math.min(State.osIndex, Math.max(0, kept.length - 1));
    State.osUnread = Math.min(State.osUnread, kept.length);
    State.notify();
  }
  if (kept.length === 0 && sweepTimer != null) {
    window.clearInterval(sweepTimer);
    sweepTimer = null;
  }
}

function startSweeping() {
  if (sweepTimer == null) sweepTimer = window.setInterval(sweep, SWEEP_MS);
}

export function registerOsNotificationHandlers(island: Island) {
  void onEvent<OsNotification>("os-notification", (n) => handle(island, n));
  void onEvent<number>("os-notification-dismissed", (id) => removeOsNotification(id));
  void onEvent<string>("os-notifications-status", (s) => {
    State.osAccess = s;
    State.notify();
  });
  void Bridge.notificationsStatus().then((s) => {
    if (s) State.osAccess = s;
    State.notify();
  });
}

/** Drops one notification from the list (OK on the card, here or on another display). */
export function removeOsNotification(id: number) {
  const list = State.osNotifications;
  const at = list.findIndex((n) => n.id === id);
  if (at < 0) return;
  State.osNotifications = list.filter((n) => n.id !== id);
  const left = State.osNotifications.length;
  if (State.osIndex > at || State.osIndex >= left) State.osIndex = Math.max(0, State.osIndex - 1);
  State.osUnread = Math.min(State.osUnread, left);
  State.notify();
}

/** Inside the quiet hours from Settings (the span may cross midnight). */
function inQuietHours(): boolean {
  const s = State.settings;
  if (!s.notificationsQuiet) return false;
  const from = s.notificationsQuietFrom ?? 22;
  const to = s.notificationsQuietTo ?? 8;
  const h = new Date().getHours();
  return from === to ? false : from < to ? h >= from && h < to : h >= from || h < to;
}

function handle(island: Island, n: OsNotification) {
  if (!State.settings.notifications) return;
  n.receivedAt = Date.now();
  State.osNotifications = [n, ...State.osNotifications.filter((x) => x.id !== n.id)].slice(0, KEEP);
  startSweeping();
  State.osIndex = 0;
  State.osUnread++;

  const style = State.settings.notificationsStyle || "discreet";
  const quiet =
    n.muted || n.quiet || State.settings.notificationsMuted || State.paused || inQuietHours() || style === "bell";
  if (quiet) {
    State.notify();
    return;
  }

  if (State.settings.notificationsChime) Sound.play("pop");

  // Already open: the bell's dot and a glance are enough — never swap the view.
  if (State.mode === "expanded") {
    if (State.view === "notification") State.osUnread = 0;
    island.glance();
    State.notify();
    return;
  }

  const busy = State.pendingApproval != null;
  if (style === "expand" && !busy) {
    island.alert("notification");
    // Close again by itself if the user never comes near it.
    island.fsm.mouseLeft();
    island.glance();
  } else {
    island.showToast(n);
  }
  State.notify();
}
