// Entry point: boot the bridge, wire the island, start the greeting.

import "./style.css";
import { Bridge, IS_TAURI, onEvent } from "./core/bridge";
import { Sound } from "./core/sound";
import { providerDef } from "./core/providers";
import { State, type Settings } from "./core/state";
import { Island } from "./island/island";
import { registerHookHandlers } from "./island/hooks";
import { registerIntegrationHandlers, refreshConfigured } from "./island/integrations";

/** The island's API badge tells the truth about the active provider's key. */
async function refreshApiKey() {
  const def = providerDef(State.settings.provider);
  State.apiKeyPresent = !def.key || ((await Bridge.secretPresent(def.key)) ?? false);
  State.notify();
}

async function main() {
  const root = document.getElementById("root");
  if (!root) return;

  void Sound.preload();

  // Before the island exists: its canvases size their bitmaps from the pixel
  // ratio once, so the zoom correction has to land first.
  const dprBefore = window.devicePixelRatio || 1;
  await Bridge.fitZoom(dprBefore);
  // WebView2 applies the zoom a moment later; wait until the pixel ratio has
  // actually moved (or give up after a second) so nothing is sized against the
  // stale value.
  for (let i = 0; i < 40 && window.devicePixelRatio === dprBefore; i++) {
    await new Promise((r) => setTimeout(r, 25));
  }

  const island = new Island(root);

  const boot = await Bridge.boot();
  if (boot) {
    State.settings = { ...State.settings, ...boot.settings };
  }
  island.applySettings();
  State.loadIntegrationTasks();
  void refreshApiKey();

  island.setDock(boot?.dock ?? 0);
  await onEvent<number>("dock", (side) => island.setDock(side));
  await onEvent<{ x: number; y: number }>("cursor", ({ x, y }) => island.onCursor(x, y));

  /** Pause has to reach Rust too, or the pollers keep calling out. */
  const setPaused = (on: boolean) => {
    if (State.paused === on) return;
    State.paused = on;
    void Bridge.setPaused(on);
  };

  await onEvent<string>("tray", (what) => {
    switch (what) {
      case "settings":
        setPaused(false);
        island.alert("settings");
        break;
      case "open":
        setPaused(false);
        island.alert(State.defaultView());
        break;
      case "pause":
        setPaused(!State.paused);
        if (State.paused) island.fsm.forceHidden();
        else island.reveal();
        break;
    }
  });

  // Mochi speaking up on its own: a reminder coming due, or a check-in worth
  // an interruption. Shown even when no window of ours is in the foreground.
  await onEvent<{ text: string; kind: string }>("nudge", ({ text }) => {
    if (State.paused) return;
    State.noteMessage = text;
    // The attention chime the approvals use: a reminder has to be heard.
    Sound.resume();
    Sound.play("approval");
    island.alert("note");
    island.attention();
    // Let it close by itself if the user never comes near it.
    island.fsm.mouseLeft();
  });

  // Dragging the island back and forth makes Mochi dizzy.
  await onEvent<null>("shaken", () => island.shaken());

  await onEvent<null>("screen-changed", () => void Bridge.reposition());

  // The settings window writes preferences; apply them here without a restart.
  await onEvent<Settings>("settings-changed", (s) => {
    State.settings = { ...State.settings, ...s };
    island.applySettings();
    State.loadIntegrationTasks();
    void refreshConfigured();
    void refreshApiKey();
  });

  registerHookHandlers(island);
  registerIntegrationHandlers(island);

  island.launch();

  // In a plain browser there is no wake strip behind the cursor: make the whole
  // page wake the island so the visuals can be checked with `npm run dev`.
  if (!IS_TAURI) {
    document.addEventListener("click", () => Sound.resume(), { once: true });
  }
}

void main();
