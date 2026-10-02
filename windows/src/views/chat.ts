// Chat view — DOM port of PromptView / ChatBubble / TypingDotsView from
// IslandViewContent.swift.

import { h, svg, clear } from "./dom";
import { ICONS } from "./icons";
import { Bridge, type ChatContext } from "../core/bridge";
import { Sound } from "../core/sound";
import { State, type ChatMessage } from "../core/state";
import type { ViewHost } from "./views";

let nextId = 1;

function bubble(message: ChatMessage): HTMLElement {
  if (message.role === "user") {
    return h(
      "div",
      { class: "chat-row user" },
      h("div", { class: "bubble", text: message.content }),
    );
  }
  return h("div", { class: "chat-row" }, h("div", { class: "reply", text: message.content }));
}

function typingDots(): HTMLElement {
  return h(
    "div",
    { class: "chat-row" },
    h("div", { class: "typing" }, h("i"), h("i"), h("i")),
  );
}

/** The coloured chip showing what the question is about (a dropped file). */
function contextChip(label: string): HTMLElement {
  const chip = h("div", { class: "chip" }, h("i", { class: "chip-dot" }), h("span", { text: label }));
  requestAnimationFrame(() => chip.classList.add("settled"));
  return chip;
}

export function buildPrompt(onHeightChange: () => void): ViewHost {
  const chipRow = h("div", { class: "chip-row" });
  const log = h("div", { class: "chat-log" });
  const input = h("input", {
    type: "text",
    class: "chat-input",
    placeholder: "Ask me anything…",
    spellcheck: "false",
  }) as HTMLInputElement;
  const send = h("button", { class: "send-btn", title: "Send" }, svg(ICONS.arrowUp, 11));
  const look = h(
    "button",
    { class: "look-btn", title: "Let Mochi see my screen (once)" },
    svg(ICONS.eye, 15, { stroke: 1.8 }),
  );
  const bar = h("div", { class: "chat-bar" }, input, look, send);

  const el = h(
    "div",
    { class: "view" },
    h("div", { class: "card wash chat-card" }, h("div", { class: "chat-body" }, chipRow, log, bar)),
  );
  (el.querySelector(".card") as HTMLElement).style.setProperty("--wash", "rgba(99,102,241,0.5)");

  let sending = false;
  let renderedCount = -1;

  async function submit() {
    const query = input.value.trim();
    if (!query || sending) return;
    input.value = "";
    sending = true;
    Sound.play("send");

    State.chatHistory.push({ id: nextId++, role: "user", content: query });
    State.stateOverride = "thinking";
    State.notify();
    onHeightChange();

    const file = State.droppedFile;
    // A screenshot goes out once, with the message sent right after it was taken.
    const screen = State.pendingScreen;
    State.pendingScreen = null;
    const context: ChatContext | null = screen
      ? { kind: "screen", path: screen.path }
      : State.chatHistory.length === 1 && file
        ? { kind: "file", name: file.name, path: file.path }
        : null;

    try {
      const reply = await Bridge.chatSend(query, context);
      State.chatHistory.push({ id: nextId++, role: "assistant", content: reply.text });
      State.stateOverride = null;
      Sound.play("finish");
      // Mochi saved a note or set a reminder: let the island animate it.
      if (reply.remembered) window.dispatchEvent(new CustomEvent("mochi-remembered"));
    } catch (err) {
      State.stateOverride = null;
      if (String(err).includes("cancelled")) {
        // Stopped by the user: take the question back so it can be edited or resent.
        State.chatHistory.pop();
        input.value = query;
        return;
      }
      State.noteMessage = String(err).replace(/^Error:\s*/, "");
      State.view = "note";
      Sound.play("error");
    } finally {
      sending = false;
      State.notify();
      onHeightChange();
      input.focus();
      requestAnimationFrame(() => input.focus());
    }
  }

  // Nothing is captured until this is pressed — Mochi never looks on its own.
  let capturing = false;
  look.addEventListener("click", async () => {
    if (capturing || sending) return;
    capturing = true;
    try {
      const shot = await Bridge.captureScreen();
      State.pendingScreen = { path: shot.path };
      Sound.play("blip");
    } catch (err) {
      State.noteMessage = String(err).replace(/^Error:\s*/, "");
      State.view = "note";
      Sound.play("error");
    } finally {
      capturing = false;
      State.notify();
      input.focus();
    }
  });

  send.addEventListener("click", () => {
    // While a request is in flight the button is a stop button.
    if (sending) void Bridge.chatCancel();
    else void submit();
  });
  input.addEventListener("keydown", (e) => {
    if ((e as KeyboardEvent).key === "Enter") {
      e.preventDefault();
      void submit();
    }
    e.stopPropagation(); // Escape closes the island, not the chat
  });

  return {
    el,
    sync() {
      const file = State.droppedFile;
      const wantChip = `${file?.name ?? ""}|${State.pendingScreen ? "screen" : ""}`;
      if (chipRow.dataset.label !== wantChip) {
        chipRow.dataset.label = wantChip;
        clear(chipRow);
        if (file?.name) chipRow.append(contextChip(file.name));
        if (State.pendingScreen) {
          const chip = contextChip("Screen — click to remove");
          chip.classList.add("removable");
          chip.addEventListener("click", () => {
            State.pendingScreen = null;
            State.notify();
          });
          chipRow.append(chip);
        }
      }

      const thinking = State.stateOverride === "thinking";
      const count = State.chatHistory.length + (thinking ? 0.5 : 0);
      if (count !== renderedCount) {
        renderedCount = count;
        clear(log);
        for (const m of State.chatHistory) log.append(bubble(m));
        if (thinking) log.append(typingDots());
        log.scrollTop = log.scrollHeight;
      }

      input.placeholder = State.chatHistory.length === 0 ? "Ask me anything…" : "Continue…";
      // readOnly, not disabled: disabling the field drops its focus, and the user
      // would have to click it again after every message.
      input.readOnly = sending;
      send.title = sending ? "Stop" : "Send";
      send.classList.toggle("stop", sending);
      if (send.dataset.icon !== (sending ? "stop" : "send")) {
        send.dataset.icon = sending ? "stop" : "send";
        clear(send);
        send.append(svg(sending ? ICONS.stop : ICONS.arrowUp, 11));
      }
    },
    focus() {
      input.focus();
      input.select();
    },
  };
}
