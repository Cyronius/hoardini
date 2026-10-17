# Review specification (prefix REVIEW)

The review sweeps a drive (or a folder) with no model call and lists what could go and what is big but stays.
Plan (done): `specs/review/archive/review-first.md`. Code: `crates/core/src/review`, routes in `crates/server/src/review.rs`. Tests: `specs/review/tests/review.rs`
(`cargo test -p hoardini-specs --test specs review::`).

### REVIEW-UNITS: Items resolve to deletable units
**Applies to:** hoardini
**Verification:** test

`review::units::resolve(conn, env, lister, root, now)` shall walk down from `root` (a folder or a drive root in the
index) and return the units it stops at, each with its `path`, `kind` (`self`, `children` or `contents`), `why`
(`catalog`, `model-store`, `leaf`, `kept` or `disposable`), `catalog_id` when an entry decided it, `container` (the
folder the walk came from, or the store for `children` units; none for `root` itself), `size`, and for `contents` units
the `parts` past the entry's minimum age. At each folder, in order:

1. A protect note covering it makes it one `kept` unit. A location row of its own from the user with verdict `safe`
   (the Disposable answer, MEM-LOCATION, while the folder has not grown by half since) makes it one `disposable` unit. A `keep` location (MEM-LOCATION) covering it
   that came from the user, a job or the content mix makes it one `kept` unit. The walk does not go inside any of them.
2. A catalog entry naming the folder itself (`Catalog::lookup_exact`; an entry matched only through an ancestor does
   not count) whose verdict is not `keep` and whose `unit` is not `container` decides by its `unit`
   (EVID-CATALOG): `self` makes the folder one unit; `children` makes each child one unit (each grandchild when
   `unit_depth = 2`); `contents` makes one unit of the folder whose `parts` are its children, and with `min_age_hours`
   only the children not modified for that long (a child counts as modified when it or any indexed folder or file inside
   it was written or created since: installers unpack files that keep an old last write, and the index's creation time
   shows when they arrived; the index holds no change time, so the elevated helper adds that at execution,
   ACT-WINDOWS-CLEANUP-RUN, and a part is walked again before it is deleted, POLICY-REVALIDATE); a modified child folder of 1 GiB or more is split the same way, down to three levels, so the old scratch
   folders beside a live one still count. A file a `contents` entry names is a `self` unit. Rule 1 applies to each
   `children` unit and each `contents` part too: a child or part a protect note or a `keep` row covers is a `kept`
   unit of its own (and no part), and a child folder answered Disposable is a `disposable` unit.
