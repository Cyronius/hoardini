# UI specification (prefix UI)

### UI-MD-NO-HTML: Answer markdown drops raw HTML
**Applies to:** hoardini
**Verification:** test

Answer markdown (UI-ASK) shall render raw HTML in model or web-sourced text as literal text. No `script`, `iframe`, or event-handler attribute reaches the DOM, and no `javascript:` link is emitted.

**Acceptance criteria:**
- `before <script>alert(1)</script> after` → no `script` element; "before", "alert(1)", "after" appear as text
- `<img src=x onerror="alert(2)">` → no element carries `onerror`
- `<iframe src="http://example.com">inner text</iframe>` → no `iframe` element
- `[click me](javascript:alert(1))` and `<a href="javascript:alert(2)">raw link</a>` → no `a` with a `javascript:` href; link text survives
- `**bold**` still renders as formatted text

External tests: none. Test: `specs/ui/tests/markdown.test.ts`.

### UI-MD-KEEP-TEXT: Finished answer text is never cut
**Applies to:** hoardini
**Verification:** test

A finished answer (UI-ASK) shall render all of its text. The incomplete-markdown repair that hides a half-streamed tag applies only to the message still streaming, so a bare `<` (`a<b`, `<100 MB`, a quoted `<<<UNTRUSTED_WEB_CONTENT` fence) never swallows the rest of a finished message.

**Acceptance criteria:**
- `Files <100 MB stay and a<b and c` renders with exactly that text.
- `Search result: <<<UNTRUSTED_WEB_CONTENT: data [1] Unsloth https://x.example/a Safe to delete` keeps the URL and the snippet.

Test: `specs/ui/tests/markdown.test.ts`.

### UI-LAYOUT: Home is the one view
**Applies to:** hoardini
**Verification:** manual

The window shall show the header (Hoardini, the drive, "Checked N ago" with Look again now, and Settings) over Home (UI-HOME), and nothing else: no tabs and no chat panel (`ui/src/shell/Shell.tsx`). A Your call row opens its detail sheet (UI-DETAIL) beside Home from 960 px wide, and over it, as a sheet from the bottom, on a narrower window. Settings (UI-SETTINGS) opens as a drawer from the right. FirstRun still covers the app until the AI is set up. While the index refreshes, the header says "Looking N%" and a thin progress line runs under it; every control stays usable while Hoardini looks or answers. If Home fails to render, the screen says "Something went wrong on this screen." with the error and Try again, which reopens the review, while the header and Settings keep working.

**Verification (manual):**
1. In `ui/`, run `MOCK_IDLE=1 bun run mock` and `bunx vite`; open http://localhost:5193/#token=dev at 1440 × 900 and press Later if FirstRun shows.
2. The window shows the header and Home only; there is no tab bar and no chat panel.
3. Click a Your call row: its sheet opens on the right and the row is marked. Resize to 430 × 900: the sheet covers the window from the bottom; Escape closes it.
4. Post `{"rebuild":{"ms":4000}}` to `http://127.0.0.1:7043/api/mock?token=dev` and start a refresh: the header reads "Looking N%" with the line under it, and Clear, the rows and Settings all respond meanwhile.
5. Answer `POST /api/review/open` with a review whose `areas` is null (a Playwright route) and reload: Home shows "Something went wrong on this screen." with Try again; drop the route and press Try again: Home comes back.

### UI-HOME: One question, three groups
**Applies to:** hoardini
**Verification:** test

Home (`ui/src/home/Home.tsx`) shall show the latest review of the selected drive as three groups, built by `homeOf(view, proposals, gone)` in `ui/src/home/groups.ts`:
- **Junk you won't miss**: the `temp`, `caches`, `build`, `windows` and `disposable` review cards (REVIEW-BUILD), as one row with its size and one hold-to-confirm button, "Clear X" (UI-DETAIL's HoldPill: it fills red while held, says "Keep holding…" and "Deleted for good. There's no Recycle Bin step.", and does nothing if released early). Under it each part is always listed, the first 6 items with "Show N more" like Your call: each item with a tick, the item's name, its full path and one short reason, and the whole row beside the tick (name, path, reason, size and a › at the right) is one button that opens the same drill-in sheet as a Your call row (what is inside, Find out what this is, Ask, Delete, Shrink or Move), and blocked items with "Stays: reason" and no tick. Everything starts as the card ticked it.
- **Free space, keep the files**: the `compress` and `online_only` cards, the same way, with a plain "Free X" button (nothing is lost, so no hold).
- **Your call**: everything big that needs a person's judgment, biggest first: the review's large areas (REVIEW-AREAS) that are not hands off, the rarely used apps (ACT-APPS-RARELY-USED), and the `downloads`, `redownload` and `archive` cards as one row each. A row shows a plain name, one short line and the size; the 8 biggest show, then "Show N more".

