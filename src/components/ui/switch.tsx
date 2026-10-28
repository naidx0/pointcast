export function Switch({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label?: string;
}) {
  return (
    <label className="row-between" style={{ justifyContent: "flex-start", gap: 10 }}>
      <button
        type="button"
        role="switch"
        className="toggle"
        aria-checked={checked}
        onClick={() => onChange(!checked)}
      />
      {label ? <span style={{ fontSize: "var(--fs-ui)", color: "var(--text)" }}>{label}</span> : null}
    </label>
  );
}
