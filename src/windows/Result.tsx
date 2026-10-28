import { useEffect, useState } from "react";
import { Mark, PRODUCT } from "@/components/brand/Mark";
import { Icon } from "@/components/Icons";
import {
  copyAgain,
  detectTools,
  lastPrompt,
  onResult,
  openTool,
  pasteAgain,
  sendToTool,
  type DetectedTool,
  type SessionExport,
} from "@/lib/api";
import { invoke } from "@tauri-apps/api/core";

export function Result() {
  const [exp, setExp] = useState<SessionExport | null>(null);
  const [copied, setCopied] = useState(false);
  const [tools, setTools] = useState<DetectedTool[]>([]);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    lastPrompt()
      .then(setExp)
      .catch(() => {
        if (typeof location !== "undefined" && location.search.includes("preview=1")) {
          setExp(PREVIEW_EXPORT);
        }
      });
    detectTools()
      .then(setTools)
      .catch(() => setTools(PREVIEW_TOOLS));
    const u = onResult(setExp);
    return () => {
      u.then((fn) => fn());
    };
  }, []);

  if (!exp) {
    return (
      <div className="page" style={{ display: "grid", placeItems: "center", padding: 32 }}>
        <div style={{ textAlign: "center" }}>
          <Mark size="lg" />
          <p className="sub" style={{ marginTop: 12 }}>
            No session yet. Start recording from the tray.
          </p>
        </div>
      </div>
    );
  }

  const ai = tools.filter((t) => t.kind === "ai");
  const superW = tools.find((t) => t.id === "superwhisper");

  return (
    <div className="page">
      <header className="title-drag" style={{ borderBottom: "1px solid var(--line)", padding: "14px 24px 16px" }}>
        <div className="row-between" style={{ alignItems: "flex-start" }}>
          <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
            <div className="traffic" aria-hidden="true">
              <i />
              <i />
              <i />
            </div>
            <Mark />
            <div>
              <p className="kicker">{PRODUCT} · last prompt</p>
              <h1 className="title" style={{ fontSize: "var(--fs-sheet)" }}>
                Ready to paste
              </h1>
            </div>
          </div>
        </div>
      </header>

      <main style={{ padding: "16px 24px 40px", maxWidth: 820, margin: "0 auto" }}>
        <div className="capbar glassbar result-actions no-drag" style={{ position: "sticky", top: 12, zIndex: 5, marginBottom: 16, width: "100%", justifyContent: "flex-start" }}>
          <button
            type="button"
            className="pill pri"
            title="Copy HTML + plain"
            onClick={async () => {
              setBusy("copy");
              try {
                await copyAgain();
                setCopied(true);
                setTimeout(() => setCopied(false), 1400);
              } finally {
                setBusy(null);
              }
            }}
          >
            <Icon name="copy" size={14} />
            {copied ? "Copied" : "Copy"}
          </button>
          <button
            type="button"
            className="pill"
            title="Paste into the focused app"
            onClick={async () => {
              setBusy("paste");
              try {
                await pasteAgain();
              } finally {
                setBusy(null);
              }
            }}
          >
            <Icon name="paste" size={14} />
            Paste
          </button>
          <button
            type="button"
            className="ib"
            title="Open export folder"
            onClick={async () => {
              const p = await invoke<string | null>("last_folder");
              if (p) await invoke("open_folder", { path: p });
            }}
          >
            <Icon name="folder" />
          </button>
          <span className="sep" />
          {ai.map((t) => (
            <button
              key={t.id}
              type="button"
              className="pill"
              title={`Copy and open ${t.name}`}
              disabled={busy === t.id}
              onClick={async () => {
                setBusy(t.id);
                try {
                  await sendToTool(t.id);
                } catch {
                  await copyAgain();
                  await openTool(t.id);
                } finally {
                  setBusy(null);
                }
              }}
            >
              <Icon name="send" size={14} />
              {t.name}
            </button>
          ))}
          {superW ? (
            <button
              type="button"
              className="pill"
              title="Open SuperWhisper"
              onClick={() => openTool("superwhisper").catch(() => undefined)}
            >
              <Icon name="mic" size={14} />
              SuperWhisper
            </button>
          ) : null}
        </div>

        <article className="card" style={{ lineHeight: 1.62, fontSize: "var(--fs-chat)" }}>
          <PromptBody exp={exp} />
        </article>
        <p className="mono" style={{ marginTop: 12, fontSize: "var(--fs-xs)", color: "var(--text3)" }}>
          HTML clipboard carries inline images. Plain fallback starts: {exp.plain.slice(0, 100)}
          {exp.plain.length > 100 ? "…" : ""}
        </p>
      </main>
    </div>
  );
}

function PromptBody({ exp }: { exp: SessionExport }) {
  const parts = splitMarkers(exp.plain);
  return (
    <div style={{ whiteSpace: "pre-wrap" }}>
      {parts.map((p, i) =>
        p.kind === "text" ? (
          <span key={i}>{p.value}</span>
        ) : (
          <span
            key={i}
            style={{
              display: "inline-flex",
              flexDirection: "column",
              alignItems: "flex-start",
              margin: "8px 0",
              verticalAlign: "middle",
            }}
          >
            <img
              src={exp.images[p.n - 1]?.data_url}
              alt={p.value}
              style={{
                maxHeight: 280,
                maxWidth: "100%",
                borderRadius: 12,
                border: "1px solid var(--line)",
              }}
            />
            <span className="photo-chip" style={{ marginTop: 6 }}>
              {p.value}
            </span>
          </span>
        ),
      )}
    </div>
  );
}

const PREVIEW_TOOLS: DetectedTool[] = [
  { id: "chatgpt", name: "ChatGPT", path: "/preview", kind: "ai" },
  { id: "claude", name: "Claude", path: "/preview", kind: "ai" },
  { id: "cursor", name: "Cursor", path: "/preview", kind: "ai" },
  { id: "superwhisper", name: "SuperWhisper", path: "/Applications/Superwhisper.app", kind: "stt" },
];

const PREVIEW_EXPORT: SessionExport = {
  plain:
    "The Place order button is sitting on top of the total. I boxed it and dropped a pin.\n\n[image 1: 1=Place order — 28px too far left]\n\nPlease shift the primary button so it no longer overlaps the total.",
  html: "",
  markdown: "",
  images: [
    {
      n: 1,
      kind: "region",
      width: 800,
      height: 500,
      labels: [{ n: 1, text: "Place order — 28px too far left" }],
      data_url: "/freeze.png",
    },
  ],
};

function splitMarkers(plain: string): { kind: "text" | "img"; value: string; n: number }[] {
  const re = /\[image (\d+)(?::[^\]]*)?\]/g;
  const out: { kind: "text" | "img"; value: string; n: number }[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(plain))) {
    if (m.index > last) out.push({ kind: "text", value: plain.slice(last, m.index), n: 0 });
    out.push({ kind: "img", value: m[0], n: Number(m[1]) });
    last = m.index + m[0].length;
  }
  if (last < plain.length) out.push({ kind: "text", value: plain.slice(last), n: 0 });
  return out;
}