A group counts only items that are ticked, not blocked and still pending, and a card counts only while its proposal is pending and has something left to choose; a compress item counts what it saves. A part whose items need Windows to measure them makes the junk size "at least". The headline is "You can free X without losing anything." (junk plus keep-the-files), with "And N big things are waiting for your call." under it, or "N big things are waiting for your call." when nothing can go without a decision. Areas in the `system`, `protected` or `container` categories, and areas with a `keep` verdict and no action left that the user did not give, are hands off and never listed. Areas the user kept (`user` and verdict `keep`) are counted at the bottom as "N things you kept · Show", whose list offers Bring back (SRV-LOCATION-FORGET, then a new review). Paths acted on this session are left out until a new review drops them. Pressing Clear or Free approves each part with something ticked (one job per part); the row then says "Clearing… N%" or "Freeing… N%" and finally "Freed X", or the first error. Nested `data-proposal-id` markers let the mascot's trip find the row (UI-MASCOT-JOBS). When there is nothing in any group the screen says "All clean. X free on C:." and "Nothing big is waiting. I'll keep an eye on things."; before the first review it says Hoardini is looking around, and before the first index that he is taking his first look. Any finished change (a sheet's Delete, Shrink or Move, or a group's Clear or Free) hides what it touched at once, then rebuilds the list from the index, which each action has already patched for just the paths it touched (`useReview.changed()`, no drive rescan), so nothing acted on lingers.

**Acceptance criteria:**
- Cards `temp` (2 GB ticked, 1 GB unticked, 4 GB blocked), `caches` (3 GB), `windows` (an operation of unknown size), `compress` (10 GB saving 4 GB) and `online_only` (1 GB): junk is `temp`, `caches`, `windows`, 5 GB from 3 items, "at least"; keep-the-files is 5 GB; the headline sum is 10 GB; Your call is empty.
- A `done` card, a card whose only item is done, and a card with only unticked items: only the last is a part, at 0 bytes.
- With areas EncryptedClips (photos, keep, `[archive]`, 51 GB), a user-kept folder, `C:\Windows` (system) and `C:\ProgramData` (catalog keep, no actions), a Steam app of 48 GB never played, an unknown 9 GB folder and a `downloads` card of 2 + 1 GB: Your call is EncryptedClips ("Your photos and videos"), Elden Ring ("Steam game, never played"), Big ("Hoardini doesn't know what this is yet"), then the downloads row at 3 GB; kept holds the user-kept folder only.
- A path acted on this session drops its row and every row inside it.
- A tick changed on screen (the store's proposal) wins over the view's copy.

Test: `specs/ui/tests/home-groups.test.ts`.

**Verification (manual):**
1. Against the mock (UI-LAYOUT step 1), Home reads "You can free 316 GB without losing anything." and "And 23 big things are waiting for your call."; Junk you won't miss shows "at least 310 GB" and "Clear 310 GB"; Free space, keep the files shows "Free 6.5 GB"; Your call lists 8 rows and "Show 15 more".
2. Press Show on junk: the parts list their items with ticks; untick one and the size and button drop at once; reload and it stays unticked.
3. Hold Clear for less than a second and let go: nothing happens. Hold it through: the red fill and warning show while held, then the row reads "Clearing… N%", Hoardini climbs down to it (UI-MASCOT-JOBS), and it ends "Freed 310 GB".
4. Press Free 6.5 GB: it starts at once, without a hold.
5. With a kept folder (answer Keep in a sheet), the bottom reads "1 thing you kept · Show"; Show lists it with Bring back, which brings it back into Your call after the new review.

### UI-DETAIL: The detail sheet
**Applies to:** hoardini
**Verification:** manual

A Your call row's sheet (`ui/src/home/DetailSheet.tsx`) shall show its name, size and path (with a brass "Show in Explorer" button under the path, SRV-SHOW-IN-EXPLORER; every folder-listing row has the same action as a brass-tinted icon button), then Hoardini in a speech bubble saying what it is in a sentence or two, then the buttons that fit it, then the Ask box (UI-ASK):
- **An area:** the bubble is the area's own text (or, for an unknown folder, that Hoardini doesn't know what it is yet and can find out), with "Why I think so" opening the evidence: the top file types with their share, the area's hint, whether it is the only copy Hoardini knows of, and the label Hoardini files it under. "What's inside" draws its biggest children as a bar and rows with share and size; a folder row opens the same sheet for that folder, with ← Back. The buttons are Delete X (hold to confirm, SRV-ACT), Shrink when the area allows compressing, Move to the first destination when one exists, and Keep (a Keep answer, SRV-LOCATION-ANSWER, after which the row leaves Your call). A game library offers no Delete, only the advice to uninstall games from their launcher. Hoardini's guesses never hide Delete: what holds a path back is decided when it runs (POLICY-LOCATION-KEEP). An unknown folder, and a folder opened from What's inside, also gets Find out what this is, first in the row, which puts "What is this? Look inside it, and search the web if that helps. Then tell me whether to keep it, delete it, or move it elsewhere." into the Ask box's conversation (UI-ASK) and is off while an answer is coming; the Ask box stays for the user's own questions.
- **An app:** a Steam game or app with when it was last used; Uninstall (Uninstall in Steam for a game, ACT-UNINSTALL).
- **A card row:** its items with ticks and one button: Delete X (hold) for `downloads` and `redownload`, Move X to the destination for `archive`.

