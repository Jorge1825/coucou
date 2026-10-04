// Settings window — the place where anything that writes to disk is confirmed.
// Stage 2 covers the Claude Code hooks and the general preferences; API keys and
// integrations land here too in a later stage.

import "./settings.css";
import { Bridge, IS_TAURI, onEvent, type HookStatus } from "../core/bridge";
import { PROVIDERS, providerDef } from "../core/providers";
import { DEFAULT_SETTINGS, OPACITY_MIN, clampOpacity, type Settings } from "../core/state";
import { h, clear } from "../views/dom";
import { BotEngine } from "../mochi/engine";
import { Sound } from "../core/sound";
import { FACES, HATS, NECKS, isFace, isHat, isNeck } from "../mochi/accessories";
import {
  calendarSection, clipboardSection, daySection, outfitsSection, sessionsSection, systemSection, type Ctx,
} from "./features";
import { LANGUAGES, setLanguage, t } from "../core/i18n";
import { ICONS } from "../views/icons";
import { buildShell } from "./shell";

let settings: Settings = { ...DEFAULT_SETTINGS };
let version = "";

const root = document.getElementById("settings-root")!;

async function save() {
  await Bridge.saveSettings(settings);
}

// ── Reusable bits ─────────────────────────────────────────────────────────────

function toggle(on: boolean, onChange: (v: boolean) => void): HTMLElement {
  const el = h("button", { class: on ? "switch on" : "switch", "aria-pressed": on });
  el.addEventListener("click", () => {
    const next = !el.classList.contains("on");
    el.classList.toggle("on", next);
    onChange(next);
  });
  return el;
}

function statusDot(ok: boolean): HTMLElement {
  return h("i", { class: "dot", style: `background:${ok ? "#22c55e" : "#f4505e"}` });
}

function renderDiff(text: string): HTMLElement {
  const box = h("div", { class: "diff" });
  for (const line of text.split("\n")) {
    const cls = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx";
    box.append(h("div", { class: cls, text: line }));
  }
  return box;
}

// ── Claude Code section ───────────────────────────────────────────────────────

function claudeSection(status: HookStatus): HTMLElement {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:12px" });
  const section = h(
    "section",
    {},
    h("h2", {}, statusDot(status.installed), h("span", { text: "Claude Code" })),
    body,
  );

  const rebuild = async () => {
    const fresh = await Bridge.hooksStatus();
    if (fresh) Object.assign(status, fresh);
    clear(body);
    draw();
    const head = section.querySelector("h2")!;
    clear(head);
    head.append(statusDot(status.installed), h("span", { text: "Claude Code" }));
  };

  function draw() {
    body.append(
      h("div", {
        class: "hint",
        text: status.installed
          ? t("Coucou is hooked into your Claude Code sessions. Tool calls, questions and permission requests show up in the island, and you can answer them there.")
          : t("Install the hooks to see your Claude Code sessions in the island and approve permissions without leaving what you are doing."),
      }),
      h("div", { class: "row" },
        h("label", { text: "settings.json" }),
        h("span", { class: "path", text: status.settingsPath }),
      ),
      h("div", { class: "row" },
        h("label", { text: t("Relay") }),
        h("span", { class: "path", text: status.hookPath }),
        statusDot(status.hookReady),
      ),
    );

    if (!status.hookReady) {
      body.append(h("div", {
        class: "notice warn",
        text: t("coucou-hook.exe is not in place yet. Restart Coucou; if it still fails, build it with `cargo build -p coucou-hook`."),
      }));
    }

    const actions = h("div", { class: "row" });
    const install = h("button", {
      class: "primary",
      text: status.installed ? t("Reinstall hooks…") : t("Install hooks…"),
      onclick: () => showPreview(true),
    });
    // Writing hook commands that point at a relay which isn't there would give
    // every Claude Code session a broken hook and nothing to show for it.
    if (!status.hookReady) {
      install.disabled = true;
      install.title = t("The relay isn't installed yet.");
    }
    actions.append(install);
    if (status.installed) {
      actions.append(h("button", {
        class: "danger",
        text: t("Uninstall hooks…"),
        onclick: () => showPreview(false),
      }));
    }
    body.append(actions);
  }

  async function showPreview(install: boolean) {
    let preview;
    try {
      preview = await Bridge.hooksPreview(install);
    } catch (err) {
      // An unreadable or invalid settings.json stops here rather than being
      // treated as empty and written over.
      clear(body);
      body.append(
        h("div", { class: "notice err", text: String(err).replace(/^Error:\s*/, "") }),
        h("div", { class: "row" }, h("button", {
          text: t("Back"),
          onclick: () => { clear(body); draw(); },
        })),
      );
      return;
    }
    if (!preview) return;
    clear(body);
    body.append(
      h("div", {
        class: "hint",
        text: install
          ? t("This is exactly what will change in your settings.json. Your own hooks are left untouched.")
          : t("This removes Coucou's entries only. Your own hooks are left untouched."),
      }),
      renderDiff(preview.diff),
      h("div", { class: "row" },
        h("span", { class: "path", text: t("Backup → {path}", { path: preview.backup }) }),
      ),
    );
    const confirm = h("button", {
      class: install ? "primary" : "danger",
      text: install ? t("Back up and write") : t("Back up and remove"),
    });
    confirm.addEventListener("click", async () => {
      confirm.disabled = true;
      try {
        const backup = await Bridge.hooksApply(install, preview.fingerprint);
        clear(body);
        body.append(h("div", {
          class: "notice ok",
          text: t("Done. Previous settings saved as {path}. Open a new Claude Code session to pick the hooks up.", { path: backup }),
        }));
        window.setTimeout(() => void rebuild(), 2600);
      } catch (err) {
        confirm.disabled = false;
        body.append(h("div", { class: "notice err", text: t("Could not write: {error}", { error: String(err) }) }));
      }
    });
    body.append(h("div", { class: "row" }, confirm, h("button", {
      text: t("Cancel"),
      onclick: () => { clear(body); draw(); },
    })));
  }

  draw();
  return section;
}

