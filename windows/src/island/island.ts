// The island: DOM shell, sizing animation, Mochi placement, mouse handling.
// Mirrors IslandRootView.swift + IslandWindowController.swift.

import { Tracked, Spring } from "../core/anim";
import { Bridge, IS_TAURI, onDragDrop } from "../core/bridge";
import {
  EXPANDED_CORNER, EXPANDED_W, NOTCH_H, NOTCH_W, PANEL_H, PANEL_W,
  ROUNDED_CORNER, VIEW_LAYOUTS, type BotEmoteName, botGlowColor, botGlowOpacity, botPosition, chatPromptHeight,
  islandSize, VERTICAL_W,
  type IslandMode, type IslandViewName,
} from "../core/layout";
import { Sound } from "../core/sound";
import { OPACITY_MIN, State, clampOpacity, type OsNotification, type Settings } from "../core/state";
import { t } from "../core/i18n";
import { removeOsNotification } from "./notifications";
import { dismissClip, runClipAction } from "./clipboard";
import { BotEngine, hexToRGB } from "../mochi/engine";
import { isFace, isHat, isNeck, type Extras, type HatKind } from "../mochi/accessories";
import { Greeting } from "../mochi/greeting";
import { createMiniBot, pruneMiniBots, syncMiniBotStates, tickMiniBots } from "../mochi/minibots";
import { UploadCanvas } from "../upload/canvas";
import { USC, UploadSeq } from "../upload/sequence";
import { buildHeader, buildViews, type ViewActions, type ViewHost } from "../views/views";
import { h } from "../views/dom";
import { IslandStateMachine } from "./fsm";

const BOT_OVERHANG = 40;

/** Where pointing at a banner leads: a view, an action, or just open the island. */
export type BannerOpen = IslandViewName | (() => void) | null;

export interface Banner {
  /** Same key while visible + `group`: counts up instead of replacing. */
  key: string;
  title: string;
  text: string;
  icon?: string | null;
  open?: BannerOpen;
  group?: (count: number) => string;
  /** How long it stays (default 4 s). */
  ms?: number;
  /** Mochi looks at it (default yes). */
  glance?: boolean;
}

/** Width of the compact island while it carries a notification banner. */
const TOAST_W = 380;
/** How long the banner stays before the island shrinks back. */
const TOAST_MS = 4000;

/**
 * Frame budget for ambient motion — breathing, dancing, sleeping z's — when
 * nothing is being interacted with. Half of 60 Hz looks the same for a slow
 * sway and halves what the GPU has to compose for a transparent window. Anything
 * the user drives (cursor, drag, opening, tweens) still runs every frame.
 */
const AMBIENT_FRAME_MS = 1000 / 30;
/** Compact, Mochi is 20 px across: a slower budget is indistinguishable. */
const AMBIENT_FRAME_MS_COMPACT = 1000 / 20;
/** The cursor moved this recently: Mochi is following it, keep full rate. */
const CURSOR_ACTIVE_MS = 400;

/** Settings → Transparency, as CSS variables (see :root in style.css). */
function applyTransparency(s: Settings) {
  const root = document.documentElement.style;
  root.setProperty("--island-alpha", String(clampOpacity(s.islandOpacity, OPACITY_MIN.island)));
  root.setProperty("--card-alpha", String(clampOpacity(s.cardOpacity, OPACITY_MIN.card)));
  root.setProperty("--idle-alpha", String(clampOpacity(s.idleOpacity, OPACITY_MIN.idle)));
}
/** Same margin as the Rust hit test (src-tauri/src/island.rs). */
const HIT_MARGIN = 14;

/** Width of the auto-close bar when full, px. */
const COUNTDOWN_WIDTH = 160;

/** Idle time before Mochi yawns. */
const YAWN_AFTER_MS = 60_000;

/** Characters in the last exchange: what decides whether the chat card needs more height. */
function chatChars(): number {
  return State.chatHistory.slice(-2).reduce((n, m) => n + m.content.length, 0);
}

/** The three views the drop sequence owns; leaving them stops the engine. */
const UPLOAD_VIEWS: ReadonlySet<IslandViewName> = new Set(["upload", "uploading", "choose"]);

/** Seconds between the drop and the moment the progress bar starts filling. */
const PRE_PROGRESS = USC.T_PROG_START - USC.T_DROP;

const modeOrder = (m: IslandMode) => (m === "hidden" ? 0 : m === "compact" ? 1 : 2);

export class Island {
  readonly fsm = new IslandStateMachine();

  private root: HTMLElement;
  private islandEl!: HTMLElement;
  private clipEl!: HTMLElement;
  private contentEl!: HTMLElement;
  private viewsEl!: HTMLElement;
  private botCanvas!: HTMLCanvasElement;
  private botGlow!: HTMLElement;
  private greetingCanvas!: HTMLCanvasElement;
  private miniGrid!: HTMLElement;
  private countdown!: HTMLElement;
  private toastEl!: HTMLElement;
  private toastIcon!: HTMLImageElement;
  private toastApp!: HTMLElement;
  private toastText!: HTMLElement;
  private unreadDot!: HTMLElement;
  /** The banner on screen, how many it groups, and where pointing at it leads. */
  private toast: { key: string; count: number; open: BannerOpen } | null = null;
  private toastTimer: number | null = null;
  private wakeStrip!: HTMLElement;

  private header!: ViewHost;
  private views!: Map<IslandViewName, ViewHost>;
  private uploadCanvas!: UploadCanvas;

  private width = new Tracked(NOTCH_W);
  private height = new Tracked(0);
  private radius = new Tracked(ROUNDED_CORNER);
  private botCx = new Spring(46);
  private botCy = new Spring(16);
  private botSize = new Spring(10);

  private engine = new BotEngine();
  private greeting = new Greeting();

  private running = false;
  private lastFrame = 0;
  private dirty = true;
  private canvasPx = 0;

  // Rust starts the window at full size so the launch greeting has room.
  private collapsed = false;
  private collapseTimer: number | null = null;
  private pickingFile = false;
  private wasInIsland = false;
  /** Last cursor event, for the frame budget (see ambientOnly). */
  private lastCursorMs = 0;
  /** Last shape handed to Rust for the click-through test. */
  private pushedRect = { x: -1, y: -1, w: -1, h: -1 };
  private countdownTimer: number | null = null;