After an action the sheet shows its progress ("Deleting… N%"), then "Deleted. Freed X." and closes the row after a moment, or, when something was refused, a red box ("Some of it stayed." / "Couldn't delete this.") with each reason in plain words and "Try again". A reason of the form `Open in <app> (pid N)` (POLICY-IN-USE) reads "<app> has a file in here open. Close it, then try again."; pid 4 (Windows itself, holding a virtual disk while WSL or Docker runs) reads "Windows has a file in here open. WSL or Docker is probably running. Shut it down (wsl --shutdown), then try again."; the server's own text shows beneath in small type.

**Verification (manual):**
1. Against the mock, open the `models` row (FastFlowLM models): the bubble quotes its text; Why I think so lists its file types; What's inside lists Qwen3.6-35B-A3B-NPU2 first at 37%.
2. Open a child folder and press ← Back: the sheet returns to `models`.
3. Hold Delete through: "Deleting… N%" then "Deleted. Freed 104 GB.", and the row leaves Your call.
4. Answer Never touch for a folder through the old route (`POST /api/locations/answer`), then hold Delete on it: a red "Couldn't delete this." box with the protect reason, and Try again.
5. Open Games (a game library): no Delete button, only the launcher advice and Keep.
6. Open Cyberpunk 2077: "A Steam game you last played on Jun 5…" and Uninstall in Steam, which records the uninstall in the mock (`GET /api/mock/uninstalled`).
7. Open an unknown folder (covenant-agent): Find out what this is leads the buttons; press it: "What is this? Look inside it, …" shows above the Ask box, then "Looking inside…" and the answer, and the button is off until the answer ends. A known area (models) has no such button.

### UI-ASK: Ask about one item
**Applies to:** hoardini
**Verification:** manual

The Ask box at the bottom of a sheet (`ui/src/home/Ask.tsx`) shall send a question about that item to the agent on the item's own thread (AGENT-ASK: `item:` and a short hash of the path), the first question prefixed "About <path> (<name>): " so the agent knows the subject; the screen shows only what was typed. The answer streams in as markdown (UI-MD-NO-HTML, UI-MD-KEEP-TEXT); while the agent looks inside something the box says "Looking inside…", else "Thinking…", and Hoardini stomps (UI-MASCOT-FULLNESS). Stop ends the run. Reopening the sheet later loads the item's earlier questions and answers (`GET /api/thread?id=`), the last 6 turns shown. A failure shows the provider's error in one line. Nothing else on screen waits for the answer.

**Verification (manual):**
1. Against the mock, open a row, type "Do I need the saved clips?" and press Enter: "Thinking…" then "Looking inside…", the header sumo stomps, and a markdown answer with **Keep them** in bold appears.
2. Close and reopen the same row: the question and answer are still there. Open another row: its box is empty.
3. Against `hoardini serve` with a model: the run is on the item's own thread (AGENT-ASK), and the answer arrives in the box.

