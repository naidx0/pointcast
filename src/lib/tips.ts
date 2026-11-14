// Fast tooltips. The webview's own `title` tooltip waits about a second, which reads as "hovering is slow". This takes
// over every `title`: after TIP_DELAY_MS of the pointer resting on an element with one, a small tip shows beside it.
// The text moves to `data-tip` (and to `aria-label` when the element has none, so a screen reader still hears it).
// `placeTip` is pure, so node --test reads it (src/lib/tips.test.mjs).

export const TIP_DELAY_MS = 120;
/** Once one tip has shown, the next element's shows at once while the pointer moves along a row of buttons. */
export const TIP_WARM_MS = 600;
const GAP = 6;
const EDGE = 6;

export type Box = { left: number; top: number; width: number; height: number };

/** Where a tip of `tip` size goes for an element at `a` inside a `vw` x `vh` window: centred under it, above it when
 * there is no room below, and never past an edge. */
export function placeTip(a: Box, tip: { width: number; height: number }, vw: number, vh: number): { x: number; y: number } {
  const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, Math.max(lo, hi)));
  const x = clamp(a.left + a.width / 2 - tip.width / 2, EDGE, vw - tip.width - EDGE);
  const below = a.top + a.height + GAP;
  const fits = below + tip.height + EDGE <= vh;
  const y = fits ? below : clamp(a.top - GAP - tip.height, EDGE, vh - tip.height - EDGE);
  return { x, y };
}

let started = false;

export function startTips() {
  if (started || typeof document === "undefined") return;
  started = true;
  let tip: HTMLDivElement | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let shownAt = 0;
  let on: Element | null = null;

  const hide = () => {
    clearTimeout(timer);
    on = null;
    if (tip) tip.dataset.on = "0";
  };
  const show = (el: Element) => {
    const text = el.getAttribute("data-tip");
    if (!text || on !== el) return;
    tip ??= Object.assign(document.createElement("div"), { className: "pc-tip", role: "tooltip" });
    if (!tip.isConnected) document.body.appendChild(tip);
    tip.textContent = text;
    tip.dataset.on = "0";
    const r = el.getBoundingClientRect();
    const t = tip.getBoundingClientRect();
    const p = placeTip({ left: r.left, top: r.top, width: r.width, height: r.height }, { width: t.width, height: t.height }, innerWidth, innerHeight);
    tip.style.transform = `translate(${Math.round(p.x)}px, ${Math.round(p.y)}px)`;
    tip.dataset.on = "1";
    shownAt = Date.now();
  };

  document.addEventListener(
    "pointerover",
    (e) => {
      const el = (e.target as Element | null)?.closest?.("[title], [data-tip]");
      if (!el || el === on) return;
      const t = el.getAttribute("title");
      if (t) {
        el.setAttribute("data-tip", t);
        if (!el.hasAttribute("aria-label")) el.setAttribute("aria-label", t);
        el.removeAttribute("title");
      }
      hide();
      on = el;
      const warm = Date.now() - shownAt < TIP_WARM_MS;
      timer = setTimeout(() => show(el), warm ? 0 : TIP_DELAY_MS);
    },
    true,
  );
  document.addEventListener("pointerout", (e) => {
    const to = (e as PointerEvent).relatedTarget as Node | null;
    if (on && !(to && on.contains(to))) hide();
  }, true);
  for (const ev of ["pointerdown", "keydown", "scroll", "blur"]) addEventListener(ev, hide, true);
}
