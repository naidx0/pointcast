import { useEffect, useState } from "react";
import { inTauri, settingsGet, settingsPatch } from "./api";
import { waveStyleOf, type WaveStyle } from "./aurora";

export { WAVE_STYLES, waveStyleOf, type WaveStyle } from "./aurora";

/** The label each choice carries in Settings. */
export const WAVE_LABELS: Record<WaveStyle, string> = { pen: "Classic", aurora: "Aurora" };

const CHANNEL = "pointcast-wave-style";
let known: WaveStyle = "aurora";
const listeners = new Set<(s: WaveStyle) => void>();
const channel = typeof BroadcastChannel !== "undefined" ? new BroadcastChannel(CHANNEL) : null;

const publish = (s: WaveStyle) => {
  known = s;
  listeners.forEach((f) => f(s));
};
// Another window (Settings) changed it: every voice line follows at once.
if (channel) channel.onmessage = (e) => publish(waveStyleOf(e.data));

/** Save the voice wave (Settings row) and tell the other windows. Resolves to the style now kept. */
export async function waveStyleSet(style: WaveStyle): Promise<WaveStyle> {
  const want = waveStyleOf(style);
  if (inTauri) {
    const c = (await settingsPatch({ wave_style: want })) as { wave_style?: unknown };
    publish(waveStyleOf(c.wave_style));
  } else {
    publish(want);
  }
  channel?.postMessage(known);
  return known;
}

async function load() {
  if (!inTauri) return;
  try {
    const c = (await settingsGet()) as { wave_style?: unknown };
    publish(waveStyleOf(c.wave_style));
  } catch {
    /* keep what is known */
  }
}

/** The chosen voice wave, Aurora until the setting is read. Read on mount and when the window is focused again, and
 * follows a change made in another window. */
export function useWaveStyle(): WaveStyle {
  const [style, setStyle] = useState<WaveStyle>(known);
  useEffect(() => {
    listeners.add(setStyle);
    setStyle(known);
    void load();
    const onFocus = () => void load();
    window.addEventListener("focus", onFocus);
    return () => {
      listeners.delete(setStyle);
      window.removeEventListener("focus", onFocus);
    };
  }, []);
  return style;
}
