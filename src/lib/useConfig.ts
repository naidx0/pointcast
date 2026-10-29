import { useCallback, useEffect, useState } from "react";
import { inTauri, onSpeechDone, settingsGet, settingsPatch, subscribe, type Config } from "./api";
import { platform } from "./theme";

/** Live settings for a window. Outside Tauri it serves a stand-in so the UI can be walked in a browser. */
export function useConfig() {
  const [cfg, setCfg] = useState<Config | null>(null);
  const [error, setError] = useState("");

  const refresh = useCallback(async () => {
    if (!inTauri) {
      setCfg((c) => c ?? previewConfig());
      return;
    }
    try {
      setCfg(await settingsGet());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const patch = useCallback(async (p: Record<string, unknown>): Promise<string | null> => {
    if (!inTauri) {
      setCfg((c) => (c ? ({ ...c, ...p } as Config) : c));
      return null;
    }
    try {
      setCfg(await settingsPatch(p));
      setError("");
      return null;
    } catch (e) {
      const msg = String(e);
      setError(msg);
      return msg;
    }
  }, []);

  useEffect(() => {
    refresh();
    const onFocus = () => refresh();
    window.addEventListener("focus", onFocus);
    const off = subscribe(onSpeechDone(() => refresh()));
    return () => {
      window.removeEventListener("focus", onFocus);
      off();
    };
  }, [refresh]);

  return { cfg, setCfg, refresh, patch, error };
}

function previewConfig(): Config {
  const mac = platform() === "mac";
  const hk = (label: string) => ({ label, keys: label.split("+"), default: label });
  return {
    platform: mac ? "macos" : "windows",
    materials: false,
    version: "0.4.0",
    hotkeys: {
      session: hk(mac ? "Shift+Cmd+Space" : "Ctrl+Shift+Space"),
      full: hk("Ctrl+Shift+1"),
      region: hk("Ctrl+Shift+2"),
      zoom: hk("Ctrl+Shift+Z"),
    },
    hotkey_errors: [],
    talk_mode: "tap",
    model: "auto",
    device: null,
    output: "auto_paste",
    also_clipboard: true,
    export_dir: null,
    zoom_px: 512,
    mark_point: "ring",
    zoom_target: "dwell",
    zoom_sharpen: true,
    outline_element: true,
    spoken_anchors: "ring",
    key_frames: false,
    start_at_login: true,
    check_updates: true,
    update: null,
    include_app: false,
    hidden: ["1Password", "Bitwarden", "KeePass", "Dashlane", "LastPass", "Keychain Access"],
    mcp_bind: "127.0.0.1:47821",
    mcp_enabled: true,
    onboarded: false,
    sounds: true,
    speech: {
      runtime: { id: "runtime", installed: false, lib: "" },
      models: [
        { id: "moonshine-base-en", name: "Moonshine Base v2 (English)", installed: false, bytes: 0 },
        { id: "moonshine-tiny-en", name: "Moonshine Tiny v2 (English)", installed: false, bytes: 29858559 },
      ],
    },
    speech_check: null,
    devices: ["MacBook Pro Microphone", "AirPods Pro"],
    permissions: {
      microphone: "granted",
      screen: "unknown",
      accessibility: "unknown",
      gated: { microphone: true, screen: mac, accessibility: mac },
    },
    recommended: "moonshine-base-en",
  };
}