// ── AI provider section ───────────────────────────────────────────────────────

const MODELS: [string, string][] = [
  ["claude-opus-5", "Claude Opus 5"],
  ["claude-sonnet-5", "Claude Sonnet 5"],
  ["claude-haiku-4-5", "Claude Haiku 4.5"],
];

function apiSection(present: Record<string, boolean>): HTMLElement {
  const section = h("section", {});
  let notice: { ok: boolean; text: string } | null = null;

  function draw() {
    clear(section);
    const def = providerDef(settings.provider);
    const hasKey = !def.key || present[def.key];

    const provider = h("select", {}) as HTMLSelectElement;
    for (const p of PROVIDERS) provider.append(h("option", { value: p.id, text: p.name }));
    provider.value = def.id;
    provider.addEventListener("change", () => {
      settings.provider = provider.value;
      settings.providerUrl = "";
      const next = providerDef(settings.provider);
      if (next.model) settings.model = next.model;
      notice = null;
      void save();
      draw();
    });

    const url = h("input", {
      type: "text",
      placeholder: def.url || "https://your-endpoint.example/v1",
      style: "flex:1 1 auto;min-width:0",
      spellcheck: "false",
    }) as HTMLInputElement;
    url.value = settings.providerUrl;
    url.addEventListener("change", () => {
      settings.providerUrl = url.value.trim();
      void save();
    });

    const field = h("input", {
      type: "password",
      placeholder: def.key && present[def.key] ? t("••••••••••••  (stored)") : def.keyHint || t("API key"),
      style: "flex:1 1 auto;min-width:0",
      autocomplete: "off",
      spellcheck: "false",
    }) as HTMLInputElement;

    const saveKey = h("button", {
      class: "primary",
      text: t("Save key"),
      onclick: async () => {
        if (!def.key) return;
        const value = field.value.trim();
        if (!value) return;
        try {
          await Bridge.secretSet(def.key, value);
          present[def.key] = true;
          notice = { ok: true, text: t("Saved. It never touches disk.") };
        } catch (err) {
          notice = { ok: false, text: t("Could not save: {error}", { error: String(err) }) };
        }
        draw();
      },
    });

    const removeKey = h("button", {
      class: "danger",
      text: t("Remove"),
      onclick: async () => {
        if (!def.key) return;
        try {
          await Bridge.secretClear(def.key);
          present[def.key] = false;
          notice = { ok: true, text: t("Key removed.") };
        } catch (err) {
          notice = { ok: false, text: t("Could not remove: {error}", { error: String(err) }) };
        }
        draw();
      },
    });

    let modelRow: HTMLElement;
    if (def.id === "anthropic") {
      const model = h("select", {}) as HTMLSelectElement;
      for (const [id, label] of MODELS) model.append(h("option", { value: id, text: label }));
      if (!MODELS.some(([id]) => id === settings.model)) {
        model.append(h("option", { value: settings.model, text: settings.model }));
      }
      model.value = settings.model;
      model.addEventListener("change", () => {
        settings.model = model.value;
        void save();
      });
      modelRow = h("div", { class: "row" }, h("label", { text: t("Model") }), model);
    } else {
      const model = h("input", {
        type: "text",
        placeholder: def.model || t("model name"),
        style: "flex:1 1 auto;min-width:0",
        spellcheck: "false",
      }) as HTMLInputElement;
      model.value = settings.model;
      model.addEventListener("change", () => {
        settings.model = model.value.trim();
        void save();
      });
      modelRow = h("div", { class: "row" }, h("label", { text: t("Model") }), model);
    }

    section.append(
      h("h2", {}, statusDot(hasKey), h("span", { text: t("AI Provider") })),
      h("div", {
        class: "hint",
        text: def.key
          ? hasKey
            ? t("{name} key saved in the Windows Credential Manager.", { name: def.name })
            : t("No {name} key yet — the chat needs one.", { name: def.name })
          : t("Runs on your local Ollama server — no API key needed."),
      }),
      h("div", { class: "row" }, h("label", { text: t("Provider") }), provider),
      h("div", { class: "row" }, h("label", { text: t("Base URL") }), url),
      h("div", {
        class: "hint",
        text: t("Leave empty to use the provider's own URL. The chat path (/v1/messages or /v1/chat/completions) is added automatically."),
      }),
    );

    if (def.id === "custom") {
      const format = h("select", {}) as HTMLSelectElement;
      format.append(
        h("option", { value: "openai", text: "OpenAI-compatible (/chat/completions)" }),
        h("option", { value: "anthropic", text: "Anthropic (/v1/messages)" }),
      );
      format.value = settings.providerFormat;
      format.addEventListener("change", () => {
        settings.providerFormat = format.value;
        void save();
      });
      section.append(h("div", { class: "row" }, h("label", { text: t("API format") }), format));
    }

    section.append(modelRow);

    if (def.key) {
      section.append(
        h("div", { class: "row" }, h("label", { text: t("API key") }), field, saveKey, removeKey),
      );
    }
    if (notice) {
      section.append(h("div", { class: notice.ok ? "notice ok" : "notice err", text: notice.text }));
    }
  }

  draw();
  return section;
}