  // Bot hover → love (IslandWindowController.botHoverIn)
  private botHovering = false;
  private botHoverTimer: number | null = null;
  private lastLoveTime = 0;
  private botHoverStart = { x: 0, y: 0 };

  private confusedRecovery: number | null = null;
  private prevViewBeforeConfused: IslandViewName = "overview";
  private lastSyncedView: IslandViewName | null = null;

  /** Drop sequence bookkeeping: last tick played, and whether the ✓ has fired. */
  private uploadTens = 0;
  private uploadDone = false;

  constructor(root: HTMLElement) {
    this.root = root;
    this.build();
    this.wireFsm();
    this.wireInput();
    this.engine.onDizzy = () => this.handleDizzy();
    this.greeting.onComplete = () => this.fsm.greetComplete();
    State.subscribe(() => {
      this.dirty = true;
      this.ensureRunning();
    });
  }

  // ── DOM ─────────────────────────────────────────────────────────────────────

  private build() {
    const actions: ViewActions = {
      setView: (v) => this.setView(v),
      collapse: () => this.collapse(),
      minimize: () => this.minimize(),
      setFocus: (id) => {
        State.setFocus(id);
        Sound.play("blip");
      },
      openTerminal: () => {
        const cwd = State.focusTask?.sessionCwd ?? null;
        void Bridge.openInVSCode(cwd);
      },
      // The ↗ button — same targets as openAgentTarget() on macOS.
      openTarget: () => {
        const task = State.focusTask;
        if (!task) return;
        const urls: Record<string, string> = {
          integration_resend: "https://resend.com/emails",
          integration_vercel: "https://vercel.com/dashboard",
          integration_github: "https://github.com",
          integration_stripe: "https://dashboard.stripe.com/payments",
          integration_notion: "https://notion.so",
          integration_linear: "https://linear.app",
          integration_calcom: "https://app.cal.com/bookings",
          integration_spotify: "https://open.spotify.com",
        };
        if (task.source === "claudeCode") void Bridge.openInVSCode(task.sessionCwd ?? null);
        else if (task.id === "integration_n8n") void Bridge.openN8n();
        else if (urls[task.id]) void Bridge.openUrl(urls[task.id]);
      },
      openUrl: (url) => {
        if (url) void Bridge.openUrl(url);
      },
      decide: (d) => {
        const req = State.pendingApproval;
        void Bridge.log(`decide ${d} req=${req?.requestId ?? "none"}`);
        if (!req) return;
        Sound.play(d === "deny" ? "blip" : "approve");
        if (d === "allow") window.setTimeout(() => this.react("proud"), 250);
        void Bridge.approvalDecision(req.requestId, d);
        State.pendingApproval = null;
        State.isPinned = false;
        this.fsm.pinned = false;
        State.updateTask(req.taskId, "working");
        State.setPillBadge(req.taskId, null);
        this.setView(State.defaultView());
      },
      toggleSound: () => {
        State.settings.soundEnabled = !State.settings.soundEnabled;
        Sound.setEnabled(State.settings.soundEnabled);
        void Bridge.saveSettings(State.settings);
        State.notify();
      },
      setVolume: (v) => {
        State.settings.soundVolume = v;
        Sound.setVolume(v);
        void Bridge.saveSettings(State.settings);
        State.notify();
      },
      setAutoClose: (s) => {
        State.settings.autoCloseInterval = s;
        this.fsm.homeToPetitDelay = s;
        void Bridge.saveSettings(State.settings);
        State.notify();
      },
      openSettingsWindow: () => void Bridge.openSettingsWindow(),
      blip: () => Sound.play("blip"),
      pickFile: () => void this.pickFile(),
      toggleNotificationsMuted: () => {
        State.settings.notificationsMuted = !State.settings.notificationsMuted;
        Sound.play("blip");
        void Bridge.saveSettings(State.settings);
        State.notify();
      },
      muteNotificationApp: (app) => {
        const muted = State.settings.notificationsMutedApps ?? [];
        if (!muted.includes(app)) State.settings.notificationsMutedApps = [...muted, app];
        State.osNotifications = State.osNotifications.filter((n) => n.app !== app);
        State.osIndex = 0;
        Sound.play("blip");
        void Bridge.saveSettings(State.settings);
        State.notify();
      },
      browseNotifications: (delta) => {
        const last = State.osNotifications.length - 1;
        State.osIndex = Math.max(0, Math.min(last, State.osIndex + delta));
        Sound.play("blip");
        State.notify();
      },
      clipAction: (action) => runClipAction(action),
      browseClips: (delta) => {
        const last = State.clips.length - 1;
        State.clipIndex = Math.max(0, Math.min(last, State.clipIndex + delta));
        Sound.play("blip");
        State.notify();
      },
      dismissClip: () => {
        dismissClip();
        this.collapse();
      },
      dismissNotification: () => {
        const list = State.osNotifications;
        const shown = list[Math.min(State.osIndex, list.length - 1)];
        if (shown) {
          removeOsNotification(shown.id);
          // The other displays' islands hold the same notification.
          void Bridge.notificationDismiss(shown.id);
        }
        this.collapse();
      },
      openNotificationAccess: () => void Bridge.openNotificationSettings(),
      checkNotificationAccess: () => {
        void Bridge.notificationsRequestAccess().then((s) => {
          if (s) State.osAccess = s;
          State.notify();
        });
      },
    };

    this.wakeStrip = h("div", { id: "wake-strip" });
    this.botGlow = h("div", { id: "bot-glow" });
    this.botCanvas = h("canvas", { id: "bot-canvas" });
    this.greetingCanvas = h("canvas", { id: "greeting-canvas" });
    this.miniGrid = h("div", { id: "mini-grid" });
    this.countdown = h("div", { id: "countdown" });
    this.toastIcon = h("img", { class: "toast-icon", alt: "" }) as HTMLImageElement;
    this.toastApp = h("b", {});
    this.toastText = h("span", {});
    this.toastEl = h("div", { id: "toast" }, this.toastIcon, this.toastApp, this.toastText);
    this.unreadDot = h("i", { id: "unread-dot" });

    this.header = buildHeader(actions);
    this.views = buildViews(actions, () => this.animateGeometry(false));
    this.viewsEl = h("div", { id: "views" });
    for (const v of this.views.values()) this.viewsEl.append(v.el);
    this.contentEl = h("div", { id: "content" }, this.header.el, this.viewsEl);

    // The drop sequence draws the card, the bar and its own Mochi. It sits under
    // the header, which stays visible on top of it exactly as on macOS.
    this.uploadCanvas = new UploadCanvas({
      ask: () => {
        State.promptContext = State.droppedFile
          ? { kind: "file", name: State.droppedFile.name, path: State.droppedFile.path }
          : null;
        this.setView("prompt");
      },
      cancel: () => this.setView(State.defaultView()),
    });

    this.clipEl = h(
      "div",
      { id: "island-clip" },
      this.greetingCanvas,
      this.uploadCanvas.el,
      this.contentEl,
    );
    this.islandEl = h(
      "div",
      { id: "island" },
      this.clipEl,
      this.botGlow,
      this.botCanvas,
      this.miniGrid,
      this.toastEl,
      this.unreadDot,
      this.countdown,
    );

    const dpr = Math.min(2, window.devicePixelRatio || 1);
    this.greetingCanvas.width = Math.round(EXPANDED_W * dpr);
    this.greetingCanvas.height = Math.round(150 * dpr);
    this.greetingCanvas.style.width = `${EXPANDED_W}px`;
    this.greetingCanvas.style.height = "150px";

    this.root.append(this.wakeStrip, this.islandEl);
    this.applyGeometry();
  }

