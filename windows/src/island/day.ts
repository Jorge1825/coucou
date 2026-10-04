// Mochi reacts to your day: suggests a break after a long stretch at the
// keyboard, celebrates finished Claude Code runs, and makes a fuss on your
// birthday. Everything here is local — it only uses the idle time Windows
// reports and the hook events the island already receives.

import { t } from "../core/i18n";
import { State } from "../core/state";
import type { Island } from "./island";
import { onSystemStatus } from "./system";

/** Away this long (seconds) counts as having taken a break. */
const BREAK_IDLE_SECS = 5 * 60;
/** Finished runs in a day worth a little celebration. */
const MILESTONES = [3, 5, 10, 20, 50];

let activeSince: number | null = null;
let finishedToday = 0;
let countedDay = "";
let birthdayGreeted = "";

const today = () => {
  const d = new Date();
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
};

/** "MM-DD" for today. */
export function monthDay(d = new Date()): string {
  return `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export function isBirthday(): boolean {
  const p = State.settings.day;
  return !!p?.enabled && !!p.birthday && p.birthday === monthDay();
}

function duration(ms: number): string {
  const mins = Math.round(ms / 60_000);
  return mins < 60
    ? t("{n} min", { n: mins })
    : t("{h} h {m} min", { h: Math.floor(mins / 60), m: mins % 60 });
}

export function registerDayHandlers(island: Island) {
  // Breaks, from the idle time in every system sample.
  onSystemStatus((s) => {
    const p = State.settings.day;
    if (!p?.enabled || State.paused) {
      activeSince = null;
      return;
    }
    const now = Date.now();
    // Away long enough = that was a break; the stretch starts over.
    if (s.idleSecs >= BREAK_IDLE_SECS || activeSince == null) {
      activeSince = now;
      return;
    }
    if (p.breakMinutes > 0 && now - activeSince >= p.breakMinutes * 60_000 && !s.game) {
      island.showBanner({
        key: "day",
        title: t("Time for a break?"),
        text: t("You've been at it for {time} — stretch a little.", { time: duration(now - activeSince) }),
        ms: 8000,
      });
      island.react("yawn", 2.4);
      activeSince = now; // the next nudge comes after another full stretch
    }
  });

  // Celebrations: hooks.ts announces each finished run.
  window.addEventListener("coucou-finished", () => {
    const p = State.settings.day;
    if (!p?.enabled || !p.celebrate) return;
    if (countedDay !== today()) {
      countedDay = today();
      finishedToday = 0;
    }
    finishedToday++;
    if (MILESTONES.includes(finishedToday)) {
      window.setTimeout(() => {
        island.react("proud", 2.4);
        island.particles("star", 6);
        island.showBanner({
          key: "day",
          title: t("Well done!"),
          text: t("{n} tasks finished today", { n: finishedToday }),
          ms: 6000,
          glance: false,
        });
      }, 2500); // after the finish card has had its moment
    }
  });

  // Birthday: once per day, at launch or as soon as the date turns.
  const checkBirthday = () => {
    if (!isBirthday() || birthdayGreeted === today() || State.paused) return;
    birthdayGreeted = today();
    island.react("love", 3);
    island.particles("heart", 5);
    island.showBanner({
      key: "day",
      title: t("Happy birthday! 🎂"),
      text: t("Mochi dressed up for the occasion."),
      ms: 9000,
      glance: false,
    });
  };
  window.setTimeout(checkBirthday, 12_000); // after the launch greeting
  window.setInterval(checkBirthday, 30 * 60_000);
}
