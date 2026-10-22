// Color theme: follow the system, force light or dark, or custom colors (like
// Slack's). styles.css holds the light and dark palettes; a custom theme picks
// one as its base and overrides five colors, with the rest mixed from them.
import type { CustomTheme, Settings } from "./api";

type ThemeSettings = Pick<Settings, "theme" | "custom_theme">;

/** Last applied theme, read by the inline script in index.html before first paint. */
const CACHE = "writepad-theme";
const systemDark = window.matchMedia("(prefers-color-scheme: dark)");
let current: ThemeSettings | null = null;

export const PRESETS: { name: string; theme: CustomTheme }[] = [
  { name: "Paper", theme: { base: "light", sidebar: "#efe9de", background: "#f6f2ea", page: "#fffdf8", text: "#23212b", accent: "#d9774a" } },
  { name: "Night", theme: { base: "dark", sidebar: "#22212a", background: "#1b1a21", page: "#26252f", text: "#ebe6dc", accent: "#ea8b5e" } },
  { name: "Ocean", theme: { base: "light", sidebar: "#e3edf3", background: "#eef4f7", page: "#fbfdfe", text: "#1d2a33", accent: "#2a78d6" } },
  { name: "Rose", theme: { base: "light", sidebar: "#f3e4e8", background: "#f8eef0", page: "#fffafb", text: "#2e2228", accent: "#c2456e" } },
  { name: "Forest", theme: { base: "dark", sidebar: "#1c2620", background: "#161e19", page: "#212c25", text: "#e4ece4", accent: "#5cbf8a" } },
  { name: "Midnight", theme: { base: "dark", sidebar: "#1a2133", background: "#141a29", page: "#1f2740", text: "#e3e8f5", accent: "#7aa2f7" } },
];

/** Every variable a custom theme sets, so switching away can clear them all. */
const CUSTOM_VARS = ["--panel", "--bg", "--paper", "--ink", "--accent", "--accent-ink", "--muted", "--line", "--desk", "--speak", "--graph-surface"];

function customVars(c: CustomTheme): Record<string, string> {
  return {
    "--panel": c.sidebar,
    "--bg": c.background,
    "--paper": c.page,
    "--ink": c.text,
    "--accent": c.accent,
    "--accent-ink": luminance(c.accent) > 0.45 ? "#1b1a21" : "#ffffff",
    "--muted": `color-mix(in srgb, ${c.text} 58%, ${c.page})`,
    "--line": `color-mix(in srgb, ${c.text} 16%, ${c.sidebar})`,
    "--desk": c.base === "dark" ? `color-mix(in srgb, ${c.background} 78%, #000)` : `color-mix(in srgb, ${c.background} 90%, ${c.text})`,
    "--speak": `color-mix(in srgb, ${c.accent} 24%, transparent)`,
    "--graph-surface": c.page,
  };
}

export function applyTheme(s: ThemeSettings) {
  current = s;
  const root = document.documentElement;
  const mode = s.theme === "custom" ? s.custom_theme.base : s.theme === "system" ? (systemDark.matches ? "dark" : "light") : s.theme;
  root.dataset.theme = mode;
  for (const name of CUSTOM_VARS) root.style.removeProperty(name);
  const vars = s.theme === "custom" ? customVars(s.custom_theme) : {};
  for (const [name, value] of Object.entries(vars)) root.style.setProperty(name, value);
  try {
    localStorage.setItem(CACHE, JSON.stringify({ mode, system: s.theme === "system", vars }));
  } catch {
    // No storage: the theme still applies, just a frame later on next launch.
  }
}

systemDark.addEventListener("change", () => {
  if (current?.theme === "system") applyTheme(current);
});

/** Relative luminance of a #rrggbb color (WCAG). */
export function luminance(hex: string): number {
  const n = parseInt(hex.replace("#", ""), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio between two #rrggbb colors (1 to 21). */
export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}
