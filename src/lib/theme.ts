import { getCurrentWindow } from "@tauri-apps/api/window";
import { inTauri, platformInfo } from "./api";

/** Light / Dark / System. The window itself follows too, so vibrancy and Mica match the page. */
export type ThemePref = "light" | "dark" | "system";
const KEY = "pointcast.theme";

const mediaDark = () => typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;

export function themePref(): ThemePref {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : "system";
  } catch {
    return "system";
  }
}

const resolve = (pref: ThemePref): "light" | "dark" => (pref === "system" ? (mediaDark() ? "dark" : "light") : pref);

function paint(pref: ThemePref) {
  document.documentElement.dataset.theme = resolve(pref);
  if (inTauri) getCurrentWindow().setTheme(pref === "system" ? null : pref).catch(() => undefined);
}

export function setThemePref(pref: ThemePref) {
  try {
    if (pref === "system") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, pref);
  } catch {
    /* private mode */
  }
  paint(pref);
}

export function startTheme() {
  const q = new URLSearchParams(location.search).get("theme");
  if (q === "light" || q === "dark") {
    document.documentElement.dataset.theme = q;
    return;
  }
  paint(themePref());
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (themePref() === "system") paint("system");
  });
  // Other Pointcast windows change the preference; follow them.
  window.addEventListener("storage", (e) => {
    if (e.key === KEY) paint(themePref());
  });
}

export type Platform = "mac" | "win" | "linux";

export function platform(): Platform {
  const q = new URLSearchParams(location.search).get("os");
  if (q === "mac" || q === "win" || q === "linux") return q;
  const ua = navigator.userAgent;
  if (/Mac OS X|Macintosh/.test(ua)) return "mac";
  if (/Windows/.test(ua)) return "win";
  return "linux";
}

/** Tag <html> with the OS, and with `vibrant` once the window really has a material behind it. */
export function startPlatform(opts: { surface?: boolean } = {}) {
  const html = document.documentElement;
  const os = platform();
  html.classList.add(os);
  if (opts.surface) document.body.classList.add("surface");
  if (!inTauri) return;
  if (os === "mac") html.classList.add("vibrant");
  else
    platformInfo()
      .then((p) => html.classList.toggle("vibrant", p.materials))
      .catch(() => undefined);
}

export const isMac = () => platform() === "mac";