3. A Windows system area of the deny list (POLICY-DENY: the Windows folder, Program Files, `ProgramData\Microsoft`,
   `System Volume Information`, `$Recycle.Bin`, paging files, Hoardini's data) or a catalog entry with verdict `keep`
   makes it one `kept` unit. The walk then visits the catalog locations named literally inside it (no wildcards) whose
   verdict is not `keep` and whose unit is not `container`, so `Windows\Temp` and `SoftwareDistribution\Download` are
   still found.
4. A file is a `self` `leaf` unit.
5. With no catalog entry naming it, a model store makes each child one `children` unit (`why = model-store`). A folder
   is a model store when at least 80% of its bytes are model weights and at least 80% of those weight bytes lie in
   files directly inside its child folders. Weights are files of 16 MiB or more ending in `.gguf`, `.safetensors`,
   `.onnx`, `.pt` or `.ckpt`, `.bin` next to a `config.json` (checked by listing the folder), and every type a file-type
   row files under `model-weights` (MEM-FILETYPE), such as `.q4nx`.
6. Otherwise the walk goes into every child of at least 1 GiB or at least 10% of the folder and repeats; a folder with no
   such child is one `self` `leaf` unit. A catalog entry with unit `container` (AppData, `~\.lmstudio`, Program Data)
   only lets the walk go on this way.

**Acceptance criteria:**
- `~\.flm` holding `models\Qwen3.6-35B-A3B-NPU2` (60 GiB of `.q4nx`), `models\Qwen3.8-27B-NPU2` (40 GiB) and a small `logs` folder resolves to the two model folders: `children`, catalog id `flm-models`, container `~\.flm\models`; `~\.flm` itself is no unit.
- The same tree at `C:\npu`, which no catalog entry names, ends at the two `.q4nx` files (`leaf`); once `.q4nx` is filed under `model-weights` it resolves to the two model folders: `children`, `model-store`, no catalog id, container `C:\npu\models`.
- `C:\hf\bert` and `C:\hf\gpt`, each holding a `pytorch_model.bin` and a `config.json`, make `C:\hf` a model store with those two units; the same `.bin` files without a `config.json` (`C:\blobs`) end at the files.
- In `C:\code\proj`, `target` (3 GiB of build output) is one `self` unit with catalog id `rust-target` and container `C:\code\proj`; the 4 KiB `src` is not visited.
- `C:\work` goes into `big` (2,000 MiB of 50 MiB files) and `medium` (300 MiB, 12.8% of the folder) and not into `tiny` (50 MiB, 2.1%); both are `leaf` units, since no child is 1 GiB or 10% of them.
- `%TEMP%` is one `contents` unit with catalog id `temp-user`: a child folder whose file is 3 days old and a 3-day-old file are its `parts`, a child folder holding a file written an hour ago is not, and `size` is 7 GiB, the sum of the parts. A walk of the whole drive reaches the same unit.
- In `%TEMP%\claude\proj`, where `live` holds a file written an hour ago, the 40 GiB `old` folder beside it is a part and nothing in `live` is.
- In `%TEMP%`, an `unpacked` folder whose `setup.cab` was created an hour ago with a last write 30 days old is no part; a 3-day-old `old.iso` beside it is.
- `C:\SteamLibrary\steamapps\common` gives one `children` unit per game (`steam-common`).
- `%SystemRoot%` is one `kept` unit, and `%SystemRoot%\Temp` inside it is still a `contents` unit (`temp-system`).
- After the user answers Keep for `C:\code\proj`, a walk of `C:\code` gives that folder as one `kept` unit and not its `target`.
- After the user answers Disposable for `old\junk`, a drive walk gives that folder as one `disposable` unit.
- With a protect note on `%TEMP%\old-a`, `old-a` is a `kept` unit of its own, the `%TEMP%` unit's parts hold only `old-b`, and the review's `temp` card holds only `old-b`.

Tests: `specs/review/tests/review.rs`; the split, the Disposable unit and the protected part in
`specs/review/tests/build.rs`.

### REVIEW-BUILD: The review proposes cards from its units
**Applies to:** hoardini
**Verification:** test

`review::build(db, env, lister, volume_id, now)` shall run with no model call: refresh the volume's location rows
(MEM-LOCATION), resolve the volume's units (REVIEW-UNITS), and hand each unit to the first card of the registry
(`review::CARDS`, in the order below) that takes it. Each card becomes one pending proposal made through the normal
proposal path (`proposals::create_review_card`: path resolution, the deny list, blocks, caps, memory notes,
POLICY-DEFAULT-TICK; the `windows` card through `proposals::create_system_cleanup_card`), with `conversation_id`
`review`, the review's id as `review_id`, and the card's id as `card`.

| Card | Title | Takes | Kind | Ticks |
|---|---|---|---|---|
| `temp` | Temp and Windows leftovers | catalog units of category `temp`, `crash-dumps`, `logs` or `windows-leftover` | delete | POLICY-DEFAULT-TICK |
| `caches` | Caches | catalog units of category `cache`, `browser-cache`, `package-cache` or `installer-cache` | delete | POLICY-DEFAULT-TICK |
| `build` | Build output | catalog units of category `build-output`, plus `project-venv` and `pycache`; a `rust-target` only with `Cargo.toml` or `pom.xml` beside it | delete | POLICY-DEFAULT-TICK |
| `disposable` | Folders you said can go | `disposable` units (REVIEW-UNITS): the folder whole, verdict `safe` | delete | POLICY-DEFAULT-TICK (the answer lifts the unknown-location cap, POLICY-VERDICT-CAP) |
| `redownload` | Re-downloadable, verified | units whose own copy check (EVID-COPY-CHECK) is a settled `identical` answer from `huggingface`, `winget` or `archive` (in that order of preference); a duplicate, git or sync answer does not count | delete | POLICY-DEFAULT-TICK |
| `online_only` | Online-only | see below | online-only | POLICY-DEFAULT-TICK (`synced`) |
| `downloads` | Installers and archives in Downloads | children of `~\Downloads` that `downloads-installers` or `downloads-archives` names | delete | all unticked |
| `compress` | Compress in place | see below | compress, `lzx` | all unticked |
| `archive` | Send elsewhere | see below | archive, to the first destination off this volume | all unticked |
| `windows` | Cleanups only Windows can do | see below | system cleanup | POLICY-DEFAULT-TICK per operation |

No card but `windows` takes a unit that needs admin rights to delete: one inside a Windows system area of the deny list
(POLICY-DENY), one under `%ProgramData%`, or a `windows-leftover` outside the profile (`Windows.old`, `$WINDOWS.~BT`).
REVIEW-AREAS lists the ones `windows` does not take. A unit or item under a registered cloud sync root (EVID-COPY-SYNC), or
holding one, goes to no card but `online_only`, since deleting, archiving or compressing it there would change the
cloud copy too.

**Online-only** takes a unit under a sync root (any `why` but admin): the unit when nothing in it was written or read
for 90 days and the sync check run now (`copycheck::sync_result`) answers `identical`, not pinned ("Always keep on this
device") and with local data; otherwise it walks on, down to three levels, through children of 100 MiB (files) or
1 GiB (folders) and more, taking each that meets the same rule (at most 50 sync checks per unit). The reason reads
`<Note>; nothing in it opened or changed for N days.` (`Synced to OneDrive; ...`).

**Compress in place** takes, for a unit that is not admin or `disposable`, each `compress::candidates` folder of the
scope (ACT-COMPRESS-CANDIDATES) that is, holds, or lies inside the unit, is proposable from its cached estimate (1 GiB
and 20% saved), and holds no `catalog` or `disposable` unit (those go to delete cards); once every unit has its card, a
compress folder that holds or lies inside another card's item (a `redownload` file, say) is dropped, since it would be
compressed and then deleted or moved. The reason reads `Windows
compression would save about <saved> (<pct>% of it); the files stay usable.`

**Send elsewhere** takes nothing while no destination lies off this volume. Otherwise it takes a `kept` folder unit
whose location row (MEM-LOCATION) has verdict `keep` and allows `archive`: for the user's Move elsewhere answer (label
`move elsewhere`), the whole folder, whatever its size or age, verdict `review`, the answer's text as its reason; for any
other such row only a `photos` or `documents` one counts, and then the folder when it holds 5 GiB or more and nothing in
it was written or read for 180 days (`cards::last_change`, the newest mtime or atime inside), else each of its child
folders and files that meets the same rule. A user's Keep answer allows `archive` too, but is neither, so it sends
nothing.

**Cleanups only Windows can do** holds operations, not paths: first `leftovers` (SystemOp `Leftovers`,
ACT-WINDOWS-CLEANUP-RUN), made of the admin units that are exactly one of `syscleanup::leftover_targets` (`Windows\Temp`,
`SoftwareDistribution\Download`, the Delivery Optimization caches, `ProgramData\Microsoft\Windows\WER`, `Minidump`,
`MEMORY.DMP`, `LiveKernelReports`, the CBS, DISM and Panther logs), its `parts` their parts past the entry's age (the op
view lists them with sizes; the estimate is their sum); then, for a volume review of the system drive, the operations
`syscleanup::candidates` detects from `PolicyEnv::facts()` (ACT-WINDOWS-CLEANUP-DETECT). An operation of a pending or
recently declined system cleanup proposal (one made on Home, or a card of a review that stays) is left out by name.

