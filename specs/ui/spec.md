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

### UI-FIRST-RUN: First-run AI setup
**Applies to:** hoardini-ui, hoardini-server
**Verification:** manual

While `meta.ai_setup_done` is unset and no provider exists, a pop-up asks how Hoardini should think, with five choices in this order: Cerebras (recommended), Google Gemini, OpenRouter, A model on this PC, Set up a provider. Each cloud choice has three steps: "Get a free key" (opens the provider's key page in the system browser), a key field (saved through the vault), and an automatic test with the model preselected. "Later" sets done; the chat then shows a one-line "No AI is set up yet. Set up AI" notice while no provider is active. The AI tab has "Run setup again". While a 429 wait runs, a "Waiting on the free tier..." notice counts down.

**Verification (manual):**
1. Start with `HOARDINI_DATA_DIR` set to an empty scratch folder. The pop-up shows with the five choices; the first index scan runs behind it.
2. Cerebras, Gemini, OpenRouter each: Get a free key opens the system browser (not an in-app window); paste a key; Save and test ends "Working" and the provider is active with its default model. A wrong key shows the provider's error and can be retried.
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
