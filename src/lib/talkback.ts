/** Talk back in Settings: the voices offered, how the Voice select names them, and the speaking rates. */

/** A voice, as `speak_voices` lists it: Kokoro's first, then this computer's (natural ones first). `download` is the
 *  bytes to fetch before it can speak; absent once it is on this computer. */
export type SpeakVoice = { id: string; name: string; lang: string; natural: boolean; download?: number };

/** The natural voice's download, as `kokoro://progress` and `speak_fetching` give it. */
export type KokoroProgress =
  | { state: "downloading"; received: number; total: number | null }
  | { state: "unpacking" }
  | { state: "verifying" }
  | { state: "ready" }
  | { state: "failed"; reason: string };

/** Kokoro's voices are kept as "kokoro:<name>". */
export const isKokoro = (v: SpeakVoice | string) => (typeof v === "string" ? v : v.id).startsWith("kokoro:");

/** The voice used when none is chosen, once Kokoro is downloaded. */
export const RECOMMENDED = "kokoro:af_heart";

/** A size as people read it: "141 MB". */
export const megabytes = (bytes: number) => `${bytes > 0 ? Math.max(1, Math.round(bytes / 1e6)) : 0} MB`;

/** The speaking rates offered, 1 being normal, as the rate control names them. */
export const RATES: readonly (readonly [number, string])[] = [
  [0.8, "Slower"],
  [1, "Normal"],
  [1.25, "Faster"],
  [1.5, "Fastest"],
];

/** The rate offered nearest to a saved one; normal when none is saved. */
export function nearestRate(r: number | null | undefined): number {
  if (typeof r !== "number" || !Number.isFinite(r)) return 1;
  return RATES.reduce((best, [x]) => (Math.abs(x - r) < Math.abs(best - r) ? x : best), RATES[0][0]);
}

/** A language tag as people say it ("en-US" is "American English"); the tag itself where that is unknown. */
export function languageName(tag: string, locale = "en"): string {
  try {
    return new Intl.DisplayNames([locale], { type: "language" }).of(tag) ?? tag;
  } catch {
    return tag;
  }
}

/** A voice as the Voice select names it: "Ava (natural) · American English"; a Kokoro voice (its group says natural)
 *  "Heart (recommended) · American English", with "· 141 MB download" until it is downloaded. */
export function voiceLabel(v: SpeakVoice, locale?: string): string {
  const note = isKokoro(v) ? (v.id === RECOMMENDED ? " (recommended)" : "") : v.natural ? " (natural)" : "";
  const size = v.download ? ` · ${megabytes(v.download)} download` : "";
  return `${v.name}${note} · ${languageName(v.lang, locale)}${size}`;
}

/** The select's first choice, no voice chosen: the system's voice until Kokoro is downloaded, then Kokoro's best. */
export function defaultLabel(voices: SpeakVoice[]): string {
  const best = voices.find((v) => v.id === RECOMMENDED && !v.download);
  return best ? `Recommended · ${best.name} (natural)` : "System default";
}

/** The status line under the Voice select while the natural voice downloads; null when there is nothing to say. */
export function fetchLine(p: KokoroProgress | null | undefined): string | null {
  if (!p) return null;
  switch (p.state) {
    case "downloading":
      return p.total ? `Downloading the natural voice… ${megabytes(p.received)} of ${megabytes(p.total)}` : "Downloading the natural voice…";
    case "unpacking":
    case "verifying":
      return "Setting up the natural voice…";
    case "ready":
      return "The natural voice is ready.";
    case "failed":
      return `The natural voice didn't download: ${p.reason}`;
  }
}

const base = (tag: string) => tag.split(/[-_]/)[0].toLowerCase();

/** The select's groups: Kokoro's voices, then this computer's natural ones, then those in the person's language
 *  `here`, then the rest. Empty groups are left out, and each keeps the order it was listed in. */
export function voiceGroups(voices: SpeakVoice[], here: string): { title: string; voices: SpeakVoice[] }[] {
  const mine = base(here);
  const system = voices.filter((v) => !isKokoro(v));
  return [
    { title: "Natural (Kokoro)", voices: voices.filter((v) => isKokoro(v)) },
    { title: "Natural", voices: system.filter((v) => v.natural) },
    { title: languageName(mine, here), voices: system.filter((v) => !v.natural && base(v.lang) === mine) },
    { title: "Other languages", voices: system.filter((v) => !v.natural && base(v.lang) !== mine) },
  ].filter((g) => g.voices.length > 0);
}

/** Voices for the browser preview. */
export const previewVoices: SpeakVoice[] = [
  { id: "kokoro:af_heart", name: "Heart", lang: "en-US", natural: true, download: 132_353_410 },
  { id: "kokoro:bf_emma", name: "Emma", lang: "en-GB", natural: true, download: 132_353_410 },
  { id: "com.apple.voice.premium.en-US.Zoe", name: "Zoe", lang: "en-US", natural: true },
  { id: "com.apple.voice.enhanced.en-GB.Serena", name: "Serena", lang: "en-GB", natural: true },
  { id: "com.apple.voice.compact.en-US.Samantha", name: "Samantha", lang: "en-US", natural: false },
  { id: "com.apple.voice.compact.en-GB.Daniel", name: "Daniel", lang: "en-GB", natural: false },
  { id: "com.apple.voice.compact.fr-FR.Thomas", name: "Thomas", lang: "fr-FR", natural: false },
];
