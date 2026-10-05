// Island views — DOM ports of IslandViewContent.swift. Paddings, font sizes,
// colours and wording are copied from the Swift views so both platforms read
// identically.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { Ticker } from "./ticker";
import { SPOTIFY_ID, State, type AgentTask } from "../core/state";
import {
  CLIP_ACTIONS, clipActionLabel, clipKind, clipKindLabel, currentClip, type ClipAction,
} from "../island/clipboard";
import { Bridge } from "../core/bridge";
import { washRGBA, type IslandViewName, type Wash } from "../core/layout";
import { createMiniBot, pruneMiniBots } from "../mochi/minibots";
import { buildPrompt } from "./chat";
import { buildChoose, buildUpload, buildUploading } from "./upload";
import { renderIntegrationCard, spotifyKey, tickSpotifyCard, timeAgo, type IntegrationCardHooks } from "./integrations";
import { t } from "../core/i18n";

export interface ViewActions {
  setView(v: IslandViewName): void;
  collapse(): void;
  setFocus(id: string): void;
  openTerminal(): void;
  /** The ↗ button: opens whatever the focused pill points at. */
  openTarget(): void;
  openUrl(url: string): void;
  decide(d: "allow" | "deny"): void;
  toggleSound(): void;
  setVolume(v: number): void;
  /** The header's — button: fold to the compact island right away. */
  minimize(): void;
  setAutoClose(seconds: number): void;
  openSettingsWindow(): void;
  blip(): void;
  /** Clicking the drop zone: pick a file with the system dialog instead of dragging. */
  pickFile(): void;
  /** Windows notifications: silence all / one app, page through, grant access. */
  toggleNotificationsMuted(): void;
  muteNotificationApp(app: string): void;
  browseNotifications(delta: number): void;
  /** OK on the card: drop the notification shown from the list, then close. */
  dismissNotification(): void;
  /** Clipboard card: run an action on the copy shown, page through, drop it. */
  clipAction(action: ClipAction): void;
  browseClips(delta: number): void;
  dismissClip(): void;
  openNotificationAccess(): void;
  checkNotificationAccess(): void;
}

export interface ViewHost {
  el: HTMLElement;
  sync(): void;
  /** Called when the view becomes active, for views with a text field. */
  focus?(): void;
  /** Called every frame while the view is on screen. */
  tick?(nowMs: number): void;
}

// ── Shared pieces ─────────────────────────────────────────────────────────────

function card(wash: Wash, ...children: (Node | string)[]): HTMLElement {
  const el = h("div", { class: wash ? "card wash" : "card" }, ...children);
  if (wash) el.style.setProperty("--wash", washRGBA(wash));
  return el;
}

function btn(
  label: string,
  kind: "primary" | "secondary",
  onClick: () => void,
  kbd?: string,
): HTMLElement {
  return h(
    "button",
    { class: `btn ${kind}`, onclick: onClick },
    h("span", { text: label }),
    kbd ? h("span", { class: "kbd", text: kbd }) : null,
  );
}

/** AgentWho — coloured dot + task name + grey label. */
function agentWho(task: AgentTask | null, label: string): HTMLElement {
  const row = h("div", { class: "who-row" });
  if (task) {
    row.append(dot(task.color, 8), h("span", { class: "n", text: task.name }));
  }
  row.append(h("span", { text: label }));
  return row;
}

function stack(padLeft: number, padRight: number, ...children: Node[]): HTMLElement {
  const el = h("div", { class: "stack" }, ...children);
  el.style.padding = `4px ${padRight}px 4px ${padLeft}px`;
  return el;
}

// ── Header ────────────────────────────────────────────────────────────────────

