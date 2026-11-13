import { Fragment, createElement, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState, type PointerEvent as RPointerEvent, type ReactNode, type RefObject } from "react";
import { ArrowUp, Check, ChevronRight, Copy, ImagePlus, MessagesSquare, PenLine, Plus, ScrollText, Square, X } from "lucide-react";
import {
  inTauri,
  liveAskTyped,
  liveClose,
  liveDropPicture,
  liveFit,
  panelSizeRead,
  panelSizeSave,
  liveMark,
  liveOpenSettings,
  permissionOpen,
  liveRetry,
  liveSent,
  liveTakePicture,
  liveState,
  liveStop,
  onListAsk,
  onLiveDelta,
  onLiveLevel,
  onLiveState,
  onSummaryState,
  preview,
  sessionsDelete,
  sessionsExport,
  sessionsList,
  sessionsNew,
  sessionsOpen,
  sessionsRename,
  sessionsSearch,
  subscribe,
  summaryGet,
  summaryMake,
  summaryStop,
  takeListAsk,
} from "@/lib/api";
import {
  ENDED_MS,
  dragSize,
  panelWidth,
  parsePanelSize,
  initialLive,
  isPinned,
  liveReduce,
  liveView,
  openNumber,
  panelHeight,
  pictureCaption,
  revealSlice,
  revealStep,
  sentLine,
  settledSplit,
  type LiveDelta,
  type LivePicture,
  type LiveView,
  type NoPictureView,
  type Phase,
  type Tone,
} from "@/lib/live";
import {
  FOLLOW_MAX,
  canSend,
  earlierRows,
  fieldPlaceholder,
  followKey,
  followUpShown,
  isFocusKey,
  rowLabel,
  sendText,
  sentView,
  type ReplayView,
  type SentView,
  type TurnRow,
} from "@/lib/conversation";
import { renameValue, searchQuery, sessionRows, turnsLabel, type SessionMeta } from "@/lib/sessions";
import { canSummarize, noSummary, summaryReduce, summaryText, summaryView, type SummaryModel } from "@/lib/summary";
import { marginPath, restPath } from "@/lib/line";
import { platform } from "@/lib/theme";
import { useConfig } from "@/lib/useConfig";
import { chordText } from "@/lib/keys";
import { Line, type LevelSource } from "@/components/Line";
import { Keycaps } from "@/components/Shortcut";
import { parseMarkdown, renderMarkdown, settleTail, type CodeBlock, type H } from "@/lib/markdown";
import { PressButton } from "@/components/PressButton";
import { MarkUpControl, MarkedBadge } from "@/components/MarkUp";
import { isMarkKey, markedLabel } from "@/lib/markup";

const noop = () => undefined;
const plain = (e: unknown) => String(e).replace(/^Error: */, "");
/** The panel's line hears the engine's `loudness` stream (the preview's made-up voice in a browser). */
const liveLevels: LevelSource = (set) => subscribe(onLiveLevel(set));
const typing = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));

const h = createElement as unknown as H<ReactNode>;
const ROW = 24;
/** The listening line's height: room for a tall stroke above the baseline and a little below. */
const VOICE_H = 36;
/** The resting stroke: as long as one line of text is tall, less its leading. */
const REST = 14;

/** The browser preview opens a part of the panel for a look: `sessions=1` the session list, `sent=<turn>` that turn's
 * Sent view, `expand=<turn>` an earlier turn's answer, `summary=working|done` the summary view. */
const pvAsk = (k: string) => (preview && !inTauri ? new URLSearchParams(location.search).get(k) : null);

/** The Ask panel: a question about the screen and its answer, docked at the right edge. While listening it is one quiet
 * row (the line, the red dot, the picture); once the question is asked it grows downward to the answer. After an answer
 * the session reads as a quiet column: each earlier turn one line that opens in place, the newest answer open, and one
 * slim field at the foot for a typed follow-up. The line carries listening, thinking and answering; everything around
 * it is the app's native look. */
