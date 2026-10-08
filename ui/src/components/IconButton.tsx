import type { ButtonHTMLAttributes } from "react";

export function IconButton({
  className = "",
  size = "md",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { size?: "sm" | "md" }) {
  const box = size === "sm" ? "h-7 w-7" : "h-8 w-8";
  return (
    <button
      type="button"
      className={`grid place-items-center shrink-0 rounded-lg ${box} hover:bg-ink-800 disabled:opacity-40 disabled:hover:bg-transparent transition ${className}`}
      {...rest}
    />
  );
}
