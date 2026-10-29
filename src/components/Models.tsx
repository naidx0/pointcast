import { useEffect, useRef, useState } from "react";
import { Check } from "lucide-react";
import {
  downloadModel,
  inTauri,
  MODEL_INFO,
  onSpeechDone,
  onSpeechProgress,
  subscribe,
  type Config,
} from "@/lib/api";

export function chosenModel(cfg: Config) {
  return cfg.model === "auto" ? cfg.recommended : cfg.model;
}

export function speechReady(cfg: Config) {
  const id = chosenModel(cfg);
  return !!cfg.speech.runtime?.installed && !!cfg.speech.models?.some((m) => m.id === id && m.installed);
}

/** Pick an on-device model and download it (runtime first, if missing) with live progress. */
export function ModelPicker({
  cfg,
  patch,
  refresh,
}: {
  cfg: Config;
  patch: (p: Record<string, unknown>) => Promise<string | null>;
  refresh: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [pct, setPct] = useState<number | null>(null);
  const [step, setStep] = useState("");
  const [err, setErr] = useState("");
  const chosen = chosenModel(cfg);
  const models = [...(cfg.speech.models ?? [])].sort((a, b) =>
    a.id === cfg.recommended ? -1 : b.id === cfg.recommended ? 1 : 0,
  );
  const ready = speechReady(cfg);
  const chosenRef = useRefValue(chosen);

  useEffect(() => {
    return subscribe(
      onSpeechProgress((p) => {
        if (p.state === "downloading" && p.total) {
          setPct(Math.min(99, Math.round(((p.received ?? 0) / p.total) * 100)));
          setStep(p.part === "runtime" ? "Downloading the speech engine…" : "Downloading the model…");
        } else if (p.state === "unpacking") {
          setPct(null);
          setStep("Unpacking…");
        } else if (p.state === "checking") {
          setPct(null);
          setStep("Checking…");
        }
      }),
      onSpeechDone((d) => {
        if (!d.ok) {
          setBusy(false);
          setErr(d.error || "The download did not finish. Check your connection and try again.");
          return;
        }
        if (d.id === "runtime") {
          downloadModel(chosenRef.current).catch((e) => setErr(String(e)));
          return;
        }
        setBusy(false);
        setPct(null);
        setStep("");
        refresh();
      }),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function download() {
    setErr("");
    setBusy(true);
    setPct(null);
    setStep("Starting…");
    if (!inTauri) {
      let p = 0;
      const id = setInterval(() => {
        p += 9;
        setPct(Math.min(99, p));
        setStep("Downloading the model…");
        if (p >= 99) {
          clearInterval(id);
          setBusy(false);
          setPct(null);
          patch({
            speech: {
              ...cfg.speech,
              runtime: { ...(cfg.speech.runtime ?? { id: "runtime", lib: "" }), installed: true },
              models: cfg.speech.models?.map((m) => (m.id === chosen ? { ...m, installed: true } : m)),
            },
          });
        }
      }, 120);
      return;
    }
    try {
      await downloadModel(cfg.speech.runtime?.installed ? chosen : "runtime");
    } catch (e) {
      setBusy(false);
      setErr(String(e));
    }
  }

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
            disabled={busy}
            onClick={() => patch({ model: m.id })}
          >
            <span className="radio" />
            <div className="grow">
              <div className="ttl">
                {m.name.replace(/ v2 \(English\)$/, "").replace(" v2", "")}
                {m.id === cfg.recommended ? (
                  <span className="badge" style={{ marginLeft: 8, height: 18, verticalAlign: 1 }}>
                    Recommended
                  </span>
                ) : null}
              </div>
              <div className="sub">
                {info?.blurb ?? "English, on-device."} {info ? `· ${info.size}` : ""}
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
      <div style={{ minHeight: 44, display: "flex", flexDirection: "column", justifyContent: "center", gap: 8 }}>
        {busy ? (
          <>
            <div className="row-flex" style={{ display: "flex", justifyContent: "space-between" }}>
              <span className="t-caption">{step}</span>
              <span className="t-caption" style={{ fontVariantNumeric: "tabular-nums" }}>
                {pct != null ? `${pct}%` : ""}
              </span>
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
            Ready. Nothing you say leaves this computer.
          </span>
        ) : (
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <button type="button" className="btn primary" onClick={download}>
              Download {(models.find((m) => m.id === chosen)?.name ?? "model").replace(/ v2.*$/, "")}
            </button>
            <span className="t-small">Downloads once. Works offline after that.</span>
          </div>
        )}
        {err ? (
          <span className="t-caption" style={{ color: "var(--red-ink)" }}>
            {err}
          </span>
        ) : null}
      </div>
    </div>
  );
}

function useRefValue<T>(v: T) {
  const r = useRef(v);
  r.current = v;
  return r;
}
