import { createElement, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState, type ReactNode } from "react";
import { Check, Copy, Square, X } from "lucide-react";
import {
  inTauri,
  liveClose,
  liveDropPicture,
  liveFit,
  liveOpenSettings,
  liveRetry,
  liveState,
  liveStop,
  onLiveDelta,
  onLiveLevel,
  onLiveState,
  subscribe,
} from "@/lib/api";
import { ENDED_MS, initialLive, isPinned, liveReduce, liveView, sentLine, type LiveDelta, type LivePicture, type LiveView } from "@/lib/live";
import {
  FLOOR_START,
  VOICE_DX,
  VOICE_STEP_MS,
  loopsPath,
  marginPath,
  restPath,
  smoothLevel,
  trackFloor,
  voiceLevel,
  voicePath,
} from "@/lib/line";
import { parseMarkdown, renderMarkdown, settleTail, type CodeBlock, type H } from "@/lib/markdown";

const noop = () => undefined;
const h = createElement as unknown as H<ReactNode>;
const ROW = 24;
/** The listening line's height: room for a tall stroke above the baseline and a little below. */
const VOICE_H = 36;
/** The resting stroke: as long as one line of text is tall, less its leading. */
const REST = 14;

/** The Ask panel: a question about the screen and its answer, docked at the right edge. While listening it is one quiet
 * row (the line, the red dot, the picture); once the question is asked it grows downward to the answer. The line carries
 * listening, thinking and answering; everything around it is the app's native look. */