  // ── FSM ─────────────────────────────────────────────────────────────────────

  private wireFsm() {
    this.fsm.homeToPetitDelay = State.settings.autoCloseInterval;
    this.fsm.onTransition = (from, to) => {
      // Retracting from an open island: this is when it docks to the nearest
      // screen edge. While it is open it stays wherever the user put it.
      if ((to === "petit" || to === "hidden") && (from === "home" || from === "coucou")) {
        void Bridge.dockNearest();
      }
      switch (to) {
        case "hidden":
          this.setMode("hidden");
          break;
        case "petit":
          if (from === "coucou") this.greeting.interrupt();
          else if (from === "hidden") Sound.play("peek");
          this.setMode("compact");
          if (from === "coucou") State.view = State.defaultView();
          if (!this.wasInIsland) this.fsm.mouseLeft();
          break;
        case "home":
          this.expand(State.defaultView());
          if (!this.wasInIsland) this.fsm.mouseLeft();
          break;
        case "coucou":
          this.expand("greeting");
          this.greeting.start();
          break;
      }
      State.notify();
    };
  }

  launch() {
    this.fsm.launch();
  }

  // ── Mode / view ─────────────────────────────────────────────────────────────

  private setMode(mode: IslandMode) {
    const prev = State.mode;
    if (mode === prev) return;
    State.mode = mode;
    if (mode === "expanded") Sound.play("open");
    if (prev === "expanded") {
      Sound.play("close");
      State.isPinned = false;
      void Bridge.focusWindow(false);
    }
    if (mode !== "expanded") {
      this.engine.resetMorph();
      // Nothing can be seen of the sequence once the island is shut, and leaving
      // it running would keep the frame loop awake — the island must cost
      // nothing while hidden.
      UploadSeq.deactivate();
    }
    this.updateWindowCollapsed();
    this.animateGeometry(modeOrder(mode) < modeOrder(prev));
    State.notify();
  }

  /** True while the drop sequence owns the island body. */
  private get uploadActive(): boolean {
    return State.mode === "expanded" && UploadSeq.isActive && UPLOAD_VIEWS.has(State.view);
  }

  /** Navigating out of the drop flow ends the sequence, as on macOS. */
  private stopSequenceIfLeaving(view: IslandViewName) {
    if (UploadSeq.isActive && !UPLOAD_VIEWS.has(view)) UploadSeq.deactivate();
  }

  expand(view: IslandViewName) {
    this.stopSequenceIfLeaving(view);
    State.view = view;
    if (State.mode !== "expanded") this.setMode("expanded");
    else this.animateGeometry(false);
    State.lastActivity = performance.now();
    State.notify();
  }

  setView(view: IslandViewName) {
    this.stopSequenceIfLeaving(view);
    if (State.mode !== "expanded") {
      this.fsm.forceHome();
      State.view = view;
      this.animateGeometry(false);
      State.notify();
      return;
    }
    const grew = VIEW_LAYOUTS[view].height >= VIEW_LAYOUTS[State.view].height;
    State.view = view;
    State.lastActivity = performance.now();
    this.animateGeometry(!grew);
    State.notify();
  }

  collapse() {
    State.isPinned = false;
    this.fsm.pinned = false;
    // Drive the state machine rather than the mode: setting the mode behind its
    // back left it thinking the island was still open, and a click on the compact
    // island then did nothing — the island could never be reopened.
    this.fsm.forcePetit();
  }

  /**
   * The header's minimize button: fold right now, no countdown. An approval
   * still waiting keeps its badge on the compact island, so it isn't lost.
   */
  minimize() {
    if (State.pendingApproval) State.setPillBadge(State.pendingApproval.taskId, "approval");
    this.collapse();
  }

  /** Alert from the hook server: open on this view. Pinned alerts never auto-close. */
  alert(view: IslandViewName) {
    this.fsm.pinned = State.isPinned;
    this.fsm.forceHome();
    this.expand(view);
  }

  reveal() {
    this.fsm.reveal();
  }

  // ── Notification banner ─────────────────────────────────────────────────────

  /**
   * The discreet way in: the compact island widens into a one-line banner for
   * a few seconds, Mochi glances at it, then everything shrinks back and only a
   * dot remains. A burst from the same app becomes one banner with a count.
   */
  showToast(n: OsNotification) {
    this.showBanner({
      key: `os:${n.app}`,
      title: n.app,
      text: [n.title, n.body].filter((x) => x && x !== n.app).join(" · "),
      icon: n.icon,
      open: "notification",
      group: (count) => t("{n} new notifications", { n: count }),
    });
  }