**Items.** A `contents` unit gives one item per part, any other unit one item. A compress item carries `saves`, what
its cached estimate says compressing saves (`compress::saved_bytes`; `proposals::get` fills it for every compress
proposal), since compressing frees that and not the folder's size. An item's verdict is its catalog entry's
(the gate raises it to the cap) and its reason cites what Hoardini knows: `<entry name>: <first sentence of its notes>`;
for a part, `<entry name>: untouched for N days.`; for Send elsewhere, `<Label>: nothing in it opened or changed for N
days.` (the user's answer quotes the answer's text); for Re-downloadable, the copy note (`In winget as Contoso.Tool 2.1.`,
or `<entry name>: <note>.` under a catalog entry). When `location::cleaned_for` finds a `cleaned` entry for the item
(MEM-LEARN-JOB), ` You cleaned this on MM-DD (<size>).` follows (`Cleaned::reason`). An item a `kept` entry names is left
out of every card while it is under 1.5 times the kept size (`location::was_kept`), so what the user left unticked comes
back only once it grows by half. Items are sorted biggest first and a card lists the first 50; its summary then ends `N
smaller items (<size>) are not listed.`

**Exclusions.** A unit or item that is, contains, or lies inside an item of another pending, approved or running proposal
(one made on Home, or a card of a review that stays) or of a proposal declined in the last 30 days is left out, of cards and
of Large areas; a `contents` unit is left out only when such an item is the unit or holds it, and otherwise each part is
left out on its own, so one temp part in another card leaves the others in the `temp` card.
System cleanup proposals count by operation (above), not by the paths their items name.

