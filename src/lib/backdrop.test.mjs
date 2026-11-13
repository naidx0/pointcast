import { test } from "node:test";
import assert from "node:assert/strict";
import { GLACIER, glacierSvg, grainSvg, svgUrl } from "./backdrop.ts";

test("the Glacier painting is the icon's blues, in both shades, and the same every time", () => {
  for (const shade of ["light", "dark"]) {
    const svg = glacierSvg(shade);
    assert.equal(svg, glacierSvg(shade), "deterministic: one fixed seed");
    for (const c of GLACIER[shade]) assert.ok(svg.includes(`stop-color="${c}"`), `${shade} ${c}`);
    // The icon's blue (hue about 226°, strongly saturated) is one of its strokes.
    const hue = (h) => {
      const [r, g, b] = [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
      const mx = Math.max(r, g, b), mn = Math.min(r, g, b), d = mx - mn;
      const H = d === 0 ? 0 : mx === r ? ((g - b) / d) % 6 : mx === g ? (b - r) / d + 2 : (r - g) / d + 4;
      return { h: (H * 60 + 360) % 360, s: mx === 0 ? 0 : d / mx };
    };
    assert.ok(GLACIER[shade].some((c) => Math.abs(hue(c).h - 226) < 8 && hue(c).s > 0.7), `${shade} has the icon blue`);
    assert.ok(Math.abs(hue("#2f5bff").h - 226) < 8, "the check knows the icon blue");
    assert.ok(!GLACIER[shade].some((c) => Math.abs(hue(c).h - 20) < 30 && hue(c).s > 0.5), `${shade}: no warm strokes`);
    // Motion blur runs along the strokes (x), barely across them.
    const [x, y] = svg.match(/stdDeviation="([\d.]+) ([\d.]+)"/).slice(1).map(Number);
    assert.ok(x > 4 * y, `blur ${x} along, ${y} across`);
  }
  assert.notEqual(glacierSvg("light"), glacierSvg("dark"));
});

test("the painting and the grain are self-contained images: nothing loads from outside", () => {
  for (const svg of [glacierSvg("light"), glacierSvg("dark"), grainSvg()]) {
    assert.ok(!/https?:\/\/(?!www\.w3\.org\/2000\/svg)/.test(svg), "no external reference");
    const url = svgUrl(svg);
    assert.ok(url.startsWith("data:image/svg+xml"));
    assert.equal(decodeURIComponent(url.slice(url.indexOf(",") + 1)), svg, "round-trips");
  }
});
