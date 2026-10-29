import { useEffect, useRef, useState } from "react";
import { HOTKEY_FIELD, hotkeysSuspend, inTauri, type Action, type Config } from "@/lib/api";
import { keycaps, readChord } from "@/lib/keys";

export const ACTIONS: { id: Action; name: string; sub: string }[] = [
  { id: "session", name: "Start or stop", sub: "Talk while you point. Press again to finish." },
  { id: "region", name: "Mark up a region", sub: "Freeze the screen, box it, then pin, draw or zoom." },
  { id: "full", name: "Capture the screen", sub: "Add the whole screen as it is right now." },
  { id: "zoom", name: "Capture around the pointer", sub: "A close-up of whatever is under your cursor." },
];

export function Keycaps({ label, size }: { label: string; size?: "lg" }) {
  return (
    <span className="keys">
      {keycaps(label).map((k, i) => (
        <kbd key={i} className={`kc ${size ?? ""}`}>
          {k}
        </kbd>
      ))}
    </span>
  );
}

/** A shortcut you change by clicking it and pressing the new keys. Esc cancels. */
export function ShortcutField({
  action,
  cfg,
  patch,
  setCfg,
}: {
  action: Action;
  cfg: Config;
  patch: (p: Record<string, unknown>) => Promise<string | null>;
  setCfg: (c: Config) => void;
}) {
  const [listening, setListening] = useState(false);
  const [live, setLive] = useState<string[]>([]);
  const [err, setErr] = useState("");
  const ref = useRef<HTMLButtonElement | null>(null);
  const taken = cfg.hotkey_errors.find((e) => e.action === action)?.error;
  const label = cfg.hotkeys[action].label;

  useEffect(() => {
    if (!listening) return;
    if (inTauri) hotkeysSuspend(true).catch(() => undefined);
    let finished = false;
    const finish = async (chord: string | null) => {
      if (finished) return;
      finished = true;
      setListening(false);
      setLive([]);
      if (chord && chord !== label) {
        const e = await patch({ [HOTKEY_FIELD[action]]: chord });
        setErr(e ? e.replace(/^Error:\s*/, "") : "");
      }
      if (inTauri) hotkeysSuspend(false).then(setCfg).catch(() => undefined);
    };
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) return void finish(null);
      const r = readChord(e);
      setLive(r.live);
      if (r.done) void finish(r.done);
    };
    const onUp = (e: KeyboardEvent) => {
      const r = readChord(e);
      setLive(r.live);
    };
    const onBlur = () => void finish(null);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onUp, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onUp, true);
      window.removeEventListener("blur", onBlur);
      if (!finished && inTauri) hotkeysSuspend(false).catch(() => undefined);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [listening]);

  return (
    <span style={{ display: "inline-flex", flexDirection: "column", alignItems: "flex-end", gap: 4 }}>
      <button
        ref={ref}
        type="button"
        className="btn"
        aria-label={`Change shortcut, now ${label}`}
        style={{
          height: "auto",
          minHeight: "var(--h-control)",
          padding: "3px 6px",
          boxShadow: listening ? "0 0 0 3px color-mix(in srgb, var(--accent) 45%, transparent)" : undefined,
        }}
        onClick={() => {
          setErr("");
          setListening((v) => !v);
        }}
      >
        {listening ? (
          live.length ? (
            <span className="keys">
              {live.map((k, i) => (
                <kbd key={i} className="kc">
                  {k}
                </kbd>
              ))}
            </span>
          ) : (
            <span className="muted" style={{ padding: "0 6px" }}>
              Type shortcut…
            </span>
          )
        ) : (
          <>
            <Keycaps label={label} />
            <span className="kc-change">Change</span>
          </>
        )}
      </button>
      {err || taken ? (
        <span className="t-small" style={{ color: "var(--red-ink)", maxWidth: 240, textAlign: "right" }}>
          {err || `${taken}. Click to choose another.`}
        </span>
      ) : null}
    </span>
  );
}