export function Live() {
  const [m, dispatch] = useReducer(liveReduce, undefined, initialLive);
  const [now, setNow] = useState(() => Date.now());
  const [big, setBig] = useState(false);
  // The session list, open in place of the conversation.
  const [sheet, setSheet] = useState(() => pvAsk("sessions") === "1");
  // The turn whose Sent view is open, and the earlier turns opened to their answers.
  const [sentOf, setSentOf] = useState<number | null>(() => Number(pvAsk("sent")) || null);
  const [opened, setOpened] = useState<ReadonlySet<number>>(() => new Set(pvAsk("expand") ? [Number(pvAsk("expand"))] : []));
  // The session's summary, and whether its view is open in place of the conversation.
  const [sum, sumDispatch] = useReducer(summaryReduce, undefined, () => noSummary());
  const [sumOpen, setSumOpen] = useState(() => !!pvAsk("summary"));
  // A turn a summary point referred to: opened and brought into view once the conversation is back.
  const [goto, setGoto] = useState<number | null>(null);
  // The follow-up field's "a picture goes along", here so the M key knows whether there is a picture to mark up.
  const [fieldPic, setFieldPic] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);
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

  // Home's "All sessions" opens the panel on its list.
  useEffect(() => {
    if (takeListAsk()) setSheet(true);
    return onListAsk(() => setSheet(true));
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
  const earlier = useMemo(() => earlierRows(m.snap), [m.snap]);
  // The open turn's number in the session (never the panel's own question count).
  const turnN = openNumber(m.snap);
  // A conversation once the session has a turn before the open one, even before those turns arrive (joining a closed
  // session they come a moment later): the header does not flip from the question to the title.
  const multi = earlier.length > 0 || turnN > 1;
  const showField = followUpShown(v) && !sheet && !sumOpen;
  const session = m.snap.session.id;
  // Every turn the session has, the open one once it is answered.
  const turnCount = Math.max(m.snap.session.turns, earlier.length + (v.complete ? 1 : 0));
  const hasSummary = sum.state !== "none";
  const offerSummary = !!session && canSummarize(turnCount) && !hasSummary;
  const turnNumbers = useMemo(() => [...earlier.map((r) => r.n), ...(v.complete ? [turnN] : [])], [earlier, v.complete, turnN]);
  // The question's picture can still be marked up: heard and not sent yet, or a typed question's picture to come.
  const canMark = (v.canDrop && !!v.thumb) || (followUpShown(v) && (fieldPic || !!v.next));
  // A first question: nothing answered in this session yet.
  const first = v.empty || (!m.snap.session.turns && !earlier.length && !v.complete);
  // Why the field's last ask or mark-up was refused, said quietly under it, until the panel moves on.
  const [fieldNote, setFieldNote] = useState("");
  useEffect(() => setFieldNote(""), [m.snap.state, session]);

  useEffect(() => setBig(false), [m.snap.turn, v.thumb == null, v.compact]);
  // Another session: nothing of the last one stays open. The first id to arrive is the same session, not another.
  const lastSession = useRef("");
  useEffect(() => {
    const was = lastSession.current;
    lastSession.current = session || was;
    if (!was || !session || was === session) return;
    setOpened(new Set());
    setSentOf(null);
    setSumOpen(false);
  }, [session]);

  // The summary follows the session: what it has kept, then how a summary being made goes.
  useEffect(() => subscribe(onSummaryState((e) => sumDispatch({ type: "event", event: e }))), []);
  useEffect(() => {
    sumDispatch({ type: "session", id: session });
    if (!session) return;
    let live = true;
    summaryGet(session)
      .then((s) => live && sumDispatch({ type: "got", id: session, summary: s }))
      .catch(noop);
    // The preview's `summary=working` shows a summary under way.
    if (pvAsk("summary") === "working") void summaryMake(session).catch(noop);
    return () => {
      live = false;
    };
  }, [session]);
  const makeSummary = () => {
    if (!session) return;
    rememberScroll();
    sumDispatch({ type: "make", id: session });
    setSheet(false);
    setSentOf(null);
    setSumOpen(true);
    summaryMake(session).catch((e) => sumDispatch({ type: "failed", id: session, error: plain(e) }));
  };
  const toggleSummary = () => {
    if (!sumOpen) rememberScroll();
    setSheet(false);
    setSumOpen((o) => !o);
  };
  // The switch starts off, in a new session too: a picture goes only when the person took one and saw it waiting. It
  // is off again once nothing waits (sent with a question, or taken off).
  useEffect(() => {
    if (!v.next) setFieldPic(false);
  }, [v.empty, !!v.next]);

  // Not sent: said for a moment, then the panel hides.
  useEffect(() => {
    if (!v.hide) return;
    const id = setTimeout(() => liveClose().catch(noop), ENDED_MS);
    return () => clearTimeout(id);
  }, [v.hide, m.snap.turn]);

  // Esc always works: it makes the picture small again, closes the session list, the summary or the Sent view, stops
  // what is under way, or closes. Delete takes the picture out; M marks it up. Ctrl+K (Cmd+K) goes to the follow-up
  // field. The field keeps its own Esc.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if (e.key === "Escape") {
        e.preventDefault();
        if (big) setBig(false);
        else if (sheet) setSheet(false);
        else if (sumOpen) setSumOpen(false);
        else if (sentOf != null) setSentOf(null);
        else (v.stop ? liveStop() : liveClose()).catch(noop);
      } else if ((e.key === "Delete" || e.key === "Backspace") && (v.canDrop || (showField && !!v.next)) && !typing(e.target)) {
        e.preventDefault();
        liveDropPicture().catch(noop);
      } else if (canMark && isMarkKey(e) && !typing(e.target)) {
        e.preventDefault();
        liveMark().catch((err) => setFieldNote(plain(err)));
      } else if (showField && isFocusKey(e, platform() === "mac")) {
        e.preventDefault();
        field.current?.focus();
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [big, sheet, sumOpen, sentOf, showField, v.stop, v.canDrop, canMark, !!v.next]);

  // Nothing animates while the window is hidden.
  useEffect(() => {
    const vis = () => document.documentElement.classList.toggle("live-hidden", document.hidden);
    vis();
    document.addEventListener("visibilitychange", vis);
    return () => document.removeEventListener("visibilitychange", vis);
  }, []);

  // Follow the newest line unless the person scrolled up to read. A conversation opened complete shows its newest turn
  // with the last few earlier ones above it (all of them when they fit).
  const body = useRef<HTMLDivElement>(null);
  const now_ = useRef<HTMLElement>(null);
  const pinned = useRef(true);
  const followed = useRef(false);
  const placed = useRef(false);
  useEffect(() => {
    pinned.current = true;
    followed.current = false;
    placed.current = false;
  }, [m.snap.turn, session]);
  useLayoutEffect(() => {
    const b = body.current;
    if (!b || sheet) return;
    // An answer opened complete starts at its top; one that streamed stays at the line just written.
    if (v.phase !== "shown") followed.current = true;
    if (pinned.current && followed.current) b.scrollTop = b.scrollHeight;
    else if (!placed.current && now_.current) {
      placed.current = true;
      b.scrollTop = Math.max(0, now_.current.offsetTop - Math.round(b.clientHeight * 0.4));
    }
  }, [m.text, v.phase, v.note, v.error, sheet, earlier.length]);
  const onScroll = () => {
    const b = body.current;
    if (b) pinned.current = isPinned(b.scrollTop, b.scrollHeight, b.clientHeight);
  };

  // The summary takes the conversation's place: back to the conversation is back where the reader was, unless a point
  // sent them to a turn, which then opens and comes into view.
  const savedScroll = useRef<number | null>(null);
  const rememberScroll = () => {
    if (!sumOpen && !sheet) savedScroll.current = body.current?.scrollTop ?? null;
  };
  const wasSum = useRef(sumOpen);
  useLayoutEffect(() => {
    const b = body.current;
    const back = wasSum.current && !sumOpen;
    wasSum.current = sumOpen;
    if (!b) return;
    if (sumOpen) b.scrollTop = 0;
    else if (back && goto == null && savedScroll.current != null) b.scrollTop = savedScroll.current;
  }, [sumOpen, goto]);
  useEffect(() => {
    if (goto == null || sumOpen || sheet) return;
    const el = document.getElementById(`turn-${goto}`);
    if (!el) return setGoto(null);
    pinned.current = false;
    el.scrollIntoView({ block: "start" });
    // Focus after the press's own mouse-down, whose default focus would land where the reference was.
    const id = setTimeout(() => {
      el.querySelector<HTMLElement>("button")?.focus({ preventScroll: true });
      setGoto(null);
    }, 0);
    return () => clearTimeout(id);
  }, [goto, sumOpen, sheet]);
  const goTurn = (n: number) => {
    if (earlier.some((r) => r.n === n)) setOpened((o) => new Set(o).add(n));
    setSumOpen(false);
    setGoto(n);
  };

  // The window's height follows the content, from one row up to 60 % of the screen, and opens to a reading surface
  // once an answer shows (Tauri only; the preview lays out by CSS). The scrolling body counts at its content's height.
  const answered = !v.compact && (v.phase === "answering" || v.phase === "shown") && !v.empty;
  const root = useRef<HTMLDivElement>(null);
  const lastFit = useRef("");
  // The size the person dragged the panel to (the corner grip): its width always, its height once it shows more than
  // the listening row. Kept across launches.
  const [size, setSize] = useState(() => parsePanelSize(panelSizeRead()));
  const dragging = useRef(false);
  const width = panelWidth(size?.w, screen.availWidth);
  useEffect(() => {
    const r = root.current;
    if (!inTauri || !r) return;
    const fit = () => {
      let want = 0;
      if (dragging.current) return;
      for (const c of Array.from(r.children) as HTMLElement[]) {
        if (c.classList.contains("live-sr") || c.classList.contains("live-grip")) continue;
        want += c.classList.contains("live-body") ? ((c.firstElementChild as HTMLElement | null)?.offsetHeight ?? 0) : c.offsetHeight;
      }
      const chosen = !v.compact && size?.h != null;
      want = panelHeight(want, screen.availHeight, answered || chosen, size?.h);
      if (want > 0 && `${width}x${want}` !== lastFit.current) {
        lastFit.current = `${width}x${want}`;
        liveFit(want, width).catch(noop);
      }
    };
    const ro = new ResizeObserver(fit);
    ro.observe(r);
    for (const c of Array.from(r.querySelectorAll(".live > *, .live-inner"))) ro.observe(c);
    fit();
    return () => ro.disconnect();
  }, [v.phase, v.compact, big, !!v.thumb, !!v.label, sheet, showField, sumOpen, v.empty, answered, width, size?.h]);

  if (v.phase === "closed") return <div className="live closed" ref={root} />;

  const sr = (
    <div className="live-sr" aria-live="polite" role="status">
      {v.announce}
    </div>
  );
  const bigPic = big && v.thumb ? <BigPicture pic={v.thumb} close={() => setBig(false)} /> : null;
  // While listening, what the picture is of sits on its own quiet line under the row, centred.
  const cap = v.compact && v.thumb && v.phase !== "discarded" ? pictureCaption(v.label) : "";

  if (v.compact)
    return (
      <div className={`live compact phase-${v.phase}`} ref={root} aria-label="Prompt with your screen">
        <div className={`live-row${cap ? " capped" : ""}`}>
          {v.phase === "discarded" ? (
            <p className="live-ended live-in">{v.note}</p>
          ) : (
            <>
              <StopButton stop />
              <div className="live-row-line">
                {v.line === "voice" ? <Line mode="listen" look="breath" className="live-voice" height={VOICE_H} levels={liveLevels} /> : <LineRow v={v} />}
              </div>
              {/* Without a picture the same width stays empty on the right, so the line keeps its centre. */}
              {v.thumb ? <Picture pic={v.thumb} label={v.label} big={big} setBig={setBig} canDrop={v.canDrop} under /> : <span className="live-row-end" aria-hidden="true" />}
            </>
          )}
        </div>
        {cap ? (
          <p className="live-cap live-in">
            <span className="live-cap-text" title={cap}>
              {cap}
            </span>
            {/* Only while the picture has not left the computer; a refusal has nowhere to be said in one row. */}
            {v.canDrop ? <MarkUpControl marked={!!v.thumb?.marked} /> : null}
          </p>
        ) : v.noPicture && v.phase !== "discarded" ? (
          <NoPictureLine n={v.noPicture} />
        ) : null}
        {bigPic}
        {sr}
      </div>
    );

  const asked = v.phase === "answering" || v.phase === "shown";
  // A spoken follow-up being heard (liveView keeps the conversation for it): the listening row takes the field's place.
  const hearing = v.phase === "listening" || v.phase === "finishing";
  const toggleSent = (n: number) => setSentOf((s) => (s === n ? null : n));
  const toggleOpen = (n: number) =>
    setOpened((o) => {
      const next = new Set(o);
      if (!next.delete(n)) next.add(n);
      return next;
    });
  const conversation = (
    <>
      {earlier.length ? (
        <ol className="live-turns" aria-label="Earlier questions">
          {earlier.map((r) => (
            <EarlierTurn
              key={r.n}
              r={r}
              open={opened.has(r.n)}
              toggle={() => toggleOpen(r.n)}
              provider={v.provider}
              sent={sentOf === r.n}
              toggleSent={() => toggleSent(r.n)}
            />
          ))}
        </ol>
      ) : null}
      {hearing ? null : (
      <section className="live-now" id={`turn-${turnN}`} ref={now_} aria-label={multi ? `Question ${turnN}` : undefined}>
        {multi ? <Question key={m.snap.turn} text={v.question} now /> : null}
        {v.thumb ? (
          <div className="live-pic">
            {/* A marked picture says so: a small mark on its thumbnail, and in words beside it. */}
            <Picture pic={v.thumb} label={markedLabel(v.label, !!v.thumb.marked)} big={big} setBig={setBig} canDrop={v.canDrop} />
            {v.thumb.marked ? <MarkedBadge /> : null}
          </div>
        ) : v.noPicture ? (
          <NoPictureLine n={v.noPicture} />
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
        {asked && m.text ? <Answer text={m.text} phase={v.phase} tone={v.tone} /> : null}
        {v.complete && m.text ? <TurnFoot text={m.text} line={sentLine(v)} sent={sentOf === turnN} toggleSent={() => toggleSent(turnN)} /> : null}
        {/* Summarize stays under the answer's foot; what was sent opens below it, so it never reads as a row of that list. */}
        {v.complete && offerSummary ? (
          <div className="live-next live-in">
            <PressButton className="live-text" title="Key points and what is still open, from this session" act={makeSummary}>
              Summarize
            </PressButton>
          </div>
        ) : null}
        {sentOf === turnN && v.complete ? <SentPanel turn={turnN} /> : null}
      </section>
      )}
    </>
  );
  return (
    <div className={`live phase-${v.phase}${multi ? " multi" : ""}${v.empty ? " empty" : ""}${answered ? " answered" : ""}`} ref={root} aria-label="Prompt with your screen">
      <header className="live-head">
        {sheet ? (
          <span className="live-title">Sessions</span>
        ) : multi || sumOpen ? (
          <span className="live-title" title={v.title}>
            {v.title}
          </span>
        ) : v.empty ? (
          <span className="live-title">New session</span>
        ) : (
          <Question key={m.snap.turn} text={v.question} />
        )}
        {hasSummary && !sheet ? (
          <PressButton className={`live-sumbtn${sumOpen ? " on" : ""}`} aria-pressed={sumOpen} title={sumOpen ? "Back to the conversation" : "This session's summary"} act={toggleSummary}>
            Summary
          </PressButton>
        ) : null}
        <PressButton
          className={`live-hb${sheet ? " on" : ""}`}
          title="Sessions"
          aria-label="Sessions"
          aria-expanded={sheet}
          act={() => setSheet((s) => !s)}
        >
          <MessagesSquare size={16} strokeWidth={1.5} />
        </PressButton>
        <StopButton stop={v.stop} />
      </header>
      <div className="live-body" ref={body} onScroll={onScroll}>
        <div className="live-inner">
          {sheet ? (
            <SessionSheet
              current={session}
              title={m.snap.session.title || v.title}
              turns={turnCount}
              summary={hasSummary}
              summarize={canSummarize(turnCount) ? (hasSummary ? toggleSummary : makeSummary) : null}
              done={() => setSheet(false)}
            />
          ) : sumOpen ? (
            <SummaryPanel sum={sum} turns={turnNumbers} make={makeSummary} stop={() => void summaryStop(session).catch(noop)} back={() => setSumOpen(false)} go={goTurn} />
          ) : v.empty ? (
            <EmptySession />
          ) : (
            conversation
          )}
        </div>
      </div>
      {showField ? (
        <FollowUp field={field} first={first} pic={fieldPic} setPic={setFieldPic} next={v.next} note={fieldNote} setNote={setFieldNote} />
      ) : hearing && !sheet && !sumOpen ? (
        <footer className="live-follow live-hearing live-in" aria-label="Your follow-up">
          <div className="live-row">
            <div className="live-row-line">
              {v.line === "voice" ? <Line mode="listen" look="breath" className="live-voice" height={VOICE_H} levels={liveLevels} /> : <LineRow v={v} />}
            </div>
            {v.thumb ? <Picture pic={v.thumb} label={v.label} big={big} setBig={setBig} canDrop={v.canDrop} under /> : null}
          </div>
          {v.thumb ? (
            <p className="live-cap">
              <span className="live-cap-text">{pictureCaption(v.label)}</span>
              {v.canDrop ? <MarkUpControl marked={!!v.thumb.marked} /> : null}
            </p>
          ) : v.noPicture ? (
            <NoPictureLine n={v.noPicture} />
          ) : null}
          {bigPic}
        </footer>
      ) : null}
      {sr}
      <Grip
        drag={(on) => (dragging.current = on)}
        resize={(w, h) => liveFit(h, w).catch(noop)}
        done={(next) => {
          lastFit.current = "";
          setSize(next);
          panelSizeSave(next);
        }}
      />
    </div>
  );
}

/** The corner grip at the panel's bottom left: drag it to make the panel wider (it stays docked at the right edge) or
 * taller; the arrow keys do the same in 20 px steps, and a double click goes back to the usual size. */
function Grip({ drag, resize, done }: { drag: (on: boolean) => void; resize: (w: number, h: number) => void; done: (s: { w: number; h: number | null } | null) => void }) {
  const from = useRef<{ x: number; y: number; w: number; h: number } | null>(null);
  const at = useRef({ w: 0, h: 0 });
  const frame = useRef(0);
  const scr = () => ({ w: screen.availWidth, h: screen.availHeight });
  const move = (e: RPointerEvent) => {
    const f = from.current;
    if (!f) return;
    at.current = dragSize(f, e.screenX - f.x, e.screenY - f.y, scr());
    cancelAnimationFrame(frame.current);
    frame.current = requestAnimationFrame(() => resize(at.current.w, at.current.h));
  };
  const end = (e: RPointerEvent) => {
    if (!from.current) return;
    from.current = null;
    e.currentTarget.releasePointerCapture(e.pointerId);
    drag(false);
    done({ ...at.current });
  };
  return (
    <div
      className="live-grip"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the panel"
      title="Drag to resize · double-click for the usual size"
      tabIndex={0}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        from.current = { x: e.screenX, y: e.screenY, w: window.innerWidth, h: window.innerHeight };
        at.current = { w: window.innerWidth, h: window.innerHeight };
        drag(true);
      }}
      onPointerMove={move}
      onPointerUp={end}
      onPointerCancel={end}
      onDoubleClick={() => done(null)}
      onKeyDown={(e) => {
        const step = { ArrowLeft: [-20, 0], ArrowRight: [20, 0], ArrowUp: [0, -20], ArrowDown: [0, 20] }[e.key];
        if (!step) return;
        e.preventDefault();
        e.stopPropagation();
        done(dragSize({ w: window.innerWidth, h: window.innerHeight }, step[0], step[1], scr()));
      }}
    />
  );
}