### UI-SETTINGS: Settings fit on one screen
**Applies to:** hoardini
**Verification:** manual

Settings (`ui/src/drawer/Drawer.tsx`) shall open on five rows: **Brain** (the active provider and model; Change opens the AI setup, UI-FIRST-RUN), **Move big things to** (the destinations, or "Not set up. Adds a Move button for your own files."; Set up opens the destinations page), **Things you kept** (how many, with Show listing them with Bring back), **Look** (the theme picker itself, UI-THEME-TAB), and **Text size** (Small / Medium / Large, applied at once and kept across reloads; Medium is the default and its smallest text is 13px). Under them, Advanced lists AI providers, Places to move things, Web lookups, Hoardini's notes and Scanning (UI-SETTINGS-AUTOSAVE), each a page with ← Settings. The tip line sits at the bottom (UI-TIP-LINE).

**Verification (manual):**
1. Against the mock, press Settings: the five rows show with "LM Studio (qwen3.8-27b)", "NAS", "Nothing yet", Look on Match Windows and Text size on Medium.
   Press Large under Text size: all text grows at once; reload and it stays Large. Press Small and it shrinks.
2. Press Dark under Look: the app turns dark at once and the dark themes list under it.
3. Press Change on Brain: Settings closes and AI setup opens.
4. Open each Advanced page and come back with ← Settings.

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

### UI-MASCOT-FULLNESS: Hoardini's pose is the app's status
**Applies to:** hoardini
**Verification:** manual