  /**
   * A one-line banner on the compact island: the discreet way for anything to
   * say something (notifications, clipboard, system, calendar, Mochi's day).
   * Pointing at it follows `open`. Upright on a side edge there is no room for
   * a line of text, so Mochi only glances.
   */
  showBanner(b: Banner) {
    if (this.toast && this.toast.key === b.key && b.group) {
      this.toast.count++;
    } else {
      this.toast = { key: b.key, count: 1, open: b.open ?? null };
    }
    this.toast.open = b.open ?? null;
    const count = this.toast.count;
    this.toastApp.textContent = b.title;
    this.toastText.textContent = count > 1 && b.group ? b.group(count) : b.text;
    this.toastIcon.style.display = b.icon ? "" : "none";
    if (b.icon && this.toastIcon.src !== b.icon) this.toastIcon.src = b.icon;

    if (State.mode === "hidden") this.reveal();
    if (this.toastTimer != null) window.clearTimeout(this.toastTimer);
    this.toastTimer = window.setTimeout(() => this.endToast(), b.ms ?? TOAST_MS);
    this.animateGeometry(false);
    if (b.glance !== false) this.glance();
    State.notify();
  }

  /** Takes a banner down early (e.g. the meeting it counted down to started). */
  hideBanner(key: string) {
    if (this.toast?.key === key) this.endToast();
  }

  private endToast() {
    if (this.toastTimer != null) window.clearTimeout(this.toastTimer);
    this.toastTimer = null;
    if (!this.toast) return;
    this.toast = null;
    this.animateGeometry(true);
    State.notify();
  }

  /** What Mochi wears by itself right now (see outfits.ts); null = what you picked. */
  setOutfit(hat: HatKind | null, extras: Extras) {
    const same =
      this.engine.autoHat === hat &&
      JSON.stringify(this.engine.autoExtras) === JSON.stringify(extras);
    if (same) return;
    this.engine.autoHat = hat;
    this.engine.autoExtras = extras;
    this.ensureRunning();
  }

  /** A few particles around Mochi (sweat when the PC struggles, stars to celebrate…). */
  particles(type: "sweat" | "star" | "spark" | "heart" | "z", count: number) {
    if (State.paused || State.mode === "hidden") return;
    this.engine.emit(type, count);
    this.ensureRunning();
  }

  /** Mochi looks over at what just came in (the banner sits to his right). */
  glance() {
    if (State.paused || State.mode === "hidden") return;
    this.engine.glance(1);
    this.ensureRunning();
  }

  /** Mochi waves a hand to get the user's attention (a reminder just popped up). */
  attention() {
    this.engine.attention();
  }

  /** An alert stopped waiting for an answer: let the island auto-close again. */
  dropPin() {
    this.fsm.pinned = false;
  }

  // ── File drop ───────────────────────────────────────────────────────────────

  private onDragDrop(e: { type: string; paths?: string[] }) {
    if (e.type !== "over") void Bridge.log(`drag ${e.type} ${e.paths?.length ?? 0} file(s)`);
    if (State.paused) return;
    switch (e.type) {
      case "enter":
      case "over": {
        if (State.fileDragOver) return;
        State.fileDragOver = true;
        this.engine.animateMorph(1);
        // enterZone must run before the island expands, so the sequence is
        // already active by the time the view becomes `upload`.
        UploadSeq.enterZone(State.mouseInIsland.x, State.mouseInIsland.y);
        this.alert("upload");
        break;
      }
      case "leave": {
        if (!State.fileDragOver) return;
        State.fileDragOver = false;
        this.engine.animateMorph(0);
        // The island deliberately stays open: the drag session is still alive.
        UploadSeq.exitZone();
        State.notify();
        break;
      }
      case "drop": {
        State.fileDragOver = false;
        const path = e.paths?.[0];
        if (!path) {
          this.engine.animateMorph(0);
          this.setView(State.defaultView());
          return;
        }
        this.swallow(path);
        break;
      }
    }
  }

  /** A reaction with one of Mochi's emotes — only worth the frames while it can be seen. */
  react(emote: BotEmoteName, duration?: number) {
    if (State.paused || State.mode === "hidden") return;
    this.engine.triggerEmote(emote, duration);
    this.ensureRunning();
  }

  /** Click on the drop zone: choose a file with the system dialog. */
  private async pickFile() {
    if (State.paused || this.pickingFile) return;
    this.pickingFile = true;
    // The dialog takes focus away; without the pin the island would auto-close.
    this.fsm.pinned = true;
    try {
      const path = await Bridge.pickFile();
      if (path) this.swallow(path);
    } finally {
      this.pickingFile = false;
      this.fsm.pinned = State.isPinned;
    }
  }

  /**
   * Mochi eats the file. Nothing here waits on the file system: the copy into
   * the inbox runs in the background and swaps the path in when it lands, so a
   * slow disk can never stall the animation — same as FileDropHandler on macOS.
   */
  private swallow(path: string) {
    const name = path.split(/[\\/]/).pop() || "file";
    State.droppedFile = { name, path };
    State.promptContext = { kind: "file", name, path };
    State.chatHistory = [];
    void Bridge.chatReset();

    // A drag has already activated the sequence on entry; picking a file with the
    // dialog hasn't, and an inactive sequence draws a bar frozen at 0 %.
    if (!UploadSeq.isActive) UploadSeq.enterZone(State.mouseInIsland.x, State.mouseInIsland.y);
    UploadSeq.performDrop(State.uploadDuration);
    this.uploadTens = 0;
    this.uploadDone = false;

    this.engine.gulp();
    Sound.play("approve");
    this.engine.triggerEmote("happy");
    this.engine.animateMorph(0);

    State.uploadProgress = 0;
    this.setView("uploading");
    this.ensureRunning();

    void Bridge.ingestFile(path)
      .then((file) => {
        State.droppedFile = { name: file.name, path: file.path };
        State.promptContext = { kind: "file", name: file.name, path: file.path };
        State.notify();
      })
      .catch((err) => {
        UploadSeq.deactivate();
        State.noteMessage = String(err).replace(/^Error:\s*/, "");
        this.engine.animateMorph(0);
        this.setView("note");
        Sound.play("error");
        window.setTimeout(() => this.setView(State.defaultView()), 2400);
      });
  }

  /**
   * Sounds and view changes hung off the canvas timeline: a `tick` every 10 %,
   * the ✓ chime when the bar completes, then `choose` once Mochi has grown back.
   */
  private stepSequence() {
    const since = UploadSeq.sinceDrop();
    if (since == null) return;
    const dur = State.uploadDuration;
    const p = Math.max(0, Math.min(1, (since - PRE_PROGRESS) / dur));

    const tens = Math.floor(p * 10);
    if (tens > this.uploadTens && tens < 10) {
      this.uploadTens = tens;
      Sound.play("tick");
    }

    if (!this.uploadDone && since >= PRE_PROGRESS + dur) {
      this.uploadDone = true;
      Sound.play("approve");
      this.engine.triggerEmote("happy");
    }
    // The extra second is the grow-back, after which the choose card is up.
    if (since >= PRE_PROGRESS + dur + 1 && State.view === "uploading") {
      this.setView("choose");
    }
  }