/** The question goes without a picture of the screen: one quiet line that says why, and Allow when Pointcast only
 * needs the permission (it opens the Screen Recording pane of System Settings). */
function NoPictureLine({ n }: { n: NoPictureView }) {
  return (
    <p className="live-cap live-in" role="status">
      <span className="live-cap-text" title={n.text}>
        {n.text}
      </span>
      {n.allow ? (
        <PressButton className="live-markup" title="Open Screen Recording in System Settings" act={() => void permissionOpen("screen").catch(noop)}>
          Allow
        </PressButton>
      ) : null}
    </p>
  );
}

/** Stop while anything is under way, Close otherwise: always visible, never only on hover. Esc does the same. */
function StopButton({ stop }: { stop: boolean }) {
  return (
    <PressButton
      className={`live-hb${stop ? " stop" : ""}`}
      title={stop ? "Stop (Esc)" : "Close (Esc)"}
      aria-label={stop ? "Stop" : "Close"}
      act={() => void (stop ? liveStop() : liveClose()).catch(noop)}
    >
      {stop ? <Square size={8} fill="currentColor" strokeWidth={0} /> : <X size={16} strokeWidth={1.5} />}
    </PressButton>
  );
}

/** The question as asked: one line, the whole of it on hover or when opened. `now`: the head of the newest turn in a
 * conversation, under the session's title. */