export function Live() {
  const [m, dispatch] = useReducer(liveReduce, undefined, initialLive);
  const [now, setNow] = useState(() => Date.now());
  const [big, setBig] = useState(false);
  const pending = useRef<LiveDelta[]>([]);
  const frame = useRef(0);

  // Snapshots apply at once; streamed text is gathered and painted once per animation frame.
  useEffect(() => {
    const flush = () => {
      frame.current = 0;
      const d = pending.current;
      pending.current = [];
      if (d.length) dispatch({ type: "deltas", deltas: d });
    };
    const off = subscribe(
      onLiveState((s) => dispatch({ type: "snapshot", snap: s, now: Date.now() })),
      onLiveDelta((d) => {
        pending.current.push(d);
        if (!frame.current) frame.current = requestAnimationFrame(flush);
      }),
    );
    liveState()
      .then((s) => s && dispatch({ type: "snapshot", snap: s, now: Date.now() }))
      .catch(noop);
    return () => {
      off();
      cancelAnimationFrame(frame.current);
    };
  }, []);

  // The wait lines need a clock only while the request waits.
  const sending = m.snap.state === "sending";
  useEffect(() => {
    if (!sending) return;
    setNow(Date.now());
    const id = setInterval(() => !document.hidden && setNow(Date.now()), 500);
    return () => clearInterval(id);
  }, [sending, m.waitFrom]);

  const v = liveView(m, sending ? now : 0);

  useEffect(() => setBig(false), [m.snap.turn, v.thumb == null, v.compact]);

  // Not sent: said for a moment, then the panel hides.
  useEffect(() => {
    if (!v.hide) return;
    const id = setTimeout(() => liveClose().catch(noop), ENDED_MS);
    return () => clearTimeout(id);
  }, [v.hide, m.snap.turn]);

  // Esc always works: it makes the picture small again, stops what is under way, or closes. Delete takes the picture out.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        if (big) setBig(false);
        else (v.stop ? liveStop() : liveClose()).catch(noop);
      } else if ((e.key === "Delete" || e.key === "Backspace") && v.canDrop) {
        e.preventDefault();
        liveDropPicture().catch(noop);
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [big, v.stop, v.canDrop]);

  // Nothing animates while the window is hidden.
  useEffect(() => {
    const vis = () => document.documentElement.classList.toggle("live-hidden", document.hidden);
    vis();
    document.addEventListener("visibilitychange", vis);
    return () => document.removeEventListener("visibilitychange", vis);
  }, []);

  // Follow the newest line unless the person scrolled up to read.
  const body = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const followed = useRef(false);
  useEffect(() => {
    pinned.current = true;
    followed.current = false;
  }, [m.snap.turn]);
  useLayoutEffect(() => {
    const b = body.current;
    // An answer opened complete starts at its top; one that streamed stays at the line just written.
    if (v.phase !== "shown") followed.current = true;
    if (b && pinned.current && followed.current) b.scrollTop = b.scrollHeight;
  }, [m.text, v.phase, v.note, v.error]);
  const onScroll = () => {
    const b = body.current;
    if (b) pinned.current = isPinned(b.scrollTop, b.scrollHeight, b.clientHeight);
  };

  // The window's height follows the content, from one row up to 60 % of the screen (Tauri only; the preview lays out
  // by CSS). The scrolling body counts at its content's height.
  const root = useRef<HTMLDivElement>(null);
  const lastFit = useRef(0);
  useEffect(() => {
    const r = root.current;
    if (!inTauri || !r) return;
    const fit = () => {
      let want = 0;
      for (const c of Array.from(r.children) as HTMLElement[]) {
        if (c.classList.contains("live-sr")) continue;
        want += c.classList.contains("live-body") ? ((c.firstElementChild as HTMLElement | null)?.offsetHeight ?? 0) : c.offsetHeight;
      }
      want = Math.min(Math.round(screen.availHeight * 0.6), want);
      if (want > 0 && want !== lastFit.current) {
        lastFit.current = want;
        liveFit(want).catch(noop);
      }
    };
    const ro = new ResizeObserver(fit);
    ro.observe(r);
    for (const c of Array.from(r.querySelectorAll(".live > *, .live-inner"))) ro.observe(c);
    fit();
    return () => ro.disconnect();
  }, [v.phase, v.compact, big, !!v.thumb]);

  if (v.phase === "closed") return <div className="live closed" ref={root} />;

  const sr = (
    <div className="live-sr" aria-live="polite" role="status">
      {v.announce}
    </div>
  );
  const bigPic = big && v.thumb ? <BigPicture pic={v.thumb} close={() => setBig(false)} /> : null;

  if (v.compact)
    return (
      <div className={`live compact phase-${v.phase}`} ref={root} aria-label="Ask about the screen">
        <div className="live-row">
          {v.phase === "discarded" ? (
            <p className="live-ended live-in">{v.note}</p>
          ) : (
            <>
              <StopButton stop />
              <div className="live-row-line">{v.line === "voice" ? <VoiceLine /> : <LineRow v={v} />}</div>
              {v.thumb ? <Picture pic={v.thumb} label={v.label} big={big} setBig={setBig} canDrop={v.canDrop} under /> : null}
            </>
          )}
        </div>
        {bigPic}
        {sr}
      </div>
    );

  const asked = v.phase === "answering" || v.phase === "shown";
  return (
    <div className={`live phase-${v.phase}`} ref={root} aria-label="Ask about the screen">
      <header className="live-head">
        <Question key={m.snap.turn} text={v.question} />
        <StopButton stop={v.stop} />
      </header>
      <div className="live-body" ref={body} onScroll={onScroll}>
        <div className="live-inner">
          {v.thumb ? (
            <div className="live-pic">
              <Picture pic={v.thumb} label={v.label} big={big} setBig={setBig} canDrop={v.canDrop} />
            </div>
          ) : null}
          {bigPic}
          {v.line === "loops" ? (
            <div className="live-think">
              <LineRow v={v} />
              {v.provider ? <span className="live-provider">{v.provider}</span> : null}
            </div>
          ) : null}
          {v.note ? <p className="live-note live-in">{v.note}</p> : null}
          {v.error ? <ErrorLine err={v.error} /> : null}
          {asked ? <Answer text={m.text} v={v} /> : null}
        </div>
      </div>
      {v.complete ? <Foot text={m.text} v={v} /> : null}
      {sr}
    </div>
  );
}

