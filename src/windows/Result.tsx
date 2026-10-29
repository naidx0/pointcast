import { useCallback, useEffect, useState } from "react";
import { ClipboardPaste, Copy, FolderOpen, Pencil, Send, Trash2, X } from "lucide-react";
import {
  detectTools,
  historyCopy,
  historyDelete,
  historyEdit,
  historyGet,
  historyList,
  historyPaste,
  historyRemove,
  inTauri,
  lastFolder,
  onHistoryChanged,
  onResult,
  openFolder,
  openTool,
  preview,
  sendToTool,
  subscribe,
  type DetectedTool,
  type HistoryMeta,
  type SessionExport,
} from "@/lib/api";
import { platform } from "@/lib/theme";

/** Every prompt you made, newest first; the selected one as it will read in the AI tool. */
export function Result() {
  const [entries, setEntries] = useState<HistoryMeta[]>([]);
  const [sel, setSel] = useState<string | null>(null);
  const [exp, setExp] = useState<SessionExport | null>(null);
  const [done, setDone] = useState<"" | "copied" | "pasted">("");
  const [tools, setTools] = useState<DetectedTool[]>([]);
  const [folder, setFolder] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Bumped when a picture is marked up again, so the selected prompt reloads in place.
  const [rev, setRev] = useState(0);
  const mac = platform() === "mac";

  const load = useCallback(async (pick?: string | null) => {
    if (!inTauri) {
      if (preview) {
        setEntries(PREVIEW_ENTRIES);
        setSel((s) => pick ?? s ?? PREVIEW_ENTRIES[0].id);
        setExp(PREVIEW_EXPORT);
      }
      return;
    }
    const h = await historyList().catch(() => ({ entries: [] as HistoryMeta[], last: null }));
    setEntries(h.entries);
    setSel((s) => {
      const want = pick ?? s ?? h.last ?? h.entries[0]?.id ?? null;
      return h.entries.some((e) => e.id === want) ? want : (h.entries[0]?.id ?? null);
    });
    lastFolder()
      .then(setFolder)
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    load();
    if (inTauri) detectTools().then(setTools).catch(() => undefined);
    else if (preview) setTools(PREVIEW_TOOLS);
    const onFocus = () => load();
    window.addEventListener("focus", onFocus);
    const off = subscribe(
      onResult(() => load(null)),
      onHistoryChanged(() => {
        setRev((r) => r + 1);
        load(null);
      }),
    );
    return () => {
      window.removeEventListener("focus", onFocus);
      off();
    };
  }, [load]);

  useEffect(() => {
    if (!sel || !inTauri) return;
    let live = true;
    historyGet(sel)
      .then((e) => live && setExp(e))
      .catch(() => live && setExp(null));
    return () => {
      live = false;
    };
  }, [sel, rev]);

  const flash = (k: "copied" | "pasted") => {
    setDone(k);
    setTimeout(() => setDone(""), 1600);
  };
  const run = async (f: () => Promise<void>, k?: "copied" | "pasted") => {
    setBusy(true);
    try {
      if (inTauri) await f();
      if (k) flash(k);
    } catch {
      /* the button simply does not flash */
    } finally {
      setBusy(false);
    }
  };

  const ai = tools.filter((t) => t.kind === "ai");
  const cur = entries.find((e) => e.id === sel);

  return (
    <div className="result">
      <header className="result-bar drag">
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="t-head" style={{ fontSize: 14 }}>
            History
          </div>
          <div className="t-small">
            {entries.length ? `${entries.length} ${entries.length === 1 ? "prompt" : "prompts"} on this computer` : " "}
          </div>
        </div>
        {sel && exp ? (
          <>
            {ai.map((t) => (
              <button
                key={t.id}
                type="button"
                className="btn"
                title={`Copy and paste into ${t.name}`}
                disabled={busy}
                onClick={() =>
                  run(async () => {
                    await historyCopy(sel);
                    await sendToTool(t.id).catch(() => openTool(t.id));
                  })
                }
              >
                <Send size={13} />
                {t.name}
              </button>
            ))}
            {folder && sel === entries[0]?.id ? (
              <button
                type="button"
                className="btn icon"
                title="Show in folder"
                aria-label="Show in folder"
                onClick={() => run(() => openFolder(folder))}
              >
                <FolderOpen size={15} />
              </button>
            ) : null}
            <button
              type="button"
              className="btn icon"
              title="Delete"
              aria-label="Delete this prompt"
              disabled={busy}
              onClick={() =>
                run(async () => {
                  await historyDelete(sel);
                  setSel(null);
                  await load(null);
                })
              }
            >
              <Trash2 size={15} />
            </button>
            <button type="button" className="btn" disabled={busy} onClick={() => run(() => historyCopy(sel), "copied")}>
              <Copy size={14} />
              {done === "copied" ? "Copied" : "Copy"}
            </button>
            <button
              type="button"
              className="btn primary"
              title={`Paste into the app behind this window (${mac ? "⌘V" : "Ctrl+V"})`}
              disabled={busy}
              onClick={() => run(() => historyPaste(sel), "pasted")}
            >
              <ClipboardPaste size={14} />
              {done === "pasted" ? "Pasted" : "Paste into App"}
            </button>
          </>
        ) : null}
      </header>
      {entries.length ? (
        <div className="hist">
          <nav className="hist-list" aria-label="Past prompts">
            {entries.map((e) => (
              <button
                key={e.id}
                type="button"
                className="hist-item"
                aria-current={e.id === sel ? "true" : undefined}
                onClick={() => setSel(e.id)}
              >
                <span className="hist-when">
                  {when(e.created_ms)}
                  {e.recovered ? <span className="badge warn hist-tag">Recovered</span> : null}
                </span>
                <span className="hist-text">{e.preview || (e.images ? "Pictures only" : "Empty")}</span>
                {e.images ? (
                  <span className="t-small">
                    {e.images} {e.images === 1 ? "picture" : "pictures"}
                  </span>
                ) : null}
              </button>
            ))}
          </nav>
          <main className="result-body">
            {cur?.recovered ? (
              <p className="t-caption" style={{ margin: "0 0 14px", color: "var(--orange-ink)" }}>
                Pointcast closed while this was recording. Everything transcribed or captured before then was kept.
              </p>
            ) : null}
            {exp ? (
              <PromptDoc
                exp={exp}
                onEdit={sel ? (n) => void run(() => historyEdit(sel, n)) : undefined}
                onRemove={
                  sel
                    ? (n) =>
                        void run(async () => {
                          setExp(await historyRemove(sel, n));
                          await load(sel);
                        }, "copied")
                    : undefined
                }
              />
            ) : null}
          </main>
        </div>
      ) : (
        <main className="result-body" style={{ display: "grid", placeItems: "center", textAlign: "center" }}>
          <div>
            <div className="t-head">No prompts yet</div>
            <p className="t-caption" style={{ marginTop: 4 }}>
              Each recording you finish is kept here, on this computer.
            </p>
          </div>
        </main>
      )}
    </div>
  );
}