function Question({ text, now = false }: { text: string; now?: boolean }) {
  const [open, setOpen] = useState(false);
  if (!text) return <span className="live-question" />;
  return (
    <PressButton
      className={`live-question${now ? " now" : ""}${open ? " open" : ""}`}
      title={open ? undefined : text}
      aria-expanded={open}
      act={() => setOpen((o) => !o)}
    >
      {text}
    </PressButton>
  );
}

/** An earlier turn: its question on one line; a press opens its answer in place. Sent shows on hover or focus, and in
 * the opened turn. */
function EarlierTurn({
  r,
  open,
  toggle,
  provider,
  sent,
  toggleSent,
}: {
  r: TurnRow;
  open: boolean;
  toggle: () => void;
  provider: string;
  sent: boolean;
  toggleSent: () => void;
}) {
  const line = r.note && r.note !== "No answer" ? r.note : sentLine({ thumb: r.picture, provider, sent: true });
  return (
    <li className={`live-prev${open ? " open" : ""}`} id={`turn-${r.n}`}>
      <div className="live-prev-row">
        <PressButton className="live-prev-q" aria-expanded={open} aria-label={rowLabel(r, open)} title={open ? undefined : r.question || r.line} act={toggle}>
          <span>{r.line}</span>
        </PressButton>
        {open ? null : (
          <PressButton className="live-prev-sent" aria-expanded={sent} aria-label={`What was sent with question ${r.n}`} act={toggleSent}>
            Sent
          </PressButton>
        )}
      </div>
      {open ? (
        <div className="live-prev-a">
          {r.answer ? <Answer text={r.answer} phase="shown" tone="quiet" /> : <p className="live-note">{r.error || r.note}</p>}
          <TurnFoot text={r.answer} line={line} sent={sent} toggleSent={toggleSent} />
        </div>
      ) : null}
      {sent ? <SentPanel turn={r.n} /> : null}
    </li>
  );
}

