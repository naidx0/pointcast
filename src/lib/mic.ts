import type { UiState } from "./api";
import { platform } from "./theme";

/** What to tell the user when the microphone gives nothing, or very little. `lines` are the same message cut short
 * for the recording capsule, which has room for two short lines. */
export function micNotice(quiet: NonNullable<UiState["quiet_mic"]>): { full: string; lines: [string, string] } {
  const mac = platform() === "mac";
  if (quiet === "silent")
    return {
      full: `No sound from the microphone. Check the input in ${mac ? "System Settings › Sound" : "Windows Sound settings"}.`,
      lines: ["No sound from the mic", mac ? "Check Settings › Sound" : "Check Sound settings"],
    };
  return {
    full: "Your microphone is very quiet. Raise its input volume or move closer.",
    lines: ["Your mic is very quiet", "Raise volume, move closer"],
  };
}