export function buildHeader(actions: ViewActions): ViewHost {
  const tabHome = h("button", { class: "tab", title: t("Overview"), onclick: () => go("overview") }, svg(ICONS.house, 13));
  const tabChat = h("button", { class: "tab", title: t("Ask"), onclick: () => go("prompt") }, svg(ICONS.bubble, 13));
  const tabDrop = h("button", { class: "tab", title: t("Drop"), onclick: () => go("upload") }, svg(ICONS.plus, 13));

  const gearBtn = h("button", { title: t("Settings"), onclick: () => go("settings") }, svg(ICONS.gear, 14));
  const resetBtn = h(
    "button",
    { title: t("Reset position"), onclick: () => void Bridge.resetPosition() },
    svg(ICONS.target, 14),
  );
  const soundBtn = h("button", { title: t("Mute"), onclick: () => actions.toggleSound() }, svg(ICONS.speakerOn, 14));
  // Windows notifications: the dot says something arrived while nobody looked.
  const bellDot = h("i", { class: "bell-dot" });
  const bellBtn = h(
    "button",
    { class: "bell", title: t("Notifications"), onclick: () => go("notification") },
    svg(ICONS.bell, 14),
    bellDot,
  );
  // Fold the island now instead of waiting for the auto-close countdown.
  const minimizeBtn = h(
    "button",
    { class: "minimize", title: t("Minimize"), onclick: () => actions.minimize() },
    svg(ICONS.minimize, 14),
  );

  function go(v: IslandViewName) {
    actions.blip();
    actions.setView(v);
  }

  const el = h(
    "div",
    { id: "header" },
    h("div", { class: "tabs" }, tabHome, tabChat, tabDrop),
    h("div", { class: "header-actions" }, bellBtn, gearBtn, resetBtn, soundBtn, minimizeBtn),
  );

  return {
    el,
    sync() {
      const v = State.view;
      tabHome.classList.toggle("on", v === "overview" || v === "empty");
      tabChat.classList.toggle("on", v === "prompt");
      tabDrop.classList.toggle("on", v === "upload");
      gearBtn.classList.toggle("on", v === "settings");
      clear(gearBtn);
      gearBtn.append(svg(v === "settings" ? ICONS.gearFill : ICONS.gear, 14));
      clear(soundBtn);
      soundBtn.append(svg(State.settings.soundEnabled ? ICONS.speakerOn : ICONS.speakerOff, 14));
      el.style.opacity = v === "confused" ? "0" : "1";
      const muted = State.settings.notificationsMuted;
      bellBtn.style.display = State.settings.notifications ? "" : "none";
      bellBtn.classList.toggle("on", v === "notification");
      bellBtn.title = muted ? t("Notifications (silenced)") : t("Notifications");
      if (bellBtn.dataset.muted !== String(muted)) {
        bellBtn.dataset.muted = String(muted);
        bellBtn.replaceChildren(svg(muted ? ICONS.bellSlash : ICONS.bell, 14), bellDot);
      }
      bellDot.style.display = State.osUnread > 0 ? "" : "none";
    },
  };
}

// ── Overview ──────────────────────────────────────────────────────────────────

function buildOverview(actions: ViewActions): ViewHost {
  const ticker = new Ticker();
  const who = h("div", { class: "who" });
  const tickerBody = h("div", { class: "card-body" }, who, ticker.el);
  const leftBody = h("div", { class: "left-body" });
  const jump = h(
    "button",
    { class: "icon-btn jump", title: t("Open"), onclick: () => actions.openTarget() },
    svg(ICONS.arrowUpRight, 8),
  );
  const left = card(null, leftBody, jump);
  const pills = h("div", { class: "pills" });
  const right = card(null, pills);

  const el = h("div", { class: "view overview" },
    h("div", { class: "left" }, left),
    h("div", { class: "right" }, right),
  );

  let pillIds = "";
  let detailOpen = false;
  let lastFocus: string | null = null;
  let mode: "ticker" | "card" | null = null;
  let cardKey = "";

  const hooks: IntegrationCardHooks = {
    get detailOpen() {
      return detailOpen;
    },
    openDetail() {
      detailOpen = true;
      cardKey = "";
      State.notify();
    },
    closeDetail() {
      detailOpen = false;
      cardKey = "";
      State.notify();
    },
    openSettings: () => actions.openSettingsWindow(),
  };

  return {
    el,
    tick(nowMs: number) {
      if (mode === "ticker") ticker.tick(nowMs);
      else if (mode === "card" && lastFocus === SPOTIFY_ID) tickSpotifyCard();
    },
    sync() {
      const task = State.focusTask;
      if (task?.id !== lastFocus) {
        lastFocus = task?.id ?? null;
        detailOpen = false;
        cardKey = "";
        mode = null;
      }

      // VS Code with a live Claude Code session keeps the ticker; every other
      // pill shows its own card, exactly like IntegrationCardView.
      const sessionActive =
        task?.source === "claudeCode" &&
        (task.id !== "integration_claude" || task.state !== "idle" || task.steps.length > 0);

      if (task && sessionActive) {
        if (mode !== "ticker") {
          clear(leftBody);
          leftBody.append(tickerBody);
          mode = "ticker";
          cardKey = "";
        }
        clear(who);
        who.append(
          dot(task.color, 7),
          h("span", { class: "name", text: task.name }),
          h("span", { class: "tool", text: task.source === "claudeCode" ? "Claude Code" : "n8n" }),
        );
        if (task.steps.length > 1) {
          who.append(h("span", {
            class: "count",
            text: `${Math.min(task.stepIndex + 1, task.steps.length)}/${task.steps.length}`,
          }));
        }
        ticker.sync(task);
      } else if (task) {
        const info = State.integrations[task.id];
        const key = [
          task.id, detailOpen, task.state, task.steps.join("|"),
          info?.loaded, info?.error, info?.configured,
          JSON.stringify(info?.data ?? {}),
          task.id === SPOTIFY_ID ? spotifyKey() : "",
        ].join("~");
        if (key !== cardKey) {
          cardKey = key;
          mode = "card";
          clear(leftBody);
          leftBody.append(renderIntegrationCard(task, hooks));
        }
      }

      jump.style.display = detailOpen ? "none" : "";

      const others = State.otherTasks.slice(0, 4);
      // With no other integration to show, an empty right-hand card is just noise:
      // drop it and let the main card use the whole width.
      el.classList.toggle("solo", others.length === 0);
      const pillKey = others.map((t) => `${t.id}:${t.pillBadge ?? ""}`).join("|");
      if (pillKey !== pillIds) {
        pillIds = pillKey;
        clear(pills);
        for (const t of others) pills.append(buildPill(t, actions));
        pruneMiniBots();
      }
    },
  };
}