/** Under a finished answer: what left the computer, which opens to the whole of it, and Copy. */
function TurnFoot({ text, line, sent, toggleSent }: { text: string; line: string; sent: boolean; toggleSent: () => void }) {
  const [done, copy] = useCopied();
  return (
    <div className="live-foot live-in">
      {line ? (
        <PressButton className="live-sent" aria-expanded={sent} title="See exactly what was sent" act={toggleSent}>
          {line}
        </PressButton>
      ) : (
        <span className="live-sent-none" />
      )}
      {text ? (
        <PressButton className="live-copy" act={() => copy(text)} aria-label={done ? "Copied" : "Copy the answer"}>
          {done ? <Check size={14} strokeWidth={1.75} /> : <Copy size={14} strokeWidth={1.5} />}
          <span>{done ? "Copied" : "Copy"}</span>
        </PressButton>
      ) : null}
    </div>
  );
}

/** Exactly what left the computer for one turn, read from the engine's record: the words, each picture as sent with
 * its size, the earlier questions that went along, the AI and its model, and what the AI's own tool added. */
function SentPanel({ turn }: { turn: number }) {
  const [s, setS] = useState<SentView | "none" | null>(null);
  const box = useRef<HTMLDListElement>(null);
  useEffect(() => {
    let live = true;
    setS(null);
    liveSent(turn)
      .then((r) => live && setS(sentView(r, turn) ?? "none"))
      .catch(() => live && setS("none"));
    return () => {
      live = false;
    };
  }, [turn]);
  // Opened below the fold: brought up just far enough to read.
  useEffect(() => {
    if (s && s !== "none") box.current?.scrollIntoView({ block: "nearest" });
  }, [s]);
  if (s == null) return <div className="live-sentv wait" aria-busy="true" />;
  if (s === "none") return <p className="live-sentv-none live-in">Pointcast could not read what was sent with this question.</p>;
  return (
    <dl className="live-sentv live-in" ref={box} aria-label={`What was sent with question ${turn}`}>
      <div>
        <dt>Words</dt>
        <dd className={s.noWords ? "quiet" : "words"}>{s.noWords ? "None" : s.words}</dd>
      </div>
      {s.pictures.length ? (
        s.pictures.map((p) => (
          <div key={p.n} className="pic">
            <dt>{p.label}</dt>
            <dd>
              {p.src ? <img src={p.src} alt="" draggable={false} /> : null}
              <span className="quiet">{p.size}</span>
            </dd>
          </div>
        ))
      ) : (
        <div>
          <dt>Picture</dt>
          <dd className="quiet">None</dd>
        </div>
      )}
      {s.rows.map((r) => (
        <Fragment key={r.label}>
          <div>
            <dt>{r.label}</dt>
            <dd className={r.value === "None" ? "quiet" : undefined}>{r.value}</dd>
          </div>
          {r.label === "Earlier" && s.replay ? <SentAgain r={s.replay} /> : null}
        </Fragment>
      ))}
      {s.adds.length ? (
        <div>
          <dt>{s.addsBy}</dt>
          <dd>
            <ul>
              {s.adds.map((a, i) => (
                <li key={i}>{a}</li>
              ))}
            </ul>
          </dd>
        </div>
      ) : null}
    </dl>
  );
}

/** The earlier turns sent again because the conversation had to be reopened: one row that opens to their transcript as
 * it went, and their pictures, small. */
function SentAgain({ r }: { r: ReplayView }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="again">
      <dt className="live-sr">Sent again</dt>
      <dd>
        <PressButton className="live-again" aria-expanded={open} act={() => setOpen((o) => !o)}>
          <ChevronRight className="chev" size={14} strokeWidth={1.5} aria-hidden="true" />
          <span className="t">Earlier turns sent again</span>
          <span className="quiet">{r.meta}</span>
        </PressButton>
        {open && r.text ? <pre className="live-replay live-in">{r.text}</pre> : null}
        {r.pictures.length ? (
          <ul className="live-again-pics" aria-label="Their pictures, sent again">
            {r.pictures.map((p) => (
              <li key={`${p.label}-${p.n}`} title={p.size}>
                {p.src ? <img src={p.src} alt="" draggable={false} /> : null}
                <span>{p.label}</span>
              </li>
            ))}
          </ul>
        ) : null}
      </dd>
    </div>
  );
}

