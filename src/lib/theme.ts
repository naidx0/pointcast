/** Light / Dark / System — follows OS, same contract as Sequence Nova. */
export type ThemePref = "light" | "dark" | "system";
const KEY = "pointcast.theme";

const mediaDark = () =>
  typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;

export function themePref(): ThemePref {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : "system";
  } catch {
    return "system";
  }
}

export const resolve = (pref: ThemePref, osDark: boolean): "light" | "dark" =>
  pref === "system" ? (osDark ? "dark" : "light") : pref;

const paint = (t: "light" | "dark") => {
  document.documentElement.dataset.theme = t;
  try {
    localStorage.setItem("pointcast.theme.last", t);
  } catch {
    /* private */
  }
};

let watching = false;
function watch() {
  if (watching) return;
  watching = true;
  if (typeof matchMedia === "function") {
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", (e) => {
      if (themePref() === "system") paint(e.matches ? "dark" : "light");
    });
  }
}

export function setThemePref(pref: ThemePref) {
  try {
    if (pref === "system") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, pref);
  } catch {
    /* private */
  }
  paint(resolve(pref, mediaDark()));
  watch();
}

export function startTheme(forced?: string | null) {
  const q =
    forced ??
    (typeof location !== "undefined" ? new URLSearchParams(location.search).get("theme") : null);
  if (q === "light" || q === "dark") {
    document.documentElement.dataset.theme = q;
    return;
  }
  paint(resolve(themePref(), mediaDark()));
  watch();
}

/** Tag the document with the host OS so chrome (traffic lights / caption) can adapt. */
export function startPlatform() {
  const ua = navigator.userAgent;
  const html = document.documentElement;
  html.classList.add("live", "pointcast");
  if (/Mac OS X|Macintosh/.test(ua)) html.classList.add("mac");
  else if (/Windows/.test(ua)) html.classList.add("win");
  else html.classList.add("linux");
}
