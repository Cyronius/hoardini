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
| Online-only | the throw aimed up | sails into a cloud, which puffs |
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

A proposal card whose proposal was declined, or whose job ended (done, partial, failed, or cancelled), collapses to one line: the kind's verb (Deleted, Archived, Compressed, Deduplicated, Cleaned up, Made online-only), the item count, and the bytes freed, e.g. "Deleted 12 items · freed 3.4 GB · details ▸"; a decline reads "Declined · N items". Clicking it expands the read-only item list with each item's outcome, and "hide ▴" collapses it again. Pending and running cards stay full size. When the proposal has a `follow_up_id`, the follow-up's full card renders directly under the collapsed card in the same chat slot; chains repeat. The UI refetches the proposal when its job reaches a terminal state, and a reloaded thread renders the same chain. Item rows show a muted "small" tag when `small` is true, a muted "Noted: ..." line from `memory_note`, and a muted "Copy: ..." line from `copy_note` (EVID-COPY-CHECK); the "Capped to ... by a safety rule" line appears for a `review` or `keep` cap, and for any other cap only when no Copy line explains it; the initial ticks are the server's `selected` values. Approving or declining a follow-up card or a review card (one with a `review_id`, REVIEW-BUILD) does not report back to the model; the model sees it through the open-proposals line.

**Verification (manual):**
1. In `ui/`, run `MOCK_PORT=7261 MOCK_IDLE=1 bun run mock` and Vite with its `/api` proxy pointed at it; ask "what is in lmstudio".
2. The card lists 15 items: the "tiny-test.gguf" row has a "small" tag and "Noted: C:\Users\josha\.lmstudio is disposable"; it, "keep", and the blocked item are unticked and the cache blobs are ticked.
3. Approve. When the job ends, the card becomes "Deleted 12 items · freed 53.6 GB · details ▸" and a new full card appears directly under it holding the 3 items that were not done, all unticked.
4. Click "details ▸": the original items show with "done" outcomes. Click "hide ▴".
5. Reload the page: the same collapsed card and follow-up are shown.
6. On a fresh mock, post `{"next_job":{"outcome":"failed"}}` and approve: the card collapses as "Deleted nothing · failed: Access is denied"; the follow-up holds the 12 failed items ticked plus the 3 unticked ones.
7. Decline a pending card: it collapses to "Declined · N items".
8. Ask "windows cleanup" and approve the ticked operations: the card collapses to "Cleaned up 3 items · freed 12.6 GB · details ▸" and no follow-up card appears (system cleanups get none, ACT-FOLLOW-UP).
(2026-10-09: step 8 checked against the mock in headless Chromium through Playwright.)

### UI-SYSTEM-CLEANUP-CARD: The Windows cleanup card says what Windows will do and warns before what cannot be undone
**Applies to:** hoardini
**Verification:** manual
**Depends on:** ACT-WINDOWS-CLEANUP-DETECT (the `op` view of each item)

A `system_cleanup` card ("Clean up with Windows") shows one row per operation instead of a path row: a tick, the
operation's label, its estimated saving or "size unknown", a red "cannot be undone" tag for restore points and the
Recycle Bin, the server's explanation of what Windows will do, and the detection note. The header reads "free at least
X" while a ticked operation has no estimate. Under the list it says Windows asks for administrator permission once when
a ticked operation needs it; when restore points are ticked it says "Deleted restore points cannot be brought back.", and
when the Recycle Bin is ticked that emptying it is permanent; with either ticked, approving needs "Hold to clean up"
(the `HoldButton` of ACT-DELETE-CONFIRM), otherwise one click on "Clean up". The `propose_system_cleanup` tool call
renders this card in the chat like the other `propose_*` tools.

**Verification (manual):**
1. In `ui/`, run the mock (`MOCK_IDLE=1 bun run mock`) and Vite with its `/api` proxy pointed at it; ask "windows cleanup".
2. The card reads "Clean up with Windows · free at least 12.6 GB" and lists five operations; component store, hibernation, and compaction are ticked; restore points and the Recycle Bin are unticked and tagged "cannot be undone"; hibernation shows 12.6 GB, the others "size unknown" except the Recycle Bin (3.1 GB); each row shows its explanation and note.
3. The footer says Windows asks for administrator permission once, and the button is "Clean up".
4. Tick restore points: the footer adds "Deleted restore points cannot be brought back." and the button becomes "Hold to clean up"; a click leaves the card pending.
5. Untick it and click "Clean up": the job runs and the card collapses to "Cleaned up 3 items · freed 12.6 GB · details ▸"; "details ▸" shows "done" on the three approved rows.
(2026-10-09: steps 1–5 checked against the mock in headless Chromium through Playwright; ticking the Recycle Bin was not checked.)

