import { PenLine } from "lucide-react";
import { liveMark } from "@/lib/api";
import { PressButton } from "@/components/PressButton";

/** The question's picture can be marked up before it goes: a small text button that opens the overlay on it
 * (`live_mark`); the M key does the same while the panel has focus. Once marked it reads "Marked up", and a press
 * marks it up again. For the listening row's caption line (`.live-cap`) and the field. `onError` hears a refusal in
 * the engine's words ("A question is already under way.", "The picture is being marked up."). */
export function MarkUpControl({ marked = false, className = "", onError }: { marked?: boolean; className?: string; onError?: (why: string) => void }) {
  return (
    <PressButton
      className={`live-markup${marked ? " marked" : ""}${className ? ` ${className}` : ""}`}
      title={marked ? "Marked up. Press to mark it up again (M)" : "Mark up the picture (M)"}
      aria-keyshortcuts="M"
      act={() => void liveMark().catch((e) => onError?.(String(e).replace(/^Error: */, "")))}
    >
      {marked ? <PenLine size={12} strokeWidth={1.75} aria-hidden="true" /> : null}
      {marked ? "Marked up" : "Mark up"}
    </PressButton>
  );
}

/** The small mark on a marked picture's thumbnail: a pen in a dot at its lower left corner. Decorative; the label next
 * to the thumbnail says it in words. */
export function MarkedBadge() {
  return (
    <span className="live-marked" aria-hidden="true">
      <PenLine size={10} strokeWidth={2} />
    </span>
  );
}
