import { useEffect, useMemo, useState } from "react";
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

/** The Glacier strokes (src/lib/backdrop.ts) behind a part of a window: `band` is the framed art at the top of
 * onboarding; `wash` is the faint tint behind Settings' sidebar and page title. The painting is one image, turned
 * and drifting by transform only, so the filter runs once; it holds still with reduced motion. Decorative only. */
export function Backdrop({ kind, className = "" }: { kind: "band" | "wash"; className?: string }) {
  // A wash is a tint: the light strokes in both themes (on a dark page the dark strokes vanish into it); CSS sets how
  // faint. The band is art and follows the theme.
  const theme = useShade();
  const shade: Shade = kind === "wash" ? "light" : theme;
  const art = useMemo(() => svgUrl(glacierSvg(shade)), [shade]);
  const grain = useMemo(() => `url("${svgUrl(grainSvg())}")`, []);
  return (
    <div className={`backdrop ${kind} ${className}`} aria-hidden="true">
      <img className="backdrop-art" src={art} alt="" draggable={false} decoding="async" />
      <div className="backdrop-grain" style={{ backgroundImage: grain }} />
    </div>
  );
}
