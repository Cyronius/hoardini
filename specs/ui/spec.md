# UI specification (prefix UI)

### UI-MD-NO-HTML: Chat markdown drops raw HTML
**Applies to:** hoardini
**Verification:** test

Chat markdown shall render raw HTML in model or web-sourced text as literal text. No `script`, `iframe`, or event-handler attribute reaches the DOM, and no `javascript:` link is emitted.

**Acceptance criteria:**
- `before <script>alert(1)</script> after` → no `script` element; "before", "alert(1)", "after" appear as text
- `<img src=x onerror="alert(2)">` → no element carries `onerror`
- `<iframe src="http://example.com">inner text</iframe>` → no `iframe` element
- `[click me](javascript:alert(1))` and `<a href="javascript:alert(2)">raw link</a>` → no `a` with a `javascript:` href; link text survives
- `**bold**` still renders as formatted text

External tests: none. Test: `specs/ui/tests/markdown.test.ts`.

### UI-MASCOT-FULLNESS: Sumo mascot tracks disk fullness and stomps while thinking
**Applies to:** hoardini
**Verification:** manual

The header mascot is a sumo wrestler whose body girth tracks the used fraction (1 - free/total) of the system volume (the `C:\` volume, else the first volume), growing continuously and steeply near full. While an agent run is in progress he performs the shiko stomp, raising and stomping the left leg, then the right, repeatedly. With `prefers-reduced-motion` he holds a static pose.

**Verification (manual):**
1. In `ui/`, run `MOCK_FILL=0.30 bun run mock` and `bun run dev`; open http://localhost:5193/.
2. Confirm the mascot is slim and his tooltip reads "Hoardini, a sumo wrestler - C: is 30% full".
3. Run `curl -X POST "http://127.0.0.1:7043/api/mock?token=dev" -d '{"fill":0.97}'`; confirm he smoothly widens to near the edges of his box without the header layout moving.
4. Post `{"fill":0.6}`; confirm he slims back down, and that 90% looks visibly chubbier than 60%.
5. Post `{"running_runs":1}`; confirm he alternately lifts and stomps left then right, with dust at each stomp. Post `{"running_runs":0}` to stop.
6. Enable reduced motion in the OS or browser; confirm no looping animation.
