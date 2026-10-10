// Traces: UI-THEME-CONTRAST
import { describe, expect, it } from "vitest";
import { THEMES, contrast, groupOf, onAccent, themeVars } from "@ui/theme";
import { STATES, explorePalette } from "@ui/explore/palette";

describe("UI-THEME-CONTRAST", () => {
  it("ships the midtone group and Tater Dog", () => {
    expect(THEMES.filter((t) => groupOf(t) === "midtone")).toHaveLength(22);
    expect(THEMES.find((t) => t.id === "tater")?.mode).toBe("light");
  });

  for (const t of THEMES) {
    const k = t.tokens;
    it(`${t.id} meets WCAG contrast`, () => {
      for (const bg of [k.bg0, k.bg1, k.bg2]) {
        expect(contrast(k.text, bg)).toBeGreaterThanOrEqual(4.5);
        expect(contrast(k.textDim, bg)).toBeGreaterThanOrEqual(4.5);
      }
      expect(contrast(k.muted, k.bg1)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(k.faint, k.bg1)).toBeGreaterThanOrEqual(3);
      expect(contrast(onAccent(k), k.accent)).toBeGreaterThanOrEqual(4.5);
    });
  }

  it("applies the computed on-accent color", () => {
    const rosewood = THEMES.find((t) => t.id === "rosewood")!;
    expect(themeVars(rosewood)["--color-on-accent"]).toBe(onAccent(rosewood.tokens));
  });

  it("computes contrast by the WCAG formula", () => {
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrast("#777777", "#777777")).toBeCloseTo(1, 5);
  });

  for (const t of THEMES) {
    it(`${t.id} labels every Explore block at 4.5:1`, () => {
      const p = explorePalette(t.tokens);
      const swatches = [...STATES.map((s) => p.states[s]), ...STATES.map((s) => p.frames[s]), ...p.types, p.other, p.more, p.plain, p.frame];
      for (const s of swatches) expect(contrast(s.ink, s.fill)).toBeGreaterThanOrEqual(4.5);
      expect(p.dark).toBe(t.mode === "dark");
    });
  }
});
