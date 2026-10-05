// What Mochi puts on by itself. Worked out once a minute (and whenever the
// weather, the system or the settings change) from, in order of priority:
// birthday → festive days → snow/rain → night → winter → a game running.
// The hat chosen in Settings comes back as soon as nothing applies.

import { onEvent } from "../core/bridge";
import { State, type Weather } from "../core/state";
import { NO_EXTRAS, type Extras, type HatKind } from "../mochi/accessories";
import { isBirthday } from "./day";
import type { Island } from "./island";
import { onSystemStatus } from "./system";

/** Inside a span of hours that may cross midnight (from 22 to 7…). */
export function inHours(h: number, from: number, to: number): boolean {
  if (from === to) return false;
  return from < to ? h >= from && h < to : h >= from || h < to;
}

export function computeOutfit(now = new Date()): { hat: HatKind | null; extras: Extras } {
  const o = State.settings.outfits;
  const d = State.settings.day;
  if (!o?.auto) return { hat: null, extras: NO_EXTRAS };
  const month = now.getMonth() + 1;
  const date = now.getDate();
  let hat: HatKind | null = null;
  const extras: Extras = { neck: "none", umbrella: false, mic: false };

  if (isBirthday()) hat = "party";

  if (d?.enabled && d.seasonal) {
    if (month === 12 && date >= 24 && date <= 26) hat ??= "santa";
    if (month === 10 && date === 31) hat ??= "witch";
    if ((month === 12 && date === 31) || (month === 1 && date === 1)) hat ??= "party";
  }

  const w = State.weather;
  if (o.weather && o.city && w) {
    if (w.snow) {
      hat ??= "beanie";
      extras.neck = "scarf";
    } else if (w.rain || w.storm) {
      extras.umbrella = true;
    }
  }

  if (o.night && inHours(now.getHours(), o.nightFrom, o.nightTo)) hat ??= "nightcap";

  if (d?.enabled && d.seasonal && extras.neck === "none") {
    const winter = d.hemisphere === "south" ? [6, 7, 8] : [12, 1, 2];
    if (winter.includes(month)) extras.neck = "scarf";
  }

  if (o.gamer && State.system?.game) extras.mic = true;

  return { hat, extras };
}

export function registerOutfits(island: Island) {
  const apply = () => {
    const { hat, extras } = computeOutfit();
    island.setOutfit(hat, extras);
  };
  void onEvent<Weather>("weather", (w) => {
    State.weather = w;
    apply();
  });
  let lastGame = false;
  onSystemStatus((s) => {
    if (s.game !== lastGame) {
      lastGame = s.game;
      apply();
    }
  });
  State.subscribe(() => apply());
  window.setInterval(apply, 60_000);
  apply();
}
