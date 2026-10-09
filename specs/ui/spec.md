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

### UI-MD-KEEP-TEXT: Finished chat text is never cut
**Applies to:** hoardini
**Verification:** test

A finished chat message shall render all of its text. The incomplete-markdown repair that hides a half-streamed tag applies only to the message still streaming, so a bare `<` (`a<b`, `<100 MB`, a quoted `<<<UNTRUSTED_WEB_CONTENT` fence) never swallows the rest of a finished message.

**Acceptance criteria:**
- `Files <100 MB stay and a<b and c` renders with exactly that text.
- `Search result: <<<UNTRUSTED_WEB_CONTENT: data [1] Unsloth https://x.example/a Safe to delete` keeps the URL and the snippet.

Test: `specs/ui/tests/markdown.test.ts`.

### UI-CHAT-FLOW: Chat shows a run in order and handles stop, stall, and the step cap
**Applies to:** hoardini
**Verification:** manual

The chat shall show each run in the order it happened: text before a tool round, the round's chips (name, short argument, elapsed time), then the text after it, each text segment in its own paragraph. Stop ends a run at once and keeps the question on screen. A run that ends at the tool-step cap (`run_stats.limit_reached`) offers "Keep going"; a model that is silent for the profile's timeout gets the "Keep waiting / Stop" prompt. A reloaded thread shows the same messages, chips, and card states.