function buildPill(task: AgentTask, actions: ViewActions): HTMLElement {
  const label = task.id === "integration_claude" ? "VS Code" : task.name;
  const canvas = createMiniBot(task, 24);
  const pill = h(
    "div",
    { class: "pill", onclick: () => actions.setFocus(task.id) },
    canvas,
    h("span", { class: "lbl", text: label }),
  );
  pill.style.borderColor = `${task.color}24`;
  pill.addEventListener("mouseenter", () => {
    pill.style.background = `${task.color}2e`;
    pill.style.borderColor = `${task.color}8c`;
    pill.style.boxShadow = `0 2px 10px ${task.color}59`;
    (pill.querySelector(".lbl") as HTMLElement).style.color = lighten(task.color, 0.3);
  });
  pill.addEventListener("mouseleave", () => {
    pill.style.background = "";
    pill.style.borderColor = `${task.color}24`;
    pill.style.boxShadow = "";
    (pill.querySelector(".lbl") as HTMLElement).style.color = "";
  });

  if (task.pillBadge) {
    const colors = { approval: "#F5A524", finished: "#22C55E", error: "#F4505E" } as const;
    const icons = { approval: ICONS.bang, finished: ICONS.check, error: ICONS.xmark } as const;
    const inner = h("i", { style: `background:${colors[task.pillBadge]}` }, svg(icons[task.pillBadge], 6, { stroke: task.pillBadge === "finished" ? 3 : 0 }));
    const badge = h("div", { class: "pill-badge" }, inner);
    badge.style.boxShadow = `0 0 4px ${colors[task.pillBadge]}99`;
    pill.append(badge);
  }
  return pill;
}

function lighten(hex: string, amount: number): string {
  const v = parseInt(hex.replace("#", ""), 16);
  const c = [(v >> 16) & 255, (v >> 8) & 255, v & 255].map((x) =>
    Math.min(255, Math.round(x + amount * 255)),
  );
  return `rgb(${c[0]},${c[1]},${c[2]})`;
}

// ── Empty ─────────────────────────────────────────────────────────────────────

function buildEmpty(actions: ViewActions): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 118px;flex-direction:row;align-items:center;gap:16px" },
    h(
      "div",
      { style: "display:flex;flex-direction:column;gap:5px" },
      h("div", { class: "title", text: t("Nothing running right now.") }),
      h("div", { class: "sub", text: t("Drop a file or window, or ask me anything.") }),
    ),
    h("div", { class: "grow" }),
    btn(t("Ask Claude"), "primary", () => actions.setView("prompt")),
  );
  return { el: h("div", { class: "view" }, card(null, body)), sync() {} };
}

