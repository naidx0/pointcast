import { useEffect, useMemo, useState, type MouseEvent as ReactMouseEvent } from "react";
import { glacierSvg, grainSvg, svgUrl, type Shade } from "@/lib/backdrop";

const shadeNow = (): Shade => (document.documentElement.dataset.theme === "dark" ? "dark" : "light");

/** The page's light or dark, followed as it changes. */
function useShade(): Shade {
  const [shade, setShade] = useState(shadeNow);
  useEffect(() => {
    const mo = new MutationObserver(() => setShade(shadeNow()));
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => mo.disconnect();
  }, []);
  return shade;
}

/** The Glacier strokes (src/lib/backdrop.ts): `band` is the framed art at the top of onboarding; `wash` is the soft
 * tint over Settings, from the window's top edge down. The painting is one image, turned and drifting by transform
 * only, so the filter runs once; it holds still with reduced motion. Decorative; on Windows the band also moves the
 * window (`onMouseDown`), since the window's buttons sit on it. */
export function Backdrop({ kind, className = "", onMouseDown }: { kind: "band" | "wash"; className?: string; onMouseDown?: (e: ReactMouseEvent) => void }) {
  // A wash is a tint: the light strokes in both themes (on a dark page the dark strokes vanish into it); CSS sets how
  // faint. The band is art and follows the theme.
  const theme = useShade();
  const shade: Shade = kind === "wash" ? "light" : theme;
  const art = useMemo(() => svgUrl(glacierSvg(shade)), [shade]);
  const grain = useMemo(() => `url("${svgUrl(grainSvg())}")`, []);
  // The drift rests while the window is in the background (reference study: never move behind the person's work).
  const [awake, setAwake] = useState(() => document.hasFocus());
  useEffect(() => {
    const on = () => setAwake(document.hasFocus() && !document.hidden);
    window.addEventListener("focus", on);
    window.addEventListener("blur", on);
    document.addEventListener("visibilitychange", on);
    return () => {
      window.removeEventListener("focus", on);
      window.removeEventListener("blur", on);
      document.removeEventListener("visibilitychange", on);
    };
  }, []);
  return (
    <div className={`backdrop ${kind} ${className}`} aria-hidden="true" onMouseDown={onMouseDown} style={onMouseDown ? { pointerEvents: "auto" } : undefined}>
      <img className="backdrop-art" src={art} alt="" draggable={false} decoding="async" style={{ pointerEvents: "none", animationPlayState: awake ? "running" : "paused" }} />
      <div className="backdrop-grain" style={{ backgroundImage: grain, pointerEvents: "none" }} />
    </div>
  );
}