// ── Integrations section ──────────────────────────────────────────────────────

interface IntegrationDef {
  id: string;
  name: string;
  color: string;
  /** Credential Manager keys, in the order they are shown. */
  fields: { key: string; label: string; placeholder: string; secret: boolean }[];
}

const INTEGRATIONS: IntegrationDef[] = [
  { id: "integration_stripe", name: "Stripe", color: "#0570DE",
    fields: [{ key: "stripe-api-key", label: "Secret key", placeholder: "sk_live_…", secret: true }] },
  { id: "integration_github", name: "GitHub", color: "#F4505E",
    fields: [{ key: "github-token", label: "Token", placeholder: "ghp_…", secret: true }] },
  { id: "integration_vercel", name: "Vercel", color: "#7C5CFF",
    fields: [{ key: "vercel-token", label: "Token", placeholder: "…", secret: true }] },
  { id: "integration_n8n", name: "n8n", color: "#F29B38",
    fields: [
      { key: "n8n-url", label: "Instance URL", placeholder: "https://n8n.example.com", secret: false },
      { key: "n8n-api-key", label: "API key", placeholder: "…", secret: true },
    ] },
  { id: "integration_resend", name: "Resend", color: "#22C55E",
    fields: [{ key: "resend-api-key", label: "API key", placeholder: "re_…", secret: true }] },
  { id: "integration_notion", name: "Notion", color: "#8C8C8C",
    fields: [{ key: "notion-api-key", label: "Integration token", placeholder: "ntn_…", secret: true }] },
  { id: "integration_linear", name: "Linear", color: "#5E6AD2",
    fields: [{ key: "linear-api-key", label: "Personal API key", placeholder: "lin_api_…", secret: true }] },
  { id: "integration_calcom", name: "Cal.com", color: "#C9956A",
    fields: [{ key: "calcom-api-key", label: "API key", placeholder: "cal_…", secret: true }] },
];

const MAX_ACTIVE = 4;

function integrationsSection(present: Record<string, boolean>): HTMLElement {
  const note = h("div", { class: "hint" });
  const list = h("div", { class: "int-list" });
  // One entry per integration: a compact header row that unfolds its key fields.
  // Only one is open at a time and the list scrolls inside a fixed height, so the
  // page doesn't grow with every integration that gets added.
  const items: { name: string; el: HTMLElement }[] = [];
  const search = h("input", {
    type: "search",
    class: "int-search",
    placeholder: t("Search integrations…"),
    spellcheck: "false",
    autocomplete: "off",
  }) as HTMLInputElement;
  search.addEventListener("input", () => {
    const q = search.value.trim().toLowerCase();
    for (const item of items) item.el.hidden = q !== "" && !item.name.toLowerCase().includes(q);
  });

  function updateNote() {
    const used = settings.activeIntegrations.length;
    note.textContent = t("Pick up to {max} pills to show next to Mochi — {used}/{max} in use. Keys are stored in the Windows Credential Manager, never on disk.", { max: MAX_ACTIVE, used });
  }

  for (const def of INTEGRATIONS) {
    const active = settings.activeIntegrations.includes(def.id);
    const sw = h("button", { class: active ? "switch on" : "switch" });
    sw.addEventListener("click", () => {
      const on = settings.activeIntegrations.includes(def.id);
      if (on) {
        settings.activeIntegrations = settings.activeIntegrations.filter((x) => x !== def.id);
      } else {
        if (settings.activeIntegrations.length >= MAX_ACTIVE) return;
        settings.activeIntegrations = [...settings.activeIntegrations, def.id];
      }
      sw.classList.toggle("on", !on);
      updateNote();
      void save();
    });

    const status = h("span", { class: "int-status" });
    const refreshStatus = () => {
      const ready = def.fields.every((f) => present[f.key]);
      status.textContent = ready ? t("Key stored") : t("Not set up");
      status.classList.toggle("ok", ready);
    };
    refreshStatus();

    const rows = h("div", { class: "int-fields" });
    for (const field of def.fields) {
      const input = h("input", {
        type: field.secret ? "password" : "text",
        placeholder: present[field.key] ? t("••••••••  (stored)") : field.placeholder,
        autocomplete: "off",
        spellcheck: "false",
        style: "flex:1 1 auto;min-width:0",
      }) as HTMLInputElement;
      const saveBtn = h("button", { text: t("Save") });
      const dotEl = statusDot(present[field.key] ?? false);
      saveBtn.addEventListener("click", async () => {
        const value = input.value.trim();
        try {
          await Bridge.secretSet(field.key, value);
          present[field.key] = value.length > 0;
          refreshStatus();
          input.value = "";
          input.placeholder = value ? t("••••••••  (stored)") : field.placeholder;
          dotEl.style.background = value ? "#22c55e" : "#f4505e";
        } catch {
          dotEl.style.background = "#f5a524";
        }
      });
      rows.append(
        h("div", { class: "row" },
          h("label", { style: "min-width:104px", text: t(field.label) }),
          input, saveBtn, dotEl,
        ),
      );
    }

    const head = h("div", { class: "int-head", role: "button", tabindex: "0" },
      sw,
      h("i", { class: "dot", style: `background:${def.color}` }),
      h("span", { class: "int-title", text: def.name }),
      status,
      h("span", { class: "int-chevron", text: "\u203a" }),
    );
    const item = h("div", { class: "int-item" }, head, h("div", { class: "int-body" }, rows));
    const toggleOpen = () => {
      const opening = !item.classList.contains("open");
      for (const other of items) other.el.classList.remove("open");
      item.classList.toggle("open", opening);
    };
    head.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).closest(".switch")) return; // the switch only switches
      toggleOpen();
    });
    head.addEventListener("keydown", (e) => {
      if ((e as KeyboardEvent).key === "Enter" || (e as KeyboardEvent).key === " ") {
        e.preventDefault();
        toggleOpen();
      }
    });
    items.push({ name: def.name, el: item });
    list.append(item);
  }

  updateNote();
  return h("section", {}, h("h2", {}, h("span", { text: t("Integrations") })), note, search, list);
}