The header mascot is a sumo wrestler whose belly tracks the used fraction (1 - free/total) of the selected drive (UI-DRIVE-SELECTOR; else the `C:\` volume, else the first watched volume), growing continuously and steeply near full, and his tooltip names that drive. His pose says what the app is doing, in this order: celebrating for 5 s after a job frees space; carrying a box while a hold-to-confirm button is held; the shiko stomp while an item question is being answered or any agent run is in progress; looking around with a magnifying glass while the index refreshes or a review is built; worried (sweat, an "o" mouth, raised brows) while the drive is at least 90% full; napping with drifting z's when Home has nothing to show; otherwise standing. He has thick brows and eye highlights, a round topknot, a belt with hanging cords, a belly fold and a floor shadow. With reduced motion none of these poses loops.

**Verification (manual):**
1. In `ui/`, run `MOCK_FILL=0.30 MOCK_IDLE=1 bun run mock` and `bunx vite`; open http://localhost:5193/#token=dev.
2. He is slim and standing; his tooltip reads "Hoardini, a sumo wrestler - C: is 30% full, relaxing".
3. Post `{"fill":0.95}` to `/api/mock?token=dev`: he widens and looks worried, with a drop of sweat.
4. Hold Clear without letting go: he carries a box; let go early: he stands again.
5. Ask about an item: he stomps until the answer ends. Start a refresh: he looks around with the magnifying glass.
6. Clear everything until Home says "All clean.": he naps, with z's drifting up.
7. Enable reduced motion: none of the poses loops.
(2026-10-09, drive switching: headless Chromium against the mock; the tooltip went from "C: is 97% full" to "D: is 56% full" and the girth followed.)

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

The climb starts when the hold completes, before the server has answered (an approve that waits on a database lock or a queued job still shows him working); the job that follows takes the trip over, and a failed approve ends it with the slip. When an approved job from a Home group (UI-HOME) starts running or is queued and the group's action area (where its Clear or Free button, then its progress and result, sit) is on screen, the header sumo hides and a travelling copy climbs down a ladder from the header to stand on the footer's top edge (1.6 s; a fat sumo bends the rungs). If the footer is not on screen but the card's top edge is, he stands on the top edge instead. When the parent card collapses mid-trip (§ UI-PROPOSAL-COLLAPSE) he moves to the follow-up card's top edge, or to the collapsed line if there is no follow-up, and is never left over nothing. A parcel pops out of the card next to him with a file or folder icon, the first item's name, the size, and "×N" for several items. The overlay never moves the card. He stands above the edge he is anchored to, never over the buttons. Whatever the job's speed, the beats last at least this long: working 2 s (even if the job finished during the climb down), the finisher about 1.8 s, the climb back 1.4 s, then pleased 3 s; about 10 s in all. While the job runs he works the parcel in a loop, and when it finishes he plays the move for its kind:

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

He then climbs back up, deflating on the way if space was freed, and in the header is pleased with himself for 3 s (chin up, eyes closed, arms crossed, a pat-pat of the hands); the rest of that celebration is skipped. A declined proposal gets no trip, only a head shake in the header. If neither the footer nor the card's top edge is on screen when the job starts, or the page is hidden, there is no trip: he stomps in the header while the job runs (at least 2 s), plays the move there when it ends, then celebrates for 3 s. After about 30 s of work he climbs back and the move plays in the header when the job ends. Only one trip happens at a time; a job that starts while he is out runs without one. Clicking anywhere sends him home quickly, and the job carries on from the header. With `prefers-reduced-motion` there is no trip and no parcel: he holds the still stomp pose while the job runs and shows the pleased face when it succeeds. The overlay never takes pointer input, and the mascot never delays or drives a job. Cards stay mounted behind hidden tabs and in a hidden chat panel (UI-LAYOUT), so he looks for the copy of the card that is on screen; a card behind another tab counts as off screen, and switching tabs during a trip sends him home as when the card scrolls away.

**Verification (manual):**
1. In `ui/`, run `MOCK_IDLE=1 MOCK_FILL=0.9 bun run mock` and `bunx vite`; open http://localhost:5193/#token=dev at about 1000 × 800.
2. Hold Clear on Junk you won't miss (UI-HOME) through. The header sumo vanishes and a copy climbs down a ladder whose rungs sag under him, lands on the junk row, and a parcel pops out beside him. He pushes it while the row reads "Clearing… N%", then charges and bumps it off the right edge, spinning. He climbs back, slimming with puffs of air, and stands in the header with arms crossed and eyes closed for 3 s, then idles.
3. Before an approval, post one of these to `http://127.0.0.1:7043/api/mock?token=dev` and then press Free (keep-the-files) or hold Clear:
   - `{"next_job":{"kind":"compress"}}`: he hops onto the parcel and bounces on it, then belly flops and the parcel goes flat.
   - `{"next_job":{"kind":"archive"}}`: he windmills his arm, then throws the parcel into a box that pops up to his right; the lid shuts.
   - `{"next_job":{"kind":"archive","cloud":true}}`: the same throw goes up into a cloud, which puffs.
   - `{"next_job":{"outcome":"failed"}}`: he slips onto his bottom with crossed-out eyes and rubs his head; the row shows the error.
   - `{"next_job":{"outcome":"cancelled"}}`: the parcel hops back and he shrugs.
4. Delete from a detail sheet (UI-DETAIL): the sheet has no footer for him to stand on, so he stomps in the header while it runs and plays the move there when it ends.

### UI-SETTINGS-AUTOSAVE: Settings save themselves; one Rescan now control
**Applies to:** hoardini
**Verification:** manual

The Scanning page (Settings → Advanced, UI-SETTINGS) shall have no Save button. A toggle or switch saves when it changes, one write at a time so the last reply shown is the last write's; a text field saves when it loses focus or 500 ms after the last keystroke (`drawer/autosave.tsx`). A "Saved" tick appears beside the control that saved, or the server's error appears inline under it and the control shows the stored value again. The tab holds the read-only index size, Rescan now, the Online checks switch ("Look up big files online", on by default; one switch for the Hugging Face and winget lookups, which sets both and records that the user answered, SRV-SETTINGS `online_checks`; it reads on while either is on); the refresh speed, threshold, worker, memory-cap, redaction, and web-search settings are gone. The AI tab's provider form has no "Redact paths" toggle (AGENT-REDACT). The header shows one small "Rescan now" button while the index is idle, which runs a refresh at the switch's speed (SRV-ONE-REFRESH); while a refresh runs, its progress bar takes the button's place, with the leaf icon marking Lean, and there is no Scan fast button.

**Verification (manual):**
1. Start `hoardini serve --port 72xx --ui-dir ui/dist` against a scratch `HOARDINI_DATA_DIR` whose meta `indexed_volumes` points at a small folder, and open the printed URL.
2. Header: once the first review is built, "Checked just now" and Look again now (UI-LAYOUT); no Rescan control in the header.
3. Settings: the index size and Rescan now; no Save button, no speed switch, and none of the removed settings.
4. Rescan now on the Scanning page: it reads "Rescanning…" and the header shows "Looking N%" until the scan finishes.
5. (Removed 2026-10-17 with the speed switch.)
6. Stop the server and flip the Online checks switch: an inline error appears under it and no tick.
7. AI tab → add or edit a provider: no "Redact paths" toggle.
8. In DevTools → Network, switch to another window and back, then minimize the browser and restore it: `/api/presence` is posted with `looking: false` on blur and `true` on focus, and the `/api/status/stream` request ends while the page is hidden and a new one starts when it is shown, without the header's "offline" badge.
9. Settings → Online checks: "Look up big files online" is checked on a fresh database. Uncheck it: "Saved" appears and `GET /api/settings` shows `asked` true with `huggingface` and `winget` false; reload and it is still unchecked. Check it: both are true.
(2026-10-09, partial: headless Chromium against `hoardini serve --port 7231 --ui-dir ui/dist` with scratch data. Steps 2 and 3 pass; Rescan now in Lean started a `background` refresh. Step 7 was checked in code only, since no provider form was opened. Step 8 passed with a simulated `visibilitychange`: the stream ended and reopened with no badge, and presence posted `false` then `true`. Steps 5 and 6 were not run; SRV-ONE-REFRESH covers the takeover server-side.)

### UI-FIRST-RUN: First-run AI setup
**Applies to:** hoardini-ui, hoardini-server
**Verification:** manual

While `meta.ai_setup_done` is unset and no provider exists, a pop-up sets up the AI. It opens on one Recommended choice, Granite 4.1 3B (PROV-BUILTIN-DOWNLOAD), with its download size and one Download button, and says the AI runs on this PC. Under it, "Other ways to connect" opens more choices in this order: the six free providers of PROV-PRESETS (Google Gemini, OpenRouter, Ollama Cloud, Mistral, Cloudflare Workers AI, Z.ai), each with one plain line on its free allowance, whether a card is needed, and whether it may learn from your questions; then A different model on this PC (Qwen3.5 9B, or a model file, PROV-BUILTIN-FILE), LM Studio, Ollama or llama.cpp (one that is already running), and Set up a provider. Cloudflare's form also asks for the account ID that goes into its base URL. Each cloud choice has three steps: "Get a free key" (opens the provider's key page in the system browser), a key field (saved through the vault), and an automatic test with the model preselected. "Later" sets done; the app then shows a one-line "No AI is set up yet. Set up AI" notice while no provider is active. The AI tab has "Run setup again". While a 429 wait runs, a "Waiting on the free tier..." notice counts down.

