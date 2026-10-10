# Policy gate (prefix POLICY)

The gate is code, not prompt. It runs when a proposal is created (`proposals::create`) and again before each item
executes (`Executor::approve` and the job thread). Canonical design: `.claude/plans/hoardini-design.md` §5.5.
Code: `crates/core/src/policy`, `crates/core/src/proposals`. Tests: `specs/policy/tests/policy.rs`.

## Model

Every requested path is resolved first: `~\...` expands to the profile, `.`/`..` collapse, and short names and junctions
in the parent chain resolve to the real location (a final link stays the link itself). A final link or cloud
placeholder, which is not opened, keeps its on-disk name, so a trailing dot or an 8.3 alias resolves to the real name.
Then one of three outcomes:

- **Denied**: the path never becomes an item. It is listed in the proposal's `dropped` with a reason. A request whose
  every path is denied fails with an error that names each path and reason.
- **Blocked**: the item is stored and shown with `status = "blocked"`, a `block_reason`, and `selected = false`. It
  cannot be approved.
- **Selectable**: `status = "pending"`; `total_alloc` sums these items only. It starts ticked or unticked per
  POLICY-DEFAULT-TICK.

Verdicts are ordered `safe` < `redownload` < `synced` < `review` < `keep`. `synced` means a cloud sync engine holds the
file: its local copy may go online-only (ACT-ONLINE-ONLY), never be deleted for it. Each item stores a `cap` (`keep` /
`review`, `redownload` or `synced` where a copy check relaxed `review`, or none) and the shown `verdict`, which is the
LLM's verdict raised to at least the cap (`Verdict::capped`); an online-only item's verdict comes from the sync check, not
the LLM. Each item's JSON also carries `copy_note`, the best copy-check answer's note or "only copy we know of"
(`copycheck::note_for`, EVID-COPY-CHECK), on cards that are pending or running, except `system_cleanup` cards, whose items are operations. `policy::cap_for(conn, path, catalog_cap, in_use)` exposes the same cap to evidence.
The locations below come from `PolicyEnv` (`system()` reads the real environment; `with_paths(profile, data_dir)` lets
tests point the profile and data folder at temp directories).

`system_cleanup` proposals (ACT-WINDOWS-CLEANUP) hold operations that Windows' own tools carry out, not paths, so they do
not go through the gate at creation or execution; the elevated helper runs only its fixed list of operations
(ACT-WINDOWS-CLEANUP-RUN).

### POLICY-DENY: Deny-listed locations are never proposed
**Applies to:** hoardini
**Verification:** test

