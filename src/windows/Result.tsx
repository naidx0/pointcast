import { useCallback, useEffect, useId, useRef, useState } from "react";
import { ClipboardPaste, Copy, FolderOpen, Mic, Pencil, RotateCw, Send, Trash2, X } from "lucide-react";
import {
  detectTools,
  historyCopy,
  historyDelete,
  historyEdit,
  historyGet,
  historyList,
  historyPaste,
  historyRemove,
  historyRetranscribe,
  historyRetranscribeCancel,
  inTauri,
  lastFolder,
  onHistoryChanged,
  onResult,
  openFolder,
  onTranscribeProgress,
  openTool,
  preview,
  ranText,
  sendToTool,
  onHistorySelect,
  subscribe,
  type DetectedTool,
  type HistoryMeta,
  type SessionExport,
} from "@/lib/api";
import { captionMarks } from "@/lib/marks";
import { platform } from "@/lib/theme";
import { UNDO_MS, close as closeUndo, due, startDelete, undo as undoDelete, undoView, visible, type Pending } from "@/lib/undo";

/** Every prompt you made, newest first; the selected one as it will read in the AI tool. */
export function Result() {
  const [entries, setEntries] = useState<HistoryMeta[]>([]);
  const [sel, setSel] = useState<string | null>(null);
  const selRef = useRef<string | null>(null);
  selRef.current = sel;
  const [exp, setExp] = useState<SessionExport | null>(null);
  const [done, setDone] = useState<"" | "copied" | "pasted">("");
  const [tools, setTools] = useState<DetectedTool[]>([]);
  const [folder, setFolder] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Why the last action did not do all it said, like a paste that could only copy.
  const [note, setNote] = useState("");
  const noteAt = useRef(0);
  // Bumped when a picture is marked up again, so the selected prompt reloads in place.
  const [rev, setRev] = useState(0);
  // A kept recording being transcribed again. `total_ms` is 0 until the first progress report.
  const [job, setJob] = useState<{ id: string; done_ms: number; total_ms: number } | null>(null);
  const [jobErr, setJobErr] = useState<{ id: string; text: string } | null>(null);
  // Cancel was pressed: the job ends after the piece under way, and its refusal is no error.
  const [cancelling, setCancelling] = useState(false);
  const cancelled = useRef(false);
  const progressAt = useRef(0);
  const mac = platform() === "mac";
  // A delete waits 5 seconds behind the Undo bar; until then the entry is only hidden (src/lib/undo.ts).
  const [pending, setPending] = useState<Pending | null>(null);
  const pendingRef = useRef<Pending | null>(null);
  const entriesRef = useRef<HistoryMeta[]>([]);
  const previewGone = useRef(new Set<string>());

  /** `latest`: a recording just finished, so its prompt is the one shown, whatever was picked before. */
  const load = useCallback(async (pick?: string | null, latest = false) => {
    if (!inTauri) {
      if (preview) {
        // ?engine=gpu|cpu: the first entry says it was transcribed on the graphics card or on the processor.
        const engine = new URLSearchParams(location.search).get("engine");
        const ran =
          engine === "gpu"
            ? { engine: "gpu", device: "NVIDIA GeForce RTX 2060 SUPER via Vulkan", transcribe_ms: 6_100, audio_ms: 121_000 }
            : engine === "cpu"
              ? { engine: "cpu-sherpa", transcribe_ms: 24_000, audio_ms: 121_000 }
              : {};
        setEntries(PREVIEW_ENTRIES.filter((e) => !previewGone.current.has(e.id)).map((e, i) => (i === 0 ? { ...e, ...ran } : e)));
        setSel((s) => pick ?? s ?? PREVIEW_ENTRIES[0].id);
        setExp(PREVIEW_EXPORT);
      }
      return;
    }
    const h = await historyList().catch(() => ({ entries: [] as HistoryMeta[], last: null }));
    setEntries(h.entries);
    setSel((s) => {
      const want = (latest ? h.last : pick) ?? s ?? h.last ?? h.entries[0]?.id ?? null;
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
    // Shown again later: an old failure no longer says anything about this visit.
    const onFocus = () => {
      if (Date.now() - noteAt.current > 2000) setNote("");
      load();
    };
    window.addEventListener("focus", onFocus);
    const off = subscribe(
      onResult(() => load(null, true)),
      onHistorySelect((c) => load(c.id)),
      onHistoryChanged((c) => {
        setRev((r) => r + 1);
        setJob((j) => (j && j.id === c.id ? null : j));
        load(null);
      }),
      onTranscribeProgress((p) => {
        progressAt.current = Date.now();
        setJob({ id: p.id, done_ms: p.done_ms, total_ms: p.total_ms });
      }),
    );
    return () => {
      window.removeEventListener("focus", onFocus);
      off();
    };
  }, [load]);

  useEffect(() => {
    if (preview && !inTauri) setExp(sel === "c" ? PREVIEW_UNCLEAR : PREVIEW_EXPORT);
    if (!sel || !inTauri) return;
    let live = true;
    historyGet(sel)
      .then((e) => live && setExp(e))
      .catch(() => live && setExp(null));
    return () => {
      live = false;
    };
  }, [sel, rev]);

  useEffect(() => setNote(""), [sel]);

  const flash = (k: "copied" | "pasted") => {
    setDone(k);
    setTimeout(() => setDone(""), 1600);
  };
  const run = async (f: () => Promise<void>, k?: "copied" | "pasted") => {
    setBusy(true);
    setNote("");
    try {
      if (inTauri) await f();
      if (k) flash(k);
    } catch (e) {
      noteAt.current = Date.now();
      setNote(String(e).replace(/^Error:\s*/, ""));
    } finally {
      setBusy(false);
    }
  };

  const retranscribe = async (id: string) => {
    setJobErr(null);
    cancelled.current = false;
    setCancelling(false);
    setJob({ id, done_ms: 0, total_ms: 0 });
    try {
      if (inTauri) await historyRetranscribe(id);
      else await simulateRetranscribe(id, setJob, () => cancelled.current);
      // Done, unless it reports progress on its own and history://changed ends it.
      if (Date.now() - progressAt.current > 2000) setJob((j) => (j && j.id === id ? null : j));
      setRev((r) => r + 1);
    } catch (e) {
      setJob(null);
      if (!cancelled.current) setJobErr({ id, text: String(e).replace(/^Error:\s*/, "") });
    } finally {
      setCancelling(false);
    }
  };

  const cancelAgain = () => {
    cancelled.current = true;
    setCancelling(true);
    if (inTauri) historyRetranscribeCancel().catch(() => undefined);
  };

  pendingRef.current = pending;
  entriesRef.current = entries;

  /** Make a delete real. A failure shows as the note, and the entry comes back with the next load. */
  const commit = useCallback(
    async (id: string) => {
      try {
        if (inTauri) await historyDelete(id);
        else previewGone.current.add(id);
      } catch (e) {
        noteAt.current = Date.now();
        setNote(String(e).replace(/^Error: ?/, ""));
      }
      await load(null);
    },
    [load],
  );

  const removeEntry = useCallback(
    (id: string) => {
      const r = startDelete(pendingRef.current, entriesRef.current, id, Date.now());
      if (r.commit) void commit(r.commit);
      pendingRef.current = r.pending;
      setPending(r.pending);
      if (id === selRef.current) setSel(r.next);
      setNote("");
    },
    [commit],
  );

  // Browser preview: ?delete=b shows that entry already deleted, with the bar up (add undo=hold to keep it).
  const previewDeleted = useRef(false);
  useEffect(() => {
    const id = preview && !inTauri ? new URLSearchParams(location.search).get("delete") : null;
    if (!id || previewDeleted.current || !entries.some((e) => e.id === id)) return;
    previewDeleted.current = true;
    removeEntry(id);
  }, [entries, removeEntry]);

  const undoEntry = useCallback(() => {
    const r = undoDelete(pendingRef.current);
    pendingRef.current = null;
    setPending(null);
    if (r.restore) setSel(r.restore);
  }, []);

  // The 5 seconds. ?undo=hold in the browser preview keeps the bar up for a look.
  useEffect(() => {
    if (!pending || (preview && !inTauri && location.search.includes("undo=hold"))) return;
    const t = setTimeout(
      () => {
        if (!due(pendingRef.current, Date.now())) return;
        const r = closeUndo(pendingRef.current);
        pendingRef.current = null;
        setPending(null);
        if (r.commit) void commit(r.commit);
      },
      Math.max(0, UNDO_MS - (Date.now() - pending.at)),
    );
    return () => clearTimeout(t);
  }, [pending, commit]);

  // The window closes: the pending delete is made real, there is nothing left to undo it from.
  useEffect(() => {
    const flush = () => {
      const r = closeUndo(pendingRef.current);
      pendingRef.current = null;
      if (r.commit && inTauri) void historyDelete(r.commit).catch(() => undefined);
    };
    window.addEventListener("pagehide", flush);
    window.addEventListener("beforeunload", flush);
    return () => {
      window.removeEventListener("pagehide", flush);
      window.removeEventListener("beforeunload", flush);
      flush();
    };
  }, []);

  // Delete removes the selected entry; Ctrl or Cmd+Z undoes it while the bar shows. Neither in a text field.
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      const t = ev.target as HTMLElement | null;
      if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
      if (ev.key === "Delete" && !ev.ctrlKey && !ev.metaKey && !ev.altKey && !ev.shiftKey && selRef.current) {
        ev.preventDefault();
        removeEntry(selRef.current);
      } else if ((ev.ctrlKey || ev.metaKey) && !ev.shiftKey && ev.key.toLowerCase() === "z" && pendingRef.current) {
        ev.preventDefault();
        undoEntry();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [removeEntry, undoEntry]);

  const shown = visible(entries, pending);
  const ai = tools.filter((t) => t.kind === "ai");
  const cur = shown.find((e) => e.id === sel);
  const bar = undoView(pending);

  return (
    <div className={`result${bar ? " has-undo" : ""}`}>
      <header className="result-bar drag">
        <div style={{ flex: "0 1 auto", minWidth: 0 }}>
          <div className="t-head" style={{ fontSize: 14 }}>
            History
          </div>
          <div className="t-small" role={note ? "alert" : undefined} style={note ? { color: "var(--orange-ink)" } : undefined}>
            {note ||
              (shown.length ? `${shown.length} ${shown.length === 1 ? "prompt" : "prompts"} on this computer` : " ")}
          </div>
        </div>
        {sel && exp ? (
          <button
            type="button"
            className="btn icon result-del"
            title={`Delete (${mac ? "Delete key" : "Del"})`}
            aria-label="Delete this prompt"
            onClick={() => removeEntry(sel)}
          >
            <Trash2 size={15} />
          </button>
        ) : null}
        <div style={{ flex: 1 }} />
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
                    await sendToTool(t.id).catch((e) => {
                      // Copied but not pasted: the tool is open, say so; anything else, just open the tool.
                      if (String(e).startsWith("Copied")) throw e;
                      return openTool(t.id);
                    });
                  })
                }
              >
                <Send size={13} />
                {t.name}
              </button>
            ))}
            {folder && sel === shown[0]?.id ? (
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
      {shown.length ? (
        <div className="hist">
          <nav className="hist-list" aria-label="Past prompts">
            {shown.map((e) => (
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
                  {e.partial ? <span className="badge warn hist-tag">Partial</span> : null}
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
            {cur && ranText(cur) ? (
              <p className="t-small ran-line" style={{ margin: "0 0 10px" }}>
                {ranText(cur)}
              </p>
            ) : null}
            {cur ? (
              <AudioBar
                audio={!!cur.has_audio}
                unclear={!!exp && exp.plain.includes("[unclear]")}
                partial={!!cur.partial}
                job={job && job.id === cur.id ? job : null}
                other={!!job && job.id !== cur.id}
                error={jobErr && jobErr.id === cur.id ? jobErr.text : ""}
                cancelling={cancelling}
                onRun={() => void retranscribe(cur.id)}
                onCancel={cancelAgain}
              />
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
      {bar ? (
        <div className="undo-bar" role="status">
          <span>{bar.text}</span>
          <span className="undo-dot" aria-hidden="true">
            ·
          </span>
          <button type="button" className="undo-btn" onClick={undoEntry}>
            {bar.action}
            <kbd>{mac ? "⌘Z" : "Ctrl+Z"}</kbd>
          </button>
        </div>
      ) : null}
    </div>
  );
}

/** Above a prompt: its audio is still here and can be transcribed again, with how that is going and Cancel. */
function AudioBar({
  audio,
  unclear,
  partial,
  job,
  other,
  error,
  cancelling,
  onRun,
  onCancel,
}: {
  audio: boolean;
  unclear: boolean;
  partial: boolean;
  job: { done_ms: number; total_ms: number } | null;
  other: boolean;
  error: string;
  cancelling: boolean;
  onRun: () => void;
  onCancel: () => void;
}) {
  if (!audio && !unclear && !partial) return null;
  const pct = job && job.total_ms > 0 ? Math.min(100, Math.floor((job.done_ms / job.total_ms) * 100)) : null;
  return (
    <div className="audio-bar">
      <div className="audio-row">
        <div className="grow t-small" style={{ display: "flex", alignItems: "center", gap: 6, fontVariantNumeric: "tabular-nums" }}>
          {job ? (
            <>
              <span className="spinner" style={{ width: 12, height: 12 }} aria-hidden="true" />
              {cancelling
                ? "Stopping after this part…"
                : pct == null
                  ? "Transcribing…"
                  : `Transcribing ${pct}%`}
            </>
          ) : audio ? (
            <>
              <Mic size={12} strokeWidth={2} aria-hidden="true" />
              Audio kept on this computer
            </>
          ) : null}
        </div>
        {job ? (
          <button
            type="button"
            className="btn"
            disabled={cancelling}
            title="Stop after the part being transcribed; this prompt stays as it was"
            onClick={onCancel}
          >
            Cancel
          </button>
        ) : audio ? (
          <button
            type="button"
            className="btn"
            disabled={other}
            title={other ? "Another recording is being transcribed" : "Write this recording down again from its audio"}
            onClick={onRun}
          >
            <RotateCw size={13} />
            Transcribe again
          </button>
        ) : null}
      </div>
      {job ? (
        <div
          className={`progress ${pct == null ? "indet" : ""}`}
          style={{ marginTop: 8 }}
          role="progressbar"
          aria-label="Transcribing again"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct ?? undefined}
        >
          <i style={pct == null ? undefined : { width: `${pct}%` }} />
        </div>
      ) : null}
      {error ? (
        <p className="t-small" role="alert" style={{ margin: "6px 0 0", color: "var(--red-ink)" }}>
          {error}
        </p>
      ) : null}
      {partial && !job ? (
        <p className="t-small" style={{ margin: "6px 0 0", color: "var(--orange-ink)" }}>
          {audio ? "Part of this recording is not transcribed yet. Transcribe again for the rest." : "Part of this recording was not transcribed."}
        </p>
      ) : unclear && !job ? (
        <p className="t-small" style={{ margin: "6px 0 0" }}>
          {audio ? "Part of this was unclear. Transcribe again, or use a larger model." : "Part of this was unclear. A larger model may hear it better."}
        </p>
      ) : null}
    </div>
  );
}

/** Browser preview: walk a kept recording through the same progress reports without a backend; `stopped` is Cancel. */
function simulateRetranscribe(
  id: string,
  setJob: (j: { id: string; done_ms: number; total_ms: number }) => void,
  stopped: () => boolean,
) {
  const total = 667_000;
  return new Promise<void>((resolve, reject) => {
    let done = 0;
    const t = setInterval(() => {
      done += 55_000;
      if (stopped()) {
        clearInterval(t);
        reject(new Error("cancelled"));
      } else if (done >= total) {
        clearInterval(t);
        resolve();
      } else setJob({ id, done_ms: done, total_ms: total });
    }, 500);
  });
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
  const uid = useId();
  // The picture a chip was just clicked for, lit for a moment.
  const [lit, setLit] = useState<number | null>(null);
  const litTimer = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => () => clearTimeout(litTimer.current), []);
  const figId = (n: number) => `pd-${uid}-${n}`;
  const show = (n: number) => {
    document.getElementById(figId(n))?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    setLit(n);
    clearTimeout(litTimer.current);
    litTimer.current = setTimeout(() => setLit(null), 1400);
  };

  const figure = (n: number) => {
    const img = exp.images[n - 1];
    return (
      <figure key={`f${n}`} id={figId(n)} className={`pd-fig${lit === n ? " lit" : ""}`}>
        {onEdit && img ? (
          <span className="pd-frame">
            <button type="button" className="pd-pic" title="Mark up this picture again" aria-label={`Edit picture ${n}`} onClick={() => onEdit(n)}>
              <img src={img.data_url} alt={`Image ${n}`} />
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
                aria-label={`Remove picture ${n}`}
                onClick={() => onRemove(n)}
              >
                <X size={12} strokeWidth={2} />
                Remove
              </button>
            ) : null}
          </span>
        ) : (
          <img src={img?.data_url} alt={`Image ${n}`} />
        )}
        <figcaption>
          [image {n}]
          {img?.labels.length ? ` · ${captionMarks(img.labels).join(" · ")}` : ""}
        </figcaption>
      </figure>
    );
  };

  // Paragraphs are the tidied text's blank lines. A picture shows once, after the first paragraph that names it.
  const placed = new Set<number>();
  return (
    <div className="prompt-doc">
      {exp.plain
        .split(/\n[ \t]*\n+/)
        .filter((para) => para.trim())
        .map((para, k) => {
          const parts = splitMarkers(para);
          const pics = parts.filter((p) => p.kind === "img" && !placed.has(p.n)).map((p) => p.n);
          pics.forEach((n) => placed.add(n));
          const onlyPics = parts.every((p) => p.kind === "img" || !p.value.trim());
          return (
            <div key={k} className="pd-block">
              {onlyPics ? null : (
                <p>
                  {parts.map((p, i) =>
                    p.kind === "text" ? (
                      <span key={i}>{p.value}</span>
                    ) : exp.images[p.n - 1] ? (
                      <button key={i} type="button" className="pd-chip" title={`Go to image ${p.n}`} onClick={() => show(p.n)}>
                        [image {p.n}]
                      </button>
                    ) : (
                      <span key={i} className="pd-chip">
                        [image {p.n}]
                      </span>
                    ),
                  )}
                </p>
              )}
              {Array.from(new Set(pics)).map(figure)}
            </div>
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
    has_audio: true,
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
    id: "d",
    created_ms: Date.now() - 7_200_000,
    duration_ms: 74_000,
    preview: "Walk through the checkout: the shipping step forgets the address when you go back.",
    images: 1,
    recovered: false,
    has_audio: true,
    partial: true,
  },
  {
    id: "c",
    created_ms: Date.now() - 90_000_000,
    duration_ms: 31_000,
    preview: "Why does this chart's y axis start at forty? [unclear]",
    images: 2,
    recovered: false,
    has_audio: true,
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

const PREVIEW_UNCLEAR: SessionExport = {
  ...PREVIEW_EXPORT,
  plain: `Why does this chart's y axis start at forty? [unclear] I expected it to begin at zero.

[image 1]`,
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
