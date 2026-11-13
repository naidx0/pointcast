import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";

/** The ring that follows the pointer while the AI acts on the computer: a soft breathing circle, with a sharper pulse
 * at each click it makes (computer/ring.rs). */
export function Ring() {
  const el = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const un = listen("ring://click", () => {
      const n = el.current;
      if (!n) return;
      n.classList.remove("clicked");
      // Re-reading layout restarts the pulse on every click.
      void n.offsetWidth;
      n.classList.add("clicked");
    });
    return () => void un.then((f) => f());
  }, []);
  return <div ref={el} className="ring" aria-hidden="true" />;
}