// ── Notifications section ─────────────────────────────────────────────────────

/** Windows notifications in the island: access, silence, pop-up, sound, muted apps. */
function notificationsSection(): HTMLElement {
  const section = h("section", {});
  let access = "unknown";
  let seen: string[] = [];

  async function refresh() {
    access = (await Bridge.notificationsStatus()) ?? "unavailable";
    seen = (await Bridge.notificationsApps()) ?? [];
    draw();
  }

  function draw() {
    clear(section);
    const allowed = access === "allowed";
    const statusText = allowed
      ? t("Allowed")
      : access === "unavailable"
        ? t("Not available on this Windows")
        : t("Not allowed yet");

    const popup = h("select", {}) as HTMLSelectElement;
    popup.append(
      h("option", { value: "discreet", text: t("Discreet — a small banner for a few seconds") }),
      h("option", { value: "expand", text: t("Open the island") }),
      h("option", { value: "bell", text: t("Only the dot on the bell") }),
    );
    popup.value = settings.notificationsStyle || "discreet";
    popup.addEventListener("change", () => {
      settings.notificationsStyle = popup.value;
      void save();
    });

    const hour = (value: number, set: (v: number) => void) => {
      const sel = h("select", {}) as HTMLSelectElement;
      for (let i = 0; i < 24; i++) {
        sel.append(h("option", { value: String(i), text: `${String(i).padStart(2, "0")}:00` }));
      }
      sel.value = String(value);
      sel.addEventListener("change", () => { set(Number(sel.value)); void save(); });
      return sel;
    };

    const mutedApps = settings.notificationsMutedApps ?? [];
    const mutedChips = h("div", { class: "chips" });
    if (mutedApps.length === 0) mutedChips.append(h("span", { class: "hint", text: t("None") }));
    for (const a of mutedApps) {
      mutedChips.append(h("button", {
        class: "chip on",
        title: t("Unmute"),
        text: `${a}  ×`,
        onclick: () => {
          settings.notificationsMutedApps = mutedApps.filter((x) => x !== a);
          void save();
          draw();
        },
      }));
    }
    const recent = seen.filter((a) => !mutedApps.includes(a));
    const recentChips = h("div", { class: "chips" });
    for (const a of recent) {
      recentChips.append(h("button", {
        class: "chip",
        title: t("Mute"),
        text: a,
        onclick: () => {
          settings.notificationsMutedApps = [...mutedApps, a];
          void save();
          draw();
        },
      }));
    }

    const parts: (Node | null)[] = [
      h("h2", {}, statusDot(allowed && settings.notifications), h("span", { text: t("Notifications") })),
      h("div", { class: "hint", text: t("Windows notifications show up in the island. Their content stays on this PC: it is never sent anywhere and never written to the log.") }),
      h("div", { class: "row" },
        h("label", { text: t("Windows access") }),
        h("span", { class: "hint", text: statusText }),
        allowed ? null : h("button", { class: "primary", text: t("Open Windows settings"), onclick: () => void Bridge.openNotificationSettings() }),
        h("button", {
          text: t("Check again"),
          onclick: async () => {
            access = (await Bridge.notificationsRequestAccess()) ?? access;
            await refresh();
          },
        }),
      ),
      allowed ? null : h("div", { class: "hint", text: t("In Windows: Settings → Privacy & security → Notifications → turn on “Notification access”. Only you can switch it on.") }),
      h("div", { class: "row" },
        h("label", { text: t("Show in the island") }),
        toggle(settings.notifications, (v) => { settings.notifications = v; void save(); }),
      ),
      h("div", { class: "row" },
        h("label", { text: t("Silence") }),
        toggle(settings.notificationsMuted, (v) => { settings.notificationsMuted = v; void save(); }),
        h("span", { class: "hint", text: t("listed under the bell, but no pop-up and no sound") }),
      ),
      h("div", { class: "row" },
        h("label", { text: t("When one arrives") }),
        popup,
      ),
      h("div", { class: "row" },
        h("label", { text: t("Soft chime") }),
        toggle(settings.notificationsChime, (v) => { settings.notificationsChime = v; void save(); }),
        h("span", { class: "hint", text: t("the app that sent it usually chimes already") }),
      ),
      h("div", { class: "row" },
        h("label", { text: t("Quiet hours") }),
        toggle(settings.notificationsQuiet, (v) => { settings.notificationsQuiet = v; void save(); }),
        hour(settings.notificationsQuietFrom ?? 22, (v) => { settings.notificationsQuietFrom = v; }),
        h("span", { class: "hint", text: "→" }),
        hour(settings.notificationsQuietTo ?? 8, (v) => { settings.notificationsQuietTo = v; }),
      ),
      h("div", { class: "hint", text: t("Nothing pops up either while Windows' Do not disturb is on or an app is full screen; they wait under the bell.") }),
      h("div", { class: "row" }, h("label", { text: t("Muted apps") }), mutedChips),
      recent.length
        ? h("div", { class: "row" }, h("label", { text: t("Seen today") }), recentChips)
        : null,
    ];
    section.append(...parts.filter((x): x is Node => x != null));
  }

  draw();
  void refresh();
  // Access can be switched on in Windows while this window is open.
  window.addEventListener("focus", () => void refresh());
  return section;
}