The gate shall refuse, with no override, any path that is, is inside, or contains: `%SystemRoot%`, a `Program Files*`
folder at a volume root (and the `%ProgramFiles*%` locations), `ProgramData\Microsoft`, `System Volume Information`,
`$Recycle.Bin`, and Hoardini's data folder. It shall refuse `pagefile.sys`, `hiberfil.sys`, and `swapfile.sys`. It shall
refuse any volume root (`C:\`, `\\server\share`), the profile root, `AppData`, and `AppData\Local`, `Roaming`, and
`LocalLow` themselves, and any ancestor of them (so `C:\Users` is refused because it holds the profile). It shall refuse
any network path that leads back to a fixed volume of this PC (`\\localhost\C$\...`, a share of a local folder; the
share reports the local volume's serial), since the rules above are written for local names. When the active
provider points at `localhost`/`127.0.0.1`/`::1` and names a model, files of that model under the LM Studio, Ollama, and
Hugging Face stores (path contains the model id, case-insensitive; Ollama blobs via the model's manifest) and every
ancestor holding them shall be refused (best effort).

**Acceptance criteria:**
- Each of `C:\`, `D:`, `\\nas\share`, `%SystemRoot%` and a file under it, `C:\Program Files\Vendor\app.exe`, `C:\Program Files (x86)`, `E:\Program Files\Steam`, `ProgramData\Microsoft\Windows\WER`, `ProgramData`, a file under `System Volume Information`, a file under `$Recycle.Bin`, `pagefile.sys`, `hiberfil.sys`, `swapfile.sys`, the profile root, `AppData`, `AppData\Local|Roaming|LocalLow`, the profile's parent, the data folder, and a file in it has a deny reason.
- `~\Downloads\big.iso`, `~\AppData\Local\pip\cache\wheel.whl`, and a file in an ordinary folder have none.
- A request made only of denied paths creates no proposal and returns an error naming the reasons ("volume root", "profile").
- A mixed request keeps the allowed items; `profile\Downloads\..\..` and `~\AppData\Local` are among the `dropped` entries.
- With an active `http://localhost:1234/v1` provider and model `qwen3.8-27b`, `~\.lmstudio\models\...\Qwen3.8-27B-GGUF\Qwen3.8-27B-Q8_0.gguf`, its folder, `models\lmstudio-community`, `models`, and `~\.lmstudio` are refused with the model id in the reason; a sibling Llama model is not. With a non-local base URL nothing is refused.
- Where the administrative share is reachable, the profile root and a file in it, written as `\\localhost\C$\...`, are refused with "this PC" in the reason, and a proposal for the profile that way creates nothing.

### POLICY-PROTECT: Protect memories block their paths
**Applies to:** hoardini
**Verification:** test

A path shall be refused when a `protect` memory covers it (the path itself, an ancestor, or a matching glob, per
MEM-RECALL-ANCESTRY) and when it contains a protected path (a `path` key below it, or a glob whose literal prefix is at
or below it, or a wildcard glob that matches something deeper inside it, found by walking the folder's index nodes up to a
bound; a folder the index cannot answer for is refused with a "review it first" reason). A protect memory added after a
proposal shall stop that item at execution.

**Acceptance criteria:**
- With protect `work\keep` ("family photos"): `work\keep\2024\a.jpg`, `work\keep`, and `work` are refused with the memory text; `work\other\b.bin` is not.
- With protect glob `work\secrets\*.kdbx`: `work\secrets\vault.kdbx` and `work\secrets` are refused.
- A proposal for `work\other\b.bin`, approved after a protect on `work\other` is added, skips the item with a "Denied: ... late protect" detail and the file stays.
- With protect glob `work\**\secrets\**`: `work\proj` (holding `app\secrets\key.pem`) and `work\proj\app` are refused naming the memory; `work\proj\app\src` and an unrelated `work\clean` are not. Before the index is built, `work\proj` is refused with "the index cannot confirm".
- Deleting the memory lifts the refusal.
- A protected offline (cloud placeholder) file requested with a trailing dot (`Tax Return 2024.pdf.`) is refused with the memory text.

### POLICY-IN-USE: Open files block delete and archive
**Applies to:** hoardini
**Verification:** test

A file held open by any process (Restart Manager) shall be blocked with `Open in <app> (pid N)`. A folder shall be
checked through its largest files (up to 500): from the index when the folder is indexed, else by listing it. The item
cap is `keep`. Archive and compress of a cloud placeholder (or a folder holding one) shall be blocked because it would
download; a missing path shall be blocked as `Not found`. A file opened after the proposal shall be skipped at execution
with the same reason.

**Acceptance criteria:**
- A file this test process holds open, an indexed folder holding it, and an unindexed folder holding another held file are each `blocked`, unselected, cap `keep`, with a reason starting `Open in ` and containing `(pid <this pid>)`; `total_alloc` is 0.
- An archive proposal for a held file is blocked the same way; approving a blocked item is rejected and the proposal stays `pending`.
- A file opened between proposal and approval is `skipped` with the `Open in` detail and still exists.
- A file with the offline attribute is blocked for archive (reason mentions the download) and selectable for delete; a missing path is blocked `Not found`.
- A folder whose open file is too small to have an index row (so the check cannot see it) is not deleted in part: the item does not finish `done`, every file in the folder still exists, and 0 bytes are freed (ACT delete renames the folder first).

### POLICY-REVALIDATE: Changed items are skipped at execution
**Applies to:** hoardini
**Verification:** test

Before an approved item runs, the gate shall compare the path with what was proposed: kind, size (files), mtime, and
file ID. Any difference, a deny reason, an open handle, or a missing path shall mark the item `skipped` with a detail;
it shall not execute. The proposal ends `partial` when some approved items ran and others did not. The open-handle
check is the slow one, so it runs first; the deny list and the kind, size, mtime, file-ID, and link comparisons run
after it, immediately before the item executes, so a path swapped for a junction during the slow check is caught.
An item of a review card (REVIEW-BUILD) inside a catalog `contents` location with a minimum age (`%TEMP%`,
`Windows\Temp`: a part, REVIEW-UNITS, which was old when proposed; a chat card's item is the user's own request and is
not walked) is walked again, links not followed and at most 250,000 entries, because a file new deep inside changes no
time the proposal compared: when the path or anything in it was written or created within that age the item is skipped with
`Changed since the proposal: something in it was written or created in the last <hours> hours`, and when the walk cannot
finish (too many entries, or a folder it cannot list) with `Too many files inside, or one unreadable, to confirm nothing
in it is new`. Last, a delete whose stored cap is `redownload` on a path no catalog entry knows (a cap a copy relaxed,
POLICY-VERDICT-CAP) runs the copy check again (`copycheck::identical_for`); when no copy counts any more (gone or changed,
in a Recycle Bin or a cleared place, or ticked on a live delete or archive card, this job's own included) the item is
`skipped` with a detail starting `Its copy no longer counts`, so two twins approved on two cards never both go.

**Acceptance criteria:**
- A folder replaced by a junction while the open-handle check runs is skipped with a detail starting `Changed since the proposal` (verified by a test-only hook between the two phases in `crates/core/src/policy/gate.rs`).
- Of four proposed files, one grown, one with only its mtime changed, and one deleted and re-created with the same size and mtime are `skipped` with a detail starting `Changed since the proposal` and still exist; the untouched one is deleted.
- `approve` returns only the untouched item's id; the proposal status is `partial`.
- Of two 3-day-old `%TEMP%` parts on a review card, ticked, one gets a file written in an already existing subfolder after the proposal: approving both skips it with `Changed since the proposal: something in it was written or created in the last 24 hours`, keeps every file in it, deletes the other part, and leaves the proposal `partial`.
- A file proposed alone as `redownload` on the strength of its duplicate twin, whose twin is then deleted by hand, is `skipped` at approval with `Its copy no longer counts...` and still exists. Of two twins on two live delete cards, once the second card's twin is ticked, approving the first card skips its item and keeps the file; approving the second then deletes only its own (`specs/policy/tests/policy.rs`).

### POLICY-VERDICT-CAP: The shown verdict never exceeds the cap
**Applies to:** hoardini
**Verification:** test

Each item's cap shall be the strongest of: the catalog cap; `review` for a path no catalog entry knows; `review` for a
single file inside a shared-blob store (the whole model folder is the safe unit); `keep` when the file is in use or the
path is denied. The stored verdict is the LLM verdict raised to the cap; a verdict above the cap stays. A copy check that
proved another copy relaxes the one cap that is about not knowing (EVID-COPY-CHECK): a current `identical` answer from
any check but `sync` turns the `review` cap of a path no catalog entry knows into `redownload`, so the item can start
ticked. A `sync` answer does that only on an online-only card, where the cap becomes `synced`; on any other card a cloud
copy never relaxes anything, because deleting a synced file deletes the cloud copy too. The offline checks run again for
the item first (`copycheck::identical_for`), so a twin deleted or an archive copy gone since the last pass never counts,
and a copy the same card would remove (a path the card also holds, or one inside it) does not count either. Nor does a
copy on this PC that is no longer there unchanged, sits in a Recycle Bin, sits in a place the catalog (the policy env's)
calls temp, cache, package, browser or installer cache, build output, logs, crash dumps or Windows leftovers
(`copycheck::in_cleared_place`), or is at or under a path ticked on a live (pending, approved or running) delete or archive
card (`copycheck::held_by_live_card`; an unticked item waits for its tick, which POLICY-REVALIDATE checks again). A shared
blob's `review`, a catalog cap, and `keep` stay, and no other confidence changes a cap. The user's Disposable answer
(MEM-LOCATION: the nearest location row from the user at or above the path, with verdict `safe`; classifier and job rows
in between do not count, `location::user_answer_covering`) is knowledge too: while it holds (the folder has not grown by half since the answer, `location::disposable_holds`) it
lifts the `review` cap of a path no catalog entry knows. On an online-only card nothing is deleted, so the catalog's caps, which are about deleting (a sync
folder's own entry caps deletes there at `review`), do not apply: the cap is `synced` when the sync check finds the item
in sync and `review` otherwise, and `keep` still wins for a path in use or denied.

**Acceptance criteria:**
- LLM `safe` on an HF hub blob file → verdict `review`, cap `review`; on `disk.vhdx` → `review`/`review`; on an unknown file → `review`/`review`.
- LLM `redownload` on a whole `models--org--name` folder → `redownload`, no cap; `safe` on `node_modules` → `safe`, no cap.
- LLM `keep` on an unknown file stays `keep` and the item starts unselected.
- `cap_for_with(.., in_use = true)` and the cap of the profile root are `keep`; every item satisfies verdict ≥ cap.
- On a delete card, unknown files the model calls `safe`: one with a current `identical` Hugging Face answer shows `redownload`, cap `redownload`, ticked, with the copy note; one with `same-name-and-size` and one with an `identical` `sync` answer show `review`/`review`, unticked; an HF blob file with an `identical` answer stays `review`/`review`. `cap_for_with` gives `redownload` for the first file and `keep` when it is in use. An in-sync file on an online-only card shows `synced`, cap `synced` (`specs/act/tests/online.rs`).
- Two 101 MiB files in a current duplicate group: one proposed alone shows `redownload`, ticked; both on one card show `review`/`review`, unticked; after the twin is deleted by hand, the first proposed alone shows `review`, unticked.
- While the first file sits ticked as `redownload` on a live card, its twin proposed on a second card shows `review`/`review`, unticked, with no copy note. A twin in a group whose other member sits in a `$Recycle.Bin` folder, or in the test catalog's `%TEMP%`, leaves the file `review`/`review`, unticked.
- An unknown `junk\x.bin` is capped `review`; after the user's Disposable answer for `junk` its cap is none and the model's `safe` stays, ticked, also for `junk\deeper\y.bin` under an `index` row of its own; in use it is `keep`. An in-sync video under `~\OneDrive` (the catalog's `onedrive-sync`, cap `review`) is `synced` and ticked on an online-only card (`specs/review/tests/build.rs`).

### POLICY-DEFAULT-TICK: Only worthwhile safe or redownload items start ticked
**Applies to:** hoardini
**Verification:** test

A new proposal's item shall start ticked (`selected = true`) only when all three hold: it is not blocked; its stored
verdict, which is already capped (POLICY-VERDICT-CAP), is `safe` or `redownload` (an item with no verdict counts as
`review`); and it is not small. An item is small when its `alloc` is under 50 MiB **and** under 2% of the proposal's
`total_alloc`, so a card made only of small files still ticks its biggest ones. Every item's JSON carries
`small: boolean`. The rule is `proposals::is_small(alloc, total)` and `proposals::default_tick(blocked, verdict, small)`.
On an online-only card (ACT-ONLINE-ONLY) the same rule ticks `synced` instead of `safe` or `redownload`
(`proposals::default_tick_for(kind, blocked, verdict, small)`); `synced` never ticks on any other card. A follow-up proposal (ACT-FOLLOW-UP) keeps the approved tick state instead, and a review card that starts unticked
(REVIEW-BUILD: Downloads, Compress in place, Send elsewhere) leaves every item unticked. Tests: `specs/policy/tests/policy.rs`;
the item JSON is also checked in `specs/srv/tests/srv.rs`, and unticked review cards in `specs/review/tests/build.rs`.

**Acceptance criteria:**
- `is_small`: 50 MiB − 1 byte of a huge total is small; exactly 50 MiB and 50 MiB + 1 are not. 1,999 of 100,000 is small; exactly 2,000 (2%) and 2,001 are not. 3 MiB of a 3 MiB + 40 KiB card is not small while 4 KiB of it is. 0 of 0 is not small.
- `default_tick` is true for unblocked, not-small `safe` and `redownload`; false when blocked, small, `synced`, `review`, `keep`, or no verdict.
- `default_tick_for(online_only, ..)` is true only for unblocked, not-small `synced`; `default_tick_for(delete, .., synced, ..)` is false, and for every other kind it equals `default_tick` (`specs/act/tests/online.rs`).
- In one proposal: a `node_modules` folder the model calls `safe` and an HF `models--org--m` folder it calls `redownload` start ticked; a `node_modules` called `review`, one with no verdict, and an unknown file called `safe` (shown as `review`) start unticked; a 4 KiB `node_modules` called `safe` is `small` and unticked; no other item is `small`.

### POLICY-ITEM-MEMORY: Each item shows the note that covers it
**Applies to:** hoardini
**Verification:** test

When a proposal is created, each item shall store `memory_note`: the text of the first non-protect, non-forgotten
memory covering its path in MEM-RECALL-ANCESTRY order (the path itself, then each ancestor, then globs), or null.
Location rows written by the classifier (sources `deny`, `catalog`, `mix`, `index`; MEM-LOCATION) are skipped, since
they only restate the catalog and the index; file-type rows and location rows from the user, a job or the web count. A
row a finished job made (source `job:<id>`, MEM-LEARN-JOB) speaks through what it cleaned: when `location::cleaned_for`
finds a `cleaned` entry for the path, the note is `You cleaned this on MM-DD (<size>).`; otherwise the row's own text
counts only while the job still describes the row (`by` `job`), and once the sweep has reclassified it, it is passed
over like a classifier row. A note the item's reason already carries (case aside) is not stored again.
Attaching a note marks that note used (MEM-USAGE); other covering notes are not touched. A protect memory still refuses
the path (POLICY-PROTECT). The server redacts the note like the other item text.

**Acceptance criteria:**
- With notes on `work` ("work folder"), `work\scratch` ("scratch is disposable"), and `work\scratch\old` ("old builds, fine to drop"): `work\scratch\old\a.bin` carries "old builds, fine to drop", `work\scratch\b.bin` carries "scratch is disposable", and a file outside `work` carries null.
- A file under a folder whose only covering row is an `index` location row carries null; once the user's answer replaces that row, the file carries "You said this folder can go."
- `work\keep\d.bin`, under both a fact and a protect ("family photos") on `work\keep`, is in `dropped` with the protect text.
- The "scratch is disposable" note's `last_used_ms` moves to the proposal time; the "work folder" note's stays at its old value.
- Under a `job:` row for `logs` that the sweep reclassified as documents and whose `cleaned` entry has rule `logs\*.log` (5 MB, 2024-01-15): `logs\a.log` carries `You cleaned this on 01-15 (5.0 MB).`, `logs\b.bin` carries null, and a file under a `job:` row still `by` `job` carries that row's text `You cleaned 3 items here.`. With the reason `Old logs. You cleaned this on 01-15 (5.0 MB).` the note is null; with `Old logs.` it is the cleaned sentence.

### POLICY-LOCATION-KEEP: Kept locations block delete and dedupe
**Applies to:** hoardini
**Verification:** test

A location row (MEM-LOCATION) whose verdict is `keep` shall block, on a path at, under, or containing the location,
delete and dedupe always, and archive and compress unless its `actions` list them (your photos allow `archive`, a Keep
answer allows `archive` and `compress`, OS files allow none). The item is stored
`blocked` with a reason naming the location and quoting the row's text, and the same check skips an approved item at
execution when the row appeared after the proposal. A location whose verdict is not `keep` blocks nothing, and no
location blocks online-only (the cloud copy stays, ACT-ONLINE-ONLY) or a system cleanup. The check is
`memory::location::keep_block(conn, env, path, action)`, called from `policy::examine` and `policy::revalidate`.

**Acceptance criteria:**
- With `work\photos` kept with `[archive]` ("Your photos and videos: ...") and `work\system` kept with none: deleting a photo or a file under `system` is `blocked` with a reason containing "a kept location" and the row's text; a file under a `safe` location stays `pending`.
- Deleting `work\photos` itself is blocked, and deleting `work`, which contains it, is blocked with "contains".
- For the photo, `examine` blocks dedupe and compress and allows archive, and an online-only block never names the kept location; for the `system` file it blocks archive; under the `safe` location dedupe is allowed.
- A delete proposed before `work\later` became a kept location is `skipped` at approval with the row's text in its detail, and the file stays.

Test: `specs/policy/tests/policy.rs`.
