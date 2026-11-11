import { createElement, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState, type ReactNode } from "react";
import { Check, Copy, Square, X } from "lucide-react";
import {
  inTauri,
  liveClose,
  liveDropPicture,
  liveFit,
  liveOpenSettings,
  liveState,
  liveStop,
  onLiveDelta,
  onLiveLevel,
  onLiveState,
  subscribe,
} from "@/lib/api";
import { initialLive, isPinned, liveReduce, liveView, sentLine, type LiveDelta, type LivePicture, type LiveView } from "@/lib/live";
import { VOICE_SLOTS, loopsPath, marginPath, restPath, voicePath } from "@/lib/line";
import { parseMarkdown, renderMarkdown, settleTail, type CodeBlock, type H } from "@/lib/markdown";

const noop = () => undefined;
const h = createElement as unknown as H<ReactNode>;
const ROW = 24;
/** The resting stroke: as long as one line of text is tall, less its leading. */
const REST = 14;

/** The Ask panel: a question about the screen and its answer, docked at the right edge. The line carries listening,
 * thinking and answering; everything around it is the app's native look. */
export function Live() {
  const [m, dispatch] = useReducer(liveReduce, undefined, initialLive);
  const [now, setNow] = useState(() => Date.now());
  const [levels, setLevels] = useState<{ v: number[]; first: number }>({ v: [], first: 0 });
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

  // The microphone level, only while listening, one sample per ~90 ms and one paint per frame.
  const listening = m.snap.state === "listening";
  useEffect(() => {
    setLevels({ v: [], first: 0 });
    if (!listening) return;
    const buf: number[] = [];
    let first = 0;
    let last = 0;
    let raf = 0;
    const off = subscribe(
      onLiveLevel((x) => {
        const t = performance.now();
        if (t - last < 80) return;
        last = t;
        buf.push(x);
        if (buf.length > VOICE_SLOTS) {
          buf.shift();
          first++;
        }
        if (!raf)
          raf = requestAnimationFrame(() => {
            raf = 0;
            setLevels({ v: buf.slice(), first });
          });
      }),
    );
    return () => {
      off();
      cancelAnimationFrame(raf);
    };
  }, [listening, m.snap.turn]);

  const v = liveView(m, sending ? now : 0);

  useEffect(() => setBig(false), [m.snap.turn, v.thumb == null]);

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

  // The window's height follows the content, up to 60 % of the screen (Tauri only; the preview lays out by CSS).
  const root = useRef<HTMLDivElement>(null);
  const inner = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!inTauri || !root.current || !inner.current) return;
    let last = 0;
    const fit = () => {
      const r = root.current;
      if (!r) return;
      const head = r.querySelector<HTMLElement>(".live-head")?.offsetHeight ?? 0;
      const foot = r.querySelector<HTMLElement>(".live-foot")?.offsetHeight ?? 0;
      const want = Math.min(Math.round(screen.availHeight * 0.6), head + (inner.current?.offsetHeight ?? 0) + foot);
      if (want !== last) {
        last = want;
        liveFit(want).catch(noop);
      }
    };
    const ro = new ResizeObserver(fit);
    ro.observe(inner.current);
    ro.observe(root.current);
    return () => ro.disconnect();
  }, []);

  if (v.phase === "closed") return <div className="live closed" ref={root} />;

  const asked = v.phase === "answering" || v.phase === "shown";
  return (
    <div className={`live phase-${v.phase}`} ref={root} aria-label="Ask about the screen">
      <header className="live-head">
        <span className="live-title" title={v.title || undefined}>
          {v.title}
        </span>
        {v.provider ? <span className={`live-provider${v.inFlight ? " on" : ""}`}>{v.provider}</span> : null}
        <button
          type="button"
          className="live-hb"
          title={v.stop ? "Stop (Esc)" : "Close (Esc)"}
          aria-label={v.stop ? "Stop" : "Close"}
          onClick={() => (v.stop ? liveStop() : liveClose()).catch(noop)}
        >
          {v.stop ? <Square size={10} fill="currentColor" strokeWidth={0} /> : <X size={15} strokeWidth={1.75} />}
        </button>
      </header>
      <div className="live-body" ref={body} onScroll={onScroll}>
        <div className="live-inner" ref={inner}>
          {v.thumb || v.words ? (
            <div className={`live-q${big ? " big" : ""}`}>
              {v.thumb ? <Thumb pic={v.thumb} big={big} setBig={setBig} canDrop={v.canDrop} /> : null}
              {v.words ? <p className={`live-words${v.hearing || asked ? " quiet" : ""}`}>{v.words}</p> : null}
            </div>
          ) : null}
          {v.line === "voice" || v.line === "loops" ? <LineRow key={v.line} v={v} levels={levels} /> : null}
          {v.note ? <p className="live-note live-in">{v.note}</p> : null}
          {v.error ? <ErrorLine err={v.error} /> : null}
          {asked ? <Answer text={m.text} v={v} /> : null}
        </div>
      </div>
      {v.complete ? <Foot text={m.text} v={v} /> : null}
      <div className="live-sr" aria-live="polite" role="status">
        {v.announce}
      </div>
    </div>
  );
}