### UI-SETTINGS-AUTOSAVE: Settings save themselves; one Rescan now control
**Applies to:** hoardini
**Verification:** manual

The Settings tab shall have no Save button. A toggle or switch saves when it changes; a text field saves when it loses focus or 500 ms after the last keystroke (`drawer/autosave.tsx`). A "Saved" tick appears beside the control that saved, or the server's error appears inline under it and the control shows the stored value again. The tab holds the Fast / Lean switch ("Slower, stays out of your way" / "Finishes sooner, uses more CPU"; SRV-SETTINGS `index_speed`), the read-only index size, the Online checks switches (Hugging Face, winget; turning one on or off also records that the user answered, SRV-SETTINGS `online_checks`), and Clear chat history; the threshold, worker, memory-cap, redaction, and web-search settings are gone. The AI tab's provider form has no "Redact paths" toggle (AGENT-REDACT). The header shows one small "Rescan now" button while the index is idle, which runs a refresh at the switch's speed (SRV-ONE-REFRESH); while a refresh runs, its progress bar takes the button's place, with the leaf icon marking Lean, and there is no Scan fast button.

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

### UI-ONLINE-CONSENT: Online copy checks are asked about once
**Applies to:** hoardini
**Verification:** manual

While `online_checks.asked` is false and the last copy-check pass found candidates waiting on an online check
(`GET /api/copy-checks/waiting` above 0), a notice above the chat (`components/OnlineChecksConsent.tsx`, placed like the
AI notices) asks: "Check big files against Hugging Face and winget?", says how many big files wait, that Hoardini sends
file names and package or repo names and never file contents or hashes (the published hash is fetched and compared on
this PC), and that checks against the user's own copies, git, and cloud sync always run. Allow turns both services on,
Not now turns both off, and either records the answer (`asked` true) so the notice never returns; Choose in Settings opens
the Settings tab, whose switches also count as an answer. The component takes an optional `waiting` count, so the review
can host it later. The notice polls the waiting count every 30 s while unanswered.

**Verification (manual):**
1. In `ui/`, run the mock on a free port (`MOCK_PORT=7343 MOCK_IDLE=1 bun run mock`) and Vite with its `/api` proxy pointed at it; open the page.
2. The notice shows "Check big files against Hugging Face and winget?" and "4 big files".
3. Choose in Settings: the Settings tab shows Online checks with Hugging Face and winget off. Turn Hugging Face on: "Saved" appears, `GET /api/settings` shows `asked` true and only `huggingface` on, and after a reload the notice is gone.
4. Reset the mock (`PUT /api/settings {"online_checks": {"asked": false, ...}}`), reload, press Not now: the notice goes and both services are off with `asked` true. Reset again and press Allow: both are on.
(2026-10-09: steps 1-4 pass, headless Chromium (Playwright) against the mock on port 7343 and Vite on 5293.)

### UI-ONLINE-ONLY-CARD: The online-only card
**Applies to:** hoardini
**Verification:** manual

An `online_only` proposal card (ACT-ONLINE-ONLY) is headed "Make online-only · free <size>", explains that it "Frees the
copy on this PC. Each file stays in its cloud folder and downloads again when you open it. Files not in sync yet are left
as they are.", shows each item's `synced` or `review` badge and its Copy line, and approves with one click on "Make
online-only" (it deletes nothing, so there is no hold). Finished, it collapses to "Made online-only N items · freed X"
(UI-PROPOSAL-COLLAPSE), and the mascot throws the parcel up into a cloud (UI-MASCOT-JOBS).

