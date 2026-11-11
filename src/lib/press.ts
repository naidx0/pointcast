/** A button on a window shown without focus (the Ask panel) acts on the press itself: on Windows the first mouse-down
 * on an inactive webview only takes focus, so its `click` never comes. The click stays as the fallback (the keyboard,
 * or a press that did not reach the page) and never acts a second time for a press that already did. */
export type Press = {
  onPointerDown: (e: { button: number; isPrimary?: boolean }) => void;
  onClick: (e: { detail: number }) => void;
};

/** A click this soon after an acted press is that press's own click. */
const SAME_PRESS_MS = 1500;

export function pressOnce(act: () => void, now: () => number = () => Date.now()): Press {
  let downAt = -Infinity;
  return {
    onPointerDown(e) {
      if (e.button !== 0 || e.isPrimary === false) return;
      downAt = now();
      act();
    },
    onClick(e) {
      // A keyboard click (detail 0) always acts; a mouse click right after its own press does not act again.
      const same = e.detail > 0 && now() - downAt < SAME_PRESS_MS;
      downAt = -Infinity;
      if (!same) act();
    },
  };
}