// ── Transparency section ──────────────────────────────────────────────────────

type OpacityKey = "islandOpacity" | "cardOpacity" | "idleOpacity";

const TRANSPARENCY_PRESETS: { label: string; values: Record<OpacityKey, number> }[] = [
  { label: "Solid", values: { islandOpacity: 1, cardOpacity: 1, idleOpacity: 1 } },
  { label: "Glass", values: { islandOpacity: 0.75, cardOpacity: 0.55, idleOpacity: 1 } },
  { label: "Ghost", values: { islandOpacity: 0.55, cardOpacity: 0.35, idleOpacity: 0.5 } },
];

/** Island background, cards and fade-when-away. Changes show live on every display. */
function transparencySection(): HTMLElement {
  let saveTimer: number | null = null;
  // Sliders fire on every pixel: the islands update at once, the file a beat later.
  const saveSoon = () => {
    if (saveTimer != null) window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => {
      saveTimer = null;
      void save();
    }, 150);
  };

  const sliders: { key: OpacityKey; input: HTMLInputElement; out: HTMLElement }[] = [];

  function slider(key: OpacityKey, min: number, label: string, hint: string): HTMLElement {
    const input = h("input", {
      type: "range", min: String(min), max: "1", step: "0.05",
      value: String(clampOpacity(settings[key], min)),
    }) as HTMLInputElement;
    const out = h("span", { class: "hint", style: "min-width:38px;text-align:right" });
    const show = () => { out.textContent = `${Math.round(Number(input.value) * 100)} %`; };
    show();
    input.addEventListener("input", () => {
      settings[key] = Number(input.value);
      show();
      markPreset();
      saveSoon();
    });
    sliders.push({ key, input, out });
    return h("div", { class: "row", title: hint },
      h("label", { text: label }),
      input,
      out,
    );
  }

  const presets = h("div", { class: "chips" });
  const presetButtons = TRANSPARENCY_PRESETS.map((p) => {
    const b = h("button", {
      class: "chip",
      text: t(p.label),
      onclick: () => {
        Object.assign(settings, p.values);
        for (const s of sliders) {
          s.input.value = String(settings[s.key]);
          s.out.textContent = `${Math.round(settings[s.key] * 100)} %`;
        }
        markPreset();
        void save();
      },
    });
    return { p, b };
  });
  function markPreset() {
    for (const { p, b } of presetButtons) {
      const on = (Object.keys(p.values) as OpacityKey[]).every(
        (k) => Math.abs(settings[k] - p.values[k]) < 0.001,
      );
      b.classList.toggle("on", on);
    }
  }
  presets.append(...presetButtons.map((x) => x.b));

  const rows = [
    slider("islandOpacity", OPACITY_MIN.island, t("Island background"), t("the black shape")),
    slider("cardOpacity", OPACITY_MIN.card, t("Cards"), t("panels inside the island")),
    slider("idleOpacity", OPACITY_MIN.idle, t("When away"), t("whole island while the mouse is elsewhere")),
  ];
  markPreset();

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("Transparency") })),
    h("div", { class: "hint", text: t("Background and cards only change the panels — text and Mochi stay sharp. “When away” fades the whole island until the mouse comes back.") }),
    h("div", { class: "row" }, h("label", { text: t("Preset") }), presets),
    ...rows,
  );
}

// ── General section ───────────────────────────────────────────────────────────

/** Turns a keydown into "Ctrl+Alt+M" (null while only modifiers are held). */
function comboFromEvent(e: KeyboardEvent): string | null {
  let key: string | null = null;
  if (/^Key[A-Z]$/.test(e.code)) key = e.code.slice(3);
  else if (/^Digit[0-9]$/.test(e.code)) key = e.code.slice(5);
  else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(e.code)) key = e.code;
  else if (e.code === "Space") key = "Space";
  else if (e.code === "Enter") key = "Enter";
  else if (e.code === "Tab") key = "Tab";
  if (!key) return null;
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Win"].filter(Boolean);
  return [...mods, key].join("+");
}

