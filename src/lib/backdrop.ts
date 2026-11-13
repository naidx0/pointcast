// The Glacier backdrop (owner, 2026-11-12: "glacier … with our icon blue theme"): broad, motion-blurred strokes of
// the app icon's blues, painted by an SVG filter (a gradient pushed sideways by stretched noise, blurred along the
// strokes). Built once as an image, so the page only moves a picture: no filter runs while it drifts. Pure, so
// node --test reads it (src/lib/backdrop.test.mjs).

export type Shade = "light" | "dark";

/** The icon's blues from ice to deep, as gradient stops left to right. Light reads as morning frost; dark keeps the
 * same strokes at night, with the icon blue as its brightest band. */
export const GLACIER: Record<Shade, readonly string[]> = {
  light: ["#f4f8ff", "#dde8ff", "#adc4ff", "#6f9cff", "#3a6bff", "#2b55f0", "#8fa9ec", "#e8efff"],
  dark: ["#0b1022", "#13204a", "#1b2f6c", "#2a4fd0", "#16245a", "#0e1834", "#22408f", "#0d1530"],
};

/** The painting as an SVG document `size` px square: the strokes run along x; the page turns the picture. */
export function glacierSvg(shade: Shade, size = 1200): string {
  const stops = GLACIER[shade]
    .map((c, i, all) => `<stop offset="${(i / (all.length - 1)).toFixed(3)}" stop-color="${c}"/>`)
    .join("");
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 1000 1000" preserveAspectRatio="none">` +
    `<defs><linearGradient id="p" x1="0.22" x2="0.78" y1="0" y2="0">${stops}</linearGradient>` +
    `<filter id="s" x="-10%" y="-10%" width="120%" height="120%" color-interpolation-filters="sRGB">` +
    `<feTurbulence type="fractalNoise" baseFrequency="0.0016 0.022" numOctaves="3" seed="7" result="n"/>` +
    `<feDisplacementMap in="SourceGraphic" in2="n" scale="380" xChannelSelector="R" yChannelSelector="G" result="d"/>` +
    `<feGaussianBlur in="d" stdDeviation="34 5"/></filter></defs>` +
    `<rect x="-100" y="-100" width="1200" height="1200" fill="url(#p)" filter="url(#s)"/></svg>`
  );
}

/** Fine film grain as a small tile, laid over the strokes so they read as paint, not a gradient. */
export function grainSvg(size = 160): string {
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}">` +
    `<filter id="g"><feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2" stitchTiles="stitch"/></filter>` +
    `<rect width="100%" height="100%" filter="url(#g)"/></svg>`
  );
}

/** An SVG document as an image URL. */
export const svgUrl = (svg: string) => `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