  // ── Geometry ────────────────────────────────────────────────────────────────

  private targetSize(): { w: number; h: number; r: number } {
    let { w, h } = islandSize(State.mode, State.view, State.chatHistory.length, this.dock, chatChars());
    // A notification banner widens the compact island for a moment.
    // Docked upright to a side edge, it turns horizontal for the banner and
    // goes back to the upright bar afterwards.
    if (this.toast && State.mode === "compact") {
      w = TOAST_W;
      h = NOTCH_H;
    }
    const r = State.mode === "expanded" ? EXPANDED_CORNER : ROUNDED_CORNER;
    return { w, h, r };
  }

  private animateGeometry(shrinking: boolean) {
    const { w, h, r } = this.targetSize();
    if (shrinking) {
      this.width.curveTowards(w);
      this.height.curveTowards(h);
      this.radius.curveTowards(r);
    } else {
      this.width.springTo(w);
      this.height.springTo(h);
      this.radius.springTo(r);
    }
    this.ensureRunning();
  }

  private applyGeometry() {
    const w = this.width.value;
    const hh = this.height.value;
    const r = this.radius.value;
    this.islandEl.style.width = `${w}px`;
    this.islandEl.style.height = `${hh}px`;
    // Upright (docked + retracted): round the sides facing away from the edge.
    const upright = this.dock !== 0 && State.mode !== "expanded" && !(this.toast && State.mode === "compact");
    // The side touching the screen edge is never rounded, in any size.
    this.islandEl.style.borderRadius = upright
      ? this.dock < 0
        ? `0 ${r}px ${r}px 0`
        : `${r}px 0 0 ${r}px`
      : this.dock < 0
        ? `0 0 ${r}px 0`
        : this.dock > 0
          ? `0 0 0 ${r}px`
          : `0 0 ${r}px ${r}px`;
    // Centred in the window, or flush with the side it is docked to.
    this.islandEl.style.left = `${this.anchorX(w)}px`;
    this.islandEl.style.transform = "none";
    // These follow the island as it resizes, so they belong here rather than in
    // the state-driven DOM sync.
    if (upright && State.mode === "compact") {
      this.unreadDot.style.left = `${w / 2 + 6}px`;
      this.unreadDot.style.top = "22px";
      this.miniGrid.style.left = `${w / 2 - 14.5}px`;
      this.miniGrid.style.top = `${hh - 40 - 14.5}px`;
    } else {
      this.unreadDot.style.left = "50px";
      this.unreadDot.style.top = "6px";
      this.miniGrid.style.left = `${w - 40 - 14.5}px`;
      this.miniGrid.style.top = `${hh / 2 - 14.5}px`;
    }
    this.greetingCanvas.style.left = `${(w - EXPANDED_W) / 2}px`;
    this.uploadCanvas.el.style.left = `${(w - EXPANDED_W) / 2}px`;

    const rect = { x: this.anchorX(w), y: 0, w, h: hh };
    const p = this.pushedRect;
    if (Math.abs(p.x - rect.x) > 0.5 || Math.abs(p.w - rect.w) > 0.5 || Math.abs(p.h - rect.h) > 0.5) {
      this.pushedRect = rect;
      void Bridge.setIslandRect(rect.x, rect.y, rect.w, rect.h);
    }
  }

  /** Island rect in window coordinates (origin top-left of the 720×320 window). */
  private islandRect(): { x: number; y: number; w: number; h: number } {
    const w = this.width.value;
    const hh = this.height.value;
    return { x: this.anchorX(w), y: 0, w, h: hh };
  }

  /**
   * Left edge of an island `w` wide inside the window. The window always carries
   * the full 640 px island flush with the screen edge when docked, so a narrower
   * island has to hug the same side instead of floating at the window centre.
   */
  private anchorX(w: number): number {
    const margin = (PANEL_W - EXPANDED_W) / 2;
    if (this.dock < 0) return margin;
    if (this.dock > 0) return PANEL_W - margin - w;
    return (PANEL_W - w) / 2;
  }

  // ── Window collapse (hidden → tiny wake strip, zero polling) ────────────────

  private updateWindowCollapsed() {
    if (this.collapseTimer != null) {
      window.clearTimeout(this.collapseTimer);
      this.collapseTimer = null;
    }
    if (State.mode === "hidden") {
      // Let the island finish retracting, then drop the window to the wake strip:
      // from there the OS delivers no cursor events, so nothing polls at all.
      this.collapseTimer = window.setTimeout(() => {
        this.collapseTimer = null;
        if (State.mode !== "hidden") return;
        this.collapsed = true;
        this.syncDockClass();
        void Bridge.setCollapsed(true);
      }, 420);
    } else if (this.collapsed) {
      // Grow the window back before the island animates open.
      this.collapsed = false;
      this.syncDockClass();
      void Bridge.setCollapsed(false);
    }
  }

  /** -1 / 1 = docked to the left / right screen edge, 0 = free. */
  private dock = 0;

  setDock(side: number) {
    if (side === this.dock) return;
    this.dock = side;
    // Retracted shapes change with the dock, so re-target size and Mochi.
    this.animateGeometry(false);
    this.updateBotTargets();
    this.applyGeometry();
  }

  /** Docked and retracted: the wake strip lies along the edge (see style.css). */
  private syncDockClass() {
    this.root.classList.toggle("collapsed", this.collapsed);
    this.root.classList.toggle("dock-left", this.collapsed && this.dock < 0);
    this.root.classList.toggle("dock-right", this.collapsed && this.dock > 0);
  }

  // ── Input ───────────────────────────────────────────────────────────────────

