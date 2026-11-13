/** Call mode, as the Ask panel and the capsule show it: who has the call, the words for it, and what Esc does. */

export type CallSide = "listening" | "thinking" | "speaking";
export type CallEnd = "toggle" | "esc" | "closed" | "quiet" | "hangup" | "failed";

/** How a call stands, as `call_state` and `call://state` give it. */
export type CallView = { active: boolean; side: CallSide | null; ended: CallEnd | null };

export const NO_CALL: CallView = { active: false, side: null, ended: null };

/** Which side is talking, in a word. */
export function sideWord(side: CallSide | null): string {
  return side === "speaking" ? "Speaking" : side === "thinking" ? "Thinking" : "Listening";
}

/** The pill: "In a call · Listening"; empty with no call. */
export function callLine(v: CallView): string {
  return v.active ? `In a call · ${sideWord(v.side)}` : "";
}

/** What the Call switch says on hover. */
export function callTitle(active: boolean): string {
  return active ? "End the call" : "Call: talk, hear the answer, keep talking (no keys)";
}

/** What a screen reader hears when the call changes; why it ended once it has. */
export function callAnnounce(v: CallView): string {
  if (v.active) return callLine(v);
  switch (v.ended) {
    case "quiet":
      return "Call ended: nobody spoke for 45 seconds";
    case "hangup":
      return "Call ended";
    case "failed":
      return "The call could not listen";
    case null:
      return "";
    default:
      return "Call ended";
  }
}

/** The capsule's line during a call: it follows the person while it listens and its own voice while it speaks
 * (calm when that is not known), and loops while it thinks. */
export function callLineMode(side: CallSide | null): "listen" | "think" {
  return side === "thinking" ? "think" : "listen";
}

/** What Esc does in the panel, first match wins: a big picture, the session list, the summary or a Sent view closes;
 * in a call Esc is the call's (once silences, twice ends); else reading aloud stops, then what is under way, then the
 * panel closes. */
export type EscAction = "picture" | "sheet" | "summary" | "sent" | "call" | "speech" | "stop" | "close";

export function escAction(s: { big: boolean; sheet: boolean; summary: boolean; sent: boolean; call: boolean; speaking: boolean; stop: boolean }): EscAction {
  if (s.big) return "picture";
  if (s.sheet) return "sheet";
  if (s.summary) return "summary";
  if (s.sent) return "sent";
  if (s.call) return "call";
  if (s.speaking) return "speech";
  return s.stop ? "stop" : "close";
}

/** The browser preview's call (`?call=listening|thinking|speaking`); none without it. */
export function previewCall(q: URLSearchParams): CallView {
  const side = q.get("call");
  return side === "listening" || side === "thinking" || side === "speaking" ? { active: true, side, ended: null } : NO_CALL;
}