function when(ms: number) {
  const d = new Date(ms);
  const today = new Date();
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  if (d.toDateString() === today.toDateString()) return `Today, ${time}`;
  const y = new Date(today);
  y.setDate(today.getDate() - 1);
  if (d.toDateString() === y.toDateString()) return `Yesterday, ${time}`;
  return `${d.toLocaleDateString([], { month: "short", day: "numeric" })}, ${time}`;
}

/** With `onEdit`, clicking a picture reopens it in the annotate overlay; with `onRemove`, a key frame Pointcast picked
 * can be taken out. */
export function PromptDoc({
  exp,
  onEdit,
  onRemove,
}: {
  exp: SessionExport;
  onEdit?: (n: number) => void;
  onRemove?: (n: number) => void;
}) {
  return (
    <div className="prompt-doc">
      {splitMarkers(exp.plain).map((p, i) => {
        if (p.kind === "text") return <span key={i}>{p.value}</span>;
        const img = exp.images[p.n - 1];
        return (
          <figure key={i}>
            {onEdit && img ? (
              <span className="pd-frame">
                <button type="button" className="pd-pic" title="Mark up this picture again" aria-label={`Edit picture ${p.n}`} onClick={() => onEdit(p.n)}>
                  <img src={img.data_url} alt={p.value} />
                  <span className="pd-edit" aria-hidden="true">
                    <Pencil size={12} strokeWidth={2} />
                    Edit picture
                  </span>
                </button>
                {img.auto ? (
                  <span className="pd-auto" title="Pointcast picked this moment from your recording">
                    Auto
                  </span>
                ) : null}
                {img.auto && onRemove ? (
                  <button
                    type="button"
                    className="pd-remove"
                    title="Take this picture out of the prompt"
                    aria-label={`Remove picture ${p.n}`}
                    onClick={() => onRemove(p.n)}
                  >
                    <X size={12} strokeWidth={2} />
                    Remove
                  </button>
                ) : null}
              </span>
            ) : (
              <img src={img?.data_url} alt={p.value} />
            )}
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

const PREVIEW_ENTRIES: HistoryMeta[] = [
  {
    id: "a",
    created_ms: Date.now() - 60_000,
    duration_ms: 16_000,
    preview: "The Place order button is sitting on top of the total. I boxed it and dropped a pin.",
    images: 2,
    recovered: false,
  },
  {
    id: "b",
    created_ms: Date.now() - 3_600_000,
    duration_ms: 9_000,
    preview: "Reply to Sam: the build is green, ship it after lunch.",
    images: 0,
    recovered: true,
  },
  {
    id: "c",
    created_ms: Date.now() - 90_000_000,
    duration_ms: 31_000,
    preview: "Why does this chart's y axis start at forty?",
    images: 2,
    recovered: false,
  },
];

const PREVIEW_EXPORT: SessionExport = {
  plain:
    "The Place order button is sitting on top of [image 1] the total. I boxed it and dropped a pin.\n\n[image 2: 1=Place order — 28px too far left]\n\nPlease shift the primary button so it no longer overlaps the total.",
  html: "",
  markdown: "",
  images: [
    {
      n: 1,
      kind: "full",
      width: 1600,
      height: 1000,
      labels: [],
      data_url: "/freeze.png",
      auto: true,
    },
    {
      n: 2,
      kind: "region",
      width: 800,
      height: 500,
      labels: [{ n: 1, text: "Place order — 28px too far left" }],
      data_url: "/freeze.png",
      auto: false,
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
