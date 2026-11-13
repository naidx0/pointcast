import { useEffect, useState, type MouseEvent as ReactMouseEvent } from "react";
import { holdWindow, onWinResized, subscribe, winClose, winIsDecorated, winIsMaximized, winMinimize, winToggleMaximize } from "@/lib/api";
import { platform } from "@/lib/theme";

/** A press on the bar itself (not on a button in it) moves the window by the one rule every window keeps (drag.ts): once
 * the pointer travels 3 px; a double press maximizes or restores it, as a Windows title bar does. */
export function dragWindow(e: ReactMouseEvent, maximizable = true) {
  if (e.button !== 0 || e.target !== e.currentTarget) return;
  e.preventDefault(); // taken here: the window-wide listener (startWindowDrag) leaves this press alone
  holdWindow(e.nativeEvent, e.detail === 2 && maximizable ? "zoom" : "move");
}

/** Windows: the slim bar across the top of the content, with Minimize, Maximize or Restore, and Close at Windows 11's
 * sizes and colours. No title and no icon: the sidebar already shows the app. */
export function TitleBar({ maximizable = true, scrolled = false }: { maximizable?: boolean; scrolled?: boolean }) {
  const [max, setMax] = useState(false);
  const [active, setActive] = useState(true);
  useEffect(() => {
    const sync = () => {
      winIsMaximized()
        .then(setMax)
        .catch(() => undefined);
    };
    sync();
    const off = subscribe(onWinResized(sync));
    const focus = () => setActive(document.hasFocus());
    focus();
    window.addEventListener("focus", focus);
    window.addEventListener("blur", focus);
    return () => {
      off();
      window.removeEventListener("focus", focus);
      window.removeEventListener("blur", focus);
    };
  }, []);
  const run = (f: () => Promise<void>) => () => {
    f().catch(() => undefined);
  };
  return (
    <div className={`win-bar${active ? "" : " inactive"}${scrolled ? " scrolled" : ""}`} onMouseDown={(e) => dragWindow(e, maximizable)}>
      <button type="button" className="win-cap" aria-label="Minimize" title="Minimize" onClick={run(winMinimize)}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0 5.5H10" />
        </svg>
      </button>
      {maximizable ? (
        <button type="button" className="win-cap" aria-label={max ? "Restore" : "Maximize"} title={max ? "Restore" : "Maximize"} onClick={run(winToggleMaximize)}>
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
            {max ? (
              <>
                <rect x="0.5" y="2.5" width="7" height="7" rx="1" />
                <path d="M2.5 2.5V1.5C2.5 0.95 2.95 0.5 3.5 0.5H8.5C9.05 0.5 9.5 0.95 9.5 1.5V6.5C9.5 7.05 9.05 7.5 8.5 7.5H7.5" />
              </>
            ) : (
              <rect x="0.5" y="0.5" width="9" height="9" rx="1" />
            )}
          </svg>
        </button>
      ) : null}
      <button type="button" className="win-cap close" aria-label="Close" title="Close" onClick={run(winClose)}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0.5 0.5L9.5 9.5M9.5 0.5L0.5 9.5" />
        </svg>
      </button>
    </div>
  );
}


/** Windows draws no frame around Settings and onboarding (platform.rs own_title_bar): the page draws the window's
 * buttons. True once the window says it has no frame of its own, and in the browser preview; never on macOS. */
export function useFrameless(): boolean {
  const mac = platform() === "mac";
  const [frameless, setFrameless] = useState(false);
  useEffect(() => {
    if (mac) return;
    winIsDecorated()
      .then((d) => setFrameless(!d))
      .catch(() => setFrameless(true));
  }, [mac]);
  return frameless;
}