/** The session list, in place of the conversation: this session (its title renames in place; Summarize or its Summary,
 * Export, Delete), New, a search through every kept session, and the recent ones, a small mark on those with a
 * summary. Choosing one opens it. */
function SessionSheet({
  current,
  title,
  turns,
  summary,
  summarize,
  done,
}: {
  current: string;
  title: string;
  turns: number;
  summary: boolean;
  summarize: (() => void) | null;
  done: () => void;
}) {
  const [q, setQ] = useState("");
  const [list, setList] = useState<SessionMeta[] | null>(null);
  const [note, setNote] = useState("");
  const query = searchQuery(q);
  useEffect(() => {
    let live = true;
    const id = setTimeout(
      () =>
        (query ? sessionsSearch(query) : sessionsList())
          .then((l) => live && setList(l))
          .catch((e) => live && (setList([]), setNote(plain(e)))),
      query ? 150 : 0,
    );
    return () => {
      live = false;
      clearTimeout(id);
    };
  }, [query]);
  const rows = sessionRows(list ?? [], new Date(), { current, searched: !!query }).filter((r) => query || !r.current);
  const open = (id: string) => {
    if (id === current) return done();
    sessionsOpen(id)
      .then(done)
      .catch((e) => setNote(plain(e)));
  };
  return (
    <div className="live-sheet live-in">
      {current ? <ThisSession id={current} title={title} turns={turns} summary={summary} summarize={summarize} setNote={setNote} done={done} /> : null}
      <div className="live-sheet-bar">
        <input
          className="live-search"
          type="search"
          placeholder="Search questions"
          aria-label="Search questions"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          spellCheck={false}
        />
        <PressButton
          className="live-pill"
          act={() =>
            void sessionsNew()
              .then(done)
              .catch((e) => setNote(plain(e)))
          }
        >
          <Plus size={14} strokeWidth={1.5} />
          New
        </PressButton>
      </div>
      {note ? (
        <p className="live-note" role="alert">
          {note}
        </p>
      ) : null}
      {list == null ? null : rows.length ? (
        <ul className="live-sessions" aria-label={query ? "Sessions that match" : "Recent sessions"}>
          {rows.map((r) => (
            <li key={r.id}>
              <PressButton className="live-session" aria-label={r.label} aria-current={r.current ? "true" : undefined} act={() => open(r.id)}>
                <span className="t">
                  <span className="title">{r.title}</span>
                  {r.hit && r.hit !== r.title ? <span className="hit">{r.hit}</span> : null}
                </span>
                <span className="meta">
                  {r.summary ? <ScrollText className="live-hasum" size={12} strokeWidth={1.5} aria-hidden="true" /> : null}
                  {r.turns} · {r.day}
                </span>
              </PressButton>
            </li>
          ))}
        </ul>
      ) : (
        <p className="live-empty">{query ? "Nothing matches." : "No other sessions yet."}</p>
      )}
    </div>
  );
}

/** This session: its title, which a press turns into a field to rename it in place; its questions; Export, which
 * says where it was saved with Copy; Delete, which asks once. */
function ThisSession({
  id,
  title,
  turns,
  summary,
  summarize,
  setNote,
  done,
}: {
  id: string;
  title: string;
  turns: number;
  summary: boolean;
  summarize: (() => void) | null;
  setNote: (s: string) => void;
  done: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(title);
  const [sure, setSure] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  const [copied, copy] = useCopied();
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (editing) input.current?.select();
  }, [editing]);
  const save = () => {
    setEditing(false);
    const t = renameValue(name, title);
    if (t) sessionsRename(id, t).catch((e) => setNote(plain(e)));
  };
  return (
    <div className="live-this">
      {editing ? (
        <input
          ref={input}
          className="live-rename"
          aria-label="Session title"
          value={name}
          maxLength={120}
          onChange={(e) => setName(e.target.value)}
          onBlur={save}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              save();
            } else if (e.key === "Escape") {
              e.preventDefault();
              e.stopPropagation();
              setName(title);
              setEditing(false);
            }
          }}
        />
      ) : (
        <PressButton
          className="live-this-title"
          title="Rename"
          aria-label={`${title}. Rename`}
          act={() => {
            setName(title);
            // After the press's own mouse-down: its default focus would land beside the new field and close it at once.
            setTimeout(() => setEditing(true), 0);
          }}
        >
          {title}
        </PressButton>
      )}
      <div className="live-this-meta">
        <span>{turns ? `${turnsLabel(turns)} · This session` : "This session"}</span>
        {summarize ? (
          <PressButton className="live-text" act={summarize}>
            {summary ? "Summary" : "Summarize"}
          </PressButton>
        ) : null}
        <PressButton
          className="live-text"
          act={() =>
            void sessionsExport(id)
              .then((p) => setSaved(String(p)))
              .catch((e) => setNote(plain(e)))
          }
        >
          Export
        </PressButton>
        <PressButton
          className={`live-text${sure ? " danger" : ""}`}
          onBlur={() => setSure(false)}
          act={() => {
            if (!sure) return setSure(true);
            setSure(false);
            sessionsDelete(id)
              .then(done)
              .catch((e) => setNote(plain(e)));
          }}
        >
          {sure ? "Press again to delete" : "Delete…"}
        </PressButton>
      </div>
      {saved ? (
        <div className="live-saved live-in" role="status">
          <span className="path" title={saved}>
            Saved to {saved}
          </span>
          <PressButton className="live-icon" title="Copy the folder's path" aria-label={copied ? "Copied" : "Copy the folder's path"} act={() => copy(saved)}>
            {copied ? <Check size={14} strokeWidth={1.75} /> : <Copy size={14} strokeWidth={1.5} />}
          </PressButton>
        </div>
      ) : null}
    </div>
  );
}

/** The typed question: one slim field at the foot, shown once an answer is, in a new session for its first question,
 * and after a question failed. Enter sends, Shift+Enter is a new line, Esc lets go. The cursor is put here only by a
 * click or Ctrl+K (Cmd+K); after sending the field lets go and the engine gives focus back to the app it came from. The
 * picture switch starts off; turning it on takes a picture of the screen at that moment, which waits above the field
 * (`next`), shown, and can be marked up first; turning it off, or taking the picture off, sends none. A picture marked
 * with no question under way waits there the same way. Only a waiting picture goes with the question: none is taken as
 * it is sent (owner, 2026-11-12). Refused (a question is already under way, the picture is being marked up), the words
 * stay and one quiet line says why. */
