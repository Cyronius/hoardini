import { useState } from "react";
import { TEXT_SIZES, saveTextSize, textSizePref } from "../textSize";

export function TextSizePicker() {
  const [size, setSize] = useState(textSizePref);
  return (
    <div role="tablist" aria-label="Text size" className="flex rounded-full bg-ink-800 p-0.5 text-[length:var(--fs-xs)] font-bold">
      {TEXT_SIZES.map((s) => (
        <button
          key={s.id}
          type="button"
          role="tab"
          aria-selected={size === s.id}
          className={`flex-auto whitespace-nowrap rounded-full px-2.5 py-1 ${size === s.id ? "bg-ink-900 text-paper shadow-sm" : "text-muted hover:text-paper"}`}
          onClick={() => {
            setSize(s.id);
            saveTextSize(s.id);
          }}
        >
          {s.label}
        </button>
      ))}
    </div>
  );
}
