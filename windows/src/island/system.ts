// The machine at a glance → the island. Rust reports battery, CPU, memory and
// network every few seconds; this only speaks up when something is off, once
// per episode (and again only after it recovered): a discreet banner, and Mochi
// showing it — tired on low battery, sweating when the PC struggles.

import { onEvent } from "../core/bridge";
import { t } from "../core/i18n";
import { State, type SystemStatus } from "../core/state";
import type { Island } from "./island";

/** Samples in a row (5 s each) before a CPU or network problem counts. */
const SUSTAIN = 3;

const flags = {
  battery: false,
  cpu: false,
  cpuHigh: 0,
  cpuLow: 0,
  memory: false,
  offline: false,
  offlineCount: 0,
};

type Listener = (s: SystemStatus) => void;
const listeners: Listener[] = [];

/** Other modules (Mochi's day, outfits) read the same samples. */
export function onSystemStatus(fn: Listener) {
  listeners.push(fn);
}

export function registerSystemHandlers(island: Island) {
  void onEvent<SystemStatus>("system-status", (s) => {
    State.system = s;
    for (const fn of listeners) fn(s);
    evaluate(island, s);
  });
}

function evaluate(island: Island, s: SystemStatus) {
  const p = State.settings.system;
  if (!p?.enabled || State.paused) return;
  const say = (title: string, text: string) =>
    island.showBanner({ key: "system", title, text, ms: 6000 });

  // Battery: low and not charging.
  if (s.battery != null && !s.charging && p.batteryLow > 0 && s.battery <= p.batteryLow) {
    if (!flags.battery) {
      flags.battery = true;
      say(t("Battery low"), t("{n}% left — time to plug in", { n: s.battery }));
      if (p.react) island.react("yawn", 2.4);
    }
  } else if (s.charging || (s.battery ?? 100) > p.batteryLow + 5) {
    flags.battery = false;
  }

  // CPU: high for a while, not a single spike.
  if (p.cpuHigh > 0) {
    if (s.cpu >= p.cpuHigh) {
      flags.cpuHigh++;
      flags.cpuLow = 0;
    } else if (s.cpu < p.cpuHigh - 10) {
      flags.cpuLow++;
      flags.cpuHigh = 0;
    }
    if (!flags.cpu && flags.cpuHigh >= SUSTAIN) {
      flags.cpu = true;
      say(t("Your PC is working hard"), t("CPU at {n}%", { n: s.cpu }));
      if (p.react) island.particles("sweat", 3);
    } else if (flags.cpu && flags.cpuLow >= SUSTAIN) {
      flags.cpu = false;
    }
  }

  // Memory.
  if (p.memoryHigh > 0 && s.memory >= p.memoryHigh) {
    if (!flags.memory) {
      flags.memory = true;
      say(t("Memory almost full"), t("{n}% in use — closing a few apps would help", { n: s.memory }));
      if (p.react) island.particles("sweat", 2);
    }
  } else if (s.memory < p.memoryHigh - 5) {
    flags.memory = false;
  }

  // Internet.
  if (p.offline) {
    flags.offlineCount = s.online ? 0 : flags.offlineCount + 1;
    if (!flags.offline && flags.offlineCount >= SUSTAIN) {
      flags.offline = true;
      say(t("No internet connection"), t("Mochi will tell you when it's back"));
      if (p.react) island.react("surprised");
    } else if (flags.offline && s.online) {
      flags.offline = false;
      say(t("Back online"), t("The connection is back"));
      if (p.react) island.react("happy");
    }
  }
}
