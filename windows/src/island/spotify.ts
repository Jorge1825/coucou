// Spotify → island state. Rust reads Windows' media session for Spotify and
// emits `spotify` whenever the track or play state changes. A Spotify pill
// appears while the app is open; Mochi wears its headphones while it plays.

import { Bridge, onEvent } from "../core/bridge";
import { Sound } from "../core/sound";
import { SPOTIFY_COLOR, SPOTIFY_ID, State, type NowPlaying } from "../core/state";
import type { Island } from "./island";

export function registerSpotifyHandlers(island: Island) {
  void onEvent<NowPlaying>("spotify", (np) => handle(island, np));
}

/** The island's buttons. Optimistic on play/pause so the icon flips at once. */
export function spotifyControl(action: "play_pause" | "next" | "previous") {
  const np = State.spotify;
  if (np && action === "play_pause") {
    State.spotify = { ...np, playing: !np.playing, positionMs: spotifyPosition() };
    State.spotifyAt = performance.now();
    State.notify();
  }
  Sound.play("blip");
  void Bridge.spotifyControl(action);
}

/** Current position, moved forward locally since the last reading. */
export function spotifyPosition(): number {
  const np = State.spotify;
  if (!np) return 0;
  const pos = np.playing ? np.positionMs + (performance.now() - State.spotifyAt) : np.positionMs;
  return np.durationMs > 0 ? Math.min(np.durationMs, pos) : pos;
}

function handle(island: Island, np: NowPlaying) {
  const prev = State.spotify;
  const on = State.settings.spotify && np.active && !State.paused;
  State.spotify = on ? np : null;
  State.spotifyAt = performance.now();

  if (!on) {
    if (State.tasks.some((t) => t.id === SPOTIFY_ID)) State.removeTask(SPOTIFY_ID);
    State.notify();
    return;
  }

  let task = State.tasks.find((t) => t.id === SPOTIFY_ID);
  if (!task) {
    // Right after VS Code, so it is always among the visible pills.
    const at = State.tasks.findIndex((t) => t.id === "integration_claude") + 1;
    task = {
      id: SPOTIFY_ID, name: "Spotify", color: SPOTIFY_COLOR, state: "idle",
      stepIndex: 0, steps: [], source: "spotify", isIntegration: true,
    };
    State.tasks.splice(at, 0, task);
  }
  task.emote = np.playing ? "happy" : null;
  task.steps = np.title ? [np.artist ? `${np.title} — ${np.artist}` : np.title] : [];
  task.stepIndex = 0;

  // A new song starting: peek out so the user sees Mochi put the headphones on.
  const newTrack = np.title && (np.title !== prev?.title || np.artist !== prev?.artist);
  const started = np.playing && !prev?.playing;
  if (np.playing && (newTrack || started)) island.reveal();

  State.notify();
}
