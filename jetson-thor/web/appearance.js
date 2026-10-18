import { $, element } from "./dom.js";
import { preferences } from "./history.js";

// Swatches preview each palette; full color tokens live in themes.css.
const THEMES = [
  { id: "graphite", name: "Graphite", kind: "dark", bg: "#111113", accent: "#c5b5fd" },
  { id: "midnight", name: "Midnight", kind: "dark", bg: "#0c1220", accent: "#8eb8ff" },
  { id: "obsidian", name: "Obsidian", kind: "dark", bg: "#08090b", accent: "#e0e2eb" },
  { id: "github-dark", name: "GitHub Dark", kind: "dark", bg: "#0d1117", accent: "#79c0ff" },
  { id: "tokyo-night", name: "Tokyo Night", kind: "dark", bg: "#1a1b26", accent: "#7aa2f7" },
  { id: "catppuccin", name: "Catppuccin Mocha", kind: "dark", bg: "#1e1e2e", accent: "#cba6f7" },
  { id: "rose-pine", name: "Ros\u00e9 Pine", kind: "dark", bg: "#191724", accent: "#ebbcba" },
  { id: "ayu-dark", name: "Ayu Dark", kind: "dark", bg: "#0b0e14", accent: "#e6b450" },
  { id: "kanagawa", name: "Kanagawa", kind: "dark", bg: "#1f1f28", accent: "#7e9cd8" },
  { id: "everforest", name: "Everforest", kind: "dark", bg: "#232a2e", accent: "#a7c080" },
  { id: "vesper", name: "Vesper", kind: "dark", bg: "#101010", accent: "#ffc799" },
  { id: "synthwave", name: "Synthwave", kind: "dark", bg: "#191329", accent: "#f19ce4" },
  { id: "night", name: "Night", kind: "dark", bg: "#141312", accent: "#7fc8a0" },
  { id: "deep-ocean", name: "Deep Ocean", kind: "dark", bg: "#0f111a", accent: "#84ffff" },
  { id: "nord", name: "Nord", kind: "dark", bg: "#2e3440", accent: "#88c0d0" },
  { id: "dracula", name: "Dracula", kind: "dark", bg: "#282a36", accent: "#bd93f9" },
  { id: "one-dark", name: "One Dark", kind: "dark", bg: "#282c34", accent: "#61afef" },
  { id: "solarized-dark", name: "Solarized Dark", kind: "dark", bg: "#002b36", accent: "#2aa198" },
  { id: "gruvbox-dark", name: "Gruvbox Dark", kind: "dark", bg: "#282828", accent: "#b8bb26" },
  { id: "monokai", name: "Monokai", kind: "dark", bg: "#272822", accent: "#a6e22e" },
  { id: "paper", name: "Paper", kind: "light", bg: "#fbfaf8", accent: "#2f6f4f" },
  { id: "github-light", name: "GitHub Light", kind: "light", bg: "#ffffff", accent: "#0969da" },
  { id: "solarized-light", name: "Solarized Light", kind: "light", bg: "#fdf6e3", accent: "#268bd2" },
  { id: "system", name: "Follow device", kind: "system", bg: "#f6f8fc", accent: "#1f6fb2" },
];

const mobile = matchMedia("(max-width: 900px)");
let collapsed = preferences.get("sidebar-collapsed", false);
let drawerOpen = false;
let themeFilter = "dark";

function syncSidebar() {
  const open = mobile.matches ? drawerOpen : !collapsed;
  document.body.classList.toggle("sidebar-collapsed", !mobile.matches && collapsed);
  document.body.classList.toggle("history-open", mobile.matches && drawerOpen);
  $("sidebar").inert = !open;
  $("history-backdrop").hidden = !mobile.matches || !drawerOpen;
  document.querySelector(".chat-panel").inert = mobile.matches && drawerOpen;
  $("open-history").setAttribute("aria-expanded", String(open));
  $("open-history").setAttribute("aria-label", open ? "Hide sidebar" : "Show sidebar");
  $("open-history").title = open ? "Hide sidebar" : "Show sidebar";
}

function setSidebar(open) {
  if (mobile.matches) drawerOpen = open;
  else {
    collapsed = !open;
    preferences.set("sidebar-collapsed", collapsed);
  }
  syncSidebar();
  if (open && mobile.matches) $("close-history").focus();
  else $("open-history").focus();
}

export function closeHistoryOnMobile() {
  if (mobile.matches && drawerOpen) setSidebar(false);
}

function applyTheme(id, persist = false) {
  const theme = THEMES.find((item) => item.id === id) || THEMES[0];
  if (theme.id === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme.id;

  $("theme-label").textContent = theme.name;
  $("open-themes").setAttribute("aria-label", `Choose theme, current: ${theme.name}`);
  $("theme-status").textContent = `${theme.name} · Saved on this device`;
  $("theme-grid").querySelectorAll("button").forEach((button) => {
    button.setAttribute("aria-pressed", String(button.dataset.themeChoice === theme.id));
  });
  if (persist) preferences.set("theme", theme.id);
}

function renderThemes() {
  $("theme-grid").replaceChildren();
  THEMES.filter((theme) => themeFilter === "all" || theme.kind === themeFilter).forEach((theme) => {
    const button = element("button", "theme-choice");
    button.type = "button";
    button.dataset.themeChoice = theme.id;
    button.setAttribute("aria-label", theme.name);
    button.style.setProperty("--preview-bg", theme.bg);
    button.style.setProperty("--preview-accent", theme.accent);
    const preview = element("span", "theme-preview");
    preview.setAttribute("aria-hidden", "true");
    preview.append(element("i"), element("i"), element("i"));
    button.append(preview, element("span", "theme-name", theme.name), element("span", "theme-check", "✓"));
    button.addEventListener("click", () => applyTheme(theme.id, true));
    $("theme-grid").append(button);
  });
  applyTheme(preferences.get("theme", "graphite"));
}

function trapDrawerFocus(event) {
  if (!mobile.matches || !drawerOpen) return;
  if (event.key === "Escape") { setSidebar(false); return; }
  if (event.key !== "Tab" || document.querySelector("dialog[open]")) return;
  const nodes = [...$("sidebar").querySelectorAll("a, button:not(:disabled)")];
  const first = nodes[0];
  const last = nodes.at(-1);
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
}

export function initAppearance() {
  syncSidebar();
  $("open-history").addEventListener("click", () => setSidebar(mobile.matches ? !drawerOpen : collapsed));
  $("close-history").addEventListener("click", () => setSidebar(false));
  $("history-backdrop").addEventListener("click", () => setSidebar(false));
  mobile.addEventListener("change", () => { drawerOpen = false; syncSidebar(); });
  document.addEventListener("keydown", trapDrawerFocus);

  renderThemes();
  $("open-themes").addEventListener("click", () => $("themes").showModal());
  document.querySelectorAll("[data-filter]").forEach((button) => {
    button.addEventListener("click", () => {
      themeFilter = button.dataset.filter;
      document.querySelectorAll("[data-filter]").forEach((item) => item.setAttribute("aria-pressed", String(item === button)));
      renderThemes();
    });
  });
  $("themes").addEventListener("click", (event) => {
    const bounds = $("themes").getBoundingClientRect();
    if (event.target === $("themes") && (event.clientX < bounds.left || event.clientX > bounds.right || event.clientY < bounds.top || event.clientY > bounds.bottom)) $("themes").close();
  });
}
