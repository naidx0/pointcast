import { useEffect, useState } from "react";
import { Check } from "lucide-react";
import {
  inTauri,
  onSpeechDone,
  onSpeechProgress,
  speechCancel,
  speechRemove,
  speechSetup,
  speechStatus,
  subscribe,
  type Config,
  type SpeechModel,
  type SpeechProgress,
} from "@/lib/api";

/** How each model is offered: plain names, what it is best at, and what it costs. */
const OFFER: Record<string, { name: string; tag: string; blurb: string; order: number }> = {
  "moonshine-base-en": { name: "Moonshine Base", tag: "Recommended", blurb: "Fast and accurate. English.", order: 0 },
  "moonshine-tiny-en": { name: "Moonshine Tiny", tag: "Fastest", blurb: "Smallest and quickest. English.", order: 1 },
  "parakeet-tdt-v2-en": {
    name: "Parakeet v2",
    tag: "Most accurate",
    blurb: "NVIDIA's best English model, with punctuation.",
    order: 2,
  },
  "whisper-turbo": {
    name: "Whisper Turbo",
    tag: "Any language",
    blurb: "OpenAI's large-v3-turbo. 99 languages. Slower on older computers.",
    order: 3,
  },
  "parakeet-tdt-v3": {
    name: "Parakeet v3",
    tag: "European languages",
    blurb: "25 languages, detected automatically. Fast.",
    order: 4,
  },
};

const offer = (m: SpeechModel) =>
  OFFER[m.id] ?? { name: m.name.replace(/ v2.*$/, ""), tag: "", blurb: "", order: 9 };