/** Stop while anything is under way, Close otherwise: always visible, never only on hover. Esc does the same. */
function StopButton({ stop }: { stop: boolean }) {
  return (
    <button
      type="button"
      className={`live-hb${stop ? " stop" : ""}`}
      title={stop ? "Stop (Esc)" : "Close (Esc)"}
      aria-label={stop ? "Stop" : "Close"}
      onClick={() => (stop ? liveStop() : liveClose()).catch(noop)}
    >
      {stop ? <Square size={8} fill="currentColor" strokeWidth={0} /> : <X size={15} strokeWidth={1.75} />}
    </button>
  );
}

/** The question as asked: one line, the whole of it on hover or when opened. */
function Question({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  if (!text) return <span className="live-question" />;
  return (
    <button type="button" className={`live-question${open ? " open" : ""}`} title={open ? undefined : text} aria-expanded={open} onClick={() => setOpen(!open)}>
      {text}
    </button>
  );
}

/** Exactly the picture that will be sent, with the app it is of. When the picture changes (the person moved to another
 * app) the new one fades in over the old. A click enlarges it below; on hover or focus, or with Delete, it comes out. */
function Picture({
  pic,
  label,
  big,
  setBig,
  canDrop,
  under = false,
}: {
  pic: LivePicture;
  label: string;
  big: boolean;
  setBig: (b: boolean) => void;
  canDrop: boolean;
  under?: boolean;
}) {
  const [layers, setLayers] = useState<{ cur: LivePicture; prev: LivePicture | null; n: number }>({ cur: pic, prev: null, n: 0 });
  useEffect(() => {
    setLayers((l) => (l.cur.src === pic.src ? l : { cur: pic, prev: l.cur, n: l.n + 1 }));
  }, [pic.src]);
  const what = label ? `The picture of ${label}` : "The picture";
  return (
    <figure className={`live-shot${under ? " under" : ""}`}>
      <div className="live-thumb">
        <button
          type="button"
          className="live-thumb-btn"
          aria-expanded={big}
          aria-label={`${what}${canDrop ? " that will be sent" : ""}: ${big ? "make it small" : "enlarge it"}`}
          title={`${canDrop ? `${what} that will be sent` : `${what} that was sent`} (${pic.w} × ${pic.h})`}
          onClick={() => setBig(!big)}
        >
          {layers.prev ? <img key={layers.n - 1} className="old" src={layers.prev.src} alt="" draggable={false} /> : null}
          <img
            key={layers.n}
            className={layers.prev ? "new" : undefined}
            src={layers.cur.src}
            alt=""
            draggable={false}
            onAnimationEnd={() => setLayers((l) => (l.prev ? { ...l, prev: null } : l))}
          />
        </button>
        {canDrop ? (
          <button type="button" className="live-drop" title="Don't send the picture (Delete)" aria-label="Don't send the picture" onClick={() => liveDropPicture().catch(noop)}>
            <X size={11} strokeWidth={2.25} />
          </button>
        ) : null}
      </div>
      {label ? (
        <figcaption className="live-label" title={label}>
          {label}
        </figcaption>
      ) : null}
    </figure>
  );
}

/** The picture enlarged in place, full width, until clicked or Esc. */
function BigPicture({ pic, close }: { pic: LivePicture; close: () => void }) {
  return (
    <button type="button" className="live-big live-in" style={{ aspectRatio: `${pic.w || 16} / ${pic.h || 9}` }} aria-label="Make the picture small" onClick={close}>
      <img src={pic.src} alt="" draggable={false} />
    </button>
  );
}

/** Listening: the line drawn by the voice. The level is eased toward the voice every frame (quick to rise, slow to
 * fall) and laid down as one broad stroke every VOICE_STEP_MS while the paper moves smoothly under the pen. Drawn
 * straight into the SVG, so React does not render 60 times a second. Reduced motion: a still line and the dot. */
function VoiceLine() {
  const ref = useRef<HTMLDivElement>(null);
  const path = useRef<SVGPathElement>(null);
  const dot = useRef<SVGCircleElement>(null);
  const [w, setW] = useState(0);
  // What the pen has written survives a change of width (the picture taken out, a longer app name).
  const pen = useRef({ raw: 0, floor: FLOOR_START, env: 0, acc: 0, first: 0, hist: [] as number[] });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const size = () => setW(Math.max(80, Math.round(el.clientWidth)));
    size();
    const ro = new ResizeObserver(size);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  useEffect(() => {
    if (!w) return;
    const box = { w, h: VOICE_H };
    const paint = (d: string, tip: { x: number; y: number }) => {
      path.current?.setAttribute("d", d);
      dot.current?.setAttribute("cx", String(tip.x));
      dot.current?.setAttribute("cy", String(tip.y));
    };
    const still = voicePath([], box);
    paint(still.d, still.tip);
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const s = pen.current;
    let last = performance.now();
    const keep = Math.ceil(w / VOICE_DX) + 2;
    const off = subscribe(onLiveLevel((x) => (s.raw = x)));
    let raf = 0;
    const draw = (t: number) => {
      raf = requestAnimationFrame(draw);
      const dt = Math.min(100, Math.max(0, t - last));
      last = t;
      if (document.hidden) return;
      s.floor = trackFloor(s.floor, s.raw, dt);
      s.env = smoothLevel(s.env, voiceLevel(s.raw, s.floor), dt);
      s.acc += dt;
      while (s.acc >= VOICE_STEP_MS) {
        s.acc -= VOICE_STEP_MS;
        s.hist.push(s.env);
      }
      while (s.hist.length > keep) {
        s.hist.shift();
        s.first++;
      }
      const { d, tip } = voicePath(s.hist, box, { first: s.first, shift: s.acc / VOICE_STEP_MS, live: s.env });
      paint(d, tip);
    };
    raf = requestAnimationFrame(draw);
    return () => {
      off();
      cancelAnimationFrame(raf);
    };
  }, [w]);
  return (
    <div className="live-voice" ref={ref} aria-hidden="true">
      {w ? (
        <svg className="ln ink" width={w} height={VOICE_H} viewBox={`0 0 ${w} ${VOICE_H}`}>
          <path ref={path} />
          <circle className="tip" ref={dot} r={3} />
        </svg>
      ) : null}
    </div>
  );
}

/** Finishing and thinking: the line looping while the words are finished or the AI is asked. */
function LineRow({ v }: { v: LiveView }) {
  const { d, width } = loopsPath({ w: 220, h: ROW }, { n: v.compact ? 5 : 7, pitch: 19 });
  return (
    <div className="live-line live-in" aria-hidden="true">
      <svg className={`ln loops ${v.tone}`} width={width} height={ROW} viewBox={`0 0 ${width} ${ROW}`}>
        <path className="ghost" d={d} />
        <path className="tail" pathLength={100} d={d} />
        <path className="head" pathLength={100} d={d} />
      </svg>
    </div>
  );
}

function ErrorLine({ err }: { err: NonNullable<LiveView["error"]> }) {
  const act = () =>
    (err.action === "settings" ? liveOpenSettings() : err.action === "retry" ? liveRetry() : err.action === "stop" ? liveStop() : liveClose()).catch(
      noop,
    );
  return (
    <div className="live-err live-in">
      <svg className="ln warn" width={16} height={20} viewBox="0 0 16 20" aria-hidden="true">
        <path d={restPath(0.75, 10, 12)} />
      </svg>
      <span className="live-err-text">{err.text}</span>
      {err.action ? (
        <button type="button" className="btn" onClick={act}>
          {err.label}
        </button>
      ) : null}
    </div>
  );
}

/** Answering: the markdown, with the line down its margin turning under the line being written; its end is the caret.
 * Shown: the caret fades and the line rests as a short quiet stroke. */
function Answer({ text, v }: { text: string; v: LiveView }) {
  const streaming = v.phase === "answering";
  const blocks = useMemo(() => parseMarkdown(streaming ? settleTail(text) : text), [text, streaming]);
  const box = useRef<HTMLDivElement>(null);
  const end = useRef<HTMLSpanElement>(null);
  const margin = useRef<SVGPathElement>(null);
  const caret = useRef<SVGCircleElement>(null);
  const tail = <span className="md-end" ref={end} key="md-end" />;
  const nodes = renderMarkdown(blocks, h, { tail, code: (b, key, t) => <Code key={key} b={b} tail={t} /> });

  useLayoutEffect(() => {
    const place = () => {
      const b = box.current;
      if (!b) return;
      const br = b.getBoundingClientRect();
      let endX = 0.75;
      let endY = 18;
      const e = end.current;
      if (e && text) {
        const code = e.closest(".md-code");
        if (code) {
          // Under a code block the line turns just below it, not through it.
          endY = code.getBoundingClientRect().bottom - br.top + 4;
          endX = 14;
        } else {
          const er = e.getBoundingClientRect();
          endY = Math.round(er.top + er.height / 2 - br.top) + 11;
          endX = Math.max(14, Math.round(er.right - br.left) + 3);
        }
      }
      endY = Math.min(endY, br.height + 2);
      const p = margin.current;
      if (p) {
        p.setAttribute("d", marginPath({ x: 0.75, top: 4, bottom: endY, end: endX }));
        // Shown: the pen lifts and the line withdraws to a short stroke beside the first line (a dash offset, so the
        // change is a transition and not a redraw).
        const len = p.getTotalLength();
        p.style.strokeDasharray = `${len} ${len}`;
        p.style.strokeDashoffset = String(v.phase === "shown" ? Math.max(0, len - REST) : 0);
      }
      caret.current?.setAttribute("cx", String(endX));
      caret.current?.setAttribute("cy", String(endY));
    };
    place();
    const ro = new ResizeObserver(place);
    if (box.current) ro.observe(box.current);
    return () => ro.disconnect();
  });

  return (
    <div className={`live-answer md ${v.phase}`} ref={box}>
      <svg className={`ln ${v.tone}`} aria-hidden="true">
        <path className="margin" ref={margin} />
        <circle className="caret" ref={caret} r={2.5} />
      </svg>
      {nodes.length ? nodes : <p>{tail}</p>}
    </div>
  );
}

function useCopied(): [boolean, (text: string) => void] {
  const [done, setDone] = useState(false);
  useEffect(() => {
    if (!done) return;
    const id = setTimeout(() => setDone(false), 1500);
    return () => clearTimeout(id);
  }, [done]);
  return [
    done,
    (text: string) => {
      navigator.clipboard
        ?.writeText(text)
        .then(() => setDone(true))
        .catch(noop);
    },
  ];
}

function Code({ b, tail }: { b: CodeBlock; tail: ReactNode }) {
  const [done, copy] = useCopied();
  return (
    <div className="md-code">
      <pre>
        <code>
          {b.v}
          {tail}
        </code>
      </pre>
      <button type="button" className="md-copy" title="Copy code" aria-label={done ? "Copied" : "Copy code"} onClick={() => copy(b.v)}>
        {done ? <Check size={13} strokeWidth={2} /> : <Copy size={13} strokeWidth={1.75} />}
      </button>
    </div>
  );
}

/** Only when the answer is complete: what left the computer, and Copy. */
function Foot({ text, v }: { text: string; v: LiveView }) {
  const [done, copy] = useCopied();
  return (
    <footer className="live-foot live-in">
      <span className="live-sent">{sentLine(v)}</span>
      <button type="button" className="live-copy" onClick={() => copy(text)} aria-label={done ? "Copied" : "Copy the answer"}>
        {done ? <Check size={13} strokeWidth={2} /> : <Copy size={13} strokeWidth={1.75} />}
        <span>{done ? "Copied" : "Copy"}</span>
      </button>
    </footer>
  );
}