/** Exactly the picture that will be sent: small, enlarged in place on a click, and taken out on hover or with Delete. */
function Thumb({ pic, big, setBig, canDrop }: { pic: LivePicture; big: boolean; setBig: (b: boolean) => void; canDrop: boolean }) {
  return (
    <div className={`live-thumb${big ? " big" : ""}`} style={big ? { aspectRatio: `${pic.w} / ${pic.h}` } : undefined}>
      <button
        type="button"
        className="live-thumb-btn"
        aria-expanded={big}
        aria-label={big ? "Make the picture small" : "Enlarge the picture"}
        title={`${canDrop ? "The picture that will be sent" : "The picture that was sent"} (${pic.w} × ${pic.h})`}
        onClick={() => setBig(!big)}
      >
        <img src={pic.src} alt="" draggable={false} />
      </button>
      {canDrop ? (
        <button
          type="button"
          className="live-drop"
          title="Don't send the picture (Delete)"
          aria-label="Don't send the picture"
          onClick={() => liveDropPicture().catch(noop)}
        >
          <X size={12} strokeWidth={2.25} />
        </button>
      ) : null}
    </div>
  );
}

/** Listening and thinking: the line drawn by the voice, or looping while the AI is asked. */
function LineRow({ v, levels }: { v: LiveView; levels: { v: number[]; first: number } }) {
  const ref = useRef<HTMLDivElement>(null);
  const [w, setW] = useState(348);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const size = () => setW(Math.max(120, Math.round(el.clientWidth)));
    size();
    const ro = new ResizeObserver(size);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const box = { w, h: ROW };
  if (v.line === "voice") {
    const { d, tip } = voicePath(levels.v, box, { first: levels.first });
    return (
      <div className="live-line live-in" ref={ref} aria-hidden="true">
        <svg className="ln ink" width={w} height={ROW} viewBox={`0 0 ${w} ${ROW}`}>
          <path d={d} />
          <circle className="tip" cx={tip.x} cy={tip.y} r={3} />
        </svg>
      </div>
    );
  }
  const { d, width } = loopsPath({ w: Math.min(w, 220), h: ROW }, { n: 7, pitch: 19 });
  return (
    <div className="live-line live-in" ref={ref} aria-hidden="true">
      <svg className={`ln loops ${v.tone}`} width={width} height={ROW} viewBox={`0 0 ${width} ${ROW}`}>
        <path className="ghost" d={d} />
        <path className="tail" pathLength={100} d={d} />
        <path className="head" pathLength={100} d={d} />
      </svg>
    </div>
  );
}

function ErrorLine({ err }: { err: NonNullable<LiveView["error"]> }) {
  const act = () => (err.action === "settings" ? liveOpenSettings() : err.action === "stop" ? liveStop() : liveClose()).catch(noop);
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