  private wireInput() {
    // The wake strip is the only thing the OS can hit while the island is hidden.
    this.wakeStrip.addEventListener("mouseenter", () => {
      Sound.resume();
      if (State.mode === "hidden") this.fsm.mouseEntered();
    });

    this.islandEl.addEventListener("mousedown", (e) => {
      Sound.resume();
      State.lastActivity = performance.now();
      if (State.mode !== "expanded") {
        this.fsm.click();
        return;
      }
      if (this.isBotHit(e.clientX, e.clientY)) {
        this.cancelBotHover();
        this.engine.slap();
      }
      this.watchForDrag(e);
    });

    // Once a file has hovered the island, the drop card is drawn on a canvas and
    // the HTML card underneath stops taking clicks — so the "click to choose a
    // file" of the HTML card has to be offered from here too.
    this.islandEl.addEventListener("click", (e) => {
      if (State.view !== "upload" || !this.uploadActive || UploadSeq.dropped) return;
      if (this.header.el.contains(e.target as Node)) return;
      void this.pickFile();
    });

    window.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && State.mode === "expanded" && !State.isPinned) this.collapse();
      State.lastActivity = performance.now();
    });

    this.fsm.onHomeCollapseTimer = (deadline) => this.syncCountdown(deadline);

    // The chat (or anything else) asking Mochi to show a feeling.
    window.addEventListener("mochi-react", (e) => {
      this.react((e as CustomEvent<BotEmoteName>).detail);
    });

    // After a minute with nobody around, Mochi yawns once; it only yawns again
    // after being touched. A 15 s tick is the whole cost of it.
    let yawned = false;
    window.setInterval(() => {
      if (State.mode === "hidden" || State.paused || State.view === "greeting") return;
      if (performance.now() - State.lastActivity < YAWN_AFTER_MS) {
        yawned = false;
        return;
      }
      if (yawned) return;
      yawned = true;
      this.react("yawn", 2.4);
    }, 15000);

    window.addEventListener("mochi-remembered", () => {
      Sound.play("approve");
      this.engine.triggerEmote("remember");
    });

    // Without preventDefault() on dragenter/dragover the page refuses every drop
    // (the "not allowed" cursor). The file itself is handled by Tauri's native
    // drag-drop event below; this only tells the page that dropping is fine.
    for (const type of ["dragenter", "dragover", "drop"]) {
      window.addEventListener(type, (e) => {
        e.preventDefault();
        const dt = (e as DragEvent).dataTransfer;
        if (dt) dt.dropEffect = "copy";
      });
    }

    void onDragDrop((e) => this.onDragDrop(e));

    // Outside Tauri (plain browser) drive the cursor from DOM events so the
    // island can be inspected with `npm run dev`.
    if (!IS_TAURI) {
      window.addEventListener("mousemove", (e) => this.onCursor(e.clientX, e.clientY));
    }
  }

  /**
   * Moving the mouse more than a few px with the button held on the island's
   * background hands the move over to Windows, so the island can be placed
   * anywhere. Controls and text fields keep their own behaviour.
   */
  private watchForDrag(down: MouseEvent) {
    if (down.button !== 0 || !IS_TAURI) return;
    const target = down.target as HTMLElement | null;
    if (target?.closest("button, input, textarea, select, a, [contenteditable], [data-nodrag]")) return;
    const startX = down.screenX;
    const startY = down.screenY;
    const stop = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", stop);
    };
    const move = (e: MouseEvent) => {
      if (e.buttons !== 1) return stop();
      if (Math.hypot(e.screenX - startX, e.screenY - startY) < 5) return;
      stop();
      void Bridge.beginDrag();
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", stop);
  }

  /** Cursor in window-logical coordinates. */
  onCursor(x: number, y: number) {
    this.lastCursorMs = performance.now();
    State.mouse = { x, y };
    const rect = this.islandRect();
    State.mouseInIsland = { x: x - rect.x, y: y - rect.y };

    // Windows sends no cursor position with an OLE drag, so the drop sequence is
    // fed from the Win32 cursor poll instead — it runs throughout the drag.
    if (UploadSeq.isActive && !UploadSeq.dropped) {
      UploadSeq.updateCursor(State.mouseInIsland.x, State.mouseInIsland.y);
    }

    const inIsland =
      x >= rect.x - HIT_MARGIN && x <= rect.x + rect.w + HIT_MARGIN &&
      y >= rect.y - HIT_MARGIN && y <= rect.y + rect.h + HIT_MARGIN;

    if (inIsland && !this.wasInIsland) {
      if (this.fsm.state === "coucou") this.greeting.hover();
      if (this.toast && State.mode === "compact") {
        // Pointing at the banner opens what it is about.
        const open = this.toast.open;
        this.endToast();
        if (typeof open === "function") open();
        else if (open) this.alert(open);
        else this.fsm.mouseEntered();
      } else {
        this.fsm.mouseEntered();
      }
    }
    if (!inIsland && this.wasInIsland) {
      this.fsm.mouseLeft();
    }
    this.wasInIsland = inIsland;
    this.islandEl.classList.toggle("away", !inIsland);

    // Bot hover → love
    const overBot = State.mode === "expanded" && State.stateOverride == null && this.isBotHit(x, y);
    if (overBot && !this.botHovering) this.botHoverIn(x, y);
    if (!overBot && this.botHovering) this.cancelBotHover();
    this.botHovering = overBot;
    if (this.botHovering) {
      const d = Math.hypot(x - this.botHoverStart.x, y - this.botHoverStart.y);
      if (d > 40) {
        this.botHoverStart = { x, y };
        this.scheduleLove();
      }
    }

    this.ensureRunning();
  }

  private isBotHit(x: number, y: number): boolean {
    const rect = this.islandRect();
    const cx = rect.x + this.botCx.value;
    const cy = rect.y + this.botCy.value;
    const radius = this.botSize.value / 2;
    return (x - cx) ** 2 + (y - cy) ** 2 <= radius * radius;
  }

  private botHoverIn(x: number, y: number) {
    if (performance.now() / 1000 - this.lastLoveTime < 6) return;
    this.botHoverStart = { x, y };
    this.engine.blink();
    this.engine.tgEs = 1.08;
    Sound.play("hover");
    this.scheduleLove();
  }

  private scheduleLove() {
    if (this.botHoverTimer != null) window.clearTimeout(this.botHoverTimer);
    this.botHoverTimer = window.setTimeout(() => {
      this.botHoverTimer = null;
      if (!this.botHovering || State.stateOverride != null) return;
      if (performance.now() / 1000 - this.lastLoveTime < 6) return;
      this.lastLoveTime = performance.now() / 1000;
      this.engine.triggerEmote("love");
      Sound.play("love");
    }, 1900);
  }

  private cancelBotHover() {
    if (this.botHoverTimer != null) window.clearTimeout(this.botHoverTimer);
    this.botHoverTimer = null;
    this.engine.tgEs = 1;
  }

  /** The user shook the island around: same dizziness as three slaps. */
  shaken() {
    if (State.mode === "hidden") return;
    this.handleDizzy();
  }

  /** Three slaps → dizzy + confused view for 3.3 s, then back. */
  private handleDizzy() {
    this.prevViewBeforeConfused = State.view;
    State.stateOverride = "dizzy";
    this.engine.setState("dizzy");
    Sound.play("dizzy");
    this.alert("confused");
    if (this.confusedRecovery != null) window.clearTimeout(this.confusedRecovery);
    this.confusedRecovery = window.setTimeout(() => {
      this.confusedRecovery = null;
      State.stateOverride = null;
      this.engine.setState(State.effectiveState);
      if (State.view === "confused") {
        const fallback = State.defaultView();
        this.setView(this.prevViewBeforeConfused === "confused" ? fallback : this.prevViewBeforeConfused);
      }
      this.engine.triggerEmote("happy");
    }, 3300);
  }

  // ── Frame loop ──────────────────────────────────────────────────────────────

  ensureRunning() {
    if (this.running) return;
    this.running = true;
    this.lastFrame = performance.now();
    requestAnimationFrame(this.frame);
  }

  /** Only slow, looping motion is left on screen (see AMBIENT_FRAME_MS). */
  private ambientOnly(nowMs: number): boolean {
    return (
      !this.dirty &&
      !this.width.animating && !this.height.animating && !this.radius.animating &&
      this.botCx.settled && this.botCy.settled && this.botSize.settled &&
      !this.engine.tweening &&
      !UploadSeq.isActive &&
      !(State.mode === "expanded" && State.view === "greeting") &&
      nowMs - this.lastCursorMs > CURSOR_ACTIVE_MS
    );
  }

  private frame = (nowMs: number) => {
    const sinceLast = nowMs - this.lastFrame;
    const budget = State.mode === "compact" ? AMBIENT_FRAME_MS_COMPACT : AMBIENT_FRAME_MS;
    if (sinceLast < budget - 1 && this.ambientOnly(nowMs)) {
      // Sleep through the rest of the slot instead of waking every vsync.
      window.setTimeout(() => requestAnimationFrame(this.frame), budget - sinceLast);
      return;
    }
    const dt = Math.min(0.05, sinceLast / 1000);
    this.lastFrame = nowMs;

    this.width.step(dt, nowMs);
    this.height.step(dt, nowMs);
    this.radius.step(dt, nowMs);
    this.applyGeometry();

    if (this.dirty) {
      this.dirty = false;
      this.syncDom();
    }

    this.updateBotTargets();
    this.botCx.step(dt);
    this.botCy.step(dt);
    this.botSize.step(dt);

    const greetingActive = State.mode === "expanded" && State.view === "greeting";
    if (greetingActive) {
      const gctx = this.greetingCanvas.getContext("2d");
      if (gctx) {
        const dpr = Math.min(2, window.devicePixelRatio || 1);
        gctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        this.greeting.draw(gctx);
      }
    } else {
      // Kept running even while the drop canvas is up, so the island's own Mochi
      // is already in the right place the moment the canvas fades out.
      this.drawBot(dt);
    }

    const uploadActive = this.uploadActive;
    if (uploadActive) this.uploadCanvas.draw(UploadSeq.frame(), nowMs / 1000);
    this.uploadCanvas.el.classList.toggle("on", uploadActive);
    this.viewsEl.classList.toggle("hidden-by-upload", uploadActive);

    tickMiniBots(
      dt,
      State.mode === "compact"
        ? this.miniGrid
        : State.mode === "expanded" && State.view === "overview"
          ? this.views.get("overview")?.el ?? null
          : null,
    );
    this.views.get(State.view)?.tick?.(nowMs);
    if (UploadSeq.isActive) this.stepSequence();

    // Nothing is drawn while the island is hidden, so nothing may keep the loop
    // alive either. This used to read `... || this.engine.busy || State.mode !==
    // "hidden"`, and engine.busy is permanently true for any state with a
    // looping animation — breathing, ratelimit sweat, sleeping z's, the search
    // sweep — so a hidden island went on burning frames in exactly the states it
    // spends most of its life in. Geometry still has to finish retracting.
    const settling =
      this.width.animating || this.height.animating || this.radius.animating;
    const busy = State.mode === "hidden"
      ? settling
      : settling ||
        !this.botCx.settled || !this.botCy.settled || !this.botSize.settled ||
        greetingActive || this.engine.busy || UploadSeq.isActive;

    if (busy) {
      requestAnimationFrame(this.frame);
    } else {
      this.running = false;
      Sound.idle();
    }
  };

  private updateBotTargets() {
    const p = botPosition(State.mode, State.view, this.height.value, State.uploadProgress);
    if (this.dock !== 0 && State.mode === "compact" && !this.toast) {
      // Upright bar: Mochi at the top, the mini grid at the bottom.
      p.cx = VERTICAL_W / 2;
      p.cy = 36;
    }
    this.botCx.target = p.cx;
    this.botCy.target = p.cy;
    this.botSize.target = p.diameter / 0.6;

    const greetingActive = State.mode === "expanded" && State.view === "greeting";
    // The drop canvas draws its own Mochi; two of them would overlap.
    const visible = p.opacity > 0 && !greetingActive && !this.uploadActive;
    this.botCanvas.style.opacity = visible ? "1" : "0";

    if (State.mode === "expanded" && State.view !== "uploading" && !greetingActive && !this.uploadActive) {
      const d = p.diameter;
      const color = botGlowColor(State.effectiveState);
      this.botGlow.style.display = "block";
      this.botGlow.style.width = `${d * 2.2}px`;
      this.botGlow.style.height = `${d * 2.2}px`;
      this.botGlow.style.left = `${this.botCx.value - d * 1.1}px`;
      this.botGlow.style.top = `${this.botCy.value - d * 1.1}px`;
      this.botGlow.style.background = `radial-gradient(circle, ${color} 0%, transparent 62%)`;
      this.botGlow.style.opacity = String(botGlowOpacity(State.effectiveState));
    } else {
      this.botGlow.style.display = "none";
    }
  }

  private drawBot(dt: number) {
    const size = this.botSize.value;
    const w = Math.max(1, Math.round(size));
    const hCss = w + BOT_OVERHANG;
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    if (this.canvasPx !== w) {
      this.canvasPx = w;
      this.botCanvas.width = Math.round(w * dpr);
      this.botCanvas.height = Math.round(hCss * dpr);
      this.botCanvas.style.width = `${w}px`;
      this.botCanvas.style.height = `${hCss}px`;
    }
    this.botCanvas.style.left = `${this.botCx.value - w / 2}px`;
    this.botCanvas.style.top = `${this.botCy.value - BOT_OVERHANG / 2 - hCss / 2}px`;

    const ctx = this.botCanvas.getContext("2d");
    if (!ctx) return;

    const focus = State.focusTask;
    this.engine.bodyColor = focus?.isIntegration ? hexToRGB(focus.color) : null;
    this.engine.particleOverhang = BOT_OVERHANG;
    this.engine.lookX = this.lookX();
    this.engine.lookY = this.lookY();
    this.engine.setMusic(State.musicPlaying);
    if (this.engine.morph > 0.3) {
      this.engine.slotHTarget = State.fileDragOver ? 0.2 : 0;
    } else {
      this.engine.slotHTarget = 0;
      if (this.engine.morph < 0.05) {
        this.engine.slotH = 0;
        this.engine.slotHVel = 0;
      }
    }
    this.engine.update(dt);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, hCss);
    this.engine.draw(ctx, w, hCss);
  }

  /** BotCanvasView.lookX / lookY — tanh of the distance to the bot. */
  private lookX(): number {
    const rect = this.islandRect();
    const botScreenX = rect.x + this.botCx.value;
    return Math.tanh((State.mouse.x - botScreenX) / 260);
  }

  private lookY(): number {
    return -Math.tanh((State.mouse.y - this.botCy.value) / 200);
  }

  /**
   * The bar that shrinks before the island closes by itself. It follows the
   * state machine's own timer (`deadline`), so it appears for every auto-close —
   * alerts included, not only after the cursor left — and it is a CSS transition,
   * so it keeps moving while the frame loop sleeps and costs no frames.
   */
  private syncCountdown(deadline: number | null) {
    if (this.countdownTimer != null) {
      window.clearTimeout(this.countdownTimer);
      this.countdownTimer = null;
    }
    const bar = this.countdown;
    bar.style.transition = "none";
    bar.style.width = "0px";
    if (deadline == null) return;

    const windowMs = Math.min(10, State.settings.autoCloseInterval * 0.6) * 1000;
    const start = () => {
      this.countdownTimer = null;
      const left = deadline - performance.now();
      if (left <= 0) return;
      bar.style.width = `${COUNTDOWN_WIDTH * Math.min(1, left / windowMs)}px`;
      void bar.offsetWidth; // commit the starting width before animating from it
      bar.style.transition = `width ${left}ms linear`;
      bar.style.width = "0px";
    };
    const wait = deadline - performance.now() - windowMs;
    if (wait <= 0) start();
    else this.countdownTimer = window.setTimeout(start, wait);
  }

  // ── DOM sync ────────────────────────────────────────────────────────────────

  private syncDom() {
    const expanded = State.mode === "expanded";
    const greetingActive = expanded && State.view === "greeting";

    // Retracted, the island shows nothing, so nothing inside it may keep animating.
    this.contentEl.classList.toggle("paused", !(expanded && !greetingActive));
    this.contentEl.style.opacity = expanded && !greetingActive ? "1" : "0";
    this.contentEl.style.pointerEvents = expanded && !greetingActive ? "auto" : "none";
    // Content faded out (compact / hidden): its CSS animations would still repaint.
    this.contentEl.classList.toggle("asleep", !expanded || greetingActive);
    this.greetingCanvas.style.display = greetingActive ? "block" : "none";

    this.header.sync();
    for (const [name, view] of this.views) {
      const on = name === State.view;
      view.el.classList.toggle("on", on);
      if (on) view.sync();
    }

    // The chat is the only view with a text field, so it is the only time the
    // island is allowed to take keyboard focus.
    if (this.lastSyncedView !== State.view) {
      const wasChat = this.lastSyncedView === "prompt";
      this.lastSyncedView = State.view;
      if (State.view === "prompt") {
        void Bridge.focusWindow(true);
        window.setTimeout(() => this.views.get("prompt")?.focus?.(), 120);
      } else if (wasChat) {
        void Bridge.focusWindow(false);
      }
    }

    // Compact mini grid
    const showGrid = State.mode === "compact";
    const toastOn = showGrid && this.toast != null;
    this.miniGrid.style.opacity = showGrid && !toastOn ? "1" : "0";
    this.toastEl.classList.toggle("on", toastOn);
    this.unreadDot.style.display = showGrid && !toastOn && State.osUnread > 0 ? "" : "none";
    if (showGrid) {
      const others = State.otherTasks.slice(0, 4);
      const key = others.map((t) => t.id).join("|");
      if (this.miniGrid.dataset.key !== key) {
        this.miniGrid.dataset.key = key;
        this.miniGrid.replaceChildren();
        for (const t of others) {
          this.miniGrid.append(createMiniBot(t, 13));
        }
        pruneMiniBots();
      }
    }

    syncMiniBotStates(State.tasks);
    this.engine.setState(State.effectiveState);
  }

  /** Applies settings coming from Rust at boot. */
  applySettings() {
    Sound.setEnabled(State.settings.soundEnabled);
    Sound.setVolume(State.settings.soundVolume);
    this.fsm.homeToPetitDelay = State.settings.autoCloseInterval;
    this.fsm.petitToHiddenDelay = Math.max(0, State.settings.hideAfter || 0);
    // Hiding completely is opt-in; when it is on, the wake strip shows a small
    // handle so the island can be found again without the tray.
    this.root.classList.toggle("auto-hide", this.fsm.petitToHiddenDelay > 0);
    const { mochiHat, mochiFace } = State.settings;
    this.engine.hat = isHat(mochiHat) ? mochiHat : "none";
    this.engine.face = isFace(mochiFace) ? mochiFace : "none";
    this.engine.neck = isNeck(State.settings.mochiNeck) ? State.settings.mochiNeck : "none";
    applyTransparency(State.settings);
    State.notify();
  }

  get panelSize() {
    return { w: PANEL_W, h: PANEL_H };
  }

  get chatHeight() {
    return chatPromptHeight(State.chatHistory.length, chatChars());
  }
}