**Verification (manual):**
1. Start with `HOARDINI_DATA_DIR` set to an empty scratch folder. The pop-up shows Granite 4.1 3B as Recommended with a Download button and "Other ways to connect" under it (press it for the other choices); the first index scan runs behind it.
2. Gemini, OpenRouter, Ollama Cloud, Mistral, Cloudflare (with an account ID) and Z.ai each: Get a free key opens the system browser (not an in-app window); paste a key; Save and test ends "Working" and the provider is active with its default model. A wrong key shows the provider's error and can be retried.
3. A model on this PC with LM Studio running lists its models and Use this makes it active; with nothing running it shows the LM Studio link and Check again.
4. Set up a provider closes the pop-up and opens the AI tab with the add form. On the first screen, Download on Granite ends with the pop-up closed and the Built-in provider active. A different model on this PC shows Qwen3.5 9B and Granite 4.1 3B with their sizes and the file box.
5. Later closes it; the "Set up AI" notice appears under the header and reopens the pop-up. After a restart the pop-up does not return.
6. An existing data dir with a provider never shows the pop-up. AI tab, Run setup again, reopens it.
7. Against a mock answering 429 with `Retry-After: 5` then 200, the notice counts down from 5 and the answer then arrives.

### UI-TIP-LINE: Tip line in Settings
**Applies to:** hoardini
**Verification:** manual

The bottom of Settings (UI-SETTINGS) shall carry one line, "♥ Like this? Tip my local NPR station", whose Tip link opens https://www.kuaf.com/donate in the system browser, with a tooltip naming KUAF as the maintainer's local NPR member station, not affiliated with Hoardini. Nothing else in the app asks for money.

**Verification (manual):**
1. Open Settings: the line sits under Advanced.
2. Click Tip: the system browser opens the KUAF donate page.

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

### UI-THEME-TAB: Group-first theme picker in Settings
**Applies to:** hoardini
**Verification:** manual

