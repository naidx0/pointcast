/** Talk back in Settings: the voices offered, how the Voice select names them, and the speaking rates. */

/** A voice of this computer, as `speak_voices` lists it (natural ones first). */
export type SpeakVoice = { id: string; name: string; lang: string; natural: boolean };

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

/** A voice as the Voice select names it: "Ava (natural) · American English". */
export const voiceLabel = (v: SpeakVoice, locale?: string) => `${v.name}${v.natural ? " (natural)" : ""} · ${languageName(v.lang, locale)}`;

const base = (tag: string) => tag.split(/[-_]/)[0].toLowerCase();

/** The select's groups: the natural voices, then those in the person's language `here`, then the rest. Empty groups
 *  are left out, and each keeps the order it was listed in. */
export function voiceGroups(voices: SpeakVoice[], here: string): { title: string; voices: SpeakVoice[] }[] {
  const mine = base(here);
  return [
    { title: "Natural", voices: voices.filter((v) => v.natural) },
    { title: languageName(mine, here), voices: voices.filter((v) => !v.natural && base(v.lang) === mine) },
    { title: "Other languages", voices: voices.filter((v) => !v.natural && base(v.lang) !== mine) },
  ].filter((g) => g.voices.length > 0);
}

/** Voices for the browser preview. */
export const previewVoices: SpeakVoice[] = [
  { id: "com.apple.voice.premium.en-US.Zoe", name: "Zoe", lang: "en-US", natural: true },
  { id: "com.apple.voice.enhanced.en-GB.Serena", name: "Serena", lang: "en-GB", natural: true },
  { id: "com.apple.voice.compact.en-US.Samantha", name: "Samantha", lang: "en-US", natural: false },
  { id: "com.apple.voice.compact.en-GB.Daniel", name: "Daniel", lang: "en-GB", natural: false },
  { id: "com.apple.voice.compact.fr-FR.Thomas", name: "Thomas", lang: "fr-FR", natural: false },
];
