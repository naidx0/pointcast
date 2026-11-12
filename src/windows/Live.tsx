import { createElement, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState, type ReactNode, type RefObject } from "react";
import { ArrowUp, Check, Copy, ImagePlus, MessagesSquare, Plus, Square, X } from "lucide-react";
import {
  inTauri,
  liveAskTyped,
  liveClose,
  liveDropPicture,
  liveFit,
  liveOpenSettings,
  liveRetry,
  liveSent,
  liveState,
  liveStop,
  onListAsk,
  onLiveDelta,
  onLiveLevel,
  onLiveState,
  preview,
  sessionsDelete,
  sessionsExport,
  sessionsList,
  sessionsNew,
  sessionsOpen,
  sessionsRename,
  sessionsSearch,
  subscribe,
  takeListAsk,
} from "@/lib/api";
import {
  ENDED_MS,
  currentTurn,
  initialLive,
  isPinned,
  liveReduce,
  liveView,
  pictureCaption,
  sentLine,
  type LiveDelta,
  type LivePicture,
  type LiveView,
  type Phase,
  type Tone,
} from "@/lib/live";
import { FOLLOW_MAX, canSend, earlierRows, followKey, followUpShown, isFocusKey, rowLabel, sendText, sentView, type SentView, type TurnRow } from "@/lib/conversation";
import { renameValue, searchQuery, sessionRows, turnsLabel, type SessionMeta } from "@/lib/sessions";
import { marginPath, restPath } from "@/lib/line";
import { platform } from "@/lib/theme";
import { Line, type LevelSource } from "@/components/Line";
import { parseMarkdown, renderMarkdown, settleTail, type CodeBlock, type H } from "@/lib/markdown";
import { PressButton } from "@/components/PressButton";

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
 * Sent view, `expand=<turn>` an earlier turn's answer. */
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
  const turnN = currentTurn(m.snap)?.n ?? m.snap.turn;
  const multi = earlier.length > 0;
  const showField = followUpShown(v) && !sheet;
  const session = m.snap.session.id;

  useEffect(() => setBig(false), [m.snap.turn, v.thumb == null, v.compact]);
  // Another session: nothing of the last one stays open. The first id to arrive is the same session, not another.
  const lastSession = useRef("");
  useEffect(() => {
    const was = lastSession.current;
    lastSession.current = session || was;
    if (!was || !session || was === session) return;
    setOpened(new Set());
    setSentOf(null);
  }, [session]);

  // Not sent: said for a moment, then the panel hides.
  useEffect(() => {
    if (!v.hide) return;
    const id = setTimeout(() => liveClose().catch(noop), ENDED_MS);
    return () => clearTimeout(id);
  }, [v.hide, m.snap.turn]);

  // Esc always works: it makes the picture small again, closes the session list or the Sent view, stops what is under
  // way, or closes. Delete takes the picture out. Ctrl+K (Cmd+K) goes to the follow-up field. The field keeps its own Esc.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if (e.key === "Escape") {
        e.preventDefault();
        if (big) setBig(false);
        else if (sheet) setSheet(false);
        else if (sentOf != null) setSentOf(null);
        else (v.stop ? liveStop() : liveClose()).catch(noop);
      } else if ((e.key === "Delete" || e.key === "Backspace") && v.canDrop && !typing(e.target)) {
        e.preventDefault();
        liveDropPicture().catch(noop);
      } else if (showField && isFocusKey(e, platform() === "mac")) {
        e.preventDefault();
        field.current?.focus();
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [big, sheet, sentOf, showField, v.stop, v.canDrop]);

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
  }, [v.phase, v.compact, big, !!v.thumb, !!v.label, sheet, showField]);

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
      <div className={`live compact phase-${v.phase}`} ref={root} aria-label="Ask about the screen">
        <div className={`live-row${cap ? " capped" : ""}`}>
          {v.phase === "discarded" ? (
            <p className="live-ended live-in">{v.note}</p>
          ) : (
            <>
              <StopButton stop />
              <div className="live-row-line">
                {v.line === "voice" ? <Line mode="listen" className="live-voice" height={VOICE_H} levels={liveLevels} /> : <LineRow v={v} />}
              </div>
              {/* Without a picture the same width stays empty on the right, so the line keeps its centre. */}
              {v.thumb ? <Picture pic={v.thumb} label={v.label} big={big} setBig={setBig} canDrop={v.canDrop} under /> : <span className="live-row-end" aria-hidden="true" />}
            </>
          )}
        </div>
        {cap ? (
          <p className="live-cap live-in" title={cap}>
            {cap}
          </p>
        ) : null}
        {bigPic}
        {sr}
      </div>
    );

  const asked = v.phase === "answering" || v.phase === "shown";
  const toggleSent = (n: number) => setSentOf((s) => (s === n ? null : n));
  const toggleOpen = (n: number) =>
    setOpened((o) => {
      const next = new Set(o);
      if (!next.delete(n)) next.add(n);
      return next;
    });
  return (
    <div className={`live phase-${v.phase}${multi ? " multi" : ""}`} ref={root} aria-label="Ask about the screen">
      <header className="live-head">
        {sheet ? (
          <span className="live-title">Sessions</span>
        ) : multi ? (
          <span className="live-title" title={v.title}>
            {v.title}
          </span>
        ) : (
          <Question key={m.snap.turn} text={v.question} />
        )}
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
            <SessionSheet current={session} title={m.snap.session.title || v.title} turns={m.snap.session.turns} done={() => setSheet(false)} />
          ) : (
            <>
              {multi ? (
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
              <section className="live-now" ref={now_} aria-label={multi ? `Question ${turnN}` : undefined}>
                {multi ? <Question key={m.snap.turn} text={v.question} now /> : null}
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
                {asked && m.text ? <Answer text={m.text} phase={v.phase} tone={v.tone} /> : null}
                {v.complete && m.text ? (
                  <TurnFoot text={m.text} line={sentLine(v)} sent={sentOf === turnN} toggleSent={() => toggleSent(turnN)} />
                ) : null}
                {sentOf === turnN && v.complete ? <SentPanel turn={turnN} /> : null}
              </section>
            </>
          )}
        </div>
      </div>
      {showField ? <FollowUp field={field} /> : null}
      {sr}
    </div>
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
    <li className={`live-prev${open ? " open" : ""}`}>
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
      .then((r) => live && setS(sentView(r) ?? "none"))
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
        <div key={r.label}>
          <dt>{r.label}</dt>
          <dd className={r.value === "None" ? "quiet" : undefined}>{r.value}</dd>
        </div>
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

/** The session list, in place of the conversation: this session (its title renames in place; Export, Delete), New,
 * a search through every kept session, and the recent ones. Choosing one opens it. */
function SessionSheet({ current, title, turns, done }: { current: string; title: string; turns: number; done: () => void }) {
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
      {current ? <ThisSession id={current} title={title} turns={turns} setNote={setNote} done={done} /> : null}
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
function ThisSession({ id, title, turns, setNote, done }: { id: string; title: string; turns: number; setNote: (s: string) => void; done: () => void }) {
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

/** The typed follow-up: one slim field at the foot, shown once an answer is. Enter sends, Shift+Enter is a new line,
 * Esc lets go. The cursor is put here only by a click or Ctrl+K (Cmd+K); after sending the field lets go and the
 * engine gives focus back to the app it came from. A fresh picture goes along only when asked for. */
function FollowUp({ field }: { field: RefObject<HTMLTextAreaElement> }) {
  const [text, setText] = useState("");
  const [pic, setPic] = useState(false);
  const [err, setErr] = useState("");
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
    setErr("");
    field.current?.blur();
    liveAskTyped(t, pic)
      .then(() => {
        setText("");
        setPic(false);
      })
      .catch((e) => setErr(plain(e)))
      .finally(() => setBusy(false));
  };
  const ready = canSend(text, busy);
  return (
    <footer className="live-follow live-in">
      <div className={`live-field${text ? " has" : ""}`}>
        <textarea
          ref={field}
          rows={1}
          placeholder="Ask a follow-up"
          aria-label="Ask a follow-up"
          aria-keyshortcuts={platform() === "mac" ? "Meta+K" : "Control+K"}
          maxLength={FOLLOW_MAX}
          value={text}
          spellCheck
          onChange={(e) => {
            setText(e.target.value);
            if (err) setErr("");
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
        <PressButton
          className="live-pic-toggle"
          aria-pressed={pic}
          aria-label="Include a new picture of the screen"
          title={pic ? "A new picture of the screen goes with it" : "Words only. Press to include a new picture of the screen"}
          act={() => setPic((p) => !p)}
        >
          <ImagePlus size={16} strokeWidth={1.5} />
        </PressButton>
        <PressButton className="live-send" aria-label="Send" title="Send (Enter)" disabled={!ready} act={send}>
          <ArrowUp size={14} strokeWidth={1.75} />
        </PressButton>
      </div>
      {err ? (
        <p className="live-follow-err" role="alert">
          {err}
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
function Answer({ text, phase, tone }: { text: string; phase: Phase; tone: Tone }) {
  const streaming = phase === "answering";
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
      <PressButton className="md-copy" title="Copy code" aria-label={done ? "Copied" : "Copy code"} act={() => copy(b.v)}>
        {done ? <Check size={14} strokeWidth={1.75} /> : <Copy size={14} strokeWidth={1.5} />}
      </PressButton>
    </div>
  );
}