/** t("Open chat shortcut"): click, press the combination; Backspace removes it, Esc cancels. */
function shortcutRow(): HTMLElement {
  const button = h("button", { class: "shortcut" }) as HTMLButtonElement;
  const note = h("span", { class: "hint" });
  const show = () => { button.textContent = settings.chatHotkey || t("None"); };
  show();

  let listening = false;
  const stop = () => {
    listening = false;
    window.removeEventListener("keydown", onKey, true);
    show();
  };
  const apply = async (combo: string) => {
    try {
      settings.chatHotkey = await Bridge.setChatHotkey(combo);
      note.textContent = combo ? t("Saved.") : t("Shortcut removed.");
    } catch (err) {
      note.textContent = String(err).replace(/^Error:\s*/, "");
    }
  };
  function onKey(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") return stop();
    if (e.key === "Backspace" || e.key === "Delete") {
      stop();
      void apply("").then(show);
      return;
    }
    const combo = comboFromEvent(e);
    if (!combo) return; // only modifiers so far
    if (!(e.ctrlKey || e.altKey || e.shiftKey || e.metaKey)) {
      note.textContent = t("Hold Ctrl, Alt, Shift or Win together with the key.");
      return;
    }
    stop();
    void apply(combo).then(show);
  }
  button.addEventListener("click", () => {
    if (listening) return stop();
    listening = true;
    button.textContent = t("Press the keys…");
    note.textContent = t("Backspace removes it, Esc cancels.");
    window.addEventListener("keydown", onKey, true);
  });
  button.addEventListener("blur", () => { if (listening) stop(); });

  return h("div", { class: "row" }, h("label", { text: t("Open chat shortcut") }), button, note);
}

function generalSection(): HTMLElement {
  const volume = h("input", {
    type: "range", min: "0", max: "0.2", step: "0.005",
    value: String(settings.soundVolume),
  }) as HTMLInputElement;
  volume.addEventListener("input", () => {
    settings.soundVolume = Number(volume.value);
    void save();
  });

  const autoClose = h("input", {
    type: "number", min: "1", max: "120", step: "1",
    value: String(Math.round(settings.autoCloseInterval)),
    style: "width:72px",
  }) as HTMLInputElement;
  autoClose.addEventListener("change", () => {
    settings.autoCloseInterval = Math.max(1, Math.min(120, Number(autoClose.value) || 15));
    autoClose.value = String(settings.autoCloseInterval);
    void save();
  });

  const lang = h("select", {}) as HTMLSelectElement;
  for (const l of LANGUAGES) {
    lang.append(h("option", { value: l.id, text: l.id === "auto" ? t("Auto (Windows language)") : l.label }));
  }
  lang.value = settings.language || "auto";
  lang.addEventListener("change", async () => {
    settings.language = lang.value;
    await save();
    // The islands reload themselves on the settings event; this window too.
    location.reload();
  });

  const hide = h("select", {}) as HTMLSelectElement;
  for (const [secs, label] of [
    [0, t("Never")], [60, t("After 1 minute")], [300, t("After 5 minutes")], [900, t("After 15 minutes")],
  ] as const) {
    hide.append(h("option", { value: String(secs), text: label }));
  }
  hide.value = String(settings.hideAfter ?? 0);
  if (hide.selectedIndex < 0) hide.value = "0";
  hide.addEventListener("change", () => {
    settings.hideAfter = Number(hide.value) || 0;
    void save();
  });

  const screen = h("select", {}) as HTMLSelectElement;
  screen.append(
    h("option", { value: "all", text: t("Every display") }),
    h("option", { value: "primary", text: t("Main display") }),
    h("option", { value: "cursor", text: t("Display under the cursor") }),
  );
  screen.value = settings.allScreens ? "all" : settings.screen;
  screen.addEventListener("change", () => {
    settings.allScreens = screen.value === "all";
    if (!settings.allScreens) settings.screen = screen.value as Settings["screen"];
    void save();
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("General") })),
    h("div", { class: "row" },
      h("label", { text: t("Language") }),
      lang,
    ),
    h("div", { class: "row" },
      h("label", { text: t("Sound") }),
      toggle(settings.soundEnabled, (v) => { settings.soundEnabled = v; void save(); }),
      volume,
    ),
    h("div", { class: "row" },
      h("label", { text: t("Auto-close") }),
      autoClose,
      h("span", { class: "hint", text: t("seconds after you leave the island") }),
    ),
    h("div", { class: "row", title: t("When hidden, a small bar at the edge of the screen marks where to point to bring it back.") },
      h("label", { text: t("Hide completely") }),
      hide,
      h("span", { class: "hint", text: t("when idle in the compact island") }),
    ),
    h("div", { class: "row" },
      h("label", { text: t("Island lives on") }),
      screen,
    ),
    shortcutRow(),
    h("div", { class: "row" },
      h("label", { text: t("Launch at startup") }),
      toggle(settings.autostart, (v) => { settings.autostart = v; void save(); }),
    ),
  );
}

// ── Mochi section ─────────────────────────────────────────────────────────────

/** Plays one of Mochi's sounds in this window, at the volume chosen in Settings. */
async function playSound(name: string) {
  await Sound.preload();
  Sound.setEnabled(settings.soundEnabled);
  Sound.setVolume(settings.soundVolume);
  Sound.resume();
  Sound.play(name);
}