// ── Approval ──────────────────────────────────────────────────────────────────

function buildApproval(actions: ViewActions): ViewHost {
  const who = h("div");
  const code = h("div", { class: "code" });
  const row = h("div", { class: "actions" });
  const el = h("div", { class: "view" }, card("amber", stack(116, 16, who, code, row)));
  let rowKey = "";
  return {
    el,
    sync() {
      clear(who);
      who.append(agentWho(State.focusTask, t("needs permission")));
      // The whole point of approving here rather than in the terminal: this line
      // is the command, the file path or the URL being authorised, not just the
      // name of the tool asking.
      code.textContent = State.pendingApproval?.command || State.pendingApproval?.tool || "…";
      // Two buttons, built once. Rebuilding them between a mouse-down and a
      // mouse-up would swallow the click, and there is nothing left to vary:
      // "Always" is gone until the remembered-rules list exists to back it.
      if (rowKey === "built") return;
      rowKey = "built";
      clear(row);
      row.append(
        btn(t("Deny"), "secondary", () => actions.decide("deny"), "N"),
        btn(t("Allow"), "primary", () => actions.decide("allow"), "Y"),
      );
    },
  };
}

// ── Question ──────────────────────────────────────────────────────────────────

function buildQuestion(): ViewHost {
  const who = h("div");
  const title = h("div", { class: "title" });
  const row = h("div", { class: "actions" });
  const el = h("div", { class: "view" }, card("cyan", stack(116, 16, who, title, row)));
  return {
    el,
    sync() {
      clear(who);
      who.append(agentWho(State.focusTask, t("Claude Code is asking a question")));
      const task = State.focusTask;
      title.textContent = task?.steps.at(-1) ?? t("Claude needs an answer.");
      clear(row);
      row.append(h("div", { class: "sub", text: t("Answer in your terminal — Coucou can't reply for you yet.") }));
    },
  };
}

// ── Error ─────────────────────────────────────────────────────────────────────

function buildError(actions: ViewActions): ViewHost {
  const who = h("div");
  const title = h("div", { class: "title", text: t("Workflow stopped.") });
  const detail = h("div", { class: "detail" });
  const row = h("div", { class: "actions" },
    btn(t("Retry"), "primary", () => actions.setView(State.defaultView())),
    btn(t("Open in n8n"), "secondary", () => actions.openUrl("")),
  );
  const el = h("div", { class: "view" }, card("red", stack(116, 16, who, title, detail, row)));
  return {
    el,
    sync() {
      const task = State.focusTask;
      clear(who);
      who.append(agentWho(task, task?.source === "n8n" ? "n8n" : "Claude Code"));
      title.textContent = task?.source === "n8n" ? t("Workflow stopped.") : t("Session stopped on an error.");
      detail.textContent = task?.steps.at(-1) ?? t("No detail available.");
    },
  };
}

// ── Finished ──────────────────────────────────────────────────────────────────

function buildFinished(actions: ViewActions): ViewHost {
  const who = h("div");
  const title = h("div", { class: "title" });
  const asked = h("div", { class: "sub", style: "white-space:nowrap;overflow:hidden;text-overflow:ellipsis" });
  const gitLine = h("div", { class: "git-line" });
  const row = h("div", { class: "actions" },
    btn(t("Open terminal"), "primary", () => actions.openTerminal()),
    btn(t("OK"), "secondary", () => actions.collapse()),
  );
  const el = h("div", { class: "view" }, card("green", stack(116, 16, who, title, asked, gitLine, row)));
  return {
    el,
    sync() {
      const task = State.focusTask;
      clear(who);
      who.append(agentWho(task, t("Claude Code finished")));
      // What it did, not just the last thing it touched; and what it was for.
      title.textContent = task?.summary ?? task?.steps.at(-1) ?? t("Session finished");
      asked.textContent = task?.lastPrompt ? t("For: “{prompt}”", { prompt: task.lastPrompt }) : "";
      const g = task?.git;
      // Five lines don't fit the card: with git news, the request shares its line.
      asked.style.display = task?.lastPrompt && !g ? "" : "none";
      gitLine.style.display = g ? "" : "none";
      if (g) {
        const parts = [
          g.files ? t("{n} files changed", { n: g.files }) : "",
          g.untracked ? t("{n} new", { n: g.untracked }) : "",
        ].filter(Boolean);
        clear(gitLine);
        if (task?.lastPrompt) gitLine.append(h("span", { class: "ask", text: asked.textContent ?? "" }));
        gitLine.append(
          h("span", { class: "stat", text: parts.join(" · ") }),
          h("b", { class: "plus", text: `+${g.insertions}` }),
          h("b", { class: "minus", text: `−${g.deletions}` }),
        );
        gitLine.title = g.names.join("\n");
      }
    },
  };
}

