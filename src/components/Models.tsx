import { useEffect, useState } from "react";
import { Check } from "lucide-react";
import {
  gpuUpgradeCancel,
  inTauri,
  onGpuUpgrade,
  onSpeechDone,
  onSpeechProgress,
  onState,
  preview,
  sessionState,
  shortDevice,
  speechCancel,
  speechRemove,
  speechSetup,
  speechStatus,
  subscribe,
  type Config,
  type GpuUpgrade,
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

/** Downloaded with an engine to run it: sherpa's runtime, or the GPU version with its helper. */
const runnable = (m: SpeechModel, engine: boolean) => m.installed && (engine || !!m.gpu?.installed);

export function speechReady(cfg: Config) {
  const id = chosenModel(cfg);
  return !!cfg.speech.models?.some((m) => m.id === id && runnable(m, !!cfg.speech.runtime?.installed));
}

/** What a download of `m` fetches: its GPU version where that can be set up here. */
const download = (m: SpeechModel) => (m.gpu?.available && !m.gpu.installed ? m.gpu.bytes : m.bytes);

/** Where the model in use runs, once the engine has said, and how fast it was here once measured. */
function where(m: SpeechModel) {
  const speed = m.speed_x ? ` · ${m.speed_x >= 10 ? Math.round(m.speed_x) : m.speed_x}x real time` : "";
  if (m.backend === "gpu")
    return { text: `Runs on your graphics card${m.device ? ` (${shortDevice(m.device)})` : ""}${speed}`, warn: false };
  if (m.backend === "cpu" && m.gpu && !m.gpu.unusable)
    return { text: `Runs on your processor${speed}. A graphics card makes it much faster.`, warn: true };
  if (m.backend === "cpu") return { text: `Runs on your processor${speed}`, warn: false };
  return null;
}

/** The GPU engine's first start here compiles the graphics card's shaders, which takes a minute or two once. */
function usePreparingGpu() {
  const [on, setOn] = useState(false);
  useEffect(() => {
    if (inTauri) sessionState().then((s) => setOn(!!s.preparing_gpu)).catch(() => undefined);
    else if (preview) setOn(new URLSearchParams(location.search).get("gpu") === "preparing");
    return subscribe(onState((s) => setOn(!!s.preparing_gpu)));
  }, []);
  return on;
}

/** The faster GPU version setting itself up in the background, live; Settings is read again once it is in or found
 * unusable. In the browser preview, `?upgrade=38` (or `checking`, `low_disk`, `failed`) shows its states. */
function useGpuUpgrade(cfg: Config, refresh: () => void) {
  const [up, setUp] = useState<GpuUpgrade | null>(cfg.gpu_upgrade ?? null);
  useEffect(() => {
    if (!inTauri && preview) {
      const q = new URLSearchParams(location.search).get("upgrade");
      if (q && /^[0-9]+$/.test(q)) setUp({ state: "downloading", percent: Number(q) });
      else if (q === "low_disk")
        setUp({ state: "low_disk", percent: 0, note: "The faster GPU version needs about 2.6 GB free and 1.2 GB are. Free some space and it is set up by itself." });
      else if (q) setUp({ state: q as GpuUpgrade["state"], percent: 100, note: q === "failed" ? "Pointcast can't reach GitHub to download the speech files." : undefined });
    }
    return subscribe(
      onGpuUpgrade((u) => {
        setUp(u);
        if (u.state === "done" || u.state === "unusable") refresh();
      }),
    );
  }, [refresh]);
  useEffect(() => {
    if (inTauri) setUp(cfg.gpu_upgrade ?? null);
  }, [cfg.gpu_upgrade?.state]);
  return up;
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

/** Whether the model in use is slower than real time here, as the last recording reported it. Remembered with the model
 * it was seen under, so picking another model starts clean. */
function useSlowEngine(model: string) {
  const [seen, setSeen] = useState<{ model: string; slow: boolean } | null>(null);
  useEffect(() => {
    const note = (slow: boolean | undefined) => slow !== undefined && setSeen({ model, slow });
    if (inTauri) sessionState().then((s) => note(s.slow_engine)).catch(() => undefined);
    else if (preview) note(new URLSearchParams(location.search).get("slow") === "1" || undefined);
    return subscribe(onState((s) => note(s.slow_engine)));
  }, [model]);
  return seen?.model === model && seen.slow;
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
  const [removeErr, setRemoveErr] = useState<{ id: string; text: string } | null>(null);
  const chosen = chosenModel(cfg);
  // Marked is the model recordings really use; picking one that is not downloaded yet does not change that.
  const active = cfg.speech_in_use ?? chosen;
  const engine = cfg.speech.runtime?.installed ?? false;
  const models = withPreview(cfg.speech.models ?? []).sort((a, b) => offer(a).order - offer(b).order);
  const busyId = job ? (job.id ?? chosen) : null;
  const slow = useSlowEngine(active);
  const preparing = usePreparingGpu();
  const upgrade = useGpuUpgrade(cfg, refresh);
  const auto = cfg.auto_gpu !== false;
  // The model to suggest instead: the fast, accurate one, or the smallest when that is already the one in use.
  const faster = models.find((m) => m.id === (active === "moonshine-base-en" ? "moonshine-tiny-en" : "moonshine-base-en"));
  // A model is on disk but the engine did not load or run it: say so instead of "Ready".
  const failed = speechReady(cfg) && cfg.speech_check && !cfg.speech_check.ok ? (cfg.speech_check.error ?? "") : "";

  return (
    <div className="cards">
      {models.map((m) => {
        const o = offer(m);
        const on = m.id === active;
        const ready = runnable(m, engine);
        // The processor version is here and the faster GPU one can be set up: it sets itself up, or (turned off, or
        // after a failed try) the switch is offered.
        const switchable = !!m.gpu?.available && m.gpu.cpu_installed && !m.gpu.installed && !m.gpu.unusable;
        const bg = m.gpu && upgrade && upgrade.state !== "idle" && upgrade.state !== "done" ? upgrade : null;
        const fetching = !!bg && (bg.state === "downloading" || bg.state === "checking");
        const offerSwitch = switchable && !fetching && (!auto || bg?.state === "failed");
        const device = on && ready && !fetching ? where(m) : null;
        const mine = busyId === m.id;
        const pct = mine && job?.phase === "download" && job.total ? Math.min(100, ((job.received ?? 0) / job.total) * 100) : null;
        const err =
          error?.id === m.id && error.text !== "Cancelled." ? error.text : removeErr?.id === m.id ? removeErr.text : "";
        const pick = () => !job && m.installed && patch({ model: m.id });
        return (
          <div
            key={m.id}
            className="card model"
            role="radio"
            aria-checked={on}
            tabIndex={0}
            onClick={pick}
            onKeyDown={(e) => (e.key === " " || e.key === "Enter") && pick()}
          >
            <span className="radio" />
            <div className="grow">
              <div className="ttl">
                {o.name}
                {o.tag ? <span className={`badge ${o.tag === "Recommended" ? "rec" : ""}`}>{o.tag}</span> : null}
              </div>
              <div className="sub">
                {m.gpu?.available ? "OpenAI's large-v3-turbo. 99 languages. Fast on your graphics card." : o.blurb} ·{" "}
                {mbText(m.gpu?.installed ? m.gpu.bytes : ready ? m.bytes : download(m))}
              </div>
              {mine ? (
                <div className="model-progress">
                  <div className={`progress ${pct == null ? "indet" : ""}`} role="progressbar" aria-valuenow={pct ?? undefined}>
                    <i style={pct == null ? undefined : { width: `${pct}%` }} />
                  </div>
                  <span className="t-small">
                    {job?.phase === "check"
                      ? preparing
                        ? "Preparing your graphics card. This happens once and takes a minute or two…"
                        : "Checking it works…"
                      : job?.phase === "unpack"
                        ? "Unpacking…"
                        : job?.total
                          ? `${mbText(job.received ?? 0)} of ${mbText(job.total)}`
                          : "Starting…"}
                  </span>
                </div>
              ) : null}
              {device && !mine && !preparing ? (
                <div className="t-small" style={{ marginTop: 4, color: device.warn ? "var(--orange-ink)" : undefined }}>
                  {device.text}
                </div>
              ) : null}
              {on && preparing && !mine ? (
                <div className="t-small" style={{ marginTop: 4 }}>
                  Preparing your graphics card. This happens once and takes a minute or two.
                </div>
              ) : null}
              {fetching && !mine ? (
                <div className="model-progress">
                  <div
                    className={`progress ${bg!.state === "checking" ? "indet" : ""}`}
                    role="progressbar"
                    aria-label="Getting the faster GPU version"
                    aria-valuenow={bg!.state === "checking" ? undefined : bg!.percent}
                  >
                    <i style={bg!.state === "checking" ? undefined : { width: `${bg!.percent}%` }} />
                  </div>
                  <span className="t-small">
                    {bg!.state === "checking" ? "Checking your graphics card…" : `Getting the faster GPU version… ${bg!.percent}%`}
                  </span>
                </div>
              ) : null}
              {bg && !fetching && bg.note && bg.state !== "unusable" && !mine ? (
                <div className="t-small" style={{ marginTop: 4, color: "var(--orange-ink)" }}>
                  {bg.note}
                </div>
              ) : null}
              {m.gpu?.unusable && on && !mine ? (
                <div className="t-small" style={{ marginTop: 4 }}>
                  Your graphics card can't run the faster version, so Whisper Turbo runs on your processor.
                </div>
              ) : null}
              {ready && offerSwitch && !mine ? (
                <div className="t-small" style={{ marginTop: 4 }}>
                  <button
                    type="button"
                    className="link"
                    disabled={!!job}
                    onClick={(e) => {
                      e.stopPropagation();
                      patch({ model: m.id });
                      start(m.id, cfg, patch);
                    }}
                  >
                    Switch to the faster GPU version ({mbText(m.gpu!.bytes)})
                  </button>
                </div>
              ) : null}
              {on && slow && faster && !mine && !device?.warn ? (
                <div className="t-small" style={{ marginTop: 4, color: "var(--orange-ink)" }}>
                  This model is slower than real time on this computer. {offer(faster).name} is{" "}
                  {faster.id === "moonshine-base-en" ? "much " : ""}faster.{" "}
                  <button
                    type="button"
                    className="link"
                    disabled={!!job}
                    onClick={(e) => {
                      e.stopPropagation();
                      patch({ model: faster.id });
                      if (!(faster.installed && engine)) start(faster.id, cfg, patch);
                    }}
                  >
                    {faster.installed && engine ? `Use ${offer(faster).name}` : `Download ${offer(faster).name}`}
                  </button>
                </div>
              ) : null}
              {mine ? null : err ? (
                <div className="t-small" role="alert" style={{ color: "var(--red-ink)", marginTop: 4 }}>
                  {err.replace(/^Error:\s*/, "")}
                  {/can't reach|download server|download failed/i.test(err) ? " Download picks up where it stopped." : ""}
                </div>
              ) : null}
            </div>
            {fetching && !mine ? (
              bg!.state === "downloading" ? (
                <button
                  type="button"
                  className="btn"
                  title="Stop downloading the faster GPU version; Whisper Turbo stays on your processor"
                  onClick={(e) => {
                    e.stopPropagation();
                    if (inTauri) gpuUpgradeCancel().catch(() => undefined);
                    else setTimeout(refresh, 0);
                  }}
                >
                  Cancel
                </button>
              ) : (
                <span className="spinner" />
              )
            ) : mine ? (
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
                    title={`Delete ${o.name} and free ${mbText(m.gpu?.installed ? m.gpu.bytes + (m.gpu.cpu_installed ? m.bytes : 0) : m.bytes)}`}
                    disabled={!!job}
                    onClick={async (e) => {
                      e.stopPropagation();
                      setRemoveErr(null);
                      if (inTauri)
                        await speechRemove(m.id).catch((x) => setRemoveErr({ id: m.id, text: String(x) }));
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
      <p
        className="t-small"
        role={failed ? "alert" : undefined}
        style={{ margin: "4px 2px 0", color: failed ? "var(--red-ink)" : undefined }}
      >
        {failed
          ? `The speech engine didn't start on this computer: ${failed.replace(/^The speech engine didn't start on this computer: /, "")}`
          : speechReady(cfg) && cfg.speech_check?.ok
            ? "Ready and tested on this computer. Nothing you say leaves it."
            : removable
            ? "Models download once and run offline on this computer."
            : "Models download once and run offline on this computer. You can switch any time in Settings."}
      </p>
    </div>
  );
}

/** The browser stand-in lists two models; show the whole lineup there too. `gpu=` shows Whisper Turbo's GPU states:
 * `switch` (the processor version is here), `gpu` or `cpu` (the GPU version in use, and where it runs), `preparing`. */
function withPreview(list: SpeechModel[]): SpeechModel[] {
  if (inTauri) return [...list];
  const gpu = new URLSearchParams(location.search).get("gpu");
  if (gpu) {
    const turbo = (m: SpeechModel): SpeechModel => ({
      ...m,
      installed: gpu !== "offer",
      gpu: { available: true, installed: gpu !== "switch" && gpu !== "offer", bytes: 574_041_195, cpu_installed: gpu === "switch" },
      backend: gpu === "cpu" ? "cpu" : gpu === "switch" ? "cpu" : "gpu",
      device: gpu === "gpu" ? "NVIDIA GeForce RTX 2060 SUPER via Vulkan" : undefined,
      speed_x: gpu === "gpu" ? 19.8 : gpu === "cpu" ? 1.6 : undefined,
    });
    return lineup(list).map((m) => (m.id === "whisper-turbo" ? turbo(m) : m));
  }
  return lineup(list);
}

function lineup(list: SpeechModel[]): SpeechModel[] {
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
        speech_check: { ok: true, error: null },
        speech: {
          ...cfg.speech,
          runtime: { ...(cfg.speech.runtime ?? { id: "runtime", lib: "" }), installed: true },
          models: withPreview(cfg.speech.models ?? []).map((m) => (m.id === id ? { ...m, installed: true } : m)),
        },
      });
    }, 700);
  }, 120);
}