Settings → Look (UI-SETTINGS, `ui/src/drawer/LookPicker.tsx`) is the theme picker: one Match Windows / Light / Midtone / Dark switch, and under it the chosen group's swatches (Match Windows shows a line saying what it picks instead). It opens on Match Windows when the look follows Windows, else on the active theme's group (tone is checked before mode, so Kraft and Sage Chalk are Midtone). Choosing a group whose theme isn't active switches to it at once (Marshmallow, the first midtone, Marshmallow Night), and a swatch then picks within the group. Matching Windows picks Marshmallow or Marshmallow Night and never a midtone. A pick applies instantly with no Save button and survives a reload. All theme fonts are bundled, so switching themes makes no network request.

**Verification (manual):**
1. In `ui/`, run `bun run dev --port 52xx --strictPort` against a mock or scratch-data server; open Settings and find Look.
2. Confirm the Match Windows / Light / Midtone / Dark switch opens on the current choice, and each group lists only its own swatches (Midtone has 22, Tater Dog is under Light). Press Midtone from a light theme: the UI turns midtone at once.
3. Click a swatch: the UI retints at once and no Save button exists. Reload and reopen Settings: the same theme is active and its group is open.
4. Pick Kraft: it is listed under Midtone. Press Match Windows: the active theme is Marshmallow or Marshmallow Night per the OS setting.
5. With the Network panel open (cache disabled), pick a theme with a non-default font (for example Space Grotesk): font files load only from the app's own origin, and none from another host.
6. On Rosewood, an accent-filled button or pill shows readable text.

### UI-DRIVE-SELECTOR: The header shows one drive, picked from a menu
**Applies to:** hoardini
**Verification:** manual

The header shall show only the selected drive, as "N free of M" over its free-space bar (INDEX-VOLUMES), so drives never stack. Its drive letter is a dropdown button (`C: ▾`) that opens a menu of every fixed drive (`GET /api/volumes`) with its letter, label (or "Local Disk"), a free-space bar, and "N free of M". A watched folder (a test setup) shows as `…\<last segment>`, with the full path as its tooltip.

- A watched drive has a check. Clicking it selects it (`PUT /api/volumes`) and closes the menu; the header bar, the mascot's girth (UI-MASCOT-FULLNESS), and Home (UI-HOME, which opens that drive's review) follow it.
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
4. Open it and click D:. The menu closes, the header shows `D:` and its bar, the mascot slims to D:'s fullness (tooltip "D: is 56% full"), and Home shows D:'s review.
5. Open it and press Add on E:. The header shows `E:`, and a fast scan starts ("Scanning fast").
6. Open it and press Stop watching on D:. The confirm step appears with focus on Cancel, and Escape cancels it. Press it again and confirm: D: loses its check and shows Add.
7. Stop watching every drive but one: the last one's Stop watching is disabled, with "At least one drive stays watched".
8. Start `hoardini serve --port 72xx --token dev --no-refresh` against a scratch `HOARDINI_DATA_DIR`, with Vite proxying to that port. Use `PUT /api/volumes` to watch three scratch folders, then rename one. The menu shows it greyed as "not connected", and Rescan now skips it (the log says `volume not connected; skipped`). Stop watching the selected folder: the log says `stopped watching; its index is deleted`, and the next folder becomes selected.

(2026-10-09: steps 1–7 pass against the mock in headless Chromium on ports 7291 and 5291. Step 8 passes against `hoardini serve --port 7291 --no-refresh` with three scratch folders: the renamed one showed "not connected" and was skipped by a refresh, and stopping the selected one deleted its 3 index rows and selected the next. The machine's `subst` drives (X:, Y:, Z:) were not listed.)
(2026-10-09: step 4 rechecked against the mock with the review: picking D: showed "Review of D:".)

## Retired

These IDs are not reused. Retired 2026-10-16 with the move to Home (simpler-model plan): UI-LAYOUT-WIDTH, UI-REVIEW, UI-REVIEW-ORDER, UI-CHAT-FLOW, UI-CONTEXT-BOUNDARY, UI-WELCOME-BACK, UI-CLEAR-HISTORY, UI-PROPOSAL-COLLAPSE, UI-SYSTEM-CLEANUP-CARD, UI-ONLINE-ONLY-CARD, UI-CHANGES-CHART, UI-TOOL-CARDS, UI-ITEM-TREE, UI-FOLLOW-UPS, UI-LEARNED-CHIP. Retired earlier: UI-EXPLORE, UI-TREEMAP-LAYOUT.
