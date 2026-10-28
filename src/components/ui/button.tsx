import * as React from "react";
import { cn } from "@/lib/utils";

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "default" | "secondary" | "ghost" | "outline" | "danger";
  size?: "default" | "sm" | "lg" | "icon";
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = "default", size = "default", ...props }, ref) => (
    <button
      ref={ref}
      className={cn(
        "pill",
        variant === "default" && "pri",
        variant === "ghost" && "ghost",
        variant === "outline" && "",
        variant === "secondary" && "",
        variant === "danger" && "danger",
        size === "lg" && "lg",
        size === "sm" && "sm",
        size === "icon" && "ib",
        className,
      )}
      {...props}
    />
  ),
);
Button.displayName = "Button";