/** Live preview of Mochi plus pickers for what he wears. */
function mochiSection(): HTMLElement {
  const SIZE = 150;
  const canvas = h("canvas", { class: "mochi-preview" }) as HTMLCanvasElement;
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  canvas.width = Math.round(SIZE * dpr);
  canvas.height = Math.round(SIZE * dpr);
  canvas.style.width = `${SIZE}px`;
  canvas.style.height = `${SIZE}px`;

  const engine = new BotEngine();
  engine.setState("idle", true);
  const applyLook = () => {
    engine.hat = isHat(settings.mochiHat) ? settings.mochiHat : "none";
    engine.face = isFace(settings.mochiFace) ? settings.mochiFace : "none";
    engine.neck = isNeck(settings.mochiNeck) ? settings.mochiNeck : "none";
  };
  applyLook();

  // The preview only animates while this window is actually on screen. The window
  // is created hidden at launch and merely hidden when closed, and WebView2 keeps
  // running animation frames for a hidden window — so Rust says when it is shown.
  let last = performance.now();
  let shown = !IS_TAURI;
  let looping = false;
  const frame = (t: number) => {
    const ctx = canvas.getContext("2d");
    if (ctx) {
      engine.update(Math.min(0.05, (t - last) / 1000));
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, SIZE, SIZE);
      engine.draw(ctx, SIZE, SIZE);
    }
    last = t;
    if (shown && !document.hidden) requestAnimationFrame(frame);
    else looping = false;
  };
  const wake = () => {
    if (looping || !shown || document.hidden) return;
    looping = true;
    last = performance.now();
    requestAnimationFrame(frame);
  };
  document.addEventListener("visibilitychange", wake);
  void onEvent<boolean>("settings-visible", (v) => {
    shown = v;
    wake();
  });
  wake();

  function picker<T extends string>(
    options: { id: T; label: string }[],
    current: () => string,
    set: (v: T) => void,
  ): HTMLElement {
    const box = h("div", { class: "chips" });
    const buttons = options.map((o) => {
      const b = h("button", {
        class: "chip",
        text: t(o.label),
        onclick: () => {
          set(o.id);
          applyLook();
          void save();
          mark();
        },
      });
      return { id: o.id, b };
    });
    const mark = () => buttons.forEach(({ id, b }) => b.classList.toggle("on", id === current()));
    mark();
    box.append(...buttons.map((x) => x.b));
    return box;
  }

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "Mochi" })),
    h("div", { class: "mochi-edit" },
      canvas,
      h("div", { class: "mochi-pickers" },
        h("div", { class: "row" },
          h("label", { text: t("Hat") }),
          picker(HATS, () => settings.mochiHat, (v) => { settings.mochiHat = v; }),
        ),
        h("div", { class: "row" },
          h("label", { text: t("Face") }),
          picker(FACES, () => settings.mochiFace, (v) => { settings.mochiFace = v; }),
        ),
        h("div", { class: "row" },
          h("label", { text: t("Neck") }),
          picker(NECKS, () => settings.mochiNeck, (v) => { settings.mochiNeck = v; }),
        ),
        h("div", { class: "row" },
          h("label", { text: t("Preview") }),
          h("div", { class: "chips" },
            h("button", { class: "chip", text: t("Reminder wave"), onclick: () => { playSound("approval"); engine.attention(); } }),
            h("button", { class: "chip", text: t("Saved a note"), onclick: () => { playSound("approve"); engine.triggerEmote("remember"); } }),
            h("button", { class: "chip", text: t("Greeting"), onclick: () => engine.greet() }),
            h("button", { class: "chip", text: t("Listening"), onclick: () => engine.setMusic(!engine.music) }),
          ),
        ),
        h("div", { class: "row" },
          h("label", { text: "Spotify" }),
          toggle(settings.spotify, (v) => { settings.spotify = v; void save(); }),
          h("span", { class: "hint", text: t("Headphones, now playing and controls") }),
        ),
      ),
    ),
  );
}

// ── Memory section ────────────────────────────────────────────────────────────

/** Everything Mochi remembers: who saved each note, when, and a way to remove it. */
function memorySection(): HTMLElement {
  const list = h("div", { class: "reminder-list" });
  const status = h("span", { class: "hint" });
  const input = h("input", {
    type: "text",
    class: "memory-input",
    placeholder: t("Add something for Mochi to remember…"),
    spellcheck: "false",
  }) as HTMLInputElement;

  const flash = (text: string) => {
    status.textContent = text;
    window.setTimeout(() => { if (status.textContent === text) status.textContent = ""; }, 5000);
  };

  async function refresh() {
    const notes = (await Bridge.memoryList()) ?? [];
    clear(list);
    if (notes.length === 0) {
      list.append(h("div", { class: "hint", text: t("Nothing yet. Tell Mochi “remember that…”, or let it pick up what matters.") }));
      return;
    }
    for (const n of notes) {
      list.append(
        h("div", { class: "reminder" },
          h("span", { class: "when", text: n.created.replace("T", " ") }),
          h("span", { class: "what", text: n.text }),
          h("span", { class: "origin", text: n.source === "mochi" ? "Mochi" : t("you") }),
          h("button", {
            title: t("Forget this"),
            text: "×",
            onclick: async () => { await Bridge.memoryDelete(n.id); void refresh(); },
          }),
        ),
      );
    }
  }
  void refresh();
  // Mochi can add notes while this window is open.
  window.addEventListener("focus", () => void refresh());
  window.setInterval(() => void refresh(), 10000);

  async function add() {
    const text = input.value.trim();
    if (!text) return;
    try {
      await Bridge.memoryAdd(text);
      input.value = "";
      void refresh();
    } catch (err) {
      flash(String(err).replace(/^Error:\s*/, ""));
    }
  }
  input.addEventListener("keydown", (e) => {
    if ((e as KeyboardEvent).key === "Enter") void add();
  });

  const clearBtn = h("button", {
    class: "danger",
    text: t("Forget everything"),
    onclick: async () => {
      if (!window.confirm(t("Delete everything Mochi remembers?"))) return;
      await Bridge.memoryClear();
      void refresh();
      flash(t("Memory cleared."));
    },
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("Memory") })),
    h("div", { class: "hint", text: t("Mochi keeps short notes about you in a file on this PC (memory.json) and reads them at the start of every chat. It saves what you ask it to remember, and picks up lasting facts on its own, but never passwords, keys or other secrets. Adding a note never changes the others.") }),
    list,
    h("div", { class: "row" }, input, h("button", { class: "primary", text: t("Add"), onclick: () => void add() })),
    h("div", { class: "row" }, clearBtn, status),
  );
}