// ── Confused ──────────────────────────────────────────────────────────────────

function buildConfused(): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 128px" },
    h("div", { class: "title", text: t("Too many hits at once.") }),
    h("div", { class: "sub", text: t("Give me a sec — back to work in three seconds.") }),
  );
  return { el: h("div", { class: "view" }, card("pink", body)), sync() {} };
}

// ── Note ──────────────────────────────────────────────────────────────────────

function buildNote(): ViewHost {
  const title = h("div", { class: "title" });
  const body = card(null, h("div", { class: "stack", style: "padding:0 18px 0 98px" }, title));
  const el = h("div", { class: "view" }, body);
  return {
    el,
    sync() {
      title.textContent = State.noteMessage ?? "";
      // A watched site or service going down is red, coming back is green.
      const wash: Wash = State.noteKind === "alert" ? "red" : State.noteKind === "recovered" ? "green" : null;
      body.classList.toggle("wash", wash !== null);
      if (wash) body.style.setProperty("--wash", washRGBA(wash));
      else body.style.removeProperty("--wash");
    },
  };
}

// ── In-island settings ────────────────────────────────────────────────────────

/** Quick auto-close choices, in seconds after the mouse leaves. */
const AUTO_CLOSE_CHOICES = [3, 10, 15, 30];

function buildSettings(actions: ViewActions): ViewHost {
  const soundSwitch = h("button", { class: "switch", onclick: () => actions.toggleSound() });
  const volume = h("input", {
    type: "range", min: "0", max: "0.2", step: "0.005",
    oninput: (e: Event) => actions.setVolume(Number((e.target as HTMLInputElement).value)),
  }) as HTMLInputElement;
  const autoLabel = h("span", {});
  const segButtons = AUTO_CLOSE_CHOICES.map((s) =>
    h("button", { onclick: () => actions.setAutoClose(s) }, `${s}s`),
  );
  const claudeBadge = h("span", { class: "status-badge" });
  const apiBadge = h("span", { class: "status-badge" });

  const rows = h(
    "div",
    { class: "settings-rows" },
    h("div", { class: "settings-row" }, soundSwitch, h("span", { text: t("Sound") }), volume),
    h(
      "div",
      { class: "settings-row" },
      svg(ICONS.timer, 12),
      autoLabel,
      h("div", { class: "seg" }, ...segButtons),
    ),
    h(
      "div",
      { class: "settings-row", style: "gap:14px" },
      claudeBadge,
      apiBadge,
      h("div", { class: "grow" }),
      h("button", {
        class: "link-btn",
        style: "color:#8e939c;font-size:11.5px",
        text: t("Settings…"),
        onclick: () => actions.openSettingsWindow(),
      }),
    ),
  );

  const el = h("div", { class: "view" },
    card(null, h("div", { class: "stack", style: "padding:14px 16px 14px 84px" }, rows)));

  return {
    el,
    sync() {
      const s = State.settings;
      soundSwitch.classList.toggle("on", s.soundEnabled);
      volume.value = String(s.soundVolume);
      volume.style.opacity = s.soundEnabled ? "1" : "0.4";
      autoLabel.textContent = t("Auto-close · {s}s", { s: Math.round(s.autoCloseInterval) });
      segButtons.forEach((b, i) => b.classList.toggle("on", s.autoCloseInterval === AUTO_CLOSE_CHOICES[i]));
      clear(claudeBadge);
      claudeBadge.append(
        dot(s.hooksInstalled ? "#22C55E" : "#F4505E", 6),
        h("span", { text: "Claude Code" }),
      );
      clear(apiBadge);
      apiBadge.append(dot(State.apiKeyPresent ? "#22C55E" : "#F4505E", 6), h("span", { text: "API" }));
    },
  };
}

// ── Windows notification ──────────────────────────────────────────────────────

