import { useEffect, useState } from "react";
import { Check } from "lucide-react";
import {
  inTauri,
  MODEL_INFO,
  onSpeechDone,
  onSpeechProgress,
  speechCancel,
  speechSetup,
  speechStatus,
  subscribe,
  type Config,
  type SpeechProgress,
} from "@/lib/api";

export function chosenModel(cfg: Config) {
  return cfg.model === "auto" ? cfg.recommended : cfg.model;
}

export function speechReady(cfg: Config) {
  const id = chosenModel(cfg);
  return !!cfg.speech.runtime?.installed && !!cfg.speech.models?.some((m) => m.id === id && m.installed);
}

const shortName = (name: string) => name.replace(/ v2.*$/, "");
const mbText = (b: number) => `${Math.round(b / 1_000_000)} MB`;

/** The one setup job (engine + model) as every window sees it, including one started elsewhere. */
export function useSpeechJob(refresh: () => void) {
  const [job, setJob] = useState<SpeechProgress | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    if (inTauri) speechStatus().then((p) => p && setJob(p)).catch(() => undefined);
    return subscribe(
      onSpeechProgress((p) => {
        setError("");
        setJob(p);
      }),
      onSpeechDone((d) => {
        setJob(null);
        if (!d.ok) setError(d.error || "Setup did not finish. Try again.");
        refresh();
      }),
    );
  }, [refresh]);

  async function start(id: string, cfg: Config, patch: (p: Record<string, unknown>) => Promise<string | null>) {
    setError("");
    setJob({ phase: "download", received: 0, total: 0 });
    if (!inTauri) return simulate(id, cfg, patch, setJob);
    try {
      await speechSetup(id);
    } catch (e) {
      setJob(null);
      setError(String(e));
    }
  }

  return { job, error, start, cancel: () => (inTauri ? speechCancel() : setJob(null)) };
}

/** Pick an on-device model and install it: one button, one progress bar, checked before "ready". */
export function ModelPicker({
  cfg,
  patch,
  refresh,
}: {
  cfg: Config;
  patch: (p: Record<string, unknown>) => Promise<string | null>;
  refresh: () => void;
}) {
  const { job, error, start, cancel } = useSpeechJob(refresh);
  const chosen = chosenModel(cfg);
  const models = [...(cfg.speech.models ?? [])].sort((a, b) =>
    a.id === cfg.recommended ? -1 : b.id === cfg.recommended ? 1 : 0,
  );
  const ready = speechReady(cfg);
  const pick = models.find((m) => m.id === chosen);
  const pct = job?.phase === "download" && job.total ? Math.min(100, ((job.received ?? 0) / job.total) * 100) : null;
  const cancelled = error === "Cancelled.";

  return (
    <div className="cards">
      {models.map((m) => {
        const info = MODEL_INFO[m.id];
        const on = m.id === chosen;
        return (
          <button
            key={m.id}
            type="button"
            className="card"
            aria-pressed={on}
            disabled={!!job}
            onClick={() => patch({ model: m.id })}
          >
            <span className="radio" />
            <div className="grow">
              <div className="ttl">
                {shortName(m.name)}
                {m.id === cfg.recommended ? (
                  <span className="badge" style={{ marginLeft: 8, height: 18, verticalAlign: 1 }}>
                    Recommended
                  </span>
                ) : null}
              </div>
              <div className="sub">
                {info?.blurb ?? "English, on this computer."} {info ? `· ${info.size}` : ""}
              </div>
            </div>
            {m.installed ? (
              <span className="check on" aria-label="Downloaded">
                <Check size={14} strokeWidth={3} />
              </span>
            ) : null}
          </button>
        );
      })}
      <div style={{ minHeight: 52, display: "flex", flexDirection: "column", justifyContent: "center", gap: 8 }}>
        {job ? (
          <>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
              <span className="t-caption">
                {job.phase === "check"
                  ? "Checking it works on this computer…"
                  : job.phase === "unpack"
                    ? "Unpacking…"
                    : job.total
                      ? `Downloading ${pick ? shortName(pick.name) : "the model"} · ${mbText(job.received ?? 0)} of ${mbText(job.total)}`
                      : "Starting…"}
              </span>
              {job.phase === "download" ? (
                <button type="button" className="btn plain" onClick={() => cancel()}>
                  Cancel
                </button>
              ) : null}
            </div>
            <div className={`progress ${pct == null ? "indet" : ""}`} role="progressbar" aria-valuenow={pct ?? undefined}>
              <i style={pct == null ? undefined : { width: `${pct}%` }} />
            </div>
          </>
        ) : ready ? (
          <span className="t-caption" style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <span className="check on" style={{ width: 16, height: 16 }}>
              <Check size={10} strokeWidth={3.5} />
            </span>
            Ready and tested on this computer. Nothing you say leaves it.
          </span>
        ) : (
          <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
            <button type="button" className="btn primary" onClick={() => start(chosen, cfg, patch)}>
              {error && !cancelled ? "Try Again" : `Download ${pick ? shortName(pick.name) : "model"}`}
            </button>
            <span className="t-small">
              {error && !cancelled ? "It picks up where it stopped." : "Downloads once. Works offline after that."}
            </span>
          </div>
        )}
        {error && !cancelled ? (
          <span className="t-caption" role="alert" style={{ color: "var(--red-ink)" }}>
            {error.replace(/^Error:\s*/, "")}
          </span>
        ) : null}
      </div>
    </div>
  );
}

/** Browser preview: walk the same states without a backend. */
function simulate(
  id: string,
  cfg: Config,
  patch: (p: Record<string, unknown>) => Promise<string | null>,
  setJob: (p: SpeechProgress | null) => void,
) {
  const total = 120_000_000;
  let got = 0;
  const t = setInterval(() => {
    got += 9_000_000;
    if (got < total) return setJob({ phase: "download", received: got, total });
    clearInterval(t);
    setJob({ phase: "check" });
    setTimeout(() => {
      setJob(null);
      patch({
        speech: {
          ...cfg.speech,
          runtime: { ...(cfg.speech.runtime ?? { id: "runtime", lib: "" }), installed: true },
          models: cfg.speech.models?.map((m) => (m.id === id ? { ...m, installed: true } : m)),
        },
      });
    }, 700);
  }, 120);
}