**Thresholds.** A review makes a card only when its items hold 500 MiB or more by the index (for `windows`, the
leftovers).

`review::get(conn, id)` and `review::latest(conn, volume_id)` read a review back as `{id, volume_id, volume,
scan_id, created_ms, cards: [{card, title, proposal_id, carried}], areas}`; `cards` are the review's proposals
that have no `parent_id`, plus the cards earlier reviews carry into it (REVIEW-SUPERSEDE, `carried` true), biggest
`total_alloc` first.

**Acceptance criteria:**
- On a drive with 600 GiB used and a NAS destination: `~\AppData\Local\Temp\old-session` (5 GiB, 400 days old) and `~\AppData\Local\CrashDumps\app` are the `temp` card, both ticked, the first `safe` with reason `User temp folder: untouched for 400 days.` and `alloc` 5 GiB; a `live` temp folder written an hour ago is in no card; Chrome's `Cache` is the `caches` card, ticked, its reason starting `Google Chrome cache: `; `code\proj\target` (with `Cargo.toml`) and `code\web\node_modules` are the `build` card, ticked, and `code\data\target` (no `Cargo.toml`) is not; `~\Downloads\setup.exe` (`redownload`) and `photos.zip` are the `downloads` card, unticked, and `notes.pdf` is in no card; `~\Pictures\2019` (30 GiB, 400 days untouched) is the `archive` card, unticked, to `NAS`, reason `Your photos and videos: nothing in it opened or changed for 400 days.`, while `~\Pictures\2026` (written yesterday) is not. The archive card comes first. Every card has `conversation_id` `review`, the review's id and its card id.
- A `node_modules` in a pending card outside the review and one declined today are in no card and no area; one declined 31 days ago is back in the `build` card.
- A 300 MiB pip cache makes no `caches` card.
- With 56 old temp parts, the `temp` card lists 50, the 40 GiB one first, and its summary ends ` 6 smaller items (375 MB) are not listed.`
- `work\setup\tool-2.1.exe` with a stored `identical` winget answer is the `redownload` card, ticked, `redownload`, reason `In winget as Contoso.Tool 2.1.`; `work\copy\image.iso` with an `identical` duplicate answer is in no card. `old\junk`, answered Disposable, is the `disposable` card (title `Folders you said can go`), ticked, reason `You said this folder can go.`. Of two `node_modules` a job row keeps, the one still at its kept size is in no card, and the one grown from 1,000 to 1,600 MiB is in the `build` card with reason `node_modules: Installed JavaScript dependencies. You cleaned this on 01-15 (28.0 GB).` and no memory note.
- Under a fake OneDrive root `~\OneDrive`, an in-sync `Videos\2019.mp4` untouched for 400 days is the `online_only` card, ticked, `synced`, reason `Synced to OneDrive; nothing in it opened or changed for 400 days.`, although `Videos` is a kept photos-and-video location and `2026.mp4` beside it was written yesterday; `2026.mp4` and the `raw.mov` that is not in sync are in no card, and the `node_modules` under the root is in no card at all.
- `data\corpus` (3 GiB of text, untouched) is in no card until its estimate is stored; with 1.2 GiB of 3 GiB saved it is the `compress` card, `lzx`, unticked, reason `Windows compression would save about 1.2 GB (40% of it); the files stay usable.` and `saves` 1.2 GiB; ticking it makes `ready_bytes` 1.2 GiB, not 3 GiB. With an `identical` archive answer for `part0.txt` inside it, `part0.txt` is the `redownload` card and there is no compress card.
- With `%TEMP%\old-a` in a pending card outside the review, the `temp` card holds `old-b`.
- `work\renders` (2 GiB, written yesterday) answered Move elsewhere and `work\kept` (6 GiB, 400 days untouched) answered Keep give no `archive` card while there is no destination; once `NAS` is added the `archive` card goes to `NAS` and holds only `work\renders`, unticked, `review`, reason `You want this folder moved to another drive, not deleted.`
- On a fake system drive `C:\` with fixed facts (64 GiB RAM, hibernation on), a 6 GiB WinSxS, 2 GiB of old `Windows\Temp` and a 25 GiB `hiberfil.sys`, the `windows` card's operations are `leftovers`, `component_store`, `hibernation`, in that order. With a pending card outside the review for hibernation the next review's card holds `leftovers` and `component_store`, and the earlier untouched card is `superseded`; once that card is declined, the next review has no `windows` card and no `Temp` area. On the fake `C:\` of REVIEW-AREAS the card holds one ticked `leftovers` item of 6.5 GiB whose parts are `SoftwareDistribution\Download\abc` (3 GiB), `Temp\setup` (2 GiB) and `ProgramData\Microsoft\Windows\WER\ReportArchive` (1.5 GiB).

Test: `specs/review/tests/build.rs`; the leftovers card on the fake `C:\` in `specs/review/tests/areas.rs`.

### REVIEW-AREAS: Large areas say what is big and why it stays
**Applies to:** hoardini
**Verification:** test

Each review shall store, in `review.areas_json`, the big things it does not propose: rows of at least 1 GiB, biggest
first, at most 30, each `{path, size, label, category, verdict, why_not, hint, unknown, actions, only_copy, user}`. `user` is true when the verdict is the user's own answer (a `user` location row, or a What is this? answer written into the stored review, SRV-LOCATION-ANSWER), so Home counts it under Things you kept instead of Your call (UI-HOME). They are
built from the units that no card took in full; what a card took, and units the exclusions of REVIEW-BUILD leave out, do
not count.

- **Rows.** A unit that needs admin rights (REVIEW-BUILD) and that the `windows` card does not take, a `target` with no
  `Cargo.toml` or `pom.xml` beside it, a unit of 1 GiB or more (left after what cards took) that is not admin, `kept`, a
  virtual disk or a game (`vm-disk`, `game-library`: always their own only copy) and that `copycheck::only_copy` calls
  the only copy we know of (EVID-COPY-CHECK: every check that applies answered
  `none`, nothing pending or waiting on consent), and a `kept` unit that has a catalog entry or lies in a Windows system
  area (the Windows folder, Program Files, paging files) are rows of their own. A kept unit's size leaves out the units
  found inside it and the `keep` catalog locations named literally inside it (`WinSxS`, `Installer`, `DriverStore`,
  `WindowsApps`), which become rows of their own. Any other unit joins the row of the deepest location row (MEM-LOCATION)
  at or above it that lies within the scope, is not the volume root, and is not a `container`. When that row is
  `unknown`, or has the category of a project row above it, the unit joins the outermost such project row instead; a
  project is a folder holding `.git`, `Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`, `pom.xml`,
  `build.gradle`, `CMakeLists.txt` or a `.sln`, so a project's unknown subfolders are one row and the "What is this?"
  answer belongs to the project. A file or folder that no location row covers joins the outermost project folder between
  the scope and it, or is its own row. A row's size is the sum of its units.
- **What it is.** `label`, `category`, `verdict` and `actions` (the proposal kinds still allowed) come from the first of:
  a location row from the user, a job or the web; the unit's catalog entry, unless it is a `container` entry (actions
  none for `keep` or admin rows, else all four); any other location row; the location classifier. A `target` with no
  marker is `unknown`.
- **Why not.** `why_not` is that knowledge's sentence, except for model stores and model weights (`Model files: Hoardini
  proposes them once it can show where else a copy exists.`), `vm-disk` (`A virtual machine's disk: deleting it deletes
  the machine inside.`) and `game-library` (`Installed games: remove them through their launcher.`). `unknown` is true
  when the category is `unknown`; its `why_not` then ends ` Something in it changed in the last 30 days.` when the
  newest write or read inside is that recent. An only-copy row (`only_copy` true) reads `Only copy we know of: <the
  notes of its copy checks, joined by "; ">.`
- **Hint.** Admin rows: `Needs admin.` (`areas::ADMIN_HINT`; the leftovers Windows' own folders hold are the `windows`
  card's, REVIEW-BUILD); the page file: `Windows manages its size.`; `WinSxS`: `Windows can trim this itself.`; the
  hibernation file: `Windows recreates it at boot; turning hibernation off removes it.`; photos and documents while no
  destination is set: `Add a destination to move them off <drive>.`; otherwise none.

**Acceptance criteria:**
- On a fake `C:\` the areas are, in order: `C:\code\covenant-agent` 161 GiB, one row although `models\tiny` inside it is a location row too, since it holds `.git` (unknown, no verdict, no actions, why_not starting `Not identified` and ending with the 30-day sentence, since a file in it was written 2 days ago); `~\Pictures` 40 GiB (`photos`, `keep`, `[archive]`, hint `Add a destination to move them off C:.`); `C:\vm\disk.vhdx` 30 GiB (`vm-disk`); `~\.lmstudio\models` 20 GiB (why_not starting `Model files: `); `C:\pagefile.sys` 16 GiB (`system`, why_not starting `Page file: `, hint `Windows manages its size.`); `C:\hiberfil.sys` 12 GiB with its hint; the Windows folder 10 GiB (23 GiB less `Temp`, `SoftwareDistribution\Download` and `WinSxS`; no hint); `WinSxS` 8 GiB (why_not starting `WinSxS component store: `, hint `Windows can trim this itself.`); `C:\Program Files` 5 GiB; `C:\code\small-proj` 2.5 GiB (unknown), shown as the project that holds a 2.5 GiB `.git\objects\pack\p.pack`; and `ProgramData\Microsoft\VisualStudio\Packages` 2 GiB with the hint `Needs admin.`. The Windows `Temp`, `SoftwareDistribution\Download` and the ProgramData `WER` reports are no rows: the one card is `windows` (REVIEW-BUILD). A 512 MiB folder is not listed, and no row is `only_copy`.
- The review read back with `review::get` has the same areas.
- In a project `proj` (with `.git`), a 3 GiB `weights\mine.bin` whose only copy check, `dup`, answered `none` is a row of its own, `only_copy`, why_not `Only copy we know of: no other file of this size on the indexed drives.`, and the project row keeps the other 2 GiB; a 4 GiB `vm\disk.vhdx` with the same answer is no only-copy row and keeps the virtual-disk why_not.
- A folder kept by the user is `user` on the stored review at once (after the answer) and after a rebuild; an unknown folder beside it is not.

Tests: `specs/review/tests/areas.rs`; the only-copy row and `user` in `specs/review/tests/build.rs`.

### REVIEW-TRIGGER: When the review is built, and its routes
**Applies to:** hoardini, hoardini-server
**Verification:** test

- `POST /api/review/open`, which the UI calls when the app opens, answers at once: the selected drive's latest volume
  review when it is fresh; otherwise, when there is none, it is more than 6 hours old, or the drive's last completed
  refresh is not the one it was built from (`review.scan_id`), it starts a build in the background and answers the
  latest review (or, before the first one, `review: null` with the drive's header, no cards, no areas) with `building:
  true`. It answers 404 while the drive has no completed index.
- `POST /api/review/run` builds one of the selected drive now and answers it once built.
- `GET /api/review` returns the selected drive's latest volume review, and `GET /api/review?id=` any review; 404 when
  there is none.

Each answers `{review: {id, created_ms, volume}, header: {root, used_bytes, free_bytes, ready_bytes}, cards:
[{card, title, proposal_id, carried}], areas: [...], proposals: {<id>: <proposal>}, building, apps: [...]}`. `ready_bytes` sums the ticked, pending items of the pending cards it lists, a compress item by its
`saves` (its `alloc` when it has no estimate); `proposals` holds every proposal of
the review, its cards and their follow-ups, and every carried card with its follow-ups, in the JSON of `GET
/api/proposals/{id}` (which carries `review_id`, `card` and `touched`); `building` is true while a review build runs; `apps` are the rarely used apps of ACT-APPS-RARELY-USED (the
SRV-APPS JSON) whose folder lies in the review's scope, so Home lists them without a second call; paths show the
profile as `~`. One build runs at a time (`review::Builds`): `open` starts the drive's build in the background, or, while another
drive's build runs, queues it to start when that one ends (a later request for a third drive takes the queued place);
`run`, the launch rebuild and the rebuild after estimates wait for a running build and then build. The build `open`
starts (`review::build_if_stale`) first checks that the drive's latest review still needs a rerun, so one queued behind
an exclusive build that left a fresh review builds nothing. A review becomes visible (`latest`, `get`, the status
stream's `review.id`) only once all its cards exist: `build` makes the cards first, then, in one immediate transaction,
supersedes what it replaces (REVIEW-SUPERSEDE) and inserts the review row; cards of a build that fails, or that a
crash left with no review row, are dropped.
`GET /api/status` and its stream carry `review: {id, created_ms}` for the selected drive's latest volume review (null
when there is none) and `review_building`; the end of every build bumps the stream, so the UI sees the new review id.
After a build, the server estimates the scope's top compress candidates on a background-priority thread
(`review::compress_estimates`, ACT-COMPRESS-CANDIDATES); when that makes a folder proposable that was not proposable
before the estimates, that the review's compress card does not hold, and that the review would not leave out (an item
of another live or recently declined proposal holds or lies inside it, or it touches a sync root), and the review is
still the latest of its scope and untouched, it is built again (`review::rebuild_if_current`).

**Acceptance criteria:**
- Before any index, `POST /api/review/open` is 404. On a fake `C:\` (1,200 GiB used, 800 GiB free) the first open answers `review: null`, `building: true`, the header root `C:\` and no cards; once the status says `review_building: false`, `GET /api/review` gives a review of `C:\`, header `{root: "C:\", used_bytes: 1200 GiB, free_bytes: 800 GiB, ready_bytes: 0}`, no cards, no proposals, `building: false`, `apps: []`, and the 161 GiB `C:\code\covenant-agent` area (`unknown`, label `unknown`, no verdict, no hint, no actions); a second open (`building: false`), `GET /api/review` and the status's `review.id` give the same id.
- After one more completed refresh, open answers the last review with `building: true` and a new review follows; with that review's `created_ms` 7 hours back, open leads to another; `POST /api/review/run` builds a review every time.
- With a fake app source, the review of `D:\` carries its rarely used app (SRV-APPS), and the review of `C:\` carries none.
- With a fake clock and a builder held at a gate: the first `Builds::open` answers building with no review and starts one build; a second open and a `start` for the same drive meanwhile start nothing; once the gate opens and the build is announced, open answers that review; 5 hours later the same one; 6 hours and 1 ms later the old one with building, then a new review built at that time. A build that fails ends and is announced too, and `exclusive` runs at the clock's time. A build for drive `b` asked for while `a`'s is held at the gate is queued (asking again changes nothing) and runs right after `a`'s, with `building` true until both are announced.
- In core, `review::open` (the same rule, building on the caller's thread) returns the same review 5 hours later and a new one after 6 hours and 1 ms.
- A volume review of 3 GiB of zeros in `data\corpus` has no compress card; `compress_estimates` then reports a newly proposable folder, `rebuild_if_current` builds a review with the card, a second `compress_estimates` reports nothing, and `rebuild_if_current` builds nothing for a review that a newer one replaced or that the user touched. With `data\corpus` proposable before and held by a pending card outside the review, `compress_estimates` reports nothing.
- An `open` while an exclusive build is held at a gate queues `build_if_stale`; once the exclusive build has made a review, the queued one builds nothing (one review row) and the next open answers `Ready`.
- While a second volume review builds (40 temp parts and two build folders), a thread polling `latest` sees either the first review or the second with all its cards, never the second beside the first's cards still pending.

Tests: `specs/srv/tests/srv.rs` (routes), `specs/review/tests/build.rs` (`review::open`, `Builds`, estimates).

### REVIEW-SUPERSEDE: A new review replaces the cards the user did not touch
**Applies to:** hoardini, hoardini-server
**Verification:** test

A proposal is `touched` once the user changes a tick (SRV-PROPOSAL-SELECTION), approves it, or declines it. A new volume
review shall set every pending, untouched card of the drive's earlier reviews to status
`superseded` and may propose their items again; touched cards stay pending, and their items stay out of the new review. The supersede happens when the review is
published (REVIEW-TRIGGER) and only for cards still untouched then: a card ticked, approved or declined while the review
was built stays, and the new cards give up any item that is, holds or lies inside an item of a proposal live or
declined at that moment (a card left empty is dropped; a `windows` card sharing an operation is dropped). Those touched
cards, and the touched follow-up of a card that is no longer live, while pending, approved or running, are listed in
every later review of the drive with `carried: true`
(REVIEW-BUILD `review::get`; its `proposals` and `ready_bytes` count them), so the review always shows every live card;
they do not make the new review touched. When the refresh the server starts at launch finishes, and the
selected drive's latest volume review was built from an older refresh and none of its cards was touched or decided, the
review is rebuilt (`review::rebuild_if_untouched`). A follow-up of a review card (ACT-FOLLOW-UP) keeps the parent's
`review_id` and `card`; it is listed in the review's `proposals`, not as a card of its own.

**Acceptance criteria:**
- After ticking only `target` on the first review's `build` card (`touched` becomes true), a review of the `code` folder makes no card, since every item is in a pending card, and replaces nothing; the next volume review sets the untouched `temp` card to `superseded`, proposes its folder again in a new `temp` card, keeps the `build` card pending and makes no `build` card of its own: it lists that card as `build`, "Build output", `carried`, among its two cards and its `proposal_ids`, `ready_bytes` is 5 GiB (the new temp item and the ticked `target`), and the review is still untouched. A review later the card is still listed; the first review lists nothing carried. Declining the new card sets `touched`; once the build card is declined, the next review no longer lists it.
- `review::rebuild_if_untouched` returns nothing while no refresh finished since the review; after one it returns a new review and the old `temp` card is `superseded`; after ticking that new card's items off and one more refresh it returns nothing, and the card stays `pending`.
- Approving one of a `temp` card's two items sets `touched`; the follow-up holds the other item unticked, with the review's `review_id`, card `temp` and conversation `review`, and the review still lists one card.
- With a review built before one more completed refresh, starting the server with its launch refresh (a fake indexer) leads `GET /api/review` to a new review id.
- Declining the first review's `temp` card while the second is being built (a hook in the fake lister) leaves the card `declined` and the second review with no `temp` card; ticking it instead leaves one live card holding the temp folder (the carried one) and `ready_bytes` 5 GiB.

Tests: `specs/review/tests/build.rs`; the launch rebuild in `specs/srv/tests/srv.rs`.

## Defaults

| Setting | Default |
|---|---|
| Card needs, in a volume review | 500 MiB of items by the index |
| Items listed per card | 50, biggest first |
| Declined items stay out | 30 days |
| Review reruns at open | 6 h after the last one, or after a refresh |
| Send elsewhere | 5 GiB or more, nothing written or read for 180 days, a destination off the volume |
| Large area | 1 GiB or more; at most 30 |
| Recent change on an unknown area | 30 days |
| Split a modified `contents` child | 1 GiB or more, down to 3 levels |
| Online-only | under a sync root, in sync, not pinned, nothing written or read for 90 days; down to 3 levels, 50 sync checks per unit |
| Compress in place | estimate saves 1 GiB and 20% (ACT-COMPRESS-CANDIDATES); top 10 candidates estimated after each build |
| A left-unticked item comes back | once it is 1.5 times its kept size |
| Windows leftovers | parts untouched for 24 h (temp) as the catalog says; the helper clears files untouched for 24 h |

## Retired

These IDs are not reused. Retired 2026-10-16 with the chat (simpler-model plan): REVIEW-LOOK-CLOSER. The folder review (`Scope::Folder`, `POST /api/review/run {path}`, `scope_path`) went the same day; rows an older build stored with a `scope_path` are ignored by `latest` and replaced like any other old card.