function buildNotification(actions: ViewActions): ViewHost {
  const icon = h("img", { class: "osn-icon", alt: "" }) as HTMLImageElement;
  const app = h("span", { class: "n" });
  const when = h("span", {});
  const pager = h("span", { class: "osn-pager" });
  const prev = h("button", { class: "icon-btn", title: t("Previous"), onclick: () => actions.browseNotifications(1) },
    svg(ICONS.chevronLeft, 8, { stroke: 2.4 }));
  const next = h("button", { class: "icon-btn", title: t("Next"), onclick: () => actions.browseNotifications(-1) },
    svg(ICONS.chevronRight, 8, { stroke: 2.4 }));
  const head = h("div", { class: "who-row osn-head" }, icon, app, when, h("div", { class: "grow" }), prev, pager, next);
  const title = h("div", { class: "title osn-title" });
  const body = h("div", { class: "sub osn-body" });

  const muteAllLabel = h("span", {});
  const muteAll = h("button", { class: "btn secondary osn-btn", onclick: () => actions.toggleNotificationsMuted() });
  const muteAppLabel = h("span", {});
  let currentApp = "";
  const muteApp = h(
    "button",
    { class: "btn secondary osn-btn", onclick: () => currentApp && actions.muteNotificationApp(currentApp) },
    muteAppLabel,
  );
  const gear = h("button", {
    class: "btn secondary osn-btn",
    title: t("Notification settings"),
    onclick: () => actions.openSettingsWindow(),
  }, svg(ICONS.gear, 12));
  const row = h("div", { class: "actions" }, muteAll, muteApp, gear, h("div", { class: "grow" }),
    btn(t("OK"), "primary", () => actions.dismissNotification()));
  const filled = stack(116, 16, head, title, body, row);

  // Nothing to show yet: say why, and offer the way to Windows' permission.
  const emptyTitle = h("div", { class: "title" });
  const emptySub = h("div", { class: "sub" });
  const emptyRow = h("div", { class: "actions" },
    btn(t("Open Windows settings"), "primary", () => actions.openNotificationAccess()),
    btn(t("Check again"), "secondary", () => actions.checkNotificationAccess()),
  );
  const empty = stack(116, 16, emptyTitle, emptySub, emptyRow);

  const el = h("div", { class: "view" }, card("indigo", filled, empty));
  let iconSrc = "";
  return {
    el,
    sync() {
      if (State.view === "notification") State.osUnread = 0;
      const list = State.osNotifications;
      const n = list[Math.min(State.osIndex, list.length - 1)];
      filled.style.display = n ? "" : "none";
      empty.style.display = n ? "none" : "";

      clear(muteAll);
      const muted = State.settings.notificationsMuted;
      muteAllLabel.textContent = muted ? t("Unsilence") : t("Silence");
      muteAll.append(svg(muted ? ICONS.bell : ICONS.bellSlash, 12), muteAllLabel);

      if (!n) {
        const allowed = State.osAccess === "allowed";
        emptyTitle.textContent = allowed
          ? t("No notifications yet.")
          : t("Coucou can't read Windows notifications yet.");
        emptySub.textContent = allowed
          ? t("New ones will show up here as they arrive.")
          : t("Turn on “Notification access” in Windows settings, then come back.");
        emptyRow.style.display = allowed ? "none" : "";
        return;
      }
      currentApp = n.app;
      if ((n.icon ?? "") !== iconSrc) {
        iconSrc = n.icon ?? "";
        icon.src = iconSrc;
      }
      icon.style.display = n.icon ? "" : "none";
      app.textContent = n.app;
      when.textContent = n.time ? timeAgo(n.time) : "";
      title.textContent = n.title;
      body.textContent = n.body;
      body.style.display = n.body ? "" : "none";
      const short = n.app.length > 14 ? `${n.app.slice(0, 13)}…` : n.app;
      muteAppLabel.textContent = t("Mute {app}", { app: short });
      const many = list.length > 1;
      for (const x of [prev, pager, next]) x.style.display = many ? "" : "none";
      pager.textContent = `${State.osIndex + 1}/${list.length}`;
      (prev as HTMLButtonElement).disabled = State.osIndex >= list.length - 1;
      (next as HTMLButtonElement).disabled = State.osIndex <= 0;
    },
  };
}

// ── Clipboard ─────────────────────────────────────────────────────────────────

