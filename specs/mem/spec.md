# Memory (prefix MEM)

Path-keyed notes the agent and the user leave about the disk. Stored in the `memory` table (SQLite, FTS5 for free text).
Canonical design: `.claude/plans/hoardini-design.md` §5.7. Code: `crates/core/src/memory`.

## Model

A row has `scope` (`path` | `glob` | `global`), `key`, `kind` (`fact` | `preference` | `protect` | `verdict`), `text`,
`source`, optional `confidence`, `created_ms`, `updated_ms`, and an optional `review_after_ms`, plus `pinned`,
`last_used_ms`, and `forgotten_ms` (MEM-LIMIT), and an optional `detail` (the `memory.detail_json` column, v5). Keys are
stored profile-relative (`~\...`), compare case-insensitively, and use `\`. Facts get `review_after_ms = now + 90 days`
unless the caller supplies one; `Memory::is_stale(now)` reports when it passed.

Sources: `user` (the drawer, a "What is this?" answer), `agent` (the `remember` tool), `web:<url>`, `catalog`, `deny`,
`mix` and `index` (the location classifier, MEM-LOCATION), `job:<id>` (a finished job) and `copy:<check>` (a copy check).

Three kinds of row:

- **Notes** have no `detail`: what the `remember` tool and the drawer write.
- **Location rows** (MEM-LOCATION): scope `path`, kind `verdict`, detail `{category, label, verdict, actions, size_at,
  assessed_ms, by, cleaned, kept}`.
- **File-type rows** (MEM-FILETYPE): scope `glob`, key `**\*.<ext>`, kind `fact`, detail `{category, app, use, copy_hint}`.

Detail fields: `category` (a catalog category such as `model-store`, or `system`, `protected`, `container`, `photos`,
`documents`, `model-weights`, `unknown`, `user`); `label`, the short name the prompt tag shows; `verdict` on the existing
scale; `actions`, the proposal kinds still allowed there (`delete`, `dedupe`, `archive`, `compress`); `app` and `use`,
what the folder or type belongs to; `copy_hint`, a copy check that knows where to look (a place to look, never a
verdict); `size_at` and `assessed_ms`, the folder size and time of the last assessment; `by`, the step that decided
(`deny`, `protect:<id>`, `catalog:<entry>`, `mix`, `index`, `user`, `job`); `cleaned` and `kept`, entries owned by job
learning (MEM-LEARN-JOB). An absent field means null or empty, and fields this list does not name are kept as written. Location
and file-type rows are kept one per scope and key (`memory::knowledge::upsert`), apart from notes on the same key, get no
`review_after_ms` (locations are reassessed instead), and reach evidence, the `recall` tool and item notes through
`recall_path` like notes do.

### MEM-RECALL-ANCESTRY: Recall by path ancestry and glob
**Applies to:** hoardini
**Verification:** test

`recall_path(path)` shall return every `path`-scope row whose key equals the path or any of its ancestors, and every
`glob`-scope row whose pattern matches the path or any ancestor, ordered most specific first (exact path, then ancestors
from nearest to farthest, then globs). Matching shall ignore case and treat `/` and `\` alike, and a path inside the user
profile shall match keys written as `~\...`. `protect_covering(path)` shall return the most specific `protect` row from
that same set, and none when only non-protect rows match.

**Acceptance criteria:**
- Rows keyed `~\.lmstudio`, `~\.lmstudio\models`, and the exact file are all returned for that file, in the order exact, parent, grandparent; a glob `~\.lmstudio\models\**` follows them.
- A sibling file and a name that merely starts with the same characters (`big.gguf.bak` for `big.gguf`, `D:\keeper` for `D:\keep`) are not returned.
- Key `~/Projects/Keep` matches `c:\USERS\Tester\projects\keep\src\main.rs` and is stored as `~\Projects\Keep`.
- Glob `**\node_modules` matches `E:\code\web\node_modules\left-pad\index.js`; glob `D:\data\*.parquet` matches `D:\data\a.parquet` but not `D:\data\sub\a.parquet`.
- `protect_covering` returns the row for `D:\keep` when asked about `D:\keep\a\b.bin`, returns a `**\*.kdbx` protect for any `.kdbx`, and returns none under a path that has only a `fact` row.

### MEM-EDIT: View, edit, pin, and delete memories
**Applies to:** hoardini
**Verification:** manual

The Memory drawer shall list every memory row sorted by last used, each with a "last used ..." label and a pin toggle, let the user change its text, kind, and key, and delete it. Forgotten rows (MEM-LIMIT) shall sit greyed at the bottom with a Restore button. A "Remember up to N notes" control shall set the limit. Rows whose source is `web:<url>` shall show that URL. Location and file-type rows shall also show their label (or app, or category) and their verdict as chips; `GET /api/memory` carries each row's `detail` (null for notes).

**Verification (manual):**
1. Ask the agent to remember a fact about a folder ("remember that D:\scratch is disposable"). A chip appears in the chat with an undo and a pin toggle, and reads "noted - pinned".
2. Open the Memory drawer. The row appears with scope `path`, key `D:\scratch`, source `agent`, pinned, and "last used just now".
3. Edit its text and kind to `protect`, save, reload the app. The edited row persists.
4. Ask the agent to propose deleting a file under `D:\scratch`. No card appears: policy refuses the path (POLICY-PROTECT), and the model gets an `Error: ... refused by policy` result that it explains.
5. Delete the row in the drawer. It disappears, and the same request now produces a card.
6. For a row created from a web search, confirm the drawer shows its source URL.
7. Ask the agent to remember a preference: the chat shows a "noted" chip; Undo turns it into "forgotten" and the row is gone from the drawer.
8. In the drawer's search box, a word from the note finds it; an unrelated word shows "No notes match."
9. Add a few unpinned notes, then lower the limit to its minimum below the note count. The oldest-used unpinned notes move, greyed, to the bottom with a Restore button; pinned and protect notes stay in place. Restore one: it returns to the top as "last used just now".
10. Toggle a row's pin off and on; the state persists across a reload.
11. With a location row (`~\Pictures`, "your photos and videos", `keep`) and a file-type row (`**\*.q4nx`, app FastFlowLM) present, open the drawer: the location row shows the chips "your photos and videos" and "keep", the type row shows "FastFlowLM" and no verdict chip, and a plain note shows neither.
(2026-10-08, scripted mock model, sandbox folder: 1-5, 7, 8 as described; 6 not run, no web-sourced row. Steps 9-10 and the pin text in 1-2 are new and unrun.)
(2026-10-09: step 11 passes against the UI mock (`MOCK_IDLE=1 bun run mock` + `vite`, Playwright headless): `~\Pictures` shows "your photos and videos" and "keep", `**\*.q4nx` shows "FastFlowLM" with no verdict chip, the plain notes show neither; step 6's web URL also shows on the `.q4nx` row.)

### MEM-USAGE: Reads mark a note as used
**Applies to:** hoardini
**Verification:** test

Each place a note reaches the model shall set its `last_used_ms` to now: `protect_covering` (the one protect note returned), `recall_path` (every note returned), the `recall` tool and `recall_text` (every note returned), and `globals_for` (every global note the user's message matches in the full-text index). Creating or editing a note also sets it. The drawer's search shall not count as a use.

**Acceptance criteria:**
- A note created at time 1,000 and then returned by `recall_path` has `last_used_ms` greater than 1,000; a sibling note not returned keeps 1,000.
- `protect_covering` raises only the protect note's `last_used_ms`, not a non-protect note covering the same path.
- `recall_text` raises the `last_used_ms` of the notes it returns; `search` does not.
- `globals_for(cap, "keep the steam library")` raises a global note mentioning "steam" and leaves one that matches nothing.
- Editing a note sets `last_used_ms` to the edit time.

### MEM-LIMIT: Least-recently-used notes are forgotten
**Applies to:** hoardini
**Verification:** test

The note limit N (default 200, minimum 10, stored in `meta.memory_limit`) shall count only live notes that are neither pinned nor `protect`; location and file-type rows (rows with a `detail`) are never counted and never forgotten. When adding a note or lowering N leaves more than N, the notes with the oldest `last_used_ms` shall get `forgotten_ms = now` until N remain. Forgotten notes shall never be deleted by this, and shall be excluded from `recall_path`, `protect_covering`, `recall_text`, `globals`, and so from evidence and the prompt. `restore` shall clear `forgotten_ms` and count as a use. Forgotten notes shall be deleted 90 days after `forgotten_ms`, when the list is read or the limit is enforced.

**Acceptance criteria:**
- With N = 10 and 12 unpinned notes used at times 1..12, adding the 13th forgets the three used at 1, 2, 3 (live count 10) and leaves the rest.
- Pinned and `protect` notes older than all others are never forgotten, however many notes there are.
- A forgotten path note is not returned by `recall_path`, a forgotten protect-less global is not in `globals`, and a forgotten note is not returned by `recall_text` but is by `search`.
- `restore` returns the note to `recall_path` with `forgotten_ms` cleared and `last_used_ms` updated.
- `set_limit(1)` stores 10; lowering the limit forgets down to the new N immediately.
- A note forgotten more than 90 days ago is gone after `list`; one forgotten 89 days ago is still listed.
- `globals` orders protect, then pinned, then most recently used.
- With N = 10, three location rows and one file-type row written first and twelve unpinned notes after them, ten notes stay live and all four detail rows stay live.

### MEM-PIN-USER: Notes the user asked for start pinned
**Applies to:** hoardini
**Verification:** test

A note whose source is `user` (typed in the drawer) shall start pinned, and so shall a `remember` call with `user_asked: true`. A `remember` call with `user_asked: false` or without it shall not pin. Pinning only exempts a note from the limit; `PUT /api/memory/{id}` with `pinned` changes it.

**Acceptance criteria:**
- `remember_with` for a `user`-source note returns `pinned: true`; for an `agent`-source note it returns `pinned: false`.
- A `NewMemory` with `pinned: true` and source `agent` is stored pinned.
- Re-remembering the same agent note with `pinned: true` pins the existing row; with `false` it leaves it pinned.
- Updating with `pinned: Some(false)` unpins.
- The `remember` tool with `user_asked: true` stores a pinned row; with `false` or absent, an unpinned one (AGENT-MEMORY-NOTED).

### MEM-LOCATION: Large folders get a location row
**Applies to:** hoardini
**Verification:** test

After each completed index refresh of a drive (`hoardini index` calls `memory::location::refresh_volume` before it reports
`done`), each large folder of that drive shall have one location row. A folder is large when it holds at least 5 GiB or
at least 1% of the drive's used space (never less than 256 MiB) and no single child holds 90% or more of it, unless a
catalog entry names the folder itself as one unit (not a `container`, such as `%TEMP%` or `~\.lmstudio\models`); a folder
inside such a folder is never large, so ten scratch folders in `%TEMP%` do not take ten places. Only the 40 biggest per
drive count, and never the drive root. `location::large_folders(conn, env, volume)` lists them. Each is
classified, with no model call, by the first step that applies:

1. **Deny list:** a Windows system area of POLICY-DENY (the Windows folder, Program Files, `ProgramData\Microsoft`,
   `System Volume Information`, `$Recycle.Bin`, paging files, Hoardini's data) is category `system`, verdict `keep`, no
   actions; a folder a protect note covers is `protected`, `keep`, no actions. Source `deny`.
2. **Catalog:** the deepest matching entry (EVID-CATALOG), unless it is a `container` entry matched only through an
   ancestor, gives its category, its name as label, its verdict hint raised to its cap, and all four actions unless the
   verdict is `keep` (then none). Source `catalog`. For a `user-data` entry the content mix is tried first and wins when
   it finds photos or documents.
3. **Container:** a folder the deny list refuses only as itself (the drive root, the profile, AppData and its three
   folders, or a folder holding one) is category `container` with the deny list's label and no verdict. Source `deny`.
4. **Content mix:** from INDEX-EXT-MIX, so only files of 1 MiB or more have a type and small-file bytes count against
   every type. When 80% or more of the bytes are photo or video types the folder is `photos` ("your photos and videos",
   `keep`, actions `[archive]`); documents, `documents` ("your documents", `keep`, `[archive]`); model weights
   (MEM-FILETYPE's list), `model-weights` (`review`, all actions). Source `mix`.
5. **Unknown:** otherwise category and label `unknown`, no verdict, no actions. Source `index`.

The row's text is one sentence naming what decided (`Your photos and videos: 92% of its bytes are images or video.`).
A row is reassessed when the folder's size has moved by 25% or more from `size_at`, or 90 days after `assessed_ms`; a
user's answer is never reassessed. A reassessment writes only where MEM-SOURCE-RANK allows; where it may not, only
`size_at` and `assessed_ms` move. The exception is a row a finished job made (source `job:<id>`, MEM-LEARN-JOB): the
reassessment writes its text, category, label, verdict, actions, `size_at`, `assessed_ms` and `by` like a classifier
row's, and its source and entries stay. When a folder is no longer large, its row is deleted unless its source is not
the classifier's (`deny`, `catalog`, `mix`, `index`), it is pinned, or it has `cleaned` or `kept` entries.

`location::answer(conn, env, path, answer, now)` records the user's answer to "What is this?" as a `user` row, pinned,
keeping `cleaned` and `kept`: `keep` gives verdict `keep` with `[archive, compress]`; `disposable`, `safe` with all four;
`move-elsewhere`, `keep` with `[archive]`; `never-touch`, `keep` with none, plus a `protect` note on the path
(POLICY-PROTECT). An answer covers everything below it, so `answer` refuses (an error, nothing written) a path that is
not absolute once `~` expands or holds a `.` or `..` component, a volume root, a deny-list area (`policy::area_of`: a
Windows system area, or a container: the drive root, the profile, AppData and its three folders, or a folder holding one
of these, such as `C:\Users`), and a folder a catalog `container` entry names exactly (`~\.lmstudio`, `%LOCALAPPDATA%`).
A Disposable answer counts (`location::disposable_holds`) only while the folder's indexed size is under half again its
`size_at`, the size answered for (a user row is never reassessed, so `size_at` stays that size); grown by half, or with
either size unknown (`location::answer_outgrown`), it lifts no cap (POLICY-VERDICT-CAP), makes no `disposable` unit
(REVIEW-UNITS), and its review area is described as if unanswered (`unknown` when nothing else knows the folder), so the
user is asked again. `location_at` finds a folder's row, `location_covering` the nearest at or above a path, and
`location::tag` the prompt tag: the label, then the verdict when there is one (`your photos and videos, keep`, `unknown`).

**Acceptance criteria:**
- On a fake drive with 1,000 GiB used: `~\Pictures` (80 GiB, 65 GiB of it `.jpg`, `.mp4` and `.heic` in two year folders) is a `mix` row keyed `~\Pictures`, kind `verdict`, category `photos`, `keep`, actions `[archive]`, `size_at` 80 GiB, `assessed_ms` the sweep time, tag `your photos and videos, keep`.
- On the same drive `C:\code\covenant-agent` (161 GiB of `.dat` in two folders) is `unknown` with no verdict, source `index`, tag `unknown`; `C:\code`, 96% of which is that folder, has no row; `~\.lmstudio\models` is a `catalog` row by `catalog:lmstudio-models`, category `model-store`, all four actions, tag `LM Studio models, redownload`; the Windows folder is a `deny` row, `system`, `keep`, no actions; the profile is a `container` tagged `your profile folder`; a 2 GiB folder and the drive root have none; `location_covering` of a path under `~\Pictures` returns its row.
- With 45 folders of 6 GiB and up under `C:\many`, `large_folders` returns 40: `C:\many` first, then the 39 biggest of the 45.
- After Disposable for a 2 GiB folder, growing it to 2.9 GiB keeps it ticked on the `disposable` card and a file inside uncapped; at 3.1 GiB no card ticks it, its area is `unknown` again, and that file's cap is `review` (`specs/review/tests/build.rs`). Answers for a relative path, a path through `..`, `C:\`, `~`, `C:\Users`, `~\AppData\Local`, `~\.lmstudio`, `C:\Windows\Temp` and a folder under Program Files are refused and write nothing (`specs/srv/tests/srv.rs`).
- With ten 6 GiB folders under `%TEMP%\claude`, `%TEMP%` is large (a `catalog:temp-user` row) although `claude` holds all of it, and no folder under `%TEMP%` is; `~\.lmstudio\models` is large and the 20 GiB model folder inside it is not; a 6 GiB `C:\data\set` still is.
- An 8 GiB folder grown 10% keeps its `assessed_ms`; grown to 11 GiB (37.5%) it is reassessed with `size_at` 11 GiB; 90 days later it is reassessed again; a folder the user answered Keep to is unchanged through all of it.
- When three 6 GiB folders shrink to 2 GiB, the plain one loses its row, one whose detail has a `kept` entry keeps it, and one the user called Disposable keeps it.
- Each answer is a pinned `user` row with the verdict and actions above and `size_at` the folder's size; Never touch also leaves a `user` protect note covering the folder; the folder still has one location row.

Test: `specs/mem/tests/knowledge.rs`.

### MEM-FILETYPE: One row per file type
**Applies to:** hoardini
**Verification:** test

`knowledge::remember_filetype(conn, ext, text, source, detail, now)` shall keep one row per extension: scope `glob`, key
`**\*.<ext>` (lower case, leading `*` and `.` dropped; only letters, digits, `_`, `-` and `.` allowed), kind `fact`,
detail `{category, app, use, copy_hint}`, the text one sentence. Writing the same extension again updates that row
(MEM-SOURCE-RANK). `filetype(conn, ext)` and `filetypes(conn)` read them, and `weight_extensions(conn)` is the fixed
weight list (`gguf`, `safetensors`, `onnx`, `pt`, `ckpt`) plus every type whose category is `model-weights`, which the
location classifier and the model-store detector (REVIEW-UNITS) use. Matching files see the row through `recall_path`
(MEM-RECALL-ANCESTRY), so it reaches evidence and each item's note (POLICY-ITEM-MEMORY).

**Acceptance criteria:**
- `.Q4NX` from a `web:` source with category `model-weights`, app FastFlowLM and copy hint `model-hub` is stored as `**\*.q4nx`, scope `glob`, kind `fact`, and `filetype("q4nx")` returns it.
- A `user` write for `q4nx` updates the same row; `filetypes` has one row; `weight_extensions` includes `q4nx`.
- `recall_path` of `D:\npu\Qwen3.8-27B-NPU2\model.q4nx` returns that row, and it is the file's item note.
- `a/b` and a blank extension are rejected.

Test: `specs/mem/tests/knowledge.rs`.

### MEM-SOURCE-RANK: A lower source never overwrites a higher one
**Applies to:** hoardini
**Verification:** test

Location and file-type rows shall rank their sources: `user` > `job:<id>` > `catalog` = `deny` = `copy:<check>` >
`mix` > `web:<url>` > `agent` > `index` and anything else. `knowledge::upsert` shall write over the row at the same
scope and key only when the new source ranks at least as high, or when both are the classifier's (`deny`, `catalog`,
`mix`, `index`), so a reassessment can re-decide its own row. Otherwise it returns the stored row unchanged and says it
did not write. A write keeps the stored `cleaned` and `kept` entries unless it brings its own.

**Acceptance criteria:**
- `source_rank` rises strictly through `index`, `agent`, `web:`, `mix`, `catalog`, `job:7`, `user`, and `deny` equals `catalog`.
- Writing one key from `agent`, `web:`, `mix`, `catalog`, `job:3` and `user` in that order rewrites the same row each time; after the `user` write, writes from `job:4`, `catalog`, `deny`, `mix`, `web:`, `agent` and `index` change nothing.
- An `index` (unknown) write does not replace a `web:` row; a `mix` write replaces a `catalog` row.

Test: `specs/mem/tests/knowledge.rs`.

### MEM-WEB-IDENTITY: Web knowledge is identity, not instruction
**Applies to:** hoardini
**Verification:** test

A location or file-type row whose source is `web:<url>` shall record only what app or purpose the folder or type belongs
to: on write its verdict and actions are dropped and its kind becomes `fact`, so the stored `detail_json` has no
`verdict` and no `actions`. Whether something can go is decided only by the catalog, the user, a finished job or a copy
check.

**Acceptance criteria:**
- A `web:` row for `~\.hipfire` written with verdict `safe`, actions `[delete]` and kind `verdict` is stored as kind `fact` with app `hipfire` and use `model cache`, and its `detail_json` has neither `verdict` nor `actions`.
- A `web:` file-type row written with verdict `redownload` has no verdict.

Test: `specs/mem/tests/knowledge.rs`.

### MEM-LEARN-JOB: Finished jobs teach the location rows
**Applies to:** hoardini
**Verification:** test

When a delete, archive or compress job ends (done, partial, failed or cancelled), the executor shall record what the
card decided, after the follow-up (ACT-FOLLOW-UP) and before the job's terminal update is broadcast
(`memory::learn::learn_from_job`):

- Each item whose status is `done` is added to `cleaned` on the location row that covers it.
- Each item left unticked on the approved card (not selected, `skipped`) is added to `kept`, except on a review card
  whose registry entry starts unticked (`downloads`, `compress`, `archive`, REVIEW-BUILD) or its follow-up: there an
  unticked item is one nobody chose, so it is not kept.
- Failed, skipped-after-approval, blocked and declined items, and dedupe jobs, add nothing.

The covering row is the nearest location row at or above the item, passing over `container` rows. Items with no
covering row get a new row with source `job:<proposal_id>`, kind `verdict`, category `unknown`, no verdict, `by`
`job`, and a one-sentence text (`You cleaned 3 items here.`, `You kept 1 item here.`). It sits at the first of these
that is not a volume root and not a system or container area (POLICY-DENY): the proposal's `anchor` when it holds the
item, else the deepest folder holding all such items of the job, else the item's parent; when none qualifies, the item
is not recorded. Later jobs add to that row, and the sweep classifies it like any other (MEM-LOCATION). The writes run
in one immediate transaction, so a job ending while another connection writes (a refresh, the copy worker, a review
build) waits on the busy timeout instead of losing what it learned.

Entries (`ms` the job's end, paths and rules as stored keys: `~\...` under the profile, `\`):

- `cleaned`: `{ms, bytes, path, rule, job, kind}`. `bytes` is the item's journaled freed bytes (ACT-JOURNAL), else its
  `alloc`. `job` is the proposal ID, and `kind` is `delete`, `archive` or `compress`.
- `rule` is a glob (`*` within a folder, `**` across folders, the characters `*?[]{}` of a name bracketed as `[*]`):
  - When a catalog entry matched (EVID-CATALOG) at the item, or above it for an entry whose unit is `self` or
    `contents`, a `**\...` pattern of that entry is anchored under the row (`D:\code\**\target`), and any other match
    is the matched folder itself when it is at or under the row.
  - Otherwise it is the exact path, except that two or more uncatalogued done files of the job with the same parent
    and extension get `<parent>\*.<ext>`.
- `kept`: `{ms, path, size, job}`, `size` the item's `alloc`.

A new entry replaces one with the same path in its list; a `cleaned` entry also removes a `kept` entry for the path.
Each list keeps the newest 100.

Reading them back:

- `location::was_kept(conn, path, size)` is true when a row at or above `path` keeps an entry for exactly that path
  and `size` is less than 1.5 times the kept size, so the review leaves it out until it grows by half.
- `location::cleaned_for(conn, path)` returns the newest `cleaned` entry, on any row at or above `path`, whose path is
  `path` or whose rule matches `path` or a folder above it. `Cleaned::reason()` reads
  `you cleaned this on 10-09 (28.0 GB)` (`archived`, `compressed` by kind; the date in UTC).

`memory::learn::learned_for(conn, proposal_id)` lists, per row holding entries of that job, `{memory_id, text,
confirmed}` (`confirmed`: the row's source is `user`, so the chip offers no Confirm):
`target folders in C:\code — cleaned 28.0 GB on 10-09`, then `; 1 item you left unticked is kept` when there are
kept entries (`2 items you left unticked in C:\code are kept` when there are only those). The text names the shared
rule (`.log files`, `target folders`), else the one item's name, else `N items`. The proposal JSON carries it as
`learned` for `done`, `partial` and `failed` delete, archive and compress proposals (SRV-LEARNED); online-only and system-cleanup jobs learn nothing. `learn::undo(conn,
proposal_id, memory_id)` removes that job's entries from the row and deletes the row when a job made it, nothing is
left in it, and it is not pinned; it returns false when the row holds nothing of that job.

**Acceptance criteria:**
- A delete of `work\code\proj\target` (under an `index` row for `work\code`), `notes\one.log`, `notes\two.log`,
  `misc\odd.dat` and a file deleted before approval, with `keep\big.bin` left unticked: the `work\code` row keeps
  source `index` and gains exactly `{ms, bytes, path, rule, job, kind}` =
  `{<job end>, <target alloc>, <target>, work\code\**\target, <proposal>, delete}`. A new `job:<proposal>` row at
  `work` reads `You cleaned 3 items here.`, has category `unknown`, no verdict and `by` `job`, and holds the two logs
  with rule `work\notes\*.log`, `odd.dat` with its own path, and `kept` `[{ms, path: keep\big.bin, size: <alloc>,
  job}]`. The vanished file is nowhere.
- That proposal's `learned` is `target folders in work\code — cleaned <size> on <MM-DD>` for the first row and
  `3 items in work — cleaned <sum> on <MM-DD>; 1 item you left unticked is kept` for the new row; its pending
  follow-up has none.
- With the proposal's `anchor` set to `work\cards`, a deleted `cards\deep\x\file.bin` gives a row at `work\cards`
  (`You cleaned file.bin here.`); a second job's file under it joins that row. Undo of the first job leaves the row
  with the second job's entry; a second undo returns false; undo of the second job then deletes the row, and so does
  undo of a job that made a row alone.
- An archive whose copy fails verification, a declined delete, and a delete dedupe leave no `learned`, no `job:` row
  and no entry naming a job.
- With kept `D:\code\big.iso` at 1,000 bytes: `was_kept` is true at 1,000 and 1,499 (any case) and false at 1,500,
  for another file, and for `D:\code`; an entry on `D:\code` still counts for `D:\code\sub\k.bin` under a nearer
  `D:\code\sub` row; an unknown entry shape is skipped.
- `cleaned_for` of `D:\code\b\target` and `D:\code\b\target\debug\x.o` returns the `D:\code\**\target` entry, whose
  reason is `you cleaned this on 01-15 (28.0 GB)`; `D:\code\logs\b.log` gives `you archived this on 01-16 (5.0 MB)`;
  `D:\code\logs\sub\c.log` and `D:\code\other.bin` give none; `D:\code\[x]\y.bin` matches its bracketed rule; a
  `~\scratch\x.bin` entry matches `C:\Users\Tester\scratch\x.bin`.
- While another connection holds the write lock for 300 ms, `learn_from_job` waits and records the done item.
- A done `go.bin` and an unticked `left.bin` on a `downloads` review card give no `kept` entry; on a `temp` card
  `left.bin` is kept.
- On a row holding 100 `cleaned` entries and a `kept` entry for `go.bin`, the same job leaves 100 `cleaned` entries,
  the oldest gone and `go.bin` last, and `kept` holding only `left.bin`.
- A `job:p9` row at `~\Pictures` holding a `cleaned` entry, swept once the folder holds 60 GiB of photos and video,
  becomes category `photos`, `keep`, actions `[archive]`, `by` `mix`, with a `Your photos and videos: ...` text, and
  keeps source `job:p9` and its entry.

Test: `specs/act/tests/learn.rs` (through the executor), `specs/mem/tests/learn.rs`, `specs/mem/tests/knowledge.rs`.
