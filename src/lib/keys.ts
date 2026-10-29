import { platform } from "./theme";

const MAC_GLYPH: Record<string, string> = {
  Ctrl: "⌃",
  Alt: "⌥",
  Shift: "⇧",
  Cmd: "⌘",
  Space: "Space",
};
const MAC_ORDER = ["Ctrl", "Alt", "Shift", "Cmd"];

/** Keycaps for a chord label from the app ("Ctrl+Shift+Space", "Shift+Cmd+Space"). */
export function keycaps(label: string): string[] {
  const parts = label.split("+").filter(Boolean);
  if (platform() !== "mac") return parts;
  const mods = MAC_ORDER.filter((m) => parts.includes(m));
  const rest = parts.filter((p) => !MAC_ORDER.includes(p));
  return [...mods, ...rest].map((p) => MAC_GLYPH[p] ?? p);
}

/** One string for prose: "⌃⇧Space" on a Mac, "Ctrl+Shift+Space" elsewhere. */
export function chordText(label: string): string {
  const caps = keycaps(label);
  if (platform() !== "mac") return caps.join("+");
  const last = caps[caps.length - 1];
  return caps.slice(0, -1).join("") + last;
}

function keyName(code: string): string | null {
  let m = /^Key([A-Z])$/.exec(code);
  if (m) return m[1];
  m = /^Digit(\d)$/.exec(code);
  if (m) return m[1];
  m = /^F(\d{1,2})$/.exec(code);
  if (m && Number(m[1]) >= 1 && Number(m[1]) <= 24) return `F${m[1]}`;
  if (code === "Space") return "Space";
  return null;
}

/** What a key event means for the shortcut being typed: keys held so far, and the finished chord. */
export function readChord(e: KeyboardEvent): { live: string[]; done: string | null } {
  const mac = platform() === "mac";
  const mods = [
    e.ctrlKey && "Ctrl",
    e.altKey && "Alt",
    e.shiftKey && "Shift",
    e.metaKey && (mac ? "Cmd" : "Win"),
  ].filter(Boolean) as string[];
  const key = keyName(e.code);
  const label = key ? [...mods, key].join("+") : null;
  return { live: keycaps([...mods, ...(key ? [key] : [])].join("+")), done: label };
}