function buildClipboard(actions: ViewActions): ViewHost {
  const kind = h("span", { class: "n" });
  const when = h("span", {});
  const pager = h("span", { class: "osn-pager" });
  const prev = h("button", { class: "icon-btn", title: t("Previous"), onclick: () => actions.browseClips(1) },
    svg(ICONS.chevronLeft, 8, { stroke: 2.4 }));
  const next = h("button", { class: "icon-btn", title: t("Next"), onclick: () => actions.browseClips(-1) },
    svg(ICONS.chevronRight, 8, { stroke: 2.4 }));
  const head = h("div", { class: "who-row osn-head" }, kind, when, h("div", { class: "grow" }), prev, pager, next);
  const text = h("div", { class: "clip-text" });
  const row = h("div", { class: "actions" });
  const emptyTitle = h("div", { class: "title", text: t("Nothing copied yet.") });
  const emptySub = h("div", { class: "sub" });
  const filled = stack(116, 16, head, text, row);
  const empty = stack(116, 16, emptyTitle, emptySub);
  const el = h("div", { class: "view" }, card("cyan", filled, empty));
  let rowKey = "";
  return {
    el,
    sync() {
      const c = currentClip();
      filled.style.display = c ? "" : "none";
      empty.style.display = c ? "none" : "";
      if (!c) {
        emptySub.textContent = State.settings.clipboard?.enabled
          ? t("Copy some text and Mochi will offer to explain, summarize, translate or fix it.")
          : t("The smart clipboard is off — turn it on in Settings.");
        return;
      }
      const k = clipKind(c.text);
      kind.textContent = clipKindLabel(k);
      when.textContent = timeAgo(c.at);
      text.textContent = c.text;
      text.classList.toggle("mono", k === "code" || k === "error");
      const many = State.clips.length > 1;
      for (const x of [prev, pager, next]) x.style.display = many ? "" : "none";
      pager.textContent = `${State.clipIndex + 1}/${State.clips.length}`;
      (prev as HTMLButtonElement).disabled = State.clipIndex >= State.clips.length - 1;
      (next as HTMLButtonElement).disabled = State.clipIndex <= 0;
      // Buttons only rebuilt when the chosen actions change (a rebuild between
      // mouse-down and mouse-up would swallow the click).
      const enabled = CLIP_ACTIONS.filter((a) => State.settings.clipboard?.actions?.includes(a));
      const key = enabled.join(",");
      if (key !== rowKey) {
        rowKey = key;
        clear(row);
        for (const a of enabled) {
          row.append(h("button", { class: "btn secondary osn-btn", text: clipActionLabel(a), onclick: () => actions.clipAction(a) }));
        }
        row.append(h("div", { class: "grow" }), btn(t("OK"), "primary", () => actions.dismissClip()));
      }
    },
  };
}

// ── Placeholders filled in later stages ───────────────────────────────────────

function buildPlaceholder(title: string, sub: string): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 118px" },
    h("div", { class: "title", text: title }),
    h("div", { class: "sub", text: sub }),
  );
  return { el: h("div", { class: "view" }, card(null, body)), sync() {} };
}

// ── Registry ──────────────────────────────────────────────────────────────────

export function buildViews(
  actions: ViewActions,
  onChatHeightChange: () => void,
): Map<IslandViewName, ViewHost> {
  const map = new Map<IslandViewName, ViewHost>();
  map.set("overview", buildOverview(actions));
  map.set("empty", buildEmpty(actions));
  map.set("approval", buildApproval(actions));
  map.set("question", buildQuestion());
  map.set("error", buildError(actions));
  map.set("finished", buildFinished(actions));
  map.set("confused", buildConfused());
  map.set("note", buildNote());
  map.set("settings", buildSettings(actions));
  map.set("notification", buildNotification(actions));
  map.set("clipboard", buildClipboard(actions));
  map.set("prompt", buildPrompt(onChatHeightChange));
  map.set("upload", buildUpload(actions));
  map.set("uploading", buildUploading());
  map.set("choose", buildChoose(actions));
  // Not in the Windows v1: sending a file by email, window attach + web result.
  map.set("mail", buildPlaceholder(t("Sending by email isn't in this version."), ""));
  map.set("searching", buildPlaceholder(t("Claude is searching…"), ""));
  map.set("result", buildPlaceholder(t("Result"), ""));
  return map;
}