// ── Reminders & interruptions section ─────────────────────────────────────────

function formatDue(due: string): string {
  // "2026-10-02T15:00" → "2026-10-02 15:00"
  return due.replace("T", " ");
}

/** What Mochi may do on its own: pending reminders and unprompted check-ins. */
function remindersSection(): HTMLElement {
  const list = h("div", { class: "reminder-list" });

  async function refresh() {
    const items = (await Bridge.remindersList()) ?? [];
    clear(list);
    if (items.length === 0) {
      list.append(h("div", { class: "hint", text: t("No reminders pending. Ask Mochi to remind you of something, or just mention it in the chat.") }));
      return;
    }
    for (const r of items) {
      list.append(
        h("div", { class: "reminder" },
          h("span", { class: "when", text: formatDue(r.due) }),
          h("span", { class: "what", text: r.text }),
          h("button", {
            title: t("Delete"),
            text: "×",
            onclick: async () => { await Bridge.remindersDelete(r.id); void refresh(); },
          }),
        ),
      );
    }
  }
  void refresh();
  // Mochi may add one while this window is open.
  window.setInterval(() => void refresh(), 15000);

  const minutes = h("select", {}) as HTMLSelectElement;
  for (const m of [15, 30, 60, 120]) {
    minutes.append(h("option", { value: String(m), text: m < 60 ? t("{n} minutes", { n: m }) : m === 60 ? t("1 hour") : t("{n} hours", { n: m / 60 }) }));
  }
  minutes.value = String(settings.proactiveMinutes);
  minutes.addEventListener("change", () => {
    settings.proactiveMinutes = Number(minutes.value) || 30;
    void save();
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("Reminders and interruptions") })),
    h("div", { class: "hint", text: t("Mochi sets reminders when you ask, and on its own when you mention something with a time. They pop up in the island at that moment, even if you are in another app.") }),
    list,
    h("div", { class: "row" },
      h("label", { text: t("Interrupt me on its own") }),
      toggle(settings.proactive, (v) => { settings.proactive = v; void save(); }),
      minutes,
      h("span", { class: "hint", text: t("between check-ins") }),
    ),
    h("div", { class: "hint", text: t("Off by default. When on, every so often Mochi sends the clock, its saved notes and your pending reminders to your AI provider and decides whether something deserves an interruption. It never sends your screen, files or chats, stays quiet from 22:00 to 08:00, and usually says nothing.") }),
  );
}

// ── Boot ──────────────────────────────────────────────────────────────────────

async function main() {
  const boot = await Bridge.boot();
  if (boot) {
    settings = { ...settings, ...boot.settings };
    version = boot.version;
  }
  setLanguage(settings.language);
  const status = (await Bridge.hooksStatus()) ?? {
    installed: false, settingsPath: "", hookPath: "", hookReady: false,
  };

  const keys = [
    "anthropic-api-key", "openai-api-key", "openrouter-api-key",
    "groq-api-key", "deepseek-api-key", "custom-api-key",
    "stripe-api-key", "github-token", "vercel-token",
    "n8n-url", "n8n-api-key", "resend-api-key", "notion-api-key", "linear-api-key", "calcom-api-key",
  ];
  const present: Record<string, boolean> = {};
  for (const k of keys) present[k] = (await Bridge.secretPresent(k)) ?? false;

  present["calendar-ics"] = (await Bridge.secretPresent("calendar-ics")) ?? false;
  const ctx: Ctx = { settings: () => settings, save, toggle, present };

  clear(root);
  root.append(
    buildShell(
      h("h1", {}, h("span", { text: "Coucou" }), h("span", { class: "version", text: version })),
      h("div", {
        class: "hint",
        text: t("No telemetry. Network requests only go to the services you configure yourself."),
      }),
      [
        { id: "general", label: t("General"), icon: ICONS.gear, sections: [generalSection(), transparencySection()] },
        {
          id: "claude", label: t("Claude Code & AI"), icon: ICONS.bubble,
          sections: [claudeSection(status), sessionsSection(ctx), apiSection(present)],
        },
        { id: "mochi", label: "Mochi", icon: ICONS.star, sections: [mochiSection(), outfitsSection(ctx), daySection(ctx)] },
        {
          id: "alerts", label: t("Notifications & alerts"), icon: ICONS.bell,
          sections: [notificationsSection(), systemSection(ctx), remindersSection()],
        },
        {
          id: "productivity", label: t("Productivity"), icon: ICONS.timer,
          sections: [calendarSection(ctx), clipboardSection(ctx), memorySection()],
        },
        { id: "integrations", label: t("Integrations"), icon: ICONS.stack, sections: [integrationsSection(present)] },
      ],
    ),
  );

  void onEvent<Settings>("settings-changed", (s) => {
    settings = { ...settings, ...s };
  });
}

void main();
