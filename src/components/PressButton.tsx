import { useMemo, useRef, type ButtonHTMLAttributes } from "react";
import { pressOnce } from "@/lib/press";

/** A button that acts on the press itself: the Ask panel and the dictation capsule are shown without focus, and on
 * Windows the first click on an inactive window only focuses it (see press.ts). Enter and Space act as on any button. */
export function PressButton({ act, ...rest }: { act: () => void } & Omit<ButtonHTMLAttributes<HTMLButtonElement>, "onClick" | "onPointerDown" | "type">) {
  const latest = useRef(act);
  latest.current = act;
  const p = useMemo(() => pressOnce(() => latest.current()), []);
  return <button type="button" {...rest} onPointerDown={(e) => p.onPointerDown(e)} onClick={(e) => p.onClick(e)} />;
}