const mbText = (b: number) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${Math.round(b / 1e6)} MB`);

export function chosenModel(cfg: Config) {
  return cfg.model === "auto" ? cfg.recommended : cfg.model;
}

export function speechReady(cfg: Config) {
  const id = chosenModel(cfg);
  return !!cfg.speech.runtime?.installed && !!cfg.speech.models?.some((m) => m.id === id && m.installed);
}

/** The one setup job (engine + model) as every window sees it, including one started elsewhere. */
export function useSpeechJob(refresh: () => void) {
  const [job, setJob] = useState<(SpeechProgress & { id?: string }) | null>(null);
  const [error, setError] = useState<{ id: string; text: string } | null>(null);

  useEffect(() => {
    if (inTauri) speechStatus().then((p) => p && setJob(p)).catch(() => undefined);
    return subscribe(
      onSpeechProgress((p) => {
        setError(null);
        setJob((j) => ({ ...p, id: j?.id }));
      }),
      onSpeechDone((d) => {
        setJob(null);
        if (!d.ok) setError({ id: d.id, text: d.error || "Setup did not finish. Try again." });
        refresh();
      }),
    );
  }, [refresh]);

  async function start(id: string, cfg: Config, patch: (p: Record<string, unknown>) => Promise<string | null>) {
    setError(null);
    setJob({ phase: "download", received: 0, total: 0, id });
    if (!inTauri) return simulate(id, cfg, patch, setJob);
    try {
      await speechSetup(id);
    } catch (e) {
      setJob(null);
      setError({ id, text: String(e) });
    }
  }

  return { job, error, start, cancel: () => (inTauri ? speechCancel() : setJob(null)) };
}

/** Pick an on-device model. Each card downloads its own model; the bar fills inside that card. */
export function ModelPicker({
  cfg,
  patch,
  refresh,
  removable,
}: {
  cfg: Config;
  patch: (p: Record<string, unknown>) => Promise<string | null>;
  refresh: () => void;
  /** Offer Remove on downloaded models that are not in use (Settings, not first-run setup). */
  removable?: boolean;
}) {
  const { job, error, start, cancel } = useSpeechJob(refresh);
  const chosen = chosenModel(cfg);
  const engine = cfg.speech.runtime?.installed ?? false;
  const models = withPreview(cfg.speech.models ?? []).sort((a, b) => offer(a).order - offer(b).order);
  const busyId = job ? (job.id ?? chosen) : null;

  return (
    <div className="cards">
      {models.map((m) => {
        const o = offer(m);
        const on = m.id === chosen;
        const ready = m.installed && engine;
        const mine = busyId === m.id;
        const pct = mine && job?.phase === "download" && job.total ? Math.min(100, ((job.received ?? 0) / job.total) * 100) : null;
        const err = error?.id === m.id && error.text !== "Cancelled." ? error.text : "";
        return (
          <div
            key={m.id}
            className="card model"
            role="radio"
            aria-checked={on}
            tabIndex={0}
            onClick={() => !job && patch({ model: m.id })}
            onKeyDown={(e) => (e.key === " " || e.key === "Enter") && !job && patch({ model: m.id })}
          >
            <span className="radio" />
            <div className="grow">
              <div className="ttl">
                {o.name}
                {o.tag ? <span className={`badge ${o.tag === "Recommended" ? "rec" : ""}`}>{o.tag}</span> : null}
              </div>
              <div className="sub">
                {o.blurb} · {mbText(m.bytes)}
              </div>
              {mine ? (
                <div className="model-progress">
                  <div className={`progress ${pct == null ? "indet" : ""}`} role="progressbar" aria-valuenow={pct ?? undefined}>
                    <i style={pct == null ? undefined : { width: `${pct}%` }} />
                  </div>
                  <span className="t-small">
                    {job?.phase === "check"
                      ? "Checking it works…"
                      : job?.phase === "unpack"
                        ? "Unpacking…"
                        : job?.total
                          ? `${mbText(job.received ?? 0)} of ${mbText(job.total)}`
                          : "Starting…"}
                  </span>
                </div>
              ) : err ? (
                <div className="t-small" role="alert" style={{ color: "var(--red-ink)", marginTop: 4 }}>
                  {err.replace(/^Error:\s*/, "")} Download picks up where it stopped.
                </div>
              ) : null}
            </div>
            {mine ? (
              job?.phase === "download" ? (
                <button type="button" className="btn" onClick={(e) => (e.stopPropagation(), cancel())}>
                  Cancel
                </button>
              ) : (
                <span className="spinner" />
              )
            ) : ready ? (
              <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                {removable && !on ? (
                  <button
                    type="button"
                    className="btn plain"
                    title={`Delete ${o.name} and free ${mbText(m.bytes)}`}
                    disabled={!!job}
                    onClick={async (e) => {
                      e.stopPropagation();
                      if (inTauri) await speechRemove(m.id).catch(() => undefined);
                      refresh();
                    }}
                  >
                    Remove
                  </button>
                ) : null}
                <span className="check on" aria-label="Downloaded">
                  <Check size={14} strokeWidth={3} />
                </span>
              </span>
            ) : (
              <button
                type="button"
                className="btn primary"
                disabled={!!job}
                onClick={(e) => {
                  e.stopPropagation();
                  patch({ model: m.id });
                  start(m.id, cfg, patch);
                }}
              >
                {err ? "Try Again" : "Download"}
              </button>
            )}
          </div>
        );
      })}
      <p className="t-small" style={{ margin: "4px 2px 0" }}>
        {speechReady(cfg)
          ? "Ready and tested on this computer. Nothing you say leaves it."
          : removable
            ? "Models download once and run offline on this computer."
            : "Models download once and run offline on this computer. You can switch any time in Settings."}
      </p>
    </div>
  );
}

/** The browser stand-in lists two models; show the whole lineup there too. */
function withPreview(list: SpeechModel[]): SpeechModel[] {
  if (inTauri) return [...list];
  const sizes: Record<string, number> = {
    "parakeet-tdt-v2-en": 482_468_385,
    "whisper-turbo": 563_790_207,
    "parakeet-tdt-v3": 487_170_055,
  };
  const extra = Object.keys(sizes)
    .filter((id) => !list.some((m) => m.id === id))
    .map((id) => ({ id, name: OFFER[id].name, installed: false, bytes: sizes[id] }));
  return [...list.map((m) => (m.bytes ? m : { ...m, bytes: 111_266_225 })), ...extra];
}

/** Browser preview: walk the same states without a backend. */
function simulate(
  id: string,
  cfg: Config,
  patch: (p: Record<string, unknown>) => Promise<string | null>,
  setJob: (p: (SpeechProgress & { id?: string }) | null) => void,
) {
  const total = 120_000_000;
  let got = 0;
  const t = setInterval(() => {
    got += 9_000_000;
    if (got < total) return setJob({ phase: "download", received: got, total, id });
    clearInterval(t);
    setJob({ phase: "check", id });
    setTimeout(() => {
      setJob(null);
      patch({
        model: id,
        speech: {
          ...cfg.speech,
          runtime: { ...(cfg.speech.runtime ?? { id: "runtime", lib: "" }), installed: true },
          models: withPreview(cfg.speech.models ?? []).map((m) => (m.id === id ? { ...m, installed: true } : m)),
        },
      });
    }, 700);
  }, 120);
}
