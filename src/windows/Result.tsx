import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Mark, PRODUCT } from "@/components/brand/Mark";
import { copyAgain, lastPrompt, onResult, type SessionExport } from "@/lib/api";
import { invoke } from "@tauri-apps/api/core";

export function Result() {
  const [exp, setExp] = useState<SessionExport | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    lastPrompt()
      .then(setExp)
      .catch(() => {
        if (typeof location !== "undefined" && location.search.includes("preview=1")) {
          setExp(PREVIEW_EXPORT);
        }
      });
    const u = onResult(setExp);
    return () => {
      u.then((fn) => fn());
    };
  }, []);

  if (!exp) {
    return (
      <div className="page" style={{ display: "grid", placeItems: "center", padding: 32 }}>
        <div style={{ textAlign: "center" }}>
          <Mark size="lg" />
          <p className="sub" style={{ marginTop: 12 }}>
            No session yet. Start recording from the tray.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="page">
      <header
        className="title-drag"
        style={{ borderBottom: "1px solid var(--line)", padding: "14px 24px 16px" }}
      >
        <div className="row-between" style={{ alignItems: "flex-start" }}>
          <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
            <div className="traffic" aria-hidden="true">
              <i />
              <i />
              <i />
            </div>
            <Mark />
            <div>
              <p className="kicker">{PRODUCT} · last prompt</p>
              <h1 className="title" style={{ fontSize: "var(--fs-sheet)" }}>
                Ready to paste
              </h1>
            </div>
          </div>
          <div className="no-drag" style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
            <Button
              onClick={async () => {
                await copyAgain();
                setCopied(true);
                setTimeout(() => setCopied(false), 1500);
              }}
            >
              {copied ? "Copied" : "Copy again"}
            </Button>
            <Button variant="secondary" onClick={() => invoke("paste_again")}>
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
      </header>

      <main style={{ padding: "20px 24px 40px", maxWidth: 820, margin: "0 auto" }}>
        <article className="card" style={{ lineHeight: 1.62, fontSize: "var(--fs-chat)" }}>
          <PromptBody exp={exp} />
        </article>
        <p className="mono" style={{ marginTop: 12, fontSize: "var(--fs-xs)", color: "var(--text3)" }}>
          HTML clipboard carries inline images. Plain fallback: {exp.plain.slice(0, 120)}
          {exp.plain.length > 120 ? "…" : ""}
        </p>
      </main>
    </div>
  );
}

function PromptBody({ exp }: { exp: SessionExport }) {
  const parts = splitMarkers(exp.plain);
  return (
    <div style={{ whiteSpace: "pre-wrap" }}>
      {parts.map((p, i) =>
        p.kind === "text" ? (
          <span key={i}>{p.value}</span>
        ) : (
          <span key={i} style={{ display: "inline-flex", flexDirection: "column", alignItems: "flex-start", margin: "8px 0", verticalAlign: "middle" }}>
            <img
              src={exp.images[p.n - 1]?.data_url}
              alt={p.value}
              style={{
                maxHeight: 280,
                maxWidth: "100%",
                borderRadius: 12,
                border: "1px solid var(--line)",
              }}
            />
            <span className="photo-chip" style={{ marginTop: 6 }}>
              {p.value}
            </span>
          </span>
        ),
      )}
    </div>
  );
}

const PREVIEW_EXPORT: SessionExport = {
  plain:
    "The Place order button is sitting on top of the total. I boxed it and dropped a pin.\n\n[image 1: 1=Place order — 28px too far left]\n\nPlease shift the primary button so it no longer overlaps the total.",
  html: "",
  markdown: "",
  images: [
    {
      n: 1,
      kind: "region",
      width: 800,
      height: 500,
      labels: [{ n: 1, text: "Place order — 28px too far left" }],
      data_url: "/freeze.png",
    },
  ],
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
