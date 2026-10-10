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

### UI-LAYOUT: Review and Explore are the home tabs; the chat is a side panel
**Applies to:** hoardini
**Verification:** manual

The window shall keep the header (drive selector, mascot, Settings) and show the home tabs **Review** (UI-REVIEW; its
label carries Ready to free) and **Explore** (UI-EXPLORE) under it (`ui/src/shell/Shell.tsx`). From 1100 px wide
(UI-LAYOUT-WIDTH) the chat is a panel on the right: a "Chat" head with the active model and Hide the chat, a divider
that drags (Left and Right arrows on the focused divider step 24 px, Home and End jump to the widest and narrowest,
double-click restores the default), and once hidden a narrow rail with a Chat button and a pulsing dot while a run
streams. Below 1100 px the chat is a third tab, **Chat**, with the same dot. The panel's width and hidden state are
remembered per viewer in local storage; with storage blocked the app still opens with the defaults. Everything the chat
did before works in the panel (cards, pills, Learned chips, proposal cards, Clear chat, stall and Keep going). The chat
stays mounted while it is hidden, in the same place for both layouts, so a run keeps streaming behind another tab or
across a resize. Asking from outside the chat (Explore's Ask Hoardini and What is this? on a type, Look closer, an
area's Ask) shows the chat: the panel opens, or the Chat tab on a narrow window, and the question is sent once no run is
streaming. "Open in Explore" and `explore` pills switch to the Explore tab on that path (`ExploreLinkContext`). Explore
is mounted only while its tab is chosen, so leaving it clears its focus (SRV-EXPLORE-FOCUS); behind the Chat tab of a
narrow window it stays. On a window narrower than 640 px the header puts the drive bar on its own line under the brand
and Settings. FirstRun still covers the app until the AI is set up. Left and Right arrows (Home and End) on the tabs
select the next tab; only the selected tab is in the tab order, and each tab controls its panel (the home panel, or on a
narrow window the chat). What is typed in the chat box survives the chat reloading its thread (Look closer, Clear chat).
A home view that fails to render shows "The Review view hit a problem and stopped." (or Explore) with the error and Try
again, which reopens the review, while the header, the other tab and the chat keep working.

**Verification (manual):**
1. In `ui/`, run `MOCK_PORT=7347 MOCK_IDLE=1 bun run mock`, then `bun run build` and serve `ui/dist` with `/api` proxied to that port (or Vite with its proxy pointed there); open it with `#token=dev` at 1440 × 900.
2. Under the header the tabs read "Review 359 GB" and "Explore"; the chat panel on the right is 420 px wide with "Chat qwen3.8-27b", the starters and the message box.
3. Drag the divider 160 px to the left: the panel is 584 px wide and `localStorage['hoardini.layout']` reads `{"chatWidth":584,"collapsed":false}`. Focus the divider and press Arrow Right: 560 px.
4. Press Hide the chat: the panel goes, a rail with a Chat button stays on the right, and the review widens. Reload: still hidden. Press Show the chat: 560 px again.
5. Ask "what is in lmstudio" in the panel: the card, Hold to delete, the pills and Clear chat work as in UI-CHAT-FLOW and UI-PROPOSAL-COLLAPSE. Ask again and hide the panel while it streams: the rail's dot pulses, and showing the panel shows the run still going.
6. Resize to 430 × 900: the tabs read Review, Explore, Chat; Chat shows the same conversation; the drive bar sits on its own line under the brand and Settings.
7. At 430 px open Explore, double-click Users in the tree and press Ask Hoardini: the Chat tab opens and sends "What is C:\Users?"; press Explore again: C: › Users is still open.
8. Block local storage for the page (an init script that makes `localStorage` throw) and reload at 1440 px: the app opens with a 420 px panel, and hiding and showing it works.
9. Focus the Review tab and press Arrow Right: Explore is selected and focused, the Review tab has `tabindex="-1"`, and both tabs' `aria-controls` name the `tabpanel` `panel-home`.
10. Answer `POST /api/review/open` with a review whose `areas` is null (a Playwright route) and reload: the Review tab shows "The Review view hit a problem and stopped." with the error and Try again, and the chat and Explore still work. Drop the route and press Try again: the review comes back.
(2026-10-09: steps 1–8 pass against the mock in headless Chromium through Playwright, with `ui/dist` served and `/api` proxied to the mock; in step 5 the lmstudio card collapsed with its follow-up, Clear chat brought back the starters, and the rail's dot showed while the second run streamed behind it.)
(2026-10-09: steps 2, 9 and 10 pass against the mock in headless Chromium through Playwright at 1440 px; step 2 now reads "Review 359 GB" with the mock's new cards.)

### UI-LAYOUT-WIDTH: Where the chat goes and how wide it is
**Applies to:** hoardini
**Verification:** test

`isWide(width)`, `chatWidthFor(width, want)` and `parsePrefs(raw)` (`ui/src/shell/layout.ts`) shall decide the layout of
UI-LAYOUT: the chat is a side panel from 1100 px of window width and a tab below that; the panel is the stored width, or
420 px without one, held between 340 and 760 px and narrow enough to leave the home tabs 560 px, in whole pixels. Stored
prefs `{chatWidth, collapsed}` read back as stored; a width that is not a positive number reads as none, a `collapsed`
that is not `true` as false, and anything that is not a JSON object as the defaults.

**Acceptance criteria:**
- `isWide`: 1099 → false, 1100 → true, 430 → false.
- `chatWidthFor`: (1440, none) → 420; (1440, 200) → 340; (1440, 900) → 760; (2560, 5000) → 760; (1100, 900) → 540; (1100, none) → 420; (1440, 512.6) → 513.
- `parsePrefs`: `{"chatWidth":500,"collapsed":true}` → 500 and true; null, `not json` and `[1,2]` → none and false; `{"chatWidth":"wide","collapsed":"yes"}` → none and false; `{"chatWidth":-3,"collapsed":true}` → none and true.

Test: `specs/ui/tests/shell-layout.test.ts`.

### UI-REVIEW: The Review tab
**Applies to:** hoardini
**Verification:** manual

`ReviewView` (`ui/src/review/ReviewView.tsx`; state in `ui/src/review/state.ts`, rows in `LargeAreas.tsx` and
`RarelyUsedApps.tsx`) shall show the drive's review (REVIEW-BUILD, REVIEW-AREAS) as the Review home tab:
- **Loading:** at start and when the selected drive changes it calls `POST /api/review/open` (REVIEW-TRIGGER). An
  answer with `review: null` (the drive's first review is still building) shows "Review of C:", "Building your review…"
  with a spinner, the drive's used and free bar, and the consent and Apps you rarely use from that answer. While a build
  runs over a review on screen (the answer's `building`, or `review_building` in the status), the header says "Building a
  newer review… This one stays until it is ready."; Review now shows its own "Reviewing…" instead. When the status
  stream reports the end of a build it saw running, or `status.review.id` differs from the review on screen, it reads
  `GET /api/review`. An answer for another drive than the selected one, or older than the review on screen, is dropped.
  A 404 (no completed index) shows "Hi, I'm Hoardini." (UI-WELCOME-BACK) and opens the review when the first refresh
  ends; a first build that ends without a review shows "Could not build the review" with Try again.
- **Header:** "Review of C:", Ready to free in large type (`readyBytes` of UI-REVIEW-ORDER over the cards on screen and
  their follow-ups, carried cards included, so it follows every tick and matches the server's `ready_bytes`), a bar of
  the drive's used space with the ready part striped and the free space after it, with a key, "Built N ago · index from
  N ago", and **Review now** (`POST /api/review/run`), which reads "Reviewing…" while it runs. The online-checks consent
  (UI-ONLINE-CONSENT) follows the header and reads the review's `online`.
- **What can go:** each card as a ProposalCard titled by the card ("Build output · free 121 GB"; "up to X" while nothing
  is ticked), in the order of UI-REVIEW-ORDER fixed when the review loads. A card the server lists with `carried`
  (REVIEW-SUPERSEDE: touched in an earlier review and still live) shows in place with a "from the earlier review" tag. A
  tick is saved with `PUT /api/proposals/{id}/selection` (SRV-PROPOSAL-SELECTION), one request at a time per card with the
  latest ticks after it, and a review read in between keeps the ticks on screen; a failed save shows the error and the
  server's ticks. Every item row has ▸ (folders) for its ItemTree (UI-ITEM-TREE) and Show in Explorer. The `windows`
  card is a system cleanup card (UI-SYSTEM-CLEANUP-CARD). Approving or declining reports nothing to the chat
  (UI-PROPOSAL-COLLAPSE). With no card left the section says "Nothing new to clean on C:" and the used and free space.
- **Replaced cards:** cards a newer review no longer lists stay on screen until the page reloads (another drive starts
  without them): the ones the user touched or that ran as they are, and untouched ones as "Replaced by a newer review ·
  N items". A carried card that ran and drops out of a reread review stays the same way.
- **Apps you rarely use:** the review's `apps` (REVIEW-TRIGGER), else, from an older server, `GET /api/apps/rarely-used`
  (SRV-APPS): name, a Steam tag for games, publisher, "last run N days ago" or "never run", Show in Explorer for its
  folder, size, and **Uninstall…**, which asks "Open {name}'s own uninstaller? It asks you to confirm … Hoardini deletes
  nothing itself." with Open the uninstaller and Cancel. Open calls `POST /api/apps/{id}/uninstall` (ACT-UNINSTALL) and
  says "The uninstaller is open."; an error shows inline. A folder review shows the apps the server lists inside the
  folder. Hidden while the list is empty.
- **Large areas:** the review's `areas`, biggest first, the first 10 and then "Show all N · M more, X": name, the label as
  a tag (or "unknown"), the parent folder, size, a bar against the biggest, `why_not`, and the hint after an arrow; ▸ for
  its ItemTree unless the name ends in a file type (`.sys`, `.vhdx`, …); on hover or focus Ask (sends "What is {path} and
  is it safe to remove?" to the chat), Clean up (when the verdict is `safe` and the actions include delete, as after a
  Disposable answer) and Show in Explorer. An only-copy area (`only_copy`, REVIEW-AREAS) carries an "only copy we know
  of" tag in place of its label, its `why_not`, and "Deleting it would lose it for good, so Hoardini offers no delete
  here. Copy it somewhere else first if you want the space back."; it never offers Clean up. Unknown areas have a
  striped edge and **What is this?** Keep · Disposable · Move elsewhere · Look closer · ⋯, whose menu holds Never touch
  with a line saying what it means; Escape closes the menu and returns focus to ⋯, and tabbing out closes it. An answer
  calls `POST /api/locations/answer` (SRV-LOCATION-ANSWER) and rereads the reviews on screen, so the row shows the
  answer's label and text; a refused answer shows the server's error inline. Look closer sends the look-closer run
  (REVIEW-LOOK-CLOSER) to the chat, which opens: the chat posts `{path}` to `/api/review/look-closer` in place of
  `/api/agent/run`, reads its stream like any run, and reloads the thread when it ends, since the server stored the
  question under its own id.
- **What is this? card:** Explore's What is this? or a `what_is` pill (UI-FOLLOW-UPS) opens the Review tab with a card at
  the top for that path: name, size and file count (`GET /api/index/children`), path, ▸, Show in Explorer, the area's
  `why_not` when the path is a Large area, the same answers, and ✕. After an answer it says "Saved. {the row's text}";
  after Disposable it offers **Review it now** (a folder review). For a drive, `C:\Users` or a profile folder it offers
  no answers and says "A whole drive or profile takes no answer: what is inside is judged folder by folder."
- **Folder review:** Clean up (chat card rows, `clean_up` pills, Explore, an area row, Review it now) calls
  `POST /api/review/run {path}` and shows that review in the Review tab: "← Back to the C: review", a header reading "Clean
  up" and the path with Ready to free and Review again, its cards ("Nothing new to clean in {path}" when there are none),
  "Already in the C: review" with the drive review's pending cards that hold items in the folder, and its Large areas.
  Review again reads "Reviewing…" and is disabled while it runs; the last result stays on screen, and a failure shows
  under the header.

**Verification (manual):**
1. Mock and page as in UI-LAYOUT step 1, at 1440 × 900 in Marshmallow (the mock builds its first review in the background, `MOCK_BUILD_MS`, 2.5 s by default).
2. Right after the page opens the Review tab shows "Review of C:", "Building your review…", the used and free bar with no ready part, the consent notice and Apps you rarely use. Then the review: "359 GB ready to free", the bar with a striped ready part, "Used 1.8 TB · Ready to free 359 GB · Free 62.0 GB of 1.9 TB", "Built just now · index from 3 days ago" and the consent notice. The page made no request to `/api/copy-checks/waiting`, `/api/apps/rarely-used` or `/api/settings`.
3. What can go lists, in this order: Build output (free 121 GB), Temp and Windows leftovers (free 94.5 GB: four Claude scratch sessions and WinGet), Caches (free 61.3 GB: CacheStorage 42.1 GB, uv 13.6 GB, pip, Chrome's Cache), Re-downloadable, verified (free 42.7 GB: the llama-3.3-70b GGUF "identical to lmstudio-community/Llama-3.3-70B-Instruct-GGUF on Hugging Face." and balenaEtcher "In winget as Balena.Etcher 2.1.7.", with no Copy line under either), Cleanups only Windows can do (free at least 23.9 GB; UI-SYSTEM-CLEANUP-CARD step 6), Folders you said can go (free 9.6 GB: `C:\code\based\runs`), Online-only (free 6.5 GB: Camera Roll 2019 and drone-2018.mp4, `synced`), Installers and archives in Downloads (up to 15.4 GB), Send elsewhere (up to 11.2 GB) and Compress in place (up to 7.5 GB: Ollama's `lib\ollama` "Windows compression would save about 2.0 GB (43% of it); …" and datalab's models, 1.2 GB).
4. Untick covenant-agent's target in Build output: Ready to free reads 298 GB at once, `GET /api/proposals/{id}` shows it unticked with `touched: true`, and `GET /api/review` has `ready_bytes` of 298.2 GB. Tick it again: 359 GB. ▸ on that target lists its folders; Show in Explorer is recorded by `GET /api/mock/revealed`.
5. Post `{"next_job":{"ms":2500}}` to `/api/mock` and hold Hold to delete on Build output: the sumo climbs down to the card in the Review tab (UI-MASCOT-JOBS); when the job ends the card reads "Deleted 4 items · freed 121 GB · details ▸", the Learned chip "4 items in C:\code — cleaned … ; 1 item you left unticked is kept" sits under it, then the follow-up "Build output · up to 6.2 GB" with llama.cpp\build; Ready to free reads 238 GB.
6. Apps you rarely use lists Cyberpunk 2077 (Steam, 71.3 GB), Visual Studio Community 2022 (never run, 38.4 GB), HIP SDK Core, Visual Studio Build Tools 2022 and LLVM. Uninstall… on Visual Studio Community 2022 and Cancel: `GET /api/mock/uninstalled` is empty; again with Open the uninstaller: "The uninstaller is open." and its id is listed.
7. Large areas shows 10 rows, Games (300 GB) first, and "Show all 20 · 10 more, …"; covenant-agent (95.0 GB) is unknown with the striped edge and What is this?; hiberfil.sys (35.1 GB) ends "→ Windows recreates it at boot; turning hibernation off removes it."; there is no row for `Windows\Temp` or `SoftwareDistribution\Download` (the Windows card has them). Show all: model.q4nx (21.0 GB, `C:\AI\.flm-scratch\…`) carries "only copy we know of", reads "Only copy we know of: no other file of this size on the indexed drives; not found on Hugging Face." and the no-delete line, and on hover offers Ask and Show in Explorer but no Clean up.
8. Keep on covenant-agent: the row reads "yours" and "You keep this folder; it may still be moved or compressed."; `GET /api/memory` has a `user` row for it. Show all, then ⋯ and Never touch on .hipfire: "never touch", "Never touch this folder.", and a `protect` row. Open ⋯ on another unknown row and press Escape: the menu closes and ⋯ has the focus; open it again and press Tab: it closes.
9. Type something in the chat box, then Look closer on llama.cpp: the chat shows "What is C:\code\llama.cpp? Look inside and tell me whether to keep it, delete it, or move it elsewhere.", the answer streams in, the pill "Say what llama.cpp is" follows, and the chat box still holds what was typed.
10. Press that pill: the Review tab opens with the What is this? card for llama.cpp (24.8 GB, 4,333 files). Disposable: "Saved. You said this folder can go." and Review it now.
11. On a fresh mock, untick one Caches item, then post `{"rebuild":true}` to `/api/mock`: within a second the review is the new one; Caches is still listed, tagged "from the earlier review"; Ready to free reads 357 GB, the same as `GET /api/review`'s `ready_bytes`, which lists Caches with `carried: true`; nine "Replaced by a newer review" lines follow the other cards.
12. Post `{"rebuild":{"ms":3000}}` to `/api/mock`: the header shows "Building a newer review… This one stays until it is ready." over the review on screen, and about 3 s later the new review replaces it.
13. Press Review now: it reads "Reviewing…" and `GET /api/review` returns a new id.
14. With the mock started with `MOCK_RUN_MS=1500`, open Explore, pick `code` in the folder tree and press Clean up: the Review tab shows "← Back to the C: review", "Clean up" C:\code, "Nothing new to clean in C:\code", Build output under "Already in the C: review", and the covenant-agent and llama.cpp areas. Review again reads "Reviewing…" and is disabled until the run ends, with the last result still on screen. Back returns to the drive review.
15. Restart the mock with `MOCK_NO_INDEX=1`: the Review tab shows "Hi, I'm Hoardini." with the scan's percentage, then "Building your review…" once the fast scan ends, then the review.
16. Repeat steps 2, 3, 7 and 10 in Marshmallow Night and in Rosewood (midtone), and at 430 px wide: text reads on every surface, nothing scrolls sideways, and Look closer opens the Chat tab.
17. Against `hoardini serve --port 72xx --token dev` with a scratch `HOARDINI_DATA_DIR` and `--db` and an index of a scratch folder: the Review tab shows "Building your review…" from `POST /api/review/open`, then that folder's review once the status says the build ended; a tick survives a reload, and What is this? on an unknown area updates its row.
(2026-10-09: steps 1–16 pass against the mock in headless Chromium through Playwright, with the built UI served and `/api` proxied to the mock; step 5's sumo trip was not watched. Step 16 ran in Marshmallow Night and Rosewood at 1440 px and in Marshmallow, Marshmallow Night and Rosewood at 430 px, with no element wider than the window; an operation's label in the Windows card was cut at 430 px and now wraps.)
(2026-10-09: step 17 passes against a debug `hoardini serve --no-refresh` with scratch data, `TEMP` and `LOCALAPPDATA` pointed into the scratch folder so the watched folder is not temp, and a stub OpenAI-compatible model: "Hi, I'm Hoardini." until `PUT /api/volumes` watched the folder and its first scan ended, then "Building your review…", then "Review of …\root" with Build output (700 MB: a Rust `target` and a `node_modules`); unticking node_modules survived a reload (400 MB on screen and from the server), and Keep on the unknown `data.bin` area (1.5 GB) changed its row to "You keep this folder; …".)

### UI-REVIEW-ORDER: Card order and Ready to free
**Applies to:** hoardini
**Verification:** test

`readyBytes(proposals)`, `chainOf(id, proposals)` and `orderCards(cards, proposals)` (`ui/src/review/order.ts`) shall sum
Ready to free and order the cards of UI-REVIEW. `readyBytes` adds the items of pending proposals that are ticked, still
pending and not blocked, each by its `saves` when it has one (a compress item, REVIEW-BUILD), else by its `alloc`. `chainOf` follows `follow_up_id` from a card through its follow-ups and stops at a repeat. A card
ranks by its chain: first those with something pending and ticked, most ready first; then pending cards with nothing
ticked, biggest `total_alloc` first; then running or approved; then done, partial, failed or declined; then superseded;
then cards whose proposal is not loaded. Ties keep their input order.

**Acceptance criteria:**
- A pending card with a ticked 10 GB item, an unticked 20 GB item, a ticked done 30 GB item and a ticked blocked 40 GB item, and a declined card with a ticked 50 GB item, give 10 GB.
- `a → b → c` gives `a, b, c`; a loop `a → b → a` gives `a, b`; an unknown id gives nothing.
- Build (113 GB ticked), temp (94.5 GB), downloads (15.4 GB, none ticked), archive (11.2 GB, none ticked), a running card, a declined card, a superseded 200 GB card, a done card whose follow-up has 5 GB ticked, and a card with no proposal come out as build, temp, the done card, downloads, archive, running, declined, superseded, the missing one.
- Two cards with the same ticked size keep their input order.
- A pending compress card with a ticked 7.5 GB item that saves 3.2 GB and a ticked 2 GB item with no `saves` gives 5.2 GB.

Test: `specs/ui/tests/review-order.test.ts`.

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
3. Ask two questions. No divider is shown. Press Clear chat: the screen empties, the starters show (UI-WELCOME-BACK), and "Show earlier chat (4 messages)" appears at the top. Click it: both turns come back with the divider below them; click "Hide earlier chat" to hide them again.
4. Ask a third question: the model request carries only that question (check the mock's captured request); with the earlier chat shown, the divider sits right above it.
5. Reload the page: the same scrollback opens with the divider in the same place.
6. Set every stored message's `created_ms` 7 hours back in the database, reload, and ask again: the divider moves above the new question.
(2026-10-09: steps 1–6 passed against `hoardini serve` with a scripted mock model, driven by Playwright, when the button was Start fresh; steps 2–4 were rewritten for Clear chat the same day and have not been re-run.)

### UI-WELCOME-BACK: The review header and the chat's starters welcome the user back
**Applies to:** hoardini
**Verification:** manual

What the greeting card said about the drive shall be the review header (UI-REVIEW): the drive, its used and free space,
the index age, and Ready to free, from the `header` of `POST /api/review/open` (`root`, `used_bytes`, `free_bytes`,
`ready_bytes`) with Ready to free following the ticks on screen. The biggest movers and the free-space change are left to
the `changes` card and the history chart. The chat shows its starters (`Starters`, `components/Greeting.tsx`: "Ask
Hoardini", one line, and Show hotspots, Show duplicates when there are duplicates, and **Free space over time**, which
asks `How has my free space changed over time?`) at the end of the scrollback, above the composer, whenever
`GET /api/thread` reports `fresh`: on first launch, when the last message is more than 6 hours old, and right after Clear
chat. They hide as soon as a question is sent. While the selected drive has no completed index (the review answers 404),
the Review tab shows "Hi, I'm Hoardini." with the first scan's progress, then "Building your review…" until the first
review is built (UI-REVIEW).

**Verification (manual):**
1. On a new scratch database with an index, open the UI: the Review tab shows the review header, and the chat shows Ask Hoardini with Show hotspots and Free space over time.
2. Press Free space over time: the question `How has my free space changed over time?` is sent and the starters disappear.
3. After the answer they stay hidden. Press Clear chat: the screen empties and the starters appear.
4. Set every stored message's `created_ms` 7 hours back and reload: the starters show at the end of the scrollback.
5. Start the mock with `MOCK_NO_INDEX=1` (UI-REVIEW step 15): "Hi, I'm Hoardini." and the scan's percentage show in the Review tab until the fast scan ends, then "Building your review…", then the review header.
(2026-10-09: steps 1–4 passed in the same session as UI-CONTEXT-BOUNDARY, when this was the greeting card in the chat.)
(2026-10-09: step 5 passes against the mock in headless Chromium: "Hi, I'm Hoardini." at 8%, then "Building your review…" once the fast scan ended, then the review header.)
(2026-10-09: steps 1–4 pass against the `hoardini serve` of UI-REVIEW step 17 with its stub model: the starters showed beside the review header, Free space over time was sent and hid them, they stayed hidden after the answer, Clear chat brought them back, and after one more question and a reload they stayed hidden until the 4 stored messages were moved 7 hours back, when a reload showed them.)

### UI-CLEAR-HISTORY: Clear chat history deletes only the chat text
**Applies to:** hoardini
**Verification:** manual

`ClearHistorySection` (rendered in the Settings tab) shall offer **Clear chat history** behind a confirm step ("Delete every chat message?" with Delete messages and Cancel). Confirming calls `DELETE /api/thread`, then reopens the conversation, so the chat is empty and its starters show (UI-WELCOME-BACK). Memories, proposals, jobs, and the archive list stay. The button is disabled while a run streams; an error from the server (409 while a run is in progress) shows inline.

**Verification (manual):**
1. With a few messages, a saved memory, and a decided proposal, open Settings and press Clear chat history: the confirm step appears; Cancel leaves everything as it was.
2. Press it again and confirm: "Cleared" shows, and the chat is empty with its starters (UI-WELCOME-BACK) and no divider.
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

When an approved job from a proposal card, in the Review tab (UI-REVIEW) or the chat, starts running and the card's action footer (the area holding the Hold and Approve buttons, then the progress bar and result) is on screen, the header sumo hides and a travelling copy climbs down a ladder from the header to stand on the footer's top edge (1.6 s; a fat sumo bends the rungs). If the footer is not on screen but the card's top edge is, he stands on the top edge instead. When the parent card collapses mid-trip (§ UI-PROPOSAL-COLLAPSE) he moves to the follow-up card's top edge, or to the collapsed line if there is no follow-up, and is never left over nothing. A parcel pops out of the card next to him with a file or folder icon, the first item's name, the size, and "×N" for several items. The overlay never moves the card. He stands above the edge he is anchored to, never over the buttons. Whatever the job's speed, the beats last at least this long: working 2 s (even if the job finished during the climb down), the finisher about 1.8 s, the climb back 1.4 s, then pleased 3 s; about 10 s in all. While the job runs he works the parcel in a loop, and when it finishes he plays the move for its kind:

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

He then climbs back up, deflating on the way if space was freed, and in the header is pleased with himself for 3 s (chin up, eyes closed, arms crossed, a pat-pat of the hands); the rest of that celebration is skipped. A declined proposal gets no trip, only a head shake in the header. If neither the footer nor the card's top edge is on screen when the job starts, or the page is hidden, there is no trip: he stomps in the header while the job runs (at least 2 s), plays the move there when it ends, then celebrates for 3 s. After about 30 s of work he climbs back and the move plays in the header when the job ends. Only one trip happens at a time; a job that starts while he is out runs without one. After 5 trips within 10 minutes later jobs get the header version. Clicking anywhere sends him home quickly, and the job carries on from the header. With `prefers-reduced-motion` there is no trip and no parcel: he holds the still stomp pose while the job runs and shows the pleased face when it succeeds. The overlay never takes pointer input, and the mascot never delays or drives a job. Cards stay mounted behind hidden tabs and in a hidden chat panel (UI-LAYOUT), so he looks for the copy of the card that is on screen; a card behind another tab counts as off screen, and switching tabs during a trip sends him home as when the card scrolls away.

**Verification (manual):**
1. In `ui/`, run `MOCK_IDLE=1 MOCK_FILL=0.9 bun run mock` and `bun run dev`; open http://localhost:5193/ at about 1000 × 800 and pick the Chat tab (UI-LAYOUT).
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
13. At 1440 × 900, post `{"next_job":{"ms":2500}}` and hold Hold to delete on the Review tab's Build output card (UI-REVIEW step 5): he climbs down from the header to that card's footer in the Review tab, works the parcel ("target", "121 GB", "×4"), pushes it off, and climbs back; the card collapses with its follow-up under it.
14. Approve the Temp card in the Review tab and pick the Explore tab during the trip: he climbs home at once, and the move plays in the header when the job ends.
(2026-10-09, after the home tabs: step 2's trip ran from the Chat tab at 1000 × 800, and steps 13 and 14 pass against the mock in headless Chromium through Playwright; the moves themselves were not watched again.)
(2026-10-09: steps 2–10 checked against the mock in headless Chromium through Playwright, with compress, dedupe, failed, cancelled, and long jobs also driven through the store directly. Steps 11 and 12 checked the same way: trip from the footer with the card top off screen, sumo visible throughout, push at 2.9 s, finisher at 5.0 s, climb back at 6.8 s, home at 8.2 s with a 300 ms job.)

### UI-PROPOSAL-COLLAPSE: Finished cards collapse and a follow-up card follows
**Applies to:** hoardini
**Verification:** manual
**Depends on:** ACT-FOLLOW-UP (server side; the follow-up proposal and `parent_id` / `follow_up_id`)

A proposal card whose proposal was declined, or whose job ended (done, partial, failed, or cancelled), collapses to one line: the kind's verb (Deleted, Archived, Compressed, Deduplicated, Cleaned up, Made online-only), the item count, and the bytes freed, e.g. "Deleted 12 items · freed 3.4 GB · details ▸"; a decline reads "Declined · N items", and a review card a newer review replaced (`superseded`, REVIEW-SUPERSEDE) reads "Replaced by a newer review · N items". Clicking it expands the read-only item list with each item's outcome, and "hide ▴" collapses it again. Pending and running cards stay full size. When the proposal has a `follow_up_id`, the follow-up's full card renders directly under the collapsed card in the same chat slot; chains repeat. The UI refetches the proposal when its job reaches a terminal state, and a reloaded thread renders the same chain. Item rows have ▸ for a folder's ItemTree (UI-ITEM-TREE) and Show in Explorer until the item is done, and show a muted "small" tag when `small` is true, a muted "Noted: ..." line from `memory_note`, and a muted "Copy: ..." line from `copy_note` (EVID-COPY-CHECK) unless the item's reason already says it (as Re-downloadable and Online-only reasons do); the "Capped to ... by a safety rule" line appears for a `review` or `keep` cap, and for any other cap only when no Copy line explains it; the initial ticks are the server's `selected` values, and a review card saves each tick as it changes (UI-REVIEW). A pending card with nothing ticked is headed "up to X", the size of what it could free. A compress card whose items carry `saves` is headed "saves about X", X summing `saves` (else `alloc`) of the ticked items, of all of them when nothing is ticked or the card has ended. Approving or declining a follow-up card or a review card (one with a `review_id`, REVIEW-BUILD) does not report back to the model; the model sees it through the open-proposals line.

**Verification (manual):**
1. In `ui/`, run `MOCK_PORT=7261 MOCK_IDLE=1 bun run mock` and Vite with its `/api` proxy pointed at it; ask "what is in lmstudio".
2. The card lists 15 items: the "tiny-test.gguf" row has a "small" tag and "Noted: C:\Users\josha\.lmstudio is disposable"; it, "keep", and the blocked item are unticked and the cache blobs are ticked.
3. Approve. When the job ends, the card becomes "Deleted 12 items · freed 53.6 GB · details ▸" and a new full card appears directly under it holding the 3 items that were not done, all unticked.
4. Click "details ▸": the original items show with "done" outcomes. Click "hide ▴".
5. Reload the page: the same collapsed card and follow-up are shown.
6. On a fresh mock, post `{"next_job":{"outcome":"failed"}}` and approve: the card collapses as "Deleted nothing · failed: Access is denied"; the follow-up holds the 12 failed items ticked plus the 3 unticked ones.
7. Decline a pending card: it collapses to "Declined · N items".
8. Ask "windows cleanup" and approve the ticked operations: the card collapses to "Cleaned up 3 items · freed 12.6 GB · details ▸" and no follow-up card appears (system cleanups get none, ACT-FOLLOW-UP).
9. On a fresh mock, untick one item of the Review tab's Caches card and post `{"rebuild":true}` to `/api/mock`: the old review's other cards read "Replaced by a newer review · N items"; details ▸ lists their items with no buttons.
10. ▸ on a folder item of the lmstudio card's follow-up or a review card opens its tree; Show in Explorer is recorded by `GET /api/mock/revealed`.
(2026-10-09: step 8 checked against the mock in headless Chromium through Playwright.)
(2026-10-09: steps 9 and 10 pass against the mock in headless Chromium, step 10 on the review's Caches and Build output cards.)

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
renders this card in the chat like the other `propose_*` tools. An operation's label wraps rather than being cut. The
review's `windows` card (REVIEW-BUILD) uses the same rows under its own title; its `leftovers` operation ("Windows temp
files, update downloads, crash dumps and logs") shows the server's explanation (one administrator prompt, only files
untouched for a day go, files in use and the folders themselves stay) and, until it is done, "Show the N places it
clears", which lists each of its `parts` with its path, size and Show in Explorer.

**Verification (manual):**
1. In `ui/`, run the mock (`MOCK_IDLE=1 bun run mock`) and Vite with its `/api` proxy pointed at it; ask "windows cleanup".
2. The card reads "Clean up with Windows · free at least 12.6 GB" and lists five operations; component store, hibernation, and compaction are ticked; restore points and the Recycle Bin are unticked and tagged "cannot be undone"; hibernation shows 12.6 GB, the others "size unknown" except the Recycle Bin (3.1 GB); each row shows its explanation and note.
3. The footer says Windows asks for administrator permission once, and the button is "Clean up".
4. Tick restore points: the footer adds "Deleted restore points cannot be brought back." and the button becomes "Hold to clean up"; a click leaves the card pending.
5. Untick it and click "Clean up": the job runs and the card collapses to "Cleaned up 3 items · freed 12.6 GB · details ▸"; "details ▸" shows "done" on the three approved rows.
6. In the Review tab (UI-REVIEW step 3) the Windows card reads "Cleanups only Windows can do · free at least 23.9 GB" with three ticked operations: "Windows temp files, update downloads, crash dumps and logs" (6.4 GB, its explanation and "… in 5 places"), the component store (size unknown, "WinSxS holds 19.2 GB …") and hibernation (17.5 GB, "hiberfil.sys is 35.1 GB; the reduced size is about 17.6 GB (a fifth of the RAM)"). "Show the 5 places it clears" lists `SoftwareDistribution\Download\4b1f9c0e7d2a6f31` 3.4 GB, `LiveKernelReports\WATCHDOG` 1.1 GB, a `Windows\Temp` folder 922 MB, `WER\ReportArchive` 614 MB and `Logs\CBS\CbsPersist_20260912.cab` 410 MB; Show in Explorer on the first is recorded by `GET /api/mock/revealed`. At 430 px the labels wrap and the paths keep their ends.
7. Click "Clean up" (one click; nothing irreversible is ticked): the card collapses to "Cleaned up 3 items · freed 23.9 GB · details ▸", and details shows the leftovers row "done: 2,412 files deleted (6.4 GB); left: 3 in use, 41 newer than a day, 0 could not be deleted" with no places list.
(2026-10-09: steps 1–5 checked against the mock in headless Chromium through Playwright; ticking the Recycle Bin was not checked.)
(2026-10-09: steps 6 and 7 pass against the mock in headless Chromium through Playwright in the Review tab at 1440 px; step 6's wrapping was looked at in screenshots at 430 px in Marshmallow, Marshmallow Night and Rosewood.)

### UI-SETTINGS-AUTOSAVE: Settings save themselves; one Rescan now control
**Applies to:** hoardini
**Verification:** manual

The Settings tab shall have no Save button. A toggle or switch saves when it changes, one write at a time so the last reply shown is the last write's; a text field saves when it loses focus or 500 ms after the last keystroke (`drawer/autosave.tsx`). A "Saved" tick appears beside the control that saved, or the server's error appears inline under it and the control shows the stored value again. The tab holds the Fast / Lean switch ("Slower, stays out of your way" / "Finishes sooner, uses more CPU"; SRV-SETTINGS `index_speed`), the read-only index size, the Online checks switches (Hugging Face, winget; turning one on or off also records that the user answered, SRV-SETTINGS `online_checks`), and Clear chat history; the threshold, worker, memory-cap, redaction, and web-search settings are gone. The AI tab's provider form has no "Redact paths" toggle (AGENT-REDACT). The header shows one small "Rescan now" button while the index is idle, which runs a refresh at the switch's speed (SRV-ONE-REFRESH); while a refresh runs, its progress bar takes the button's place, with the leaf icon marking Lean, and there is no Scan fast button.

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
(`GET /api/copy-checks/waiting` above 0), a notice under the review header (`components/OnlineChecksConsent.tsx`,
UI-REVIEW) asks: "Check big files against Hugging Face and winget?", says how many big files wait, that Hoardini sends
file names and package or repo names and never file contents or hashes (the published hash is fetched and compared on
this PC), and that checks against the user's own copies, git, and cloud sync always run. Allow turns both services on,
Not now turns both off, and either records the answer (`asked` true) so the notice never returns; Choose in Settings opens
the Settings tab, whose switches also count as an answer. The review passes its `online` (`{asked, waiting}`,
REVIEW-TRIGGER), so the notice makes no request of its own; it reads the settings again once the Settings drawer was used,
and an answer marks the reviews on screen as asked. Without `online` it reads the settings and polls
`GET /api/copy-checks/waiting` every 30 s while unanswered.

**Verification (manual):**
1. In `ui/`, run the mock on a free port (`MOCK_PORT=7343 MOCK_IDLE=1 bun run mock`) and Vite with its `/api` proxy pointed at it; open the page.
2. In the Review tab, under the header, the notice shows "Check big files against Hugging Face and winget?" and "4 big files"; the page made no request to `/api/copy-checks/waiting` or `/api/settings`.
3. Choose in Settings: the Settings tab shows Online checks with Hugging Face and winget off. Turn Hugging Face on: "Saved" appears, `GET /api/settings` shows `asked` true and only `huggingface` on, and after a reload the notice is gone.
4. Reset the mock (`PUT /api/settings {"online_checks": {"asked": false, ...}}`), reload, press Not now: the notice goes and both services are off with `asked` true. Reset again and press Allow: both are on.
(2026-10-09: steps 1-4 pass, headless Chromium (Playwright) against the mock on port 7343 and Vite on 5293.)
(2026-10-09: step 2 rechecked in the Review tab against the mock; steps 3 and 4 were not run again.)
(2026-10-09: steps 2–4 pass against the mock in headless Chromium through Playwright with the notice reading the review's `online`: no request to `/api/copy-checks/waiting` or `/api/settings` at load, and turning Hugging Face on in Settings hid the notice as soon as the drawer closed.)

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
- **Map tab:** the folders card has List and Map tabs (arrow keys move between them, one tab stop, each controls the card's panel) and opens on Map when the call's `view` is `map` (AGENT-TOOL-DATA), else on List. Map draws the card's outermost rows (a row under another listed row would count twice) as a treemap (UI-TREEMAP-LAYOUT), plus `more` as one dotted block when the call was one level deep, each block with its name and size in the theme's colors; clicking a block shows its name, size and share with Ask, Clean up and Show in Explorer. **Open in Explore** shows when the shell provides an `onOpenExplore(path)` handler through `ExploreLinkContext` (`ui/src/explore/context.ts`), as the app does (UI-LAYOUT); it opens the Explore tab on the picked block, else the card's `under`, else the drive of its first row.
- **Changes card:** the headline is the free-space delta ("C: has 127 GB more free since 2:14 PM", "less free" when it fell; a time of day for a refresh earlier today, else the date), then the free-space chart (UI-CHANGES-CHART), "Freed X · Grew Y" (Freed sums removed and shrank rows, Grew sums grew and new rows, each row counted once with its listed ancestors), a Freed section (removed and shrank) and a Grew section (grew and new), each row with its signed delta, a bar, and "before → after", "new" or "removed". The volume root's own row is hidden, and a row under another row of the same section sits indented beneath it. Removed rows are struck through and have no buttons.
- **Row buttons** (shown on hover or keyboard focus, over the row's right end): ▸ opens the folder's ItemTree inline (UI-ITEM-TREE); **Ask** sends "What is {path} and is it safe to remove?"; **Clean up** (folders) opens a review of that folder in the Review tab (UI-REVIEW) and sends nothing; **Show in Explorer** (SRV-REVEAL). Ask is disabled while a run streams.
- A small **raw** toggle in the card's header shows the call's arguments and the tool text.

**Verification (manual):**
1. In `ui/`, run `MOCK_PORT=72xx MOCK_IDLE=1 bun run mock` and Vite with its `/api` proxy pointed at that port; open it at 1000 × 900.
2. Press Show hotspots. A "Largest folders on your drives" card shows 8 rows with bars and file counts and "Show all 10"; Windows says "some unreadable". A changes card reads "C: has 127 GB more free since 2:14 PM" with the chart and "Freed 124 GB · Grew 7.3 GB"; the `C:\` row is not listed; the Hugging Face model folder sits indented under `.cache` and the removed `deepseek` file under `models`, both struck through. No chip for `top_dirs`, `changes` or `suggest_next` shows.
3. Hover the `code` row: Ask, Clean up and Show in Explorer appear over the row's end and its file count hides. Press ▸: the ItemTree opens under it.
4. Press raw on the folders card: the arguments and the tool text show; press it again to hide them.
5. Press Clean up on the code row: the Review tab shows the review of C:\code (UI-REVIEW step 14) and the chat sends nothing. Ask "What's inside C:\code?", then Ask on the covenant-agent row of the new folders card: it sends "What is C:\code\covenant-agent and is it safe to remove?".
6. Ask "List the biggest files": the files card shows 8 rows with "modified" dates, "Show all 12", and "+3,184 more · 212 GB".
7. Reload: the same cards come back from the stored thread.
8. Repeat steps 2 and 6 with a dark theme (Marshmallow Night) and at 430 px wide: text, bars, and the green and red deltas stay readable, and rows do not overflow.
9. With `hoardini serve` and a model that calls `top_dirs`: the card renders live, and again after a reload from `GET /api/thread`.
10. Ask "Show me where my space goes": the folders card opens on its Map tab with a block per folder (Users, code, Games, Program Files, AI, Windows, ProgramData, …), names and sizes on the big ones. Click the code block: "code 312 GB · 18%" shows under the map with Ask, Clean up and Show in Explorer. Press List: the 8 rows show.
11. Press Map, then Open in Explore with nothing picked: the Explore tab opens on `C:\` (UI-LAYOUT).
12. Repeat step 10 in Marshmallow Night at 430 px wide: the map fits the card and its labels read.
(2026-10-09: steps 1–8 pass against the mock in headless Chromium through Playwright, in Marshmallow and Marshmallow Night. Step 9 passes against `hoardini serve` on a scratch folder with a scripted OpenAI-compatible model calling `top_dirs`, `changes` and `suggest_next`: both cards rendered live and after a reload, ▸ listed the folder from `/api/index/children`, no chip showed, and no model request held card data.)
(2026-10-09: steps 10–12 pass against the mock in headless Chromium through Playwright, in Marshmallow at 1000 px and Marshmallow Night at 430 px; step 2 still shows 8 rows and "Show all 10" with the mock's new drive shape.)
(2026-10-09: steps 5, 10 and 11 pass in the app's chat panel against the mock in headless Chromium, with Clean up opening the review of C:\code and Open in Explore the Explore tab on C:.)

### UI-ITEM-TREE: The ▸ tree under a folder
**Applies to:** hoardini
**Verification:** manual

`ItemTree` (`ui/src/components/ItemTree.tsx`; props `path`, optional `rootSize`, optional `actions(row)` slot) shall show a folder's children inline from `GET /api/index/children` (SRV-INDEX-CHILDREN), loading a level only when it is opened. Each row has ▸ when it has children, its name (full path as the tooltip), a size bar and its percentage of the parent (of `rootSize` at the top level, else of the folder), its file count, its last-modified date, the caller's buttons, and **Show in Explorer** (`POST /api/reveal`); a refusal shows inline on the row ("not found", "not on a watched drive") and keeps the row's buttons up. The buttons appear on hover or keyboard focus over the row's right end. Nested levels indent with a guide line. Under the rows come "N small files · X" when the folder has folded small files and "+N more · X" past 50 rows, which loads up to 1,000 rows (the server's cap, SRV-INDEX-CHILDREN) when pressed; past that it says "The biggest 1,000 are listed · N more · X". An empty folder says "Empty" and a failed load says why. As the card narrows, the date drops out first, then the file count, then the bar. Optional `selected`, `trail`, `onSelect(row)` and `onOpen(row)` make it a picker: with `onSelect` a row's name is a button that selects it (a double-click calls `onOpen`), the row whose path is `selected` is highlighted and scrolled into view, and the folders above it open by themselves, found by path or by `trail` (the folders on the way as the server spells them, since the profile shows as `~`). The chat cards (UI-TOOL-CARDS), the Explore view (UI-EXPLORE), the folder items of every proposal card (UI-PROPOSAL-COLLAPSE), and the review's Large areas and What is this? card (UI-REVIEW) use it.

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

Under the settled last answer the chat shall show up to 3 pills styled like the greeting's starters: hidden while a run streams, gone as soon as the user sends anything, and absent while the turn waits on a proposal card. The turn is everything after the last question or card decision. The model's pills from the turn's `suggest_next` results (AGENT-SUGGEST-NEXT) come first; pills built from the turn's cards fill the rest: from a folders card, "What's inside {name} ({size})?" for its biggest row (it sends "What's inside {path}?") and "Clean up {scope}" for its `under` folder, else its biggest row (the folder's name instead of the path when the path is long); from a files card, "What is {name} ({size})?" for its biggest file; from a changes card, "Why did {name} grow (+X)?" for its biggest top-level grower. A pill with the same kind and path as an earlier one, or the same label when it has no path, is skipped. An `ask` pill sends its label (a built one its full-path question). A `clean_up` pill opens the review of that folder in the Review tab (UI-REVIEW), and a `what_is` pill opens the What is this? card for its path there; neither sends anything. An `explore` pill calls the shell's `onOpenExplore(path)` (`ExploreLinkContext`, UI-TOOL-CARDS), which opens the Explore tab; while no handler is provided it is skipped and takes no slot. A pill of another kind than `ask` without a path is skipped. A pill never approves anything. The `suggest_next` call itself shows no chip.

**Verification (manual):**
1. Against the mock (UI-TOOL-CARDS step 1), press Show hotspots. Under the answer: "What's in covenant-agent?", "Clean up C:\code" (the model's; its third pill names a path not in the index and is dropped) and "What's inside Users (727 GB)?" (built from the folders card). No pills show while the answer streams.
2. Reload: the same three pills show.
3. Press "Clean up C:\code": the Review tab shows the review of C:\code, nothing is sent, and the pills stay. Ask "What's inside C:\code?": the new answer gets "Clean up C:\code\covenant-agent" (the model's), then "What's inside covenant-agent (161 GB)?" and "Clean up C:\code" (built).
4. Ask "List the biggest files": the pills are "Which models haven't I used lately?" and "What is llama-3.3-70b-Q4_K_M.gguf (42.5 GB)?".
5. Ask "what is in lmstudio": the turn ends on a proposal card and no pills show; approve it and the answer after the card shows no pills built from the earlier card.
6. Ask "Show me where my space goes": the pills are "Explore AppData\Local" (the model's), "Clean up C:\code" and "What's inside Users (727 GB)?"; pressing the first opens the Explore tab on `~\AppData\Local` and sends nothing.
7. Press Look closer on the llama.cpp area (UI-REVIEW step 9): the answer ends with the model's `what_is` pill "Say what llama.cpp is", which opens the What is this? card in the Review tab and sends nothing.
(2026-10-09: steps 1–5 pass against the mock in headless Chromium; against `hoardini serve` with a scripted model, the model's pill came first, then the two built from the folders card, and the same three came back after a reload.)
(2026-10-09: step 6 passes against the mock in headless Chromium; step 1 rechecked with the mock's new drive shape.)
(2026-10-09: steps 3, 6 and 7 pass in the app's chat panel against the mock in headless Chromium.)

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

`ExploreView` (`ui/src/explore/ExploreView.tsx`; props `root` (the drive or folder it opens on), `onAsk(text)`, `onCleanUp(path)`, optional `onWhatIsThis(path)` and `focus` (a path to open, from "Open in Explore" or an `explore` pill)) shall show one folder at a time in three linked panes as the Explore home tab (UI-LAYOUT), where the shell mounts it with the selected drive as `root`, the path of "Open in Explore" or an `explore` pill as `focus`, `onAsk` sending to the chat (which opens), `onCleanUp` opening the folder's review (UI-REVIEW) and `onWhatIsThis` opening the What is this? card in the Review tab:
- **Header:** breadcrumbs from the index root, the drive or a watched folder (SRV-INDEX-MAP `ancestors`), each a button; **Up** to the folder above; a switch between **What Hoardini knows** (the default) and **File type**, remembered per viewer (a radio group: arrow keys switch it, one tab stop); a legend; and "{size} · N blocks, smaller than X merged" (or "the 5,000 biggest blocks" when the map is capped).
- **Treemap:** a canvas drawing `GET /api/index/map` for the folder at the canvas size (refetched on zoom and on a resize past 64 px), laid out by UI-TREEMAP-LAYOUT with folder names and sizes in name strips, and names (and sizes, room allowing) on blocks big enough. By knowledge, a block is green when proposed, amber for review, grey for keep (lighter) and system (darker), blue for the user's data, and striped when unknown; a folder's strip is tinted by its own tag; merged "N more" blocks are dotted. By file type, files take the 8 biggest types of the `root` folder in a fixed order (dataviz categorical steps, light or dark by the theme), so a type keeps its color as you zoom; other files, folders and small files are grey. Colors come from the theme's CSS variables and follow a theme change at once; labels read at 4.5:1 (UI-THEME-CONTRAST). The canvas is one tab stop announced as an application ("treemap"). Hover shows a tooltip (name, path, size, share, files, tag). Click selects; double-click or Enter opens a folder (or a merged block's folder); Backspace or Escape goes up; arrow keys move between neighbors (Alt+arrows to the parent or first child).
- **Folders:** ItemTree (UI-ITEM-TREE) of the same folder at full width, with Zoom in on folder rows.
- **By type:** `GET /api/index/types` for the folder: swatch, extension, size, a bar, and its identity (the file-type row's text, else app and use, else the category), else the file count and **What is this?**, which asks the chat "What are .{ext} files?". Small files are one row. Clicking a type dims every block but its files (small files: the small blocks); Clear undoes it. When the map fails to load, it says "Nothing to list: the folder did not load." rather than loading forever.
- **Linking:** selecting a block, a tree row or a type selects in the other panes: the tree opens down to the selected block and marks it, the selected file's type row is marked, and the map outlines the selection. A zoom selects the folder it opened; Up selects the folder it came from.
- **Selection bar:** name, path, size, share of the view, files, and the tag (state, label, and the verdict when it is safe or redownload), with **Clean up** (folders; calls `onCleanUp`), **What is this?** (when `onWhatIsThis` is given, and not for a drive, `C:\Users` or a profile folder, which take no answer), **Ask Hoardini** (`onAsk("What is {path}?")`), **Show in Explorer** (SRV-REVEAL) and **Zoom in** (folders below the view, merged blocks).
- **Focus:** the selection (else the view's folder) is sent to `PUT /api/explore/focus` 600 ms after it settles and cleared when the view unmounts (SRV-EXPLORE-FOCUS). A `focus` that is a file opens its folder with the file selected.
- **Narrow:** below 768 px of width the panes stack: selection bar, treemap (22rem), folders, types, and the view scrolls.

**Verification (manual):**
1. In `ui/`, run `MOCK_PORT=72xx MOCK_IDLE=1 bun run mock` and Vite with its `/api` proxy on that port, and open the Explore tab at 1400 × 900 in Marshmallow (steps 1–9 first ran on a scratch page mounting `<ExploreView root="C:\" …>` full height).
2. The breadcrumb reads `C:`, the legend lists Proposed, Review, Keep, System, Your data, Unknown and merged, the tree leads with Users (727 GB), By type leads with .gguf ("model weights"), .q4nx reads "FastFlowLM NPU model weights", and the summary reads "1.8 TB · the 5,000 biggest blocks, smaller than … merged". Temp\claude and the `target` folders are green, the model folders, Docker and Games amber, Program Files and Windows dark grey, Documents, Pictures and OneDrive blue, covenant-agent's data striped.
3. Click the docker_data.vhdx block: the bar reads docker_data.vhdx, 78.2 GB, "Review Docker Desktop's WSL disk"; the tree opens Users › josha › AppData › Local › Docker › wsl › disk and marks the file; the .vhdx row is marked; within a second `GET /api/mock/focus` returns its path.
4. Press Ask Hoardini, What is this? and Show in Explorer: the handlers get "What is ~\AppData\Local\Docker\wsl\disk\docker_data.vhdx?" and the path, and the mock records the reveal. Click Games in the tree: the bar shows "Review Steam library · redownload"; Clean up calls `onCleanUp("C:\Games")`.
5. Click .q4nx in By type: everything but the .q4nx files dims; Clear restores it.
6. Switch to File type: blocks take type colors and the legend lists the 8 types; reload: File type is still on.
7. Double-click the code block: the map, breadcrumbs (C: › code) and tree show C:\code. Up returns to C: with code selected. With the map focused, Arrow Right selects the next block, Enter zooms into a folder and Backspace zooms out.
8. Open with `focus` = `~\.flm\models\Qwen3.8-27B-NPU2\model.q4nx`: the view shows Qwen3.8-27B-NPU2 with model.q4nx selected.
9. Repeat step 2 in Marshmallow Night, and at 430 px wide in both: the panes stack, labels and the selection bar's buttons read, and nothing scrolls sideways.
10. Against `hoardini serve` on a scratch index: the map and types load, a selection reaches `explore_focus`, and a chat question after it carries "The user is looking at … in Explore." (AGENT-PROMPT-CONTEXT).
11. In the app at 1440 × 900: press "Explore AppData\Local" (UI-FOLLOW-UPS step 6): the breadcrumbs read C: › Users › josha › AppData › Local and `GET /api/mock/focus` returns `~\AppData\Local`. Pick Docker in the tree and press What is this?: the Review tab shows the What is this? card for `~\AppData\Local\Docker`, and the focus is now null. Back in Explore, pick Google and press Ask Hoardini: the chat panel sends "What is ~\AppData\Local\Google?"; press Clean up: the Review tab shows the review of that folder.
12. In the app at 1440 × 900 against the mock: with C: selected the bar has Ask Hoardini but no What is this?. Focus What Hoardini knows and press Arrow Right: File type is checked and focused. Open Users › josha in the tree, pick AppData, press What is this? and Keep: the card shows "~\AppData holds other folders; answer for a folder inside it" (the mock refuses containers as the server does).
(2026-10-09: steps 1–9 pass against the mock in headless Chromium through Playwright, on a scratch page mounting ExploreView, with no element past 430 px in step 9. Step 10 passes against a release `hoardini serve` on a scratch folder: the map and types came from its index, selecting `models` in the tree stored the focus, and a chat run against a stub OpenAI-compatible model sent "The user is looking at ~\…\hdata\models in Explore." in its system message. On a copy of a real 1.6 TB C: index, the map of C: (5,000 blocks) answered in 0.35 s and its types in 0.2 s.)
(2026-10-09: step 11 passes in the app against the mock in headless Chromium; the focus went from `~\AppData\Local` to null when What is this? switched to the Review tab.)
(2026-10-09: step 12 passes in the app against the mock in headless Chromium through Playwright.)

### UI-LEARNED-CHIP: What Hoardini learned shows as a chip with Undo
**Applies to:** hoardini
**Verification:** manual

`LearnedChip` (`ui/src/components/LearnedChip.tsx`; props `text`, optional `url`, `onUndo`, optional `onPin`, optional `onConfirm`, optional `confirmed`, optional `pinned`) shall show one line: "learned", the text (the full text as its tooltip), the source link's host when there is a `url`, and its buttons. Undo turns it into "undone" with the text struck through and no buttons; Confirm turns the label into "learned · confirmed"; Pin toggles "learned · pinned" and Unpin. A chip whose row is already confirmed (`confirmed`) starts as "learned · confirmed" with no Confirm, and one whose row is pinned (`pinned`) as "learned · pinned" with Unpin, so a reload shows what the user did. A failed call shows its error inline and keeps the buttons.
- **After a job:** a finished proposal card (collapsed or opened, UI-PROPOSAL-COLLAPSE) shows one chip per entry of the proposal's `learned` (SRV-LEARNED) directly under it, above any follow-up card, with Pin (`PUT /api/memory/{id}` `pinned`) and Undo (`DELETE /api/proposals/{id}/learned/{memory_id}`).
- **After a web search:** a `web_search` call whose `data` is `{kind: "learned"}` (AGENT-WEB-LEARN; live from `tool_data`, after a reload from the thread) shows its chip under the search's tool chip, with the source link, Confirm (`POST /api/memory/{id}/confirm`) and Undo (`DELETE /api/learned/web/{memory_id}` with the item's `key` and `text`, SRV-LEARNED, so a reused id is never deleted).

**Verification (manual):**
1. In `ui/`, run `MOCK_PORT=72xx MOCK_IDLE=1 bun run mock` and Vite with its `/api` proxy pointed at it; open it at 1000 × 900.
2. Post `{"next_job": {"ms": 1200}}` to `/api/mock`, ask "what is in lmstudio" and hold to delete. When the job ends the card reads "Deleted 12 items · freed 53.6 GB · details ▸", and directly under it, above the follow-up card, a chip reads "learned 12 items in ~\.lmstudio — cleaned 53.6 GB on <MM-DD>; 2 items you left unticked are kept" with Pin and Undo.
3. Press Pin: the label becomes "learned · pinned" and the row (source `job:<id>`) is pinned in `GET /api/memory`. Reload: the chip still reads "learned · pinned" with Unpin.
4. Press Undo: the chip reads "undone" with the text struck through and no buttons; `GET /api/proposals/{id}` has `learned: []` and the row is gone. After a reload no chip shows.
5. Ask "what is ~\.hipfire": under the web search chip a chip reads "learned ~\.hipfire: model cache (hipfire)." with an `example.com` link, Confirm and Undo.
6. Press Confirm: the label becomes "learned · confirmed" and the row is source `user` and pinned. After a reload the chip is back under the web search chip, still "learned · confirmed", with no Confirm.
7. Press Undo: "undone", and the row is gone from `GET /api/memory`.
8. At 430 px wide in a dark theme each chip stays on one line, the text cut short with the full text as its tooltip.
9. Against `hoardini serve` with a scratch `HOARDINI_DATA_DIR` and `--db`, a scripted OpenAI-compatible model and a SearXNG stand-in: a model's `propose_delete` of two `.log` files and one other file in a scratch folder, approved with the other file unticked, collapses to "Deleted 2 items · freed 16.0 KB" with the chip ".log files in ~\...\work — cleaned 16.0 KB on <MM-DD>; 1 item you left unticked is kept", still there after a reload; a `web_search` about `~\.hipfire` shows its chip live and after a reload; Confirm makes the row `user`-sourced; Undo on the job chip leaves `learned: []` and no `job:` row.
(2026-10-09: steps 1–8 pass against the mock in headless Chromium through Playwright, in the default theme and at 430 px with a dark color scheme; step 9 passes against `hoardini serve` on a scratch folder with a scripted model and search stand-in, and the learning request had no tools, no profile name, and the results inside the untrusted fence.)
(2026-10-09: steps 3, 5, 6 and 7 pass against the mock in headless Chromium through Playwright, step 3 on the Review tab's Build output card after UI-REVIEW step 5: after a reload the pinned job chip read "learned · pinned" with Unpin, and the confirmed web chip "learned · confirmed" with no Confirm. Both runs of step 7 undid the web chip through `DELETE /api/memory/{id}`; its `DELETE /api/learned/web/{memory_id}` route came later and step 7 has not been run with it.)

### UI-DRIVE-SELECTOR: The header shows one drive, picked from a menu
**Applies to:** hoardini
**Verification:** manual

The header shall show only the selected drive's free-space bar (INDEX-VOLUMES), so drives never stack. Its drive letter is a dropdown button (`C: ▾`) that opens a menu of every fixed drive (`GET /api/volumes`) with its letter, label (or "Local Disk"), a free-space bar, and "N free of M". A watched folder (a test setup) shows as `…\<last segment>`, with the full path as its tooltip.

- A watched drive has a check. Clicking it selects it (`PUT /api/volumes`) and closes the menu; the header bar, the mascot's girth (UI-MASCOT-FULLNESS), and the Review tab (UI-REVIEW, which opens that drive's review) follow it.
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
4. Open it and click D:. The menu closes, the header shows `D:` and its bar, the mascot slims to D:'s fullness (tooltip "D: is 56% full"), and the Review tab reads "Review of D:".
5. Open it and press Add on E:. The header shows `E:`, and a fast scan starts ("Scanning fast").
6. Open it and press Stop watching on D:. The confirm step appears with focus on Cancel, and Escape cancels it. Press it again and confirm: D: loses its check and shows Add.
7. Stop watching every drive but one: the last one's Stop watching is disabled, with "At least one drive stays watched".
8. Start `hoardini serve --port 72xx --token dev --no-refresh` against a scratch `HOARDINI_DATA_DIR`, with Vite proxying to that port. Use `PUT /api/volumes` to watch three scratch folders, then rename one. The menu shows it greyed as "not connected", and Rescan now skips it (the log says `volume not connected; skipped`). Stop watching the selected folder: the log says `stopped watching; its index is deleted`, and the next folder becomes selected.

(2026-10-09: steps 1–7 pass against the mock in headless Chromium on ports 7291 and 5291. Step 8 passes against `hoardini serve --port 7291 --no-refresh` with three scratch folders: the renamed one showed "not connected" and was skipped by a refresh, and stopping the selected one deleted its 3 index rows and selected the next. The machine's `subst` drives (X:, Y:, Z:) were not listed.)
(2026-10-09: step 4 rechecked against the mock with the review: picking D: showed "Review of D:".)
