// Traces: UI-LAYOUT-WIDTH
import { describe, expect, it } from "vitest";
import { chatWidthFor, isWide, parsePrefs } from "@ui/shell/layout";

describe("UI-LAYOUT-WIDTH", () => {
  it("puts the chat beside the home tabs from 1100 px and in a tab below that", () => {
    expect(isWide(1099)).toBe(false);
    expect(isWide(1100)).toBe(true);
    expect(isWide(430)).toBe(false);
  });

  it("keeps the panel between 340 and 760 px and the home pane at 560 px or more", () => {
    expect(chatWidthFor(1440, null)).toBe(420);
    expect(chatWidthFor(1440, 200)).toBe(340);
    expect(chatWidthFor(1440, 900)).toBe(760);
    expect(chatWidthFor(2560, 5000)).toBe(760);
    expect(chatWidthFor(1100, 900)).toBe(540);
    expect(chatWidthFor(1100, null)).toBe(420);
    expect(chatWidthFor(1440, 512.6)).toBe(513);
  });

  it("reads the stored width and collapsed state, and falls back on anything else", () => {
    const none = { chatWidth: null, collapsed: false };
    expect(parsePrefs('{"chatWidth":500,"collapsed":true}')).toEqual({ chatWidth: 500, collapsed: true });
    expect(parsePrefs(null)).toEqual(none);
    expect(parsePrefs("not json")).toEqual(none);
    expect(parsePrefs("[1,2]")).toEqual(none);
    expect(parsePrefs('{"chatWidth":"wide","collapsed":"yes"}')).toEqual(none);
    expect(parsePrefs('{"chatWidth":-3,"collapsed":true}')).toEqual({ chatWidth: null, collapsed: true });
  });
});
