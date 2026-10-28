/** Compact stroke icons matching Sequence Nova's 16px `.i` style. */
export function Icon({
  name,
  size = 16,
}: {
  name:
    | "mic"
    | "monitor"
    | "crop"
    | "zoom"
    | "stop"
    | "x"
    | "copy"
    | "paste"
    | "folder"
    | "check"
    | "spark"
    | "send"
    | "pin"
    | "pen"
    | "hi"
    | "chev";
  size?: number;
}) {
  const common = {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.75,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    className: "i",
    "aria-hidden": true as const,
  };
  switch (name) {
    case "mic":
      return (
        <svg {...common}>
          <path d="M12 3a3 3 0 0 1 3 3v6a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3Z" />
          <path d="M19 11a7 7 0 0 1-14 0" />
          <path d="M12 18v3" />
        </svg>
      );
    case "monitor":
      return (
        <svg {...common}>
          <rect x="3" y="4" width="18" height="13" rx="2" />
          <path d="M8 21h8M12 17v4" />
        </svg>
      );
    case "crop":
      return (
        <svg {...common}>
          <path d="M6 3v12a3 3 0 0 0 3 3h12" />
          <path d="M18 21V9a3 3 0 0 0-3-3H3" />
        </svg>
      );
    case "zoom":
      return (
        <svg {...common}>
          <circle cx="11" cy="11" r="7" />
          <path d="m20 20-3.5-3.5" />
        </svg>
      );
    case "stop":
      return (
        <svg {...common}>
          <rect x="7" y="7" width="10" height="10" rx="1.5" fill="currentColor" stroke="none" />
        </svg>
      );
    case "x":
      return (
        <svg {...common}>
          <path d="M18 6 6 18M6 6l12 12" />
        </svg>
      );
    case "copy":
      return (
        <svg {...common}>
          <rect x="9" y="9" width="11" height="11" rx="2" />
          <path d="M5 15V5a2 2 0 0 1 2-2h10" />
        </svg>
      );
    case "paste":
      return (
        <svg {...common}>
          <path d="M9 5h6a2 2 0 0 1 2 2v1h1a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V10a2 2 0 0 1 2-2h1V7a2 2 0 0 1 2-2Z" />
          <path d="M9 5a2 2 0 0 0 2 2h2a2 2 0 0 0 2-2" />
        </svg>
      );
    case "folder":
      return (
        <svg {...common}>
          <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" />
        </svg>
      );
    case "check":
      return (
        <svg {...common}>
          <path d="m5 12 5 5L20 7" />
        </svg>
      );
    case "spark":
      return (
        <svg {...common}>
          <path d="m12 3 1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8Z" />
        </svg>
      );
    case "send":
      return (
        <svg {...common}>
          <path d="m5 12 14-7-7 14-2-5-5-2Z" />
        </svg>
      );
    case "pin":
      return (
        <svg {...common}>
          <path d="M12 17v5" />
          <path d="M9 3h6l1 7H8L9 3Z" />
          <path d="M8 10h8l-1.5 4h-5L8 10Z" />
        </svg>
      );
    case "pen":
      return (
        <svg {...common}>
          <path d="M12 20h9" />
          <path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" />
        </svg>
      );
    case "hi":
      return (
        <svg {...common}>
          <path d="M4 20 15 4" />
          <path d="M9 20 20 4" />
        </svg>
      );
    case "chev":
      return (
        <svg {...common}>
          <path d="m6 9 6 6 6-6" />
        </svg>
      );
  }
}
