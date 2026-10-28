import {
  ArrowUpRight,
  Check,
  ClipboardPaste,
  Copy,
  Crop,
  FolderOpen,
  Highlighter,
  MapPin,
  Mic,
  Monitor,
  Pencil,
  Search,
  Send,
  Sparkles,
  Square,
  Type,
  X,
  ChevronDown,
  type LucideProps,
} from "lucide-react";

/** One icon set (Lucide) matching Sequence Nova stroke weight. */
const base: LucideProps = {
  size: 16,
  strokeWidth: 1.75,
  absoluteStrokeWidth: false,
  "aria-hidden": true,
};

export type IconName =
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
  | "arrow"
  | "type"
  | "chev";

const map = {
  mic: Mic,
  monitor: Monitor,
  crop: Crop,
  zoom: Search,
  stop: Square,
  x: X,
  copy: Copy,
  paste: ClipboardPaste,
  folder: FolderOpen,
  check: Check,
  spark: Sparkles,
  send: Send,
  pin: MapPin,
  pen: Pencil,
  hi: Highlighter,
  arrow: ArrowUpRight,
  type: Type,
  chev: ChevronDown,
} as const;

export function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  const C = map[name];
  return <C {...base} size={size} className="i" style={{ display: "block" }} />;
}