**Verification (manual):**
1. Point a provider at a scripted OpenAI-compatible mock (or a real model) and ask for something that calls a tool without text first, then answers: the chip renders above the answer, not below it.
2. A turn of "Let me look." + a tool call + "Here is the answer." renders as two paragraphs with the chip between them.
3. Ask for a long streamed answer and press Stop: streaming stops at once, the question stays, no "no content" prompt appears, `GET /api/status` shows `running_runs: 0`, and a new question answers normally.
4. With `max_tool_steps = 3` and a model that always calls a tool: three rounds of chips, then "Hoardini used its 3 tool steps for this turn before finishing." with Keep going; Keep going sends "Continue.".
5. With a model that takes longer than the timeout to answer: "No response from the model for 2 minutes." with Keep waiting and Stop.
6. Open the thread again from Recent chats (or reload the page and open it): messages, chips, and every proposal card's status are as before.
7. Leave a proposal card pending, reload, open the thread, and approve or decline the card: the decision is recorded and a new run starts from the answer (a `tool_*` message for the proposal's tool call), so the agent continues in the same chat.
(2026-10-08: steps 1–4 and 6 pass against a scripted mock; 1 and 5 against LM Studio `minicpm5-2b`.)

### UI-NO-IPC: The app page has no usable Tauri IPC
**Applies to:** hoardini-app
**Verification:** manual

The desktop window shall load the UI from the loopback server and talk to it only over HTTP with the launch token. The
page shall have no Tauri command it can call: `window.__TAURI__` is absent (`withGlobalTauri: false`) and, because the
app ships no capability files, every `__TAURI_INTERNALS__.invoke` is rejected. Native dialogs go through the server's
`/api/dialog/pick-folder` relay, never through page IPC.

**Verification (manual):**
1. Build and run `cargo run -p hoardini-app` with `HOARDINI_DATA_DIR` set to a scratch folder and
   `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`.
2. Connect with Playwright (`chromium.connect_over_cdp("http://127.0.0.1:9333")`) and pick the `http://127.0.0.1:<port>/` page.
3. The URL has no `#token=` fragment; `typeof window.__TAURI__` is `"undefined"`; `/api/status` answers 200 with the
   session token and 401 without.
4. Each of `invoke('plugin:dialog|open')`, `invoke('plugin:app|version')`, `invoke('plugin:event|emit')`,
   `invoke('plugin:window|title')` rejects with "... not allowed".
5. `app/` has no `capabilities` folder and `tauri.conf.json` lists `"capabilities": []`.
(2026-10-08: as described. Note: Tauri 2.12 always injects `window.__TAURI_INTERNALS__` (a non-configurable property)
into every main-frame page, also external URLs, so `typeof window.__TAURI_INTERNALS__` is `"object"`; the protection is
the empty capability set, not the absence of the global.)

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

### UI-SETTINGS-AUTOSAVE: Settings save themselves; one Rescan now control
**Applies to:** hoardini
**Verification:** manual

The Settings tab shall have no Save button. A toggle or switch saves when it changes; a text field saves when it loses focus or 500 ms after the last keystroke (`drawer/autosave.tsx`). A "Saved" tick appears beside the control that saved, or the server's error appears inline under it and the control shows the stored value again. The tab holds the Fast / Lean switch ("Slower, stays out of your way" / "Finishes sooner, uses more CPU"; SRV-SETTINGS `index_speed`), the read-only index size, and Clear chat history; the threshold, worker, memory-cap, redaction, and web-search settings are gone. The AI tab's provider form has no "Redact paths" toggle (AGENT-REDACT). The header shows one small "Rescan now" button while the index is idle, which runs a refresh at the switch's speed (SRV-ONE-REFRESH); while a refresh runs, its progress bar takes the button's place, with the leaf icon marking Lean, and there is no Scan fast button.

**Verification (manual):**
1. Start `hoardini serve --port 72xx --ui-dir ui/dist` against a scratch `HOARDINI_DATA_DIR` whose meta `indexed_volumes` points at a small folder, and open the printed URL.
2. Header: once the first scan is done, "Index just now" and a "Rescan now" button; no refresh icon and no Scan fast.
3. Settings: the switch shows Lean with its hint and the index size; no Save button and none of the removed settings. Click Fast: "Saved" appears beside the switch and fades; reopen the drawer and it is still Fast; `GET /api/settings` says `fast`.
4. Click Lean, then Rescan now: the leaf and the progress bar replace the button until it finishes. With Fast set, Rescan now shows "Scanning fast" without the leaf.
5. During a Lean refresh, flip to Fast: the leaf disappears and the label becomes "Scanning fast" (takeover). Flip back to Lean: the fast refresh finishes.
6. Stop the server and flip the switch: an inline error appears under it and no tick.
7. AI tab → add or edit a provider: no "Redact paths" toggle.
8. In DevTools → Network, switch to another window and back, then minimize the browser and restore it: `/api/presence` is posted with `looking: false` on blur and `true` on focus, and the `/api/status/stream` request ends while the page is hidden and a new one starts when it is shown, without the header's "offline" badge.

### UI-FIRST-RUN: First-run AI setup
**Applies to:** hoardini-ui, hoardini-server
**Verification:** manual

While `meta.ai_setup_done` is unset and no provider exists, a pop-up asks how Hoardini should think, with five choices in this order: Google Gemini, OpenRouter, Cerebras, A model on this PC, Set up a provider. No choice carries a Recommended badge until one is picked. Each cloud choice has three steps: "Get a free key" (opens the provider's key page in the system browser), a key field (saved through the vault), and an automatic test with the model preselected. "Later" sets done; the chat then shows a one-line "No AI is set up yet. Set up AI" notice while no provider is active. The AI tab has "Run setup again". While a 429 wait runs, a "Waiting on the free tier..." notice counts down.

**Verification (manual):**
1. Start with `HOARDINI_DATA_DIR` set to an empty scratch folder. The pop-up shows with the five choices; the first index scan runs behind it.
2. Gemini, OpenRouter, Cerebras each: Get a free key opens the system browser (not an in-app window); paste a key; Save and test ends "Working" and the provider is active with its default model. A wrong key shows the provider's error and can be retried.
3. A model on this PC with LM Studio running lists its models and Use this makes it active; with nothing running it shows the LM Studio link and Check again.
4. Set up a provider closes the pop-up and opens the AI tab with the add form.
5. Later closes it; the "Set up AI" notice appears under the header and reopens the pop-up. After a restart the pop-up does not return.
6. An existing data dir with a provider never shows the pop-up. AI tab, Run setup again, reopens it.
7. Against a mock answering 429 with `Retry-After: 5` then 200, the notice counts down from 5 and the answer then arrives.

### UI-TIP-LINE: Tip line under the brand
**Applies to:** hoardini-ui, hoardini-app
**Verification:** manual

Under "made to vanish: X" the header shows "♥ Like this? **Tip** my local NPR station", copied from BlabberStack. Only "Tip" is a link, to `https://www.kuaf.com/donate`; the heart uses the accent color, the text is muted and small. The tooltip reads "KUAF, the maintainer's local NPR member station. Not affiliated with Hoardini." It is hidden below the `sm` breakpoint (640 px), where the header wraps. In the desktop app, links that open a new window (`target="_blank"`) and navigations away from the loopback server open in the system browser, routed in Rust (`on_new_window`, `on_navigation` with `tauri-plugin-opener`); the page gets no IPC (UI-NO-IPC).

**Verification (manual):**
1. Text, heart color, and tooltip match the above; hover shows the tooltip.
2. Click Tip in `cargo run -p hoardini-app`: the default browser opens kuaf.com/donate and no Hoardini window appears.
3. Resize the window under 640 px wide: the line disappears.
4. `typeof window.__TAURI__` is still `"undefined"` and `invoke('plugin:opener|open_url', ...)` still rejects (no capability).

### UI-THEME-CONTRAST: Every theme is readable
**Applies to:** hoardini
**Verification:** test

Every theme shall meet WCAG 2.x contrast for its own tokens. The on-accent color (text on accent-colored buttons and pills) is whichever of the theme's bg0 or text contrasts more with its accent.

**Acceptance criteria:**
- `text` and `textDim` on each of bg0, bg1, and bg2 → ratio ≥ 4.5
- `muted` on bg1 → ratio ≥ 4.5
- `faint` on bg1 → ratio ≥ 3
- on-accent on `accent` → ratio ≥ 4.5
- `themeVars()` sets `--color-on-accent` to that computed color

Test: `specs/ui/tests/theme-contrast.test.ts`.

### UI-THEME-TAB: Theme tab with group-first picker
**Applies to:** hoardini
**Verification:** manual

The drawer has its own Theme tab. "Follow system" sits above a Dark / Midtone / Light switch, and only the chosen group's swatches show. The tab opens on the active theme's group (tone is checked before mode, so Kraft and Sage Chalk are Midtone). Following the system picks Marshmallow or Marshmallow Night and never a midtone. A pick applies instantly with no Save button and survives a reload. All theme fonts are bundled, so switching themes makes no network request.

**Verification (manual):**
1. In `ui/`, run `bun run dev --port 52xx --strictPort` against a mock or scratch-data server; open the Theme tab.
2. Confirm the Dark / Midtone / Light switch shows first, the tab opens on the active theme's group, and each group lists only its own swatches (Midtone has 22, Tater Dog is under Light).
3. Click a swatch: the UI retints at once and no Save button exists. Reload: the same theme is active and its group is open.
4. Pick Kraft: it is listed under Midtone. Pick Follow system: the active theme is Marshmallow or Marshmallow Night per the OS setting.
5. With the Network panel open (cache disabled), pick a theme with a non-default font (for example Space Grotesk): font files load only from the app's own origin, and none from another host.
6. On Rosewood, an accent-filled button or pill shows readable text.

### UI-CHANGES-CHART: The changes card draws free space over time
**Applies to:** hoardini
**Verification:** manual

The `changes` tool card shall show the free-space chart above its usual content when the call's range holds at least
3 refreshes, and only the numbers otherwise. The points come from `GET /api/history` limited to the `range_ms` line
in the tool result, so the chart ends at the call's own latest refresh and a reload or scrolled-back card shows the
same data. The `range_ms` line is not shown in the card's result text.

**Verification (manual):**
1. With an index holding at least 3 refreshes, ask "How has my free space changed over time?". The `changes` card shows
   a "Free space over time" chart above the collapsed tool row; hovering a dot shows its date and free space.
2. Reload the page: the same chart is drawn from the stored tool result.
3. Run another refresh, then scroll back to that card: it still ends at the refresh the call saw.
4. With only 2 refreshes in the index, ask again: the card shows no chart, only the numbers.