function FollowUp({
  field,
  first,
  pic,
  setPic,
  next,
  note,
  setNote,
}: {
  field: RefObject<HTMLTextAreaElement>;
  first: boolean;
  pic: boolean;
  setPic: (f: (p: boolean) => boolean) => void;
  next: LivePicture | null;
  note: string;
  setNote: (s: string) => void;
}) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  useLayoutEffect(() => {
    const t = field.current;
    if (!t) return;
    t.style.height = "auto";
    t.style.height = `${Math.min(t.scrollHeight, 5 * 20 + 12)}px`;
  }, [text, field]);
  const send = () => {
    const t = sendText(text);
    if (!canSend(t, busy)) return;
    setBusy(true);
    setNote("");
    field.current?.blur();
    liveAskTyped(t, !!next)
      .then(() => {
        setText("");
        setPic(() => false);
      })
      .catch((e) => setNote(plain(e)))
      .finally(() => setBusy(false));
  };
  const ready = canSend(text, busy);
  // The person's own Ask chord, so the field says how to ask the follow-up by voice.
  const { cfg } = useConfig();
  const askKey = cfg?.hotkeys.ask?.label;
  const ask = fieldPlaceholder(first, askKey ? chordText(askKey) : undefined);
  return (
    <footer className="live-follow live-in">
      {next ? (
        <div className="live-nextpic live-in" title="Goes with your question">
          <span className="thumb">
            <img src={next.src} alt="" draggable={false} />
            {next.marked ? <MarkedBadge /> : null}
          </span>
          <span className="l">{next.label?.trim() || "The picture"}</span>
          <MarkUpControl marked={!!next.marked} onError={setNote} />
          <PressButton className="live-icon" title="Don't send the picture (Delete)" aria-label="Don't send the picture" act={() => void liveDropPicture().catch(noop)}>
            <X size={14} strokeWidth={1.5} />
          </PressButton>
        </div>
      ) : null}
      <div className={`live-field${text ? " has" : ""}`}>
        <textarea
          ref={field}
          rows={1}
          placeholder={ask}
          aria-label={ask}
          aria-keyshortcuts={platform() === "mac" ? "Meta+K" : "Control+K"}
          maxLength={FOLLOW_MAX}
          value={text}
          spellCheck
          onChange={(e) => {
            setText(e.target.value);
            if (note) setNote("");
          }}
          onKeyDown={(e) => {
            const act = followKey(
              { key: e.key, shiftKey: e.shiftKey, ctrlKey: e.ctrlKey, metaKey: e.metaKey, altKey: e.altKey, isComposing: e.nativeEvent.isComposing },
              text,
              busy,
            );
            if (act === "send") {
              e.preventDefault();
              send();
            } else if (act === "hold") e.preventDefault();
            else if (act === "leave") {
              e.preventDefault();
              e.stopPropagation();
              field.current?.blur();
            }
          }}
        />
        {next ? null : (
          <>
            {pic ? <MarkUpControl className="in-field" onError={setNote} /> : null}
            <PressButton
              className="live-pic-toggle"
              aria-pressed={pic}
              aria-label="Include a new picture of the screen"
              title={pic ? "A new picture of the screen goes with it" : "Words only. Press to take a picture of the screen now"}
              act={() => {
                setNote("");
                if (pic) {
                  setPic(() => false);
                  void liveDropPicture().catch(noop);
                } else
                  liveTakePicture()
                    .then(() => setPic(() => true))
                    .catch((e) => setNote(plain(e)));
              }}
            >
              <ImagePlus size={16} strokeWidth={1.5} />
            </PressButton>
          </>
        )}
        <PressButton className="live-send" aria-label="Send" title="Send (Enter)" disabled={!ready} act={send}>
          <ArrowUp size={14} strokeWidth={1.75} />
        </PressButton>
      </div>
      {note ? (
        <p className="live-follow-err" role="status">
          {note}
        </p>
      ) : null}
    </footer>
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
          <PressButton className="live-drop" title="Don't send the picture (Delete)" aria-label="Don't send the picture" act={() => void liveDropPicture().catch(noop)}>
            <X size={11} strokeWidth={2} />
          </PressButton>
        ) : null}
      </div>
      {label.trim() && !under ? (
        <figcaption className="live-label" title={label}>
          {label.trim()}
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

/** Finishing and thinking: the line looping while the words are finished or the AI is asked. */
function LineRow({ v }: { v: LiveView }) {
  return <Line mode="think" className="live-line live-in" tone={v.tone} height={ROW} loops={v.compact ? 5 : 7} />;
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
        <PressButton className="btn" act={act}>
          {err.label}
        </PressButton>
      ) : null}
    </div>
  );
}

/** Answering: the markdown, with the line down its margin turning under the line being written; its end is the caret.
 * Shown: the caret fades and the line rests as a short quiet stroke. An earlier turn opens shown, at rest. */
function Answer({ text: arrived, phase, tone }: { text: string; phase: Phase; tone: Tone }) {
  const streaming = phase === "answering";
  // What arrived is the truth; what shows follows it evenly, a little behind, and is all of it once the answer is done.
  const text = useReveal(arrived, streaming);
  // The blocks before the last settled break are parsed and drawn once; only the tail being written is read each frame.
  const k = streaming ? settledSplit(text) : 0;
  const head = text.slice(0, k);
  const headNodes = useMemo(() => (head ? renderMarkdown(parseMarkdown(head), h, { code: (b, key) => <Code key={key} b={b} tail={null} /> }) : []), [head]);
  const rest = text.slice(k);
  const blocks = useMemo(() => parseMarkdown(streaming ? settleTail(rest) : rest), [rest, streaming]);
  const box = useRef<HTMLDivElement>(null);
  const end = useRef<HTMLSpanElement>(null);
  const margin = useRef<SVGPathElement>(null);
  const caret = useRef<SVGCircleElement>(null);
  const tail = <span className="md-end" ref={end} key="md-end" />;
  const tailNodes = renderMarkdown(blocks, h, { tail, code: (b, key, t) => <Code key={key} b={b} tail={t} /> });
  const nodes = headNodes.length ? [<Fragment key="settled">{headNodes}</Fragment>, <Fragment key="tail">{tailNodes}</Fragment>] : tailNodes;

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
        p.style.strokeDashoffset = String(phase === "shown" ? Math.max(0, len - REST) : 0);
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
    <div className={`live-answer md ${phase}`} ref={box}>
      <svg className={`ln ${tone}`} aria-hidden="true">
        <path className="margin" ref={margin} />
        <circle className="caret" ref={caret} r={2.5} />
      </svg>
      {headNodes.length || tailNodes.length ? nodes : <p>{tail}</p>}
    </div>
  );
}

