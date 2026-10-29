import { useEffect, useState } from "react";
import { ClipboardPaste, Copy, FolderOpen, Send } from "lucide-react";
import {
  copyAgain,
  detectTools,
  inTauri,
  lastFolder,
  lastPrompt,
  onResult,
  openFolder,
  openTool,
  pasteAgain,
  preview,
  sendToTool,
  subscribe,
  type DetectedTool,
  type SessionExport,
} from "@/lib/api";
import { platform } from "@/lib/theme";

/** The last prompt, as it will read in the AI tool: words with the pictures where they were taken. */
export function Result() {
  const [exp, setExp] = useState<SessionExport | null>(null);
  const [done, setDone] = useState<"" | "copied" | "pasted">("");
  const [tools, setTools] = useState<DetectedTool[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (inTauri) {
      lastPrompt().then(setExp).catch(() => undefined);
      detectTools().then(setTools).catch(() => undefined);
    } else if (preview) {
      setExp(PREVIEW_EXPORT);
      setTools(PREVIEW_TOOLS);
    }
    return subscribe(onResult(setExp));
  }, []);

  const flash = (k: "copied" | "pasted") => {
    setDone(k);
    setTimeout(() => setDone(""), 1600);
  };
  const run = async (f: () => Promise<void>, k?: "copied" | "pasted") => {
    setBusy(true);
    try {
      if (inTauri) await f();
      if (k) flash(k);
    } finally {
      setBusy(false);
    }
  };

  const ai = tools.filter((t) => t.kind === "ai");
  const words = exp ? exp.plain.replace(/\[image[^\]]*\]/g, "").split(/\s+/).filter(Boolean).length : 0;
  const mac = platform() === "mac";

  return (
    <div className="result">
      <header className="result-bar drag">
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="t-head" style={{ fontSize: 14 }}>
            Last Prompt
          </div>
          {exp ? (
            <div className="t-small">
              {words} words · {exp.images.length} {exp.images.length === 1 ? "picture" : "pictures"}
            </div>
          ) : null}
        </div>
        {exp ? (
          <>
            {ai.map((t) => (
              <button
                key={t.id}
                type="button"
                className="btn"
                title={`Copy and paste into ${t.name}`}
                disabled={busy}
                onClick={() =>
                  run(() =>
                    sendToTool(t.id).catch(async () => {
                      await copyAgain();
                      await openTool(t.id);
                    }),
                  )
                }
              >
                <Send size={13} />
                {t.name}
              </button>
            ))}
            <button
              type="button"
              className="btn icon"
              title="Show in folder"
              aria-label="Show in folder"
              onClick={() =>
                run(async () => {
                  const p = await lastFolder();
                  if (p) await openFolder(p);
                })
              }
            >
              <FolderOpen size={15} />
            </button>
            <button type="button" className="btn" disabled={busy} onClick={() => run(copyAgain, "copied")}>
              <Copy size={14} />
              {done === "copied" ? "Copied" : "Copy"}
            </button>
            <button
              type="button"
              className="btn primary"
              title={`Paste into the app behind this window (${mac ? "⌘V" : "Ctrl+V"})`}
              disabled={busy}
              onClick={() => run(pasteAgain, "pasted")}
            >
              <ClipboardPaste size={14} />
              {done === "pasted" ? "Pasted" : "Paste into App"}
            </button>
          </>
        ) : null}
      </header>
      <main className="result-body">
        {exp ? (
          <PromptDoc exp={exp} />
        ) : (
          <div style={{ height: "100%", display: "grid", placeItems: "center", textAlign: "center" }}>
            <div>
              <div className="t-head">No prompt yet</div>
              <p className="t-caption" style={{ marginTop: 4 }}>
                Start a recording from the {mac ? "menu bar" : "tray"} and it will show up here.
              </p>
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

export function PromptDoc({ exp }: { exp: SessionExport }) {
  return (
    <div className="prompt-doc">
      {splitMarkers(exp.plain).map((p, i) => {
        if (p.kind === "text") return <span key={i}>{p.value}</span>;
        const img = exp.images[p.n - 1];
        return (
          <figure key={i}>
            <img src={img?.data_url} alt={p.value} />
            <figcaption>
              Image {p.n}
              {img?.labels.length ? ` · ${img.labels.map((l) => `${l.n} ${l.text}`).join(" · ")}` : ""}
            </figcaption>
          </figure>
        );
      })}
    </div>
  );
}

const PREVIEW_TOOLS: DetectedTool[] = [
  { id: "claude", name: "Claude", path: "/preview", kind: "ai" },
  { id: "chatgpt", name: "ChatGPT", path: "/preview", kind: "ai" },
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
