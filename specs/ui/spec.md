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

The chat shall show each run in the order it happened: text before a tool round, the round's chips (name, short argument, elapsed time), then the text after it, each text segment in its own paragraph. Stop ends a run at once and keeps the question on screen. A run that ends at the tool-step cap (`run_stats.limit_reached`) offers "Keep going"; a model that is silent for the profile's timeout gets the "Keep waiting / Stop" prompt. A page load opens the one conversation (`GET /api/thread`) with the same messages, chips, and card states.

**Verification (manual):**
1. Point a provider at a scripted OpenAI-compatible mock (or a real model) and ask for something that calls a tool without text first, then answers: the chip renders above the answer, not below it.
2. A turn of "Let me look." + a tool call + "Here is the answer." renders as two paragraphs with the chip between them.
3. Ask for a long streamed answer and press Stop: streaming stops at once, the question stays, no "no content" prompt appears, `GET /api/status` shows `running_runs: 0`, and a new question answers normally.
4. With `max_tool_steps = 3` and a model that always calls a tool: three rounds of chips, then "Hoardini used its 3 tool steps for this turn before finishing." with Keep going; Keep going sends "Continue.".
5. With a model that takes longer than the timeout to answer: "No response from the model for 2 minutes." with Keep waiting and Stop.
6. Reload the page: the conversation opens by itself, and messages, chips, and every proposal card's status are as before.
7. Leave a proposal card pending, reload, and approve or decline the card: the decision is recorded and a new run starts from the answer (a `tool_*` message for the proposal's tool call), so the agent continues in the same chat.
(2026-10-08: steps 1–4 and 6 pass against a scripted mock; 1 and 5 against LM Studio `minicpm5-2b`.)

### UI-CONTEXT-BOUNDARY: The chat shows where Hoardini's memory of it starts
**Applies to:** hoardini
**Verification:** manual

The chat is one continuous scrollback. When messages lie before the model's window (`has_hidden`), the chat shall draw the divider "Hoardini doesn't remember above this line" before the first visible message at or after `window_start_id`, or at the end of the scrollback when `window_start_id` is null. **Clear chat** sits left of the message box (eraser icon; tooltip "Clear the chat: Hoardini starts fresh, and earlier messages stay saved"). It calls `POST /api/thread/fresh` and empties the screen: the model forgets the chat so far, and the earlier messages stay stored behind a "Show earlier chat (N messages)" link at the top of the scrollback, which toggles to "Hide earlier chat". It is disabled while a run streams or the server is offline. Deleting messages for good is Settings' Clear chat history. There is no Recent chats button. The thread is fetched again after each run ends and after Clear chat, so the divider follows trims and session gaps.

**Verification (manual):**
1. Start `hoardini serve` with `HOARDINI_DATA_DIR` and `--db` in a scratch folder and a provider pointing at a scripted OpenAI-compatible mock; open the UI.
2. The eraser (Clear chat) sits left of the message box; the header shows only Settings, with no New chat or Recent chats button.
3. Ask two questions. No divider is shown. Press Clear chat: the screen empties, the welcome card shows, and "Show earlier chat (4 messages)" appears at the top. Click it: both turns come back with the divider below them; click "Hide earlier chat" to hide them again.
4. Ask a third question: the model request carries only that question (check the mock's captured request); with the earlier chat shown, the divider sits right above it.
5. Reload the page: the same scrollback opens with the divider in the same place.
6. Set every stored message's `created_ms` 7 hours back in the database, reload, and ask again: the divider moves above the new question.
(2026-10-09: steps 1–6 passed against `hoardini serve` with a scripted mock model, driven by Playwright, when the button was Start fresh; steps 2–4 were rewritten for Clear chat the same day and have not been re-run.)

### UI-WELCOME-BACK: The greeting welcomes the user back at a session start
**Applies to:** hoardini
**Verification:** manual

The greeting card (index age, free-space change, biggest movers, duplicates, and the starters) shall show at the end of the scrollback, above the composer, whenever `GET /api/thread` reports `fresh`: on first launch, when the last message is more than 6 hours old, and right after Clear chat. It hides as soon as a question is sent. Its starters are Show hotspots, Show duplicates (when there are duplicates), and **Free space over time**, which asks `How has my free space changed over time?`. In the review layout (REVIEW-TRIGGER) this card becomes the review header, with the same content: the drive, its used and free space, and "Ready to free" (the sum of the ticked items of the pending review cards), taken from the `header` of `POST /api/review/open` (`root`, `used_bytes`, `free_bytes`, `ready_bytes`). The data side exists; the header itself is not built yet, so the steps below still check the greeting card.

**Verification (manual):**
1. On a new scratch database with an index, open the UI: the card shows with Show hotspots and Free space over time.
2. Press Free space over time: the question `How has my free space changed over time?` is sent and the card disappears.
3. After the answer the card stays hidden. Press Clear chat: the screen empties and the card appears.
4. Set every stored message's `created_ms` 7 hours back and reload: the card shows at the end of the scrollback.
(2026-10-09: steps 1–4 pass in the same session as UI-CONTEXT-BOUNDARY.)

### UI-CLEAR-HISTORY: Clear chat history deletes only the chat text
**Applies to:** hoardini
**Verification:** manual

`ClearHistorySection` (rendered in the Settings tab) shall offer **Clear chat history** behind a confirm step ("Delete every chat message?" with Delete messages and Cancel). Confirming calls `DELETE /api/thread`, then reopens the conversation, so the chat is empty and the welcome-back card shows. Memories, proposals, jobs, and the archive list stay. The button is disabled while a run streams; an error from the server (409 while a run is in progress) shows inline.

**Verification (manual):**
1. With a few messages, a saved memory, and a decided proposal, open Settings and press Clear chat history: the confirm step appears; Cancel leaves everything as it was.
2. Press it again and confirm: "Cleared" shows, the chat is empty with the welcome-back card and no divider.
3. The Memory tab still lists the memory, `GET /api/proposals/<id>` still returns the proposal, and the next question starts a normal chat.
(2026-10-09: steps 1–3 pass with the section rendered in the Settings tab for the check; the memory was read back through `GET /api/memory` and the proposal row through the database.)

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

The header mascot is a sumo wrestler whose body girth tracks the used fraction (1 - free/total) of the selected drive (UI-DRIVE-SELECTOR; else the `C:\` volume, else the first watched volume), growing continuously and steeply near full, and his tooltip names that drive. While an agent run is in progress he performs the shiko stomp, raising and stomping the left leg, then the right, repeatedly. With `prefers-reduced-motion` he holds a static pose.

**Verification (manual):**
1. In `ui/`, run `MOCK_FILL=0.30 bun run mock` and `bun run dev`; open http://localhost:5193/.
2. Confirm the mascot is slim and his tooltip reads "Hoardini, a sumo wrestler - C: is 30% full".
3. Run `curl -X POST "http://127.0.0.1:7043/api/mock?token=dev" -d '{"fill":0.97}'`; confirm he smoothly widens to near the edges of his box without the header layout moving.
4. Post `{"fill":0.6}`; confirm he slims back down, and that 90% looks visibly chubbier than 60%.
5. Post `{"running_runs":1}`; confirm he alternately lifts and stomps left then right, with dust at each stomp. Post `{"running_runs":0}` to stop.
6. Enable reduced motion in the OS or browser; confirm no looping animation.
7. Open the drive menu and click D:: he slims to D:'s fullness and the tooltip reads "D: is 56% full". Click C: again: he widens back.
(2026-10-09, step 7: headless Chromium against the mock; the tooltip went from "C: is 97% full" to "D: is 56% full" and the girth followed.)

### UI-MASCOT-POKE: The sumo reacts when poked
**Applies to:** hoardini
**Verification:** manual

The header sumo is a button whose accessible name ends in "Poke me!". A click, or Enter or Space while he has focus, plays one reaction and then returns him to the state he was in (idle, salt, stomp, or celebrating); his announced state does not change, and poking never triggers anything else in the app. The reaction to a single poke is a jump whose height falls as his girth grows: skinny, a high jump with a tuck; middling, a hop; fat, he barely leaves the ground, lands with a thud and dust, and the header jolts 1–2 px; at the girth cap (about 97% full or more) he crouches and strains with a sweat drop and a creaking belt and does not jump. Pokes less than 1.5 s apart count as a combo: the 3rd is a shiko (one leg high, slammed down with dust) and the 6th or later makes him dizzy (stars, wobble, plop onto his bottom, about 2.5 s of sulking with arms crossed, then up). With `prefers-reduced-motion` a poke gives a happy-eyes blink and a 6-poke combo a still sulking face; nothing moves. While an approved job runs his girth stays at its pre-job size; when it ends he deflates to the new size over about 1.5 s with puffs of air. After at least 3 quiet minutes (no click or key press) he sometimes yawns, scratches his belly, or stretches, but only while the window is focused and visible: the one fidget timer exists only then.

**Verification (manual):**
1. In `ui/`, run `MOCK_IDLE=1 MOCK_FILL=0.30 bun run mock` and `bun run dev`; open http://localhost:5193/.
2. Hover the sumo: the tooltip reads "Hoardini, a sumo wrestler — C: is 30% full, relaxing. Poke me!". Click him once: a high jump with tucked legs and arms flung up. Nothing else in the app changes.
3. `curl -X POST "http://127.0.0.1:7043/api/mock?token=dev" -d '{"fill":0.8}'`, wait 2 s, click: a lower hop. Post `{"fill":0.93}`, click: a small, slow hop that lands with dust and nudges the header down. Post `{"fill":0.985}`, click: he crouches with gritted teeth, a sweat drop, and creak marks at the belt, and his feet stay down.
4. Post `{"fill":0.5}`. Click 3 times quickly: shiko. Click 6 times quickly: stars circle his head, he wobbles, plops down, sulks with crossed arms, and gets up after about 4 s. Click again 2 s later: a normal jump.
5. Press Tab until he has a focus ring, then press Enter and then Space: each plays a jump.
6. Turn on reduced motion in the OS or browser and reload. Click: his eyes turn happy for a moment and nothing moves. Click 6 times quickly: a still sulking face for about 2 s.
7. Approve a delete card (UI-MASCOT-JOBS step 2). His girth stays put while the job runs; after the finisher he slims to the new size over about 1.5 s with puffs of air at his sides.
8. Leave the window focused and idle without clicking or typing for 4 minutes: he yawns, scratches his belly, or stretches once. With DevTools, wrap `window.setTimeout` before load to count pending timers of 170 s or more: there is exactly one while the window is focused, none after it loses focus or is minimized, and one again when it is focused.
(2026-10-09: steps 2–6 and 8 checked against the mock in headless Chromium through Playwright; step 8 used a fake clock for the 3 minutes, timer counts 1/0/1/0/1 across focus, blur, focus, hidden, visible.)

### UI-MASCOT-JOBS: The sumo does the job himself
**Applies to:** hoardini
**Verification:** manual

When an approved job from a proposal card starts running and the card's action footer (the area holding the Hold and Approve buttons, then the progress bar and result) is on screen, the header sumo hides and a travelling copy climbs down a ladder from the header to stand on the footer's top edge (1.6 s; a fat sumo bends the rungs). If the footer is not on screen but the card's top edge is, he stands on the top edge instead. When the parent card collapses mid-trip (§ UI-PROPOSAL-COLLAPSE) he moves to the follow-up card's top edge, or to the collapsed line if there is no follow-up, and is never left over nothing. A parcel pops out of the card next to him with a file or folder icon, the first item's name, the size, and "×N" for several items. The overlay never moves the card. He stands above the edge he is anchored to, never over the buttons. Whatever the job's speed, the beats last at least this long: working 2 s (even if the job finished during the climb down), the finisher about 1.8 s, the climb back 1.4 s, then pleased 3 s; about 10 s in all. While the job runs he works the parcel in a loop, and when it finishes he plays the move for its kind:

| Job | Move | Parcel |
|---|---|---|
| Delete | oshidashi: charge and belly bump | flies off the right edge, spinning |
| Compress | belly flop onto it | squashes into a pancake, then hops back into the card |
| Archive to a local or SMB destination | overarm throw | lands in a cardboard box whose lid shuts |
| Archive to an rclone destination | the throw aimed up | sails into a cloud, which puffs |
| Dedupe | kashiwade clap | the twin parcels are clapped into one |
| Failed | slips, lands on his bottom, rubs his head | stays put, then goes back into the card |
| Cancelled | stops and shrugs | hops back into the card |

He then climbs back up, deflating on the way if space was freed, and in the header is pleased with himself for 3 s (chin up, eyes closed, arms crossed, a pat-pat of the hands); the rest of that celebration is skipped. A declined proposal gets no trip, only a head shake in the header. If neither the footer nor the card's top edge is on screen when the job starts, or the page is hidden, there is no trip: he stomps in the header while the job runs (at least 2 s), plays the move there when it ends, then celebrates for 3 s. After about 30 s of work he climbs back and the move plays in the header when the job ends. Only one trip happens at a time; a job that starts while he is out runs without one. After 5 trips within 10 minutes later jobs get the header version. Clicking anywhere sends him home quickly, and the job carries on from the header. With `prefers-reduced-motion` there is no trip and no parcel: he holds the still stomp pose while the job runs and shows the pleased face when it succeeds. The overlay never takes pointer input, and the mascot never delays or drives a job.

**Verification (manual):**
1. In `ui/`, run `MOCK_IDLE=1 MOCK_FILL=0.9 bun run mock` and `bun run dev`; open http://localhost:5193/ at about 1000 × 800.
2. Ask "what is in lmstudio", wait for the Delete card, and hold "Hold to delete". The header sumo vanishes and a copy climbs down a ladder whose rungs sag under him, lands on the card's footer, and a parcel ("llama-3.3-…", "52.4 GB", "×2") pops out beside him. He pushes it while the progress bar runs, then charges and bumps it off the right edge, spinning. He climbs back, slimming with puffs of air, and stands in the header with arms crossed and eyes closed for 3 s, then idles.
3. Before each new card's approval, post one of these to `http://127.0.0.1:7043/api/mock?token=dev` (the mock changes the kind when you approve, so the card's heading switches then), ask again, and approve:
   - `{"next_job":{"kind":"compress"}}`: he hops onto the parcel and bounces on it, then belly flops and the parcel goes flat.
   - `{"next_job":{"kind":"archive"}}`: he windmills his arm, then lifts the parcel overhead and throws it into a box that pops up to his right; the lid shuts.
   - `{"next_job":{"kind":"archive","cloud":true}}`: the same throw goes up into a cloud, which puffs.
   - `{"next_job":{"kind":"dedupe"}}`: two parcels, arms spread, then a clap that merges them into one, held up over his head.
   - `{"next_job":{"outcome":"failed"}}`: he slips onto his bottom with crossed-out eyes and rubs his head; the parcel stays, then sinks back into the card; the card shows the error.
   - `{"next_job":{"outcome":"cancelled"}}`: the parcel hops back into the card and he shrugs.
4. Press Cancel on a pending card: no trip; the header sumo shakes his head.
5. Make the window very short (about 200 px) so neither the footer nor the card's top is on screen, and approve: no trip. He stomps in the header for at least 2 s, plays the move there at the end, then celebrates for 3 s.
6. Post `{"next_job":{"ms":40000}}` and approve: after about 30 s he climbs back and stomps in the header; at the end the move plays in the header.
7. During a trip, click anywhere on the page: he climbs home at once, the click still lands, and the job finishes with the move in the header.
8. Approve 7 jobs within 10 minutes: the first 5 get trips, the rest the header version.
9. Turn on reduced motion and approve: no ladder or parcel; the header sumo holds the stomp pose while the job runs and shows the pleased face at the end.
10. During a trip, type in the chat box and scroll the chat: input works, he stays on his anchor as it moves, and the progress bar and the job's finish time are the same as with no trip.
11. Ask "what is in lmstudio" (the mock card has 15 items, so it is taller than the window), scroll so the Hold button is on screen and the card's top is not, and approve with `{"next_job":{"ms":300}}` posted first: he still travels, standing on the footer. The job ends during the climb; he still works for at least 2 s, plays the finisher (about 1.8 s), climbs back (1.4 s), and is pleased for 3 s. Timing the whole thing from the hold release gives about 10 s.
12. During step 11 the old card collapses and the follow-up appears; he moves to stand on the follow-up card's top and is never over empty space.
(2026-10-09: steps 2–10 checked against the mock in headless Chromium through Playwright, with compress, dedupe, failed, cancelled, and long jobs also driven through the store directly. Steps 11 and 12 checked the same way: trip from the footer with the card top off screen, sumo visible throughout, push at 2.9 s, finisher at 5.0 s, climb back at 6.8 s, home at 8.2 s with a 300 ms job.)

### UI-PROPOSAL-COLLAPSE: Finished cards collapse and a follow-up card follows
**Applies to:** hoardini
**Verification:** manual
**Depends on:** ACT-FOLLOW-UP (server side; the follow-up proposal and `parent_id` / `follow_up_id`)

A proposal card whose proposal was declined, or whose job ended (done, partial, failed, or cancelled), collapses to one line: the kind's verb (Deleted, Archived, Compressed, Deduplicated), the item count, and the bytes freed, e.g. "Deleted 12 items · freed 3.4 GB · details ▸"; a decline reads "Declined · N items". Clicking it expands the read-only item list with each item's outcome, and "hide ▴" collapses it again. Pending and running cards stay full size. When the proposal has a `follow_up_id`, the follow-up's full card renders directly under the collapsed card in the same chat slot; chains repeat. The UI refetches the proposal when its job reaches a terminal state, and a reloaded thread renders the same chain. Item rows show a muted "small" tag when `small` is true and a muted "Noted: ..." line from `memory_note`; the initial ticks are the server's `selected` values. Approving or declining a follow-up card or a review card (one with a `review_id`, REVIEW-BUILD) does not report back to the model; the model sees it through the open-proposals line.

**Verification (manual):**
1. In `ui/`, run `MOCK_PORT=7261 MOCK_IDLE=1 bun run mock` and Vite with its `/api` proxy pointed at it; ask "what is in lmstudio".
2. The card lists 15 items: the "tiny-test.gguf" row has a "small" tag and "Noted: C:\Users\josha\.lmstudio is disposable"; it, "keep", and the blocked item are unticked and the cache blobs are ticked.
3. Approve. When the job ends, the card becomes "Deleted 12 items · freed 53.6 GB · details ▸" and a new full card appears directly under it holding the 3 items that were not done, all unticked.
4. Click "details ▸": the original items show with "done" outcomes. Click "hide ▴".
5. Reload the page: the same collapsed card and follow-up are shown.
6. On a fresh mock, post `{"next_job":{"outcome":"failed"}}` and approve: the card collapses as "Deleted nothing · failed: Access is denied"; the follow-up holds the 12 failed items ticked plus the 3 unticked ones.
7. Decline a pending card: it collapses to "Declined · N items".

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
(2026-10-09, partial: headless Chromium against `hoardini serve --port 7231 --ui-dir ui/dist` with scratch data. Steps 2 and 3 pass; Rescan now in Lean started a `background` refresh. Step 7 was checked in code only, since no provider form was opened. Step 8 passed with a simulated `visibilitychange`: the stream ended and reopened with no badge, and presence posted `false` then `true`. Steps 5 and 6 were not run; SRV-ONE-REFRESH covers the takeover server-side.)

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

Right-aligned under the header's Settings icon, the header shows "♥ Like this? **Tip** my local NPR station", copied from BlabberStack. Only "Tip" is a link, to `https://www.kuaf.com/donate`; the heart uses the accent color, the text is muted and small. The tooltip reads "KUAF, the maintainer's local NPR member station. Not affiliated with Hoardini." It is hidden below the `sm` breakpoint (640 px), where the header wraps. In the desktop app, links that open a new window (`target="_blank"`) and navigations away from the loopback server open in the system browser, routed in Rust (`on_new_window`, `on_navigation` with `tauri-plugin-opener`); the page gets no IPC (UI-NO-IPC).

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
in the tool result, for the drive that line names, so the chart ends at the call's own latest refresh and a reload or scrolled-back card shows the
same data. The `range_ms` line is not shown in the card's result text.

**Verification (manual):**
1. With an index holding at least 3 refreshes, ask "How has my free space changed over time?". The `changes` card shows
   a "Free space over time" chart above the collapsed tool row; hovering a dot shows its date and free space.
2. Reload the page: the same chart is drawn from the stored tool result.
3. Run another refresh, then scroll back to that card: it still ends at the refresh the call saw.
4. With only 2 refreshes in the index, ask again: the card shows no chart, only the numbers.

### UI-DRIVE-SELECTOR: The header shows one drive, picked from a menu
**Applies to:** hoardini
**Verification:** manual

The header shall show only the selected drive's free-space bar (INDEX-VOLUMES), so drives never stack. Its drive letter is a dropdown button (`C: ▾`) that opens a menu of every fixed drive (`GET /api/volumes`) with its letter, label (or "Local Disk"), a free-space bar, and "N free of M". A watched folder (a test setup) shows as `…\<last segment>`, with the full path as its tooltip.

- A watched drive has a check. Clicking it selects it (`PUT /api/volumes`) and closes the menu; the header bar, the mascot's girth (UI-MASCOT-FULLNESS), and the welcome-back card (UI-WELCOME-BACK, through `/api/index/summary`) follow it.
- Any other drive has **Add**, which watches and selects it and closes the menu; its first scan starts at once, fast.
- Each watched drive has **Stop watching** behind a confirm step ("Stop watching D:? Its index and history are deleted. Your files stay." with Stop watching and Cancel). For the last watched drive it is disabled, with the tooltip "At least one drive stays watched". Stopping the selected drive selects the first one left.
- A drive that is not connected is greyed out and says "not connected" in place of its bar.
- A drive without a change journal (FAT, exFAT) says "no change journal: every refresh is a full scan".
- A server error shows inline at the bottom of the menu.

Keyboard: Enter or Space on the button opens the menu with focus on the selected drive. Arrow Up and Down, Home, and End move between the menu's buttons. Escape cancels an open confirm step (focus returns to that row's Stop watching), else closes the menu and returns focus to the button. A click outside the menu or tabbing out of it closes it.

**Verification (manual):**
1. In `ui/`, run `MOCK_IDLE=1 bun run mock` and `bun run dev`; open http://localhost:5193/. The header shows one bar: `C:` and its free space.
2. Click `C: ▾`. The menu lists C: Windows and D: Data with checks, E: Camera with Add and the no-change-journal note, and F: Old backup greyed out as "not connected". Focus is on C:.
3. Press Arrow Down: focus moves to C:'s Stop watching. Press Escape: the menu closes and focus is back on the button. Open it again and click outside: it closes.
4. Open it and click D:. The menu closes, the header shows `D:` and its bar, the mascot slims to D:'s fullness (tooltip "D: is 56% full"), and the welcome-back card names D:.
5. Open it and press Add on E:. The header shows `E:`, and a fast scan starts ("Scanning fast").
6. Open it and press Stop watching on D:. The confirm step appears with focus on Cancel, and Escape cancels it. Press it again and confirm: D: loses its check and shows Add.
7. Stop watching every drive but one: the last one's Stop watching is disabled, with "At least one drive stays watched".
8. Start `hoardini serve --port 72xx --token dev --no-refresh` against a scratch `HOARDINI_DATA_DIR`, with Vite proxying to that port. Use `PUT /api/volumes` to watch three scratch folders, then rename one. The menu shows it greyed as "not connected", and Rescan now skips it (the log says `volume not connected; skipped`). Stop watching the selected folder: the log says `stopped watching; its index is deleted`, and the next folder becomes selected.

(2026-10-09: steps 1–7 pass against the mock in headless Chromium on ports 7291 and 5291. Step 8 passes against `hoardini serve --port 7291 --no-refresh` with three scratch folders: the renamed one showed "not connected" and was skipped by a refresh, and stopping the selected one deleted its 3 index rows and selected the next. The machine's `subst` drives (X:, Y:, Z:) were not listed.)
