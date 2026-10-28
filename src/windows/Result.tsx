import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { copyAgain, lastPrompt, onResult, type SessionExport } from "@/lib/api";
import { invoke } from "@tauri-apps/api/core";

export function Result() {
  const [exp, setExp] = useState<SessionExport | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    lastPrompt().then(setExp).catch(() => undefined);
    const u = onResult(setExp);
    return () => {
      u.then((fn) => fn());
    };
  }, []);

  if (!exp) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-ink-950 text-ink-300">
        No session yet. Start recording from the tray.
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-ink-950 px-6 py-6 text-ink-100">
      <div className="mb-4 flex items-center justify-between">
        <div>
          <p className="text-[11px] uppercase tracking-[0.2em] text-ember-400">Assembled prompt</p>
          <h1 className="font-display text-2xl">Ready to paste</h1>
        </div>
        <div className="flex gap-2">
          <Button
            onClick={async () => {
              await copyAgain();
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            }}
          >
            {copied ? "Copied" : "Copy again"}
          </Button>
          <Button
            variant="secondary"
            onClick={() => invoke("paste_again")}
          >
            Paste
          </Button>
          <Button
            variant="outline"
            onClick={async () => {
              const p = await invoke<string | null>("last_folder");
              if (p) await invoke("open_folder", { path: p });
            }}
          >
            Open folder
          </Button>
        </div>
      </div>
      <article className="rounded-xl border border-ink-700 bg-ink-900 p-5 leading-7">
        <PromptBody exp={exp} />
      </article>
      <p className="mt-3 font-mono text-xs text-ink-500">
        HTML clipboard carries inline images. Plain fallback: {exp.plain.slice(0, 120)}
        {exp.plain.length > 120 ? "…" : ""}
      </p>
    </div>
  );
}

function PromptBody({ exp }: { exp: SessionExport }) {
  const parts = splitMarkers(exp.plain);
  return (
    <p className="whitespace-pre-wrap">
      {parts.map((p, i) =>
        p.kind === "text" ? (
          <span key={i}>{p.value}</span>
        ) : (
          <span key={i} className="mx-1 inline-flex flex-col items-start align-middle">
            <img
              src={exp.images[p.n - 1]?.data_url}
              alt={p.value}
              className="my-2 max-h-64 max-w-full rounded-md border border-ink-500/30"
            />
            <span className="font-mono text-[11px] text-ice-400">{p.value}</span>
          </span>
        ),
      )}
    </p>
  );
}

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
