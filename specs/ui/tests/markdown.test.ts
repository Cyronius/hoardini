// Traces: UI-MD-NO-HTML
import { createElement } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { Markdown } from "@ui/components/Markdown";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement | null = null;

async function render(text: string): Promise<HTMLElement> {
  host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  flushSync(() => root.render(createElement(Markdown, { text })));
  await new Promise((r) => setTimeout(r, 50));
  return host;
}

afterEach(() => {
  host?.remove();
  host = null;
});

describe("UI-MD-NO-HTML", () => {
  it("renders <script> as inert text", async () => {
    const el = await render("before <script>alert(1)</script> after");
    expect(el.querySelector("script")).toBeNull();
    expect(el.textContent).toContain("before");
    expect(el.textContent).toContain("alert(1)");
    expect(el.textContent).toContain("after");
  });

  it("drops <img onerror> as an element and keeps its text", async () => {
    const el = await render('<img src=x onerror="alert(2)">\n\nvisible');
    expect(el.querySelector("img[onerror]")).toBeNull();
    expect(el.querySelector("[onerror]")).toBeNull();
    expect(el.textContent).toContain("visible");
  });

  it("drops <iframe>", async () => {
    const el = await render('<iframe src="http://example.com">inner text</iframe>\n\nafter');
    expect(el.querySelector("iframe")).toBeNull();
    expect(el.textContent).toContain("after");
  });

  it("never emits a javascript: link, markdown or raw", async () => {
    const el = await render('[click me](javascript:alert(1))\n\n<a href="javascript:alert(2)">raw link</a>');
    const bad = [...el.querySelectorAll("a")].filter((a) => /^\s*javascript:/i.test(a.getAttribute("href") ?? ""));
    expect(bad).toHaveLength(0);
    expect(el.textContent).toContain("click me");
    expect(el.textContent).toContain("raw link");
  });

  it("still renders ordinary markdown", async () => {
    const el = await render("**bold** and a [link](https://example.com)");
    expect(el.textContent).toContain("bold and a link");
    expect(el.textContent).not.toContain("**");
  });
});