**Verification (manual):**
1. With the mock and Vite as in UI-ONLINE-CONSENT, ask "make my old onedrive photos online-only".
2. The card reads "Make online-only · free 44.3 GB" with the explanation, two `synced` items ticked with "Copy: synced to OneDrive", a `review` folder unticked with "Copy: 3 of 120 files not in sync with OneDrive yet", and a blocked `local.mp4` "Not in a cloud-synced folder"; there is no permanent-delete warning and no "Capped to" line.
3. One click on Make online-only approves; when the job ends the card reads "Made online-only 2 items · freed 44.3 GB · details ▸".
4. Ask "what is in lmstudio": the delete card's first item shows "Copy: identical to lmstudio-community/Llama-3.3-70B-Instruct-GGUF on Hugging Face".
(2026-10-09: steps 1-4 pass, headless Chromium against the mock; the mascot's move was not watched.)

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

Every theme shall meet WCAG 2.x contrast for its own tokens. The on-accent color (text on accent-colored buttons and pills) is whichever of the theme's bg0 or text contrasts more with its accent. The Explore treemap's colors (`explorePalette(tokens)`, `ui/src/explore/palette.ts`, UI-EXPLORE) come from the same tokens, and each block's label is the theme's text or bg0 when that reads at 4.5:1 on the block, else black or white; a theme in dark mode gets the dark steps of the type colors.

**Acceptance criteria:**
- `text` and `textDim` on each of bg0, bg1, and bg2 → ratio ≥ 4.5
- `muted` on bg1 → ratio ≥ 4.5
- `faint` on bg1 → ratio ≥ 3
- on-accent on `accent` → ratio ≥ 4.5
- `themeVars()` sets `--color-on-accent` to that computed color
- For every theme, the label of every Explore swatch (the six knowledge states, their folder strips, the 8 type colors, other, merged, the card map's plain block and the folder frame) reads at ≥ 4.5 on its fill, and the palette is dark exactly for dark-mode themes.

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
3 refreshes, and only the numbers otherwise. The points come from `GET /api/history` limited to the call's `data.range_ms`
for its `data.volume` (AGENT-TOOL-DATA), or, for an older result without data, to the `range_ms` line in the tool text and the drive that line names,
so the chart ends at the call's own latest refresh and a reload or scrolled-back card shows the
same data. The `range_ms` line is not shown in the card's result text or its raw view.

**Verification (manual):**
1. With an index holding at least 3 refreshes, ask "How has my free space changed over time?". The `changes` card shows
   a "Free space over time" chart under its headline (above the collapsed tool row for a result without data); hovering a dot shows its date and free space.
2. Reload the page: the same chart is drawn from the stored tool result.
3. Run another refresh, then scroll back to that card: it still ends at the refresh the call saw.
4. With only 2 refreshes in the index, ask again: the card shows no chart, only the numbers.

### UI-TOOL-CARDS: Folders, files and changes show as cards
**Applies to:** hoardini
**Verification:** manual

When a `top_dirs`, `top_files` or `changes` call has `data` (AGENT-TOOL-DATA: live from the `tool_data` event, after a reload from the thread's tool message), the chat shall show a card in place of the tool chip; a call without data (an older result, an error, no index) keeps the chip.
- **Folders and files cards:** a title ("Largest folders" or "Largest files") with the scope (the `under` folder, or "on your drives") and the files card's filters, the index age ("as of 3 days ago"), and ranked rows: name, the part of the path below the scope, size, a size bar relative to the largest row, and the file count (folders, with "some unreadable" when denied) or the last-modified date (files, last opened as the tooltip). The first 8 rows show, then "Show all N"; a footer "+N more · X" comes from `more`.
- **Map tab:** the folders card has List and Map tabs and opens on Map when the call's `view` is `map` (AGENT-TOOL-DATA), else on List. Map draws the card's outermost rows (a row under another listed row would count twice) as a treemap (UI-TREEMAP-LAYOUT), plus `more` as one dotted block when the call was one level deep, each block with its name and size in the theme's colors; clicking a block shows its name, size and share with Ask, Clean up and Show in Explorer. **Open in Explore** shows only when the shell provides an `onOpenExplore(path)` handler through `ExploreLinkContext` (`ui/src/explore/context.ts`); it opens the picked block, else the card's `under`, else the drive of its first row.
- **Changes card:** the headline is the free-space delta ("C: has 127 GB more free since 2:14 PM", "less free" when it fell; a time of day for a refresh earlier today, else the date), then the free-space chart (UI-CHANGES-CHART), "Freed X · Grew Y" (Freed sums removed and shrank rows, Grew sums grew and new rows, each row counted once with its listed ancestors), a Freed section (removed and shrank) and a Grew section (grew and new), each row with its signed delta, a bar, and "before → after", "new" or "removed". The volume root's own row is hidden, and a row under another row of the same section sits indented beneath it. Removed rows are struck through and have no buttons.
- **Row buttons** (shown on hover or keyboard focus, over the row's right end): ▸ opens the folder's ItemTree inline (UI-ITEM-TREE); **Ask** sends "What is {path} and is it safe to remove?"; **Clean up** (folders) sends "What can I clean up in {path}?"; **Show in Explorer** (SRV-REVEAL). Ask and Clean up are disabled while a run streams.
- A small **raw** toggle in the card's header shows the call's arguments and the tool text.

**Verification (manual):**
1. In `ui/`, run `MOCK_PORT=72xx MOCK_IDLE=1 bun run mock` and Vite with its `/api` proxy pointed at that port; open it at 1000 × 900.
2. Press Show hotspots. A "Largest folders on your drives" card shows 8 rows with bars and file counts and "Show all 10"; Windows says "some unreadable". A changes card reads "C: has 127 GB more free since 2:14 PM" with the chart and "Freed 124 GB · Grew 7.3 GB"; the `C:\` row is not listed; the Hugging Face model folder sits indented under `.cache` and the removed `deepseek` file under `models`, both struck through. No chip for `top_dirs`, `changes` or `suggest_next` shows.
3. Hover the `code` row: Ask, Clean up and Show in Explorer appear over the row's end and its file count hides. Press ▸: the ItemTree opens under it.
4. Press raw on the folders card: the arguments and the tool text show; press it again to hide them.
5. Press the "Clean up C:\code" pill (UI-FOLLOW-UPS), then Ask on the covenant-agent row of the new folders card: it sends "What is C:\code\covenant-agent and is it safe to remove?".
6. Ask "List the biggest files": the files card shows 8 rows with "modified" dates, "Show all 12", and "+3,184 more · 212 GB".
7. Reload: the same cards come back from the stored thread.
8. Repeat steps 2 and 6 with a dark theme (Marshmallow Night) and at 430 px wide: text, bars, and the green and red deltas stay readable, and rows do not overflow.
9. With `hoardini serve` and a model that calls `top_dirs`: the card renders live, and again after a reload from `GET /api/thread`.
10. Ask "Show me where my space goes": the folders card opens on its Map tab with a block per folder (Users, code, Games, Program Files, AI, Windows, ProgramData, …), names and sizes on the big ones. Click the code block: "code 312 GB · 18%" shows under the map with Ask, Clean up and Show in Explorer. Press List: the 8 rows show. No Open in Explore button shows, since the app has no handler yet.
11. With `ExploreLinkContext` providing a handler (a scratch page wrapping the app until the shell wires it), reload: the Map tab has Open in Explore, and pressing it with nothing picked calls the handler with `C:\`.
12. Repeat step 10 in Marshmallow Night at 430 px wide: the map fits the card and its labels read.
(2026-10-09: steps 1–8 pass against the mock in headless Chromium through Playwright, in Marshmallow and Marshmallow Night. Step 9 passes against `hoardini serve` on a scratch folder with a scripted OpenAI-compatible model calling `top_dirs`, `changes` and `suggest_next`: both cards rendered live and after a reload, ▸ listed the folder from `/api/index/children`, no chip showed, and no model request held card data.)
(2026-10-09: steps 10–12 pass against the mock in headless Chromium through Playwright, in Marshmallow at 1000 px and Marshmallow Night at 430 px; step 2 still shows 8 rows and "Show all 10" with the mock's new drive shape.)

### UI-ITEM-TREE: The ▸ tree under a folder
**Applies to:** hoardini
**Verification:** manual

`ItemTree` (`ui/src/components/ItemTree.tsx`; props `path`, optional `rootSize`, optional `actions(row)` slot) shall show a folder's children inline from `GET /api/index/children` (SRV-INDEX-CHILDREN), loading a level only when it is opened. Each row has ▸ when it has children, its name (full path as the tooltip), a size bar and its percentage of the parent (of `rootSize` at the top level, else of the folder), its file count, its last-modified date, the caller's buttons, and **Show in Explorer** (`POST /api/reveal`); a refusal shows inline on the row ("not found", "not on a watched drive") and keeps the row's buttons up. The buttons appear on hover or keyboard focus over the row's right end. Nested levels indent with a guide line. Under the rows come "N small files · X" when the folder has folded small files and "+N more · X" past 50 rows, which loads the rest when pressed; an empty folder says "Empty" and a failed load says why. As the card narrows, the date drops out first, then the file count, then the bar. Optional `selected`, `trail`, `onSelect(row)` and `onOpen(row)` make it a picker: with `onSelect` a row's name is a button that selects it (a double-click calls `onOpen`), the row whose path is `selected` is highlighted and scrolled into view, and the folders above it open by themselves, found by path or by `trail` (the folders on the way as the server spells them, since the profile shows as `~`). The chat cards (UI-TOOL-CARDS) and the Explore view (UI-EXPLORE) use it, and the review view is meant to.

**Verification (manual):**
1. Against the mock (UI-TOOL-CARDS step 1), press Show hotspots, then ▸ on `code` in the folders card.
2. The tree lists covenant-agent (52%, 161 GB, 12,137 files, a date) first, then hoardini, llama.cpp and the side projects, each with a bar; "+13 more · 1.9 GB" ends the list. Press it: all 63 folders show.
3. ▸ on covenant-agent opens its children one level deeper, indented, with "277 small files · 1.4 GB" under them.
4. Hover `covenant-agent\target` and press Show in Explorer: the mock records the path (`GET /api/mock/revealed`).
5. Against `hoardini serve`, Show in Explorer opens Explorer with the item selected (SRV-REVEAL), and on a path that is gone the row says "not found".
6. At 430 px wide the rows keep name, percentage and size; at 1000 px the bar, file count and date show too.
(2026-10-09: steps 1–4 and 6 pass against the mock in headless Chromium, and the tree also opened against `hoardini serve` (UI-TOOL-CARDS step 9); step 5 was not run, since it opens Explorer on the desktop.)

### UI-FOLLOW-UPS: Follow-up pills under the answer
**Applies to:** hoardini
**Verification:** manual

Under the settled last answer the chat shall show up to 3 pills styled like the greeting's starters: hidden while a run streams, gone as soon as the user sends anything, and absent while the turn waits on a proposal card. The turn is everything after the last question or card decision. The model's pills from the turn's `suggest_next` results (AGENT-SUGGEST-NEXT) come first; pills built from the turn's cards fill the rest: from a folders card, "What's inside {name} ({size})?" for its biggest row (it sends "What's inside {path}?") and "Clean up {scope}" for its `under` folder, else its biggest row (the folder's name instead of the path when the path is long); from a files card, "What is {name} ({size})?" for its biggest file; from a changes card, "Why did {name} grow (+X)?" for its biggest top-level grower. A pill with the same kind and path as an earlier one, or the same label when it has no path, is skipped. An `ask` pill sends its label (a built one its full-path question), and a `clean_up` pill sends "What can I clean up in {path}?". An `explore` pill calls the shell's `onOpenExplore(path)` (`ExploreLinkContext`, UI-TOOL-CARDS); while no handler is provided it is skipped and takes no slot. A pill never approves anything. The `suggest_next` call itself shows no chip.

**Verification (manual):**
1. Against the mock (UI-TOOL-CARDS step 1), press Show hotspots. Under the answer: "What's in covenant-agent?", "Clean up C:\code" (the model's; its third pill names a path not in the index and is dropped) and "What's inside Users (727 GB)?" (built from the folders card). No pills show while the answer streams.
2. Reload: the same three pills show.
3. Press "Clean up C:\code": the pills disappear at once, the question "What can I clean up in C:\code?" is sent, and the new answer gets "Clean up C:\code\covenant-agent" (the model's), then "What's inside covenant-agent (161 GB)?" and "Clean up C:\code" (built).
4. Ask "List the biggest files": the pills are "Which models haven't I used lately?" and "What is llama-3.3-70b-Q4_K_M.gguf (42.5 GB)?".
5. Ask "what is in lmstudio": the turn ends on a proposal card and no pills show; approve it and the answer after the card shows no pills built from the earlier card.
6. Ask "Show me where my space goes": with no Explore handler the pills are "Clean up C:\code", "What's inside Users (727 GB)?" and "Clean up C:\Users" (the model's "Explore AppData\Local" is skipped). With `ExploreLinkContext` providing a handler (UI-TOOL-CARDS step 11), "Explore AppData\Local" leads, and pressing it calls the handler with `~\AppData\Local` and sends nothing.
(2026-10-09: steps 1–5 pass against the mock in headless Chromium; against `hoardini serve` with a scripted model, the model's pill came first, then the two built from the folders card, and the same three came back after a reload.)
(2026-10-09: step 6 passes against the mock in headless Chromium; step 1 rechecked with the mock's new drive shape.)

### UI-TREEMAP-LAYOUT: Treemap blocks tile their folder exactly
**Applies to:** hoardini
**Verification:** test

`squarify(sizes, rect)` and `treemap(root, kids, width, height, options?)` (`ui/src/explore/layout.ts`) shall lay out the blocks of the Explore treemap (UI-EXPLORE) and the folders card's Map tab (UI-TOOL-CARDS). `squarify` places items biggest first (ties in input order) by the squarified algorithm and returns whole-pixel rectangles in input order. Edges are rounded from shared values, so the rectangles tile the parent exactly with no gap or overlap, and each area is within one pixel per side of its share; an item of size 0 gets no area. `treemap` nests: below the top level, a folder at least 44 px wide and 28 px tall gets a 16 px name strip and 2 px of padding on its other sides, a smaller one no strip (and 2 px of padding only when it is over 14 px each way), and its children tile what is left (`inner`); a child of size 0 gets no tile. The same input gives the same tiles. `tileAt(tiles, x, y)` returns the deepest tile under a point.

**Acceptance criteria:**
- `[6, 6, 4, 3, 2, 2, 1]` in 600 × 400 gives exactly (0,0,300,200), (0,200,300,200), (300,0,171,233), (471,0,129,233), (300,233,120,167), (420,233,120,167), (540,233,60,167).
- Twelve sizes from 97 down to 0.5 in a 413 × 287 rectangle at (7,3) cover it exactly with no overlap, and each area is within `w + h + 1` of its share.
- `[5, 0, 5]` in 100 × 50 gives (0,0,50,50) and (50,0,50,50), the 0 an empty rectangle, and the same result twice.
- A nested tree in 640 × 400: the root's inside is the whole canvas; a big folder's inside is its rectangle less a 16 px strip and 2 px sides; a file has no inside; a folder 0.3% of the drive has no strip; a size-0 folder has no tile; every folder's children tile its inside exactly, one level deeper; a second run is identical.
- `tileAt` gives a folder on its strip, its child inside it, the last block at the far corner, and nothing outside the canvas.

Test: `specs/ui/tests/treemap-layout.test.ts`.

### UI-EXPLORE: Explore: folder tree, treemap and file types, linked
**Applies to:** hoardini
**Verification:** manual

`ExploreView` (`ui/src/explore/ExploreView.tsx`; props `root` (the drive or folder it opens on), `onAsk(text)`, `onCleanUp(path)`, optional `onWhatIsThis(path)` and `focus` (a path to open, from "Open in Explore" or an `explore` pill)) shall show one folder at a time in three linked panes, laid out for the shell's Explore home tab:
- **Header:** breadcrumbs from the drive root (SRV-INDEX-MAP `ancestors`), each a button; **Up** to the folder above; a switch between **What Hoardini knows** (the default) and **File type**, remembered per viewer; a legend; and "{size} · N blocks, smaller than X merged" (or "the 5,000 biggest blocks" when the map is capped).
- **Treemap:** a canvas drawing `GET /api/index/map` for the folder at the canvas size (refetched on zoom and on a resize past 64 px), laid out by UI-TREEMAP-LAYOUT with folder names and sizes in name strips, and names (and sizes, room allowing) on blocks big enough. By knowledge, a block is green when proposed, amber for review, grey for keep (lighter) and system (darker), blue for the user's data, and striped when unknown; a folder's strip is tinted by its own tag; merged "N more" blocks are dotted. By file type, files take the 8 biggest types of the `root` folder in a fixed order (dataviz categorical steps, light or dark by the theme), so a type keeps its color as you zoom; other files, folders and small files are grey. Colors come from the theme's CSS variables and follow a theme change at once; labels read at 4.5:1 (UI-THEME-CONTRAST). Hover shows a tooltip (name, path, size, share, files, tag). Click selects; double-click or Enter opens a folder (or a merged block's folder); Backspace or Escape goes up; arrow keys move between neighbors (Alt+arrows to the parent or first child).
- **Folders:** ItemTree (UI-ITEM-TREE) of the same folder at full width, with Zoom in on folder rows.
- **By type:** `GET /api/index/types` for the folder: swatch, extension, size, a bar, and its identity (the file-type row's text, else app and use, else the category), else the file count and **What is this?**, which asks the chat "What are .{ext} files?". Small files are one row. Clicking a type dims every block but its files (small files: the small blocks); Clear undoes it.
- **Linking:** selecting a block, a tree row or a type selects in the other panes: the tree opens down to the selected block and marks it, the selected file's type row is marked, and the map outlines the selection. A zoom selects the folder it opened; Up selects the folder it came from.
- **Selection bar:** name, path, size, share of the view, files, and the tag (state, label, and the verdict when it is safe or redownload), with **Clean up** (folders; calls `onCleanUp`), **What is this?** (when `onWhatIsThis` is given), **Ask Hoardini** (`onAsk("What is {path}?")`), **Show in Explorer** (SRV-REVEAL) and **Zoom in** (folders below the view, merged blocks).
- **Focus:** the selection (else the view's folder) is sent to `PUT /api/explore/focus` 600 ms after it settles and cleared when the view unmounts (SRV-EXPLORE-FOCUS). A `focus` that is a file opens its folder with the file selected.
- **Narrow:** below 768 px of width the panes stack: selection bar, treemap (22rem), folders, types, and the view scrolls.

**Verification (manual):**
1. In `ui/`, run `MOCK_PORT=72xx MOCK_IDLE=1 bun run mock` and Vite with its `/api` proxy on that port, and mount `<ExploreView root="C:\" …>` full height (the shell's Explore tab, or a scratch page until it exists) at 1400 × 900 in Marshmallow.
2. The breadcrumb reads `C:`, the legend lists Proposed, Review, Keep, System, Your data, Unknown and merged, the tree leads with Users (727 GB), By type leads with .gguf ("model weights"), .q4nx reads "FastFlowLM NPU model weights", and the summary reads "1.8 TB · the 5,000 biggest blocks, smaller than … merged". Temp\claude and the `target` folders are green, the model folders, Docker and Games amber, Program Files and Windows dark grey, Documents, Pictures and OneDrive blue, covenant-agent's data striped.
3. Click the docker_data.vhdx block: the bar reads docker_data.vhdx, 78.2 GB, "Review Docker Desktop's WSL disk"; the tree opens Users › josha › AppData › Local › Docker › wsl › disk and marks the file; the .vhdx row is marked; within a second `GET /api/mock/focus` returns its path.
4. Press Ask Hoardini, What is this? and Show in Explorer: the handlers get "What is ~\AppData\Local\Docker\wsl\disk\docker_data.vhdx?" and the path, and the mock records the reveal. Click Games in the tree: the bar shows "Review Steam library · redownload"; Clean up calls `onCleanUp("C:\Games")`.
5. Click .q4nx in By type: everything but the .q4nx files dims; Clear restores it.
6. Switch to File type: blocks take type colors and the legend lists the 8 types; reload: File type is still on.
7. Double-click the code block: the map, breadcrumbs (C: › code) and tree show C:\code. Up returns to C: with code selected. With the map focused, Arrow Right selects the next block, Enter zooms into a folder and Backspace zooms out.
8. Open with `focus` = `~\.flm\models\Qwen3.8-27B-NPU2\model.q4nx`: the view shows Qwen3.8-27B-NPU2 with model.q4nx selected.
9. Repeat step 2 in Marshmallow Night, and at 430 px wide in both: the panes stack, labels and the selection bar's buttons read, and nothing scrolls sideways.
10. Against `hoardini serve` on a scratch index: the map and types load, a selection reaches `explore_focus`, and a chat question after it carries "The user is looking at … in Explore." (AGENT-PROMPT-CONTEXT).
(2026-10-09: steps 1–9 pass against the mock in headless Chromium through Playwright, on a scratch page mounting ExploreView, with no element past 430 px in step 9. Step 10 passes against a release `hoardini serve` on a scratch folder: the map and types came from its index, selecting `models` in the tree stored the focus, and a chat run against a stub OpenAI-compatible model sent "The user is looking at ~\…\hdata\models in Explore." in its system message. On a copy of a real 1.6 TB C: index, the map of C: (5,000 blocks) answered in 0.35 s and its types in 0.2 s.)

### UI-LEARNED-CHIP: What Hoardini learned shows as a chip with Undo
**Applies to:** hoardini
**Verification:** manual

`LearnedChip` (`ui/src/components/LearnedChip.tsx`; props `text`, optional `url`, `onUndo`, optional `onPin`, optional `onConfirm`) shall show one line: "learned", the text (the full text as its tooltip), the source link's host when there is a `url`, and its buttons. Undo turns it into "undone" with the text struck through and no buttons; Confirm turns the label into "learned · confirmed"; Pin toggles "learned · pinned" and Unpin. A failed call shows its error inline and keeps the buttons.
- **After a job:** a finished proposal card (collapsed or opened, UI-PROPOSAL-COLLAPSE) shows one chip per entry of the proposal's `learned` (SRV-LEARNED) directly under it, above any follow-up card, with Pin (`PUT /api/memory/{id}` `pinned`) and Undo (`DELETE /api/proposals/{id}/learned/{memory_id}`).
- **After a web search:** a `web_search` call whose `data` is `{kind: "learned"}` (AGENT-WEB-LEARN; live from `tool_data`, after a reload from the thread) shows its chip under the search's tool chip, with the source link, Confirm (`POST /api/memory/{id}/confirm`) and Undo (`DELETE /api/memory/{id}`).

**Verification (manual):**
1. In `ui/`, run `MOCK_PORT=72xx MOCK_IDLE=1 bun run mock` and Vite with its `/api` proxy pointed at it; open it at 1000 × 900.
2. Post `{"next_job": {"ms": 1200}}` to `/api/mock`, ask "what is in lmstudio" and hold to delete. When the job ends the card reads "Deleted 12 items · freed 53.6 GB · details ▸", and directly under it, above the follow-up card, a chip reads "learned 12 items in ~\.lmstudio — cleaned 53.6 GB on <MM-DD>; 2 items you left unticked are kept" with Pin and Undo.
3. Press Pin: the label becomes "learned · pinned" and the row (source `job:<id>`) is pinned in `GET /api/memory`.
4. Press Undo: the chip reads "undone" with the text struck through and no buttons; `GET /api/proposals/{id}` has `learned: []` and the row is gone. After a reload no chip shows.
5. Ask "what is ~\.hipfire": under the web search chip a chip reads "learned ~\.hipfire: model cache (hipfire)." with an `example.com` link, Confirm and Undo.
6. Press Confirm: the label becomes "learned · confirmed" and the row is source `user` and pinned. After a reload the chip is back under the web search chip.
7. Press Undo: "undone", and the row is gone from `GET /api/memory`.
8. At 430 px wide in a dark theme each chip stays on one line, the text cut short with the full text as its tooltip.
9. Against `hoardini serve` with a scratch `HOARDINI_DATA_DIR` and `--db`, a scripted OpenAI-compatible model and a SearXNG stand-in: a model's `propose_delete` of two `.log` files and one other file in a scratch folder, approved with the other file unticked, collapses to "Deleted 2 items · freed 16.0 KB" with the chip ".log files in ~\...\work — cleaned 16.0 KB on <MM-DD>; 1 item you left unticked is kept", still there after a reload; a `web_search` about `~\.hipfire` shows its chip live and after a reload; Confirm makes the row `user`-sourced; Undo on the job chip leaves `learned: []` and no `job:` row.
(2026-10-09: steps 1–8 pass against the mock in headless Chromium through Playwright, in the default theme and at 430 px with a dark color scheme; step 9 passes against `hoardini serve` on a scratch folder with a scripted model and search stand-in, and the learning request had no tools, no profile name, and the results inside the untrusted fence.)

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
