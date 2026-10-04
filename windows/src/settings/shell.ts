// The settings window's frame: a vertical menu of categories on the left, the
// chosen category's sections on the right in as many columns as fit (one, two
// or three), and a search box that looks through every section at once.

import { t } from "../core/i18n";
import { h, clear, svg } from "../views/dom";

export interface Group {
  id: string;
  label: string;
  icon: string;
  sections: HTMLElement[];
}

const LAST_KEY = "coucou.settings.tab";

export function buildShell(brand: Node, footer: Node, groups: Group[]): HTMLElement {
  const nav = h("nav", { class: "nav" });
  const title = h("h1", { class: "pane-title" });
  const cols = h("div", { class: "cols" });
  const empty = h("div", { class: "hint pane-empty" });
  const pane = h("main", { class: "pane" }, title, cols, empty);

  const search = h("input", {
    type: "search",
    class: "nav-search",
    placeholder: t("Search settings…"),
    spellcheck: "false",
    autocomplete: "off",
  }) as HTMLInputElement;

  const buttons = new Map<string, HTMLElement>();
  let current = localStorage.getItem(LAST_KEY) ?? groups[0].id;
  if (!groups.some((g) => g.id === current)) current = groups[0].id;

  function show(sections: HTMLElement[], heading: string) {
    title.textContent = heading;
    clear(cols);
    cols.append(...sections);
    empty.style.display = sections.length ? "none" : "";
    pane.scrollTop = 0;
  }

  function open(id: string) {
    current = id;
    localStorage.setItem(LAST_KEY, id);
    search.value = "";
    for (const [gid, b] of buttons) b.classList.toggle("on", gid === id);
    const g = groups.find((x) => x.id === id)!;
    show(g.sections, g.label);
  }

  search.addEventListener("input", () => {
    const q = search.value.trim().toLowerCase();
    if (!q) {
      open(current);
      return;
    }
    for (const b of buttons.values()) b.classList.remove("on");
    const hits = groups.flatMap((g) => g.sections).filter((s) => s.innerText.toLowerCase().includes(q));
    empty.textContent = t("Nothing matches “{q}”.", { q: search.value.trim() });
    show(hits, t("Search"));
  });

  for (const g of groups) {
    const b = h(
      "button",
      { class: "nav-item", onclick: () => open(g.id) },
      svg(g.icon, 14),
      h("span", { text: g.label }),
    );
    buttons.set(g.id, b);
    nav.append(b);
  }

  const side = h("aside", { class: "side" }, brand, search, nav, h("div", { class: "side-foot" }, footer));
  open(current);
  return h("div", { class: "shell" }, side, pane);
}
