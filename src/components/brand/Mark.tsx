/** Sequence Nova four-square mark — used as Pointcast's brand glyph. */
export function Mark({ className = "", size = "sm" }: { className?: string; size?: "sm" | "lg" }) {
  return (
    <span className={`mark ${size === "lg" ? "lg" : ""} ${className}`.trim()} aria-hidden="true">
      <i />
      <i />
      <i />
      <i />
    </span>
  );
}

export const PRODUCT = "Pointcast";
