import { useEffect, useState } from "react";
import { Accessibility, Mic, MonitorUp } from "lucide-react";
import {
  inTauri,
  micMonitor,
  onMicLevel,
  permissionOpen,
  permissionRequest,
  permissionsGet,
  relaunch,
  subscribe,
  type Access,
  type PermissionKind,
  type Permissions,
} from "@/lib/api";
import { platform } from "@/lib/theme";

const COPY: Record<PermissionKind, { name: string; why: string; icon: typeof Mic; tile: string }> = {
  microphone: {
    name: "Microphone",
    why: "To hear what you say. Speech is transcribed on this computer.",
    icon: Mic,
    tile: "tile-orange",
  },
  screen: {
    name: "Screen Recording",
    why: "To capture what you point at. Password managers are painted out.",
    icon: MonitorUp,
    tile: "tile-purple",
  },
  accessibility: {
    name: "Accessibility",
    why: "To paste the prompt into the app you were using. Optional.",
    icon: Accessibility,
    tile: "tile-blue",
  },
};

/** Keep the OS answer fresh while the user flips switches in System Settings. */
export function usePermissions(initial?: Permissions) {
  const [p, setP] = useState<Permissions | undefined>(initial);
  useEffect(() => {
    if (!inTauri) return;
    let live = true;
    const tick = () =>
      permissionsGet()
        .then((x) => live && setP(x))
        .catch(() => undefined);
    tick();
    const id = setInterval(tick, 1500);
    return () => {
      live = false;
      clearInterval(id);
    };
  }, []);
  return [p, setP] as const;
}

export function PermissionRows({
  perms,
  setPerms,
  meter,
}: {
  perms: Permissions;
  setPerms: (p: Permissions) => void;
  meter?: boolean;
}) {
  const kinds = (Object.keys(COPY) as PermissionKind[]).filter((k) => perms.gated[k]);
  const [asked, setAsked] = useState<Partial<Record<PermissionKind, boolean>>>({});
  const settingsName = platform() === "mac" ? "System Settings" : "Settings";

  async function allow(kind: PermissionKind) {
    setAsked((a) => ({ ...a, [kind]: true }));
    if (!inTauri) {
      setPerms({ ...perms, [kind]: "granted" });
      return;
    }
    // Opening the mic is what makes macOS show its prompt.
    if (kind === "microphone") await micMonitor(true).catch(() => undefined);
    setPerms(await permissionRequest(kind));
  }

  return (
    <div className="cards">
      {kinds.map((k) => {
        const c = COPY[k];
        const s: Access = perms[k];
        const Icon = c.icon;
        return (
          <div key={k} className="card">
            <span className={`onb-glyph ${c.tile}`} style={{ width: 32, height: 32, borderRadius: 8, margin: 0 }}>
              <Icon size={18} strokeWidth={2} />
            </span>
            <div className="grow">
              <div className="ttl">{c.name}</div>
              <div className="sub">{c.why}</div>
              {k === "microphone" && s === "granted" && meter ? <Meter /> : null}
              {k === "screen" && asked.screen && s !== "granted" ? (
                <div className="sub" style={{ marginTop: 6 }}>
                  Turned it on? macOS applies it after Pointcast reopens.{" "}
                  <button type="button" className="link" onClick={() => relaunch()}>
                    Reopen Pointcast
                  </button>
                </div>
              ) : null}
            </div>
            {s === "granted" ? (
              <span className="badge ok">Allowed</span>
            ) : s === "denied" || asked[k] ? (
              <button type="button" className="btn" onClick={() => permissionOpen(k).catch(() => undefined)}>
                Open {settingsName}
              </button>
            ) : (
              <button type="button" className="btn primary" onClick={() => allow(k)}>
                Allow…
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}

/** Twelve bars that follow the live input level. */
export function Meter() {
  const [lv, setLv] = useState<number[]>(Array(12).fill(0));
  useEffect(() => {
    if (!inTauri) {
      const id = setInterval(() => setLv((x) => [...x.slice(1), Math.random() * 0.8]), 90);
      return () => clearInterval(id);
    }
    micMonitor(true).catch(() => undefined);
    const off = subscribe(onMicLevel((l) => setLv((x) => [...x.slice(1), Math.min(1, l)])));
    return () => {
      off();
      micMonitor(false).catch(() => undefined);
    };
  }, []);
  const loud = Math.max(...lv);
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 10, marginTop: 8 }}>
      <div className="meter" aria-label="Microphone level">
        {lv.map((v, i) => (
          <i key={i} style={{ height: `${Math.max(3, v * 26)}px`, opacity: 0.35 + v * 0.65 }} />
        ))}
      </div>
      <span className="t-small">{loud > 0.15 ? "Sounds good." : "Say something — the bars should move."}</span>
    </div>
  );
}