/** The streamed text as it shows: whatever is there when the answer first appears shows at once; what arrives after is
 * revealed a few characters a frame (revealStep), so the AI's uneven chunks read as an even flow. Done, or with less
 * motion asked for, it is the whole text as it comes. */
function useReveal(text: string, streaming: boolean): string {
  const reduced = useMemo(() => typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches, []);
  const shown = useRef(text.length);
  // When each length of the text arrived, for how long the oldest character not yet shown has waited.
  const arrivals = useRef<{ len: number; at: number }[]>([]);
  const lastLen = useRef(text.length);
  const [, paint] = useState(0);
  useEffect(() => {
    if (text.length < lastLen.current) {
      shown.current = Math.min(shown.current, text.length);
      arrivals.current = [];
    } else if (text.length > lastLen.current) arrivals.current.push({ len: text.length, at: performance.now() });
    lastLen.current = text.length;
    if (!streaming || reduced) {
      shown.current = text.length;
      arrivals.current = [];
      paint((x) => x + 1);
      return;
    }
    if (shown.current >= text.length) return;
    let raf = 0;
    let last = performance.now();
    const frame = (t: number) => {
      const dt = Math.max(0, t - last);
      last = t;
      const at = Math.floor(shown.current);
      const q = arrivals.current;
      while (q.length && q[0].len <= at) q.shift();
      const was = at;
      shown.current = revealStep({ shown: shown.current, total: text.length, dt, lag: q.length ? t - q[0].at : 0 });
      if (Math.floor(shown.current) !== was) paint((x) => x + 1);
      if (shown.current < text.length) raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  }, [text, streaming, reduced]);
  return streaming && !reduced ? revealSlice(text, shown.current) : text;
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
      <PressButton className="md-copy" title="Copy code" aria-label={done ? "Copied" : "Copy code"} act={() => copy(b.v)}>
        {done ? <Check size={14} strokeWidth={1.75} /> : <Copy size={14} strokeWidth={1.5} />}
      </PressButton>
    </div>
  );
}

/** A new session, open on nothing yet: one quiet line with the Ask key; the field below is ready for a typed one. */
function EmptySession() {
  const { cfg } = useConfig();
  const key = cfg?.hotkeys.ask?.label;
  return (
    <div className="live-blank live-in">
      <span>Prompt with your screen</span>
      {key ? <Keycaps label={key} /> : null}
    </div>
  );
}

/** The session's summary, in place of the conversation: its title, the key points and what is still open, each with
 * the words it rests on (marked as yours or the AI's by one small word) and its turn, which a press goes back to; one
 * quiet line with what was kept and dropped and the AI that made it; Copy as text, Redo, Back. While it is made, the
 * line loops in the accent with Stop. */
function SummaryPanel({
  sum,
  turns,
  make,
  stop,
  back,
  go,
}: {
  sum: SummaryModel;
  turns: number[];
  make: () => void;
  stop: () => void;
  back: () => void;
  go: (n: number) => void;
}) {
  const [copied, copy] = useCopied();
  const working = sum.state === "working";
  const v = sum.summary && !working ? summaryView(sum.summary, sum.counts, turns) : null;
  return (
    <section className="live-sum" aria-label="Summary" aria-busy={working}>
      {working ? (
        <div className="live-think live-sum-work live-in">
          <Line mode="think" className="live-line" tone="accent" height={ROW} loops={7} />
          <span className="live-provider">Summarizing</span>
          <PressButton className="live-text" act={stop}>
            Stop
          </PressButton>
        </div>
      ) : null}
      {sum.state === "error" ? (
        <div className="live-err live-in">
          <svg className="ln warn" width={16} height={20} viewBox="0 0 16 20" aria-hidden="true">
            <path d={restPath(0.75, 10, 12)} />
          </svg>
          <span className="live-err-text">{sum.error}</span>
          <PressButton className="btn" act={make}>
            Try again
          </PressButton>
        </div>
      ) : null}
      {sum.state === "stopped" ? <p className="live-note live-in">{v ? "Stopped. This is the summary from before." : "Stopped"}</p> : null}
      {v ? (
        <div className="live-in">
          <h2 className="live-sum-title">{v.title}</h2>
          {v.groups.map((g) => (
            <section key={g.id} className="live-sum-group" aria-labelledby={`sum-${g.id}`}>
              <h3 id={`sum-${g.id}`}>{g.label}</h3>
              <ul>
                {g.items.map((p) => (
                  <li key={p.key} className="live-point">
                    <span className="t">{p.text}</span>
                    {p.ref ? (
                      p.go ? (
                        <PressButton className="live-ref" title={`Go to question ${p.turn}`} aria-label={`Go to question ${p.turn}`} act={() => go(p.turn)}>
                          {p.ref}
                        </PressButton>
                      ) : (
                        <span className="live-ref off" title={`Question ${p.turn}`}>
                          {p.ref}
                        </span>
                      )
                    ) : null}
                    {p.quote ? (
                      <span className="q">
                        <span className="by" title={p.by === "You" ? "Your words" : "The AI's words"}>
                          {p.by}
                        </span>
                        <span className="qt">“{p.quote}”</span>
                      </span>
                    ) : null}
                  </li>
                ))}
              </ul>
            </section>
          ))}
          {v.foot ? <p className="live-sum-foot">{v.foot}</p> : null}
        </div>
      ) : null}
      {!working && !v && sum.state !== "error" && sum.state !== "stopped" ? <p className="live-empty">No summary yet.</p> : null}
      <div className="live-sum-acts">
        {v ? (
          <PressButton className="live-text" act={() => copy(summaryText(v))} aria-label={copied ? "Copied" : "Copy the summary as text"}>
            {copied ? "Copied" : "Copy as text"}
          </PressButton>
        ) : null}
        {working ? null : (
          <PressButton className="live-text" act={make}>
            {sum.summary ? "Redo" : "Summarize"}
          </PressButton>
        )}
        <PressButton className="live-text back" act={back}>
          Back to the conversation
        </PressButton>
      </div>
    </section>
  );
}
