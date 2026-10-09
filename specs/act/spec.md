# Actions, destinations, journal (prefix ACT)

The executor runs approved proposals as background jobs. Canonical design: `.claude/plans/hoardini-design.md` §5.6.
Code: `crates/core/src/{actions,destinations}`. Tests: `specs/act/tests/act.rs`.

## Model

`Executor::approve(proposal_id, item_ids)` is the only call that changes the filesystem. It accepts a `pending`
proposal and a subset of its selectable item ids, revalidates them (POLICY-REVALIDATE), marks the proposal `approved`,
starts a job on its own thread, and returns `{approved, items, job_id}`. Jobs run one at a time (`queued` → `running` →
`done` | `failed` | `cancelled`); progress (bytes, items, current path) goes to `Executor::subscribe()` and the `job`
table; cancel takes effect between files and between 1 MiB copy chunks. Kinds: delete, archive (local folder,
SMB UNC path), compress, dedupe, online-only.

- **Delete**: `remove_file` (read-only cleared); a folder is first renamed to a temporary sibling name (Windows refuses
  while anything inside is open, so nothing is deleted then), then `remove_dir_all` (never follows links inside); a link
  item removes the link itself.
- **Archive**: copy to `<destination>\<yyyy-mm-dd local time>\<original path without the drive colon>` (`UNC\server\share\...`
  for network sources), hashing the source while copying into `<name>.hoardini-partial`; flush; re-read the copy; size
  and BLAKE3 must match; rename into place; check that every source file still has the size and mtime it had when its
  copy started and that no file was added; then delete the source. Folders copy file by file with the same check; links
  inside are skipped and named in the item detail. When an earlier `archive_entry` holds that location (or a folder
  above or inside it), the next free `<yyyy-mm-dd>-N` folder is used instead, so an archive never writes into another's
  copy. A re-run reuses a final copy whose BLAKE3 still matches. User
  folders (no catalog entry, `user-data`, `downloads`) get a `<name>.hoardini.txt` stub naming the copy; caches do not
  (`meta.archive_stubs = false` turns stubs off).
- **Compress**: formats `zip`, `7z`, `tar.zst` write `<name>.<ext>` next to the original (`<name>.<ext>.hoardini-partial`
  while writing), hashing every source file while it is added; then the archive is re-read, every entry's size and
  BLAKE3 must match, the sources must be unchanged, and only then is the archive renamed into place and the originals
  deleted (undo = extract). Links, reparse points, and cloud-only files make the item fail up front. `lzx`
  (`compact /c /exe:lzx`, for data that is not written to again) and `lznt1` (`compact /c`) run `compact.exe` in place with
  no console window (undo = `compact /u`); a folder holding a link, reparse point, or cloud-only file fails up front too,
  because `compact /s` would follow the link. Freed bytes = allocation before minus after (WOF files measured by handle).
- **Dedupe**: items are the extra copies; `keep` (from the proposal) is never listed, opened for write, or changed.
  `hardlink` (same volume only) hashes keep and extra in full at execution, creates the link under a temp name beside
  the extra, then renames it over the extra; `delete` removes the extra. Candidates come from `core::dupes`: same
  logical size, distinct `file_id` (hard links are one file), equal head+tail sample, then equal full BLAKE3, files of
  at least 100 MB by default, never cloud placeholders. Full hashes are reused while size, mtime, and `file_id` hold.
- **Online-only**: for files in a cloud-synced folder (`cloud::Cloud`, EVID-COPY-SYNC). Each file whose placeholder the
  sync engine marks in sync is unpinned and dehydrated (`CfSetPinState` unpinned, then `CfDehydratePlaceholder`, the
  same as Explorer's "Free up space"); it stays in its folder and downloads again when opened. Files not in sync are left
  as they are. Nothing is deleted, moved, or read. Undo is "Always keep on this device".
- **Journal**: one `action` row per item, written `running` before work starts and `done`/`failed`/`cancelled` with
  `freed_bytes`, `undo_json`, and `error` after; archive also writes an `archive_entry` (size, BLAKE3 of the file or of
  the folder manifest, stub path) and the action's `undo_json` before it deletes the source. The index is patched after
  each item. `vanished_total` = sum of `action.freed_bytes`.
- **Restore**: `Executor::restore(archive_entry_id)` copies the archive back to a sibling temp name, checks it against
  the recorded BLAKE3, renames it to the original path, removes the stub, sets `restored_ms`, and keeps the archived copy.
- **Destinations**: `local` (`config.path`), `smb` (`config.path` = UNC, optional `username`, password from the vault
  at `config.password` or `secret://destination/<id>/password`, else the current Windows session), `rclone`
  (`config` = `{remote, type, path, params}`; see below). Config values under keys that look secret must be
  `secret://` refs. `destinations::test` checks reachability, writability, and free space.
- **rclone**: one pinned `rclone.exe` (`core::runtime::RCLONE`, downloaded on demand, SHA-256 checked before every
  launch, never from PATH) runs as `rclone rcd` on `127.0.0.1:<random port>` with random rc credentials and an
  encrypted `rclone.conf` in the data dir whose password lives in the vault (`secret://rclone/config-pass`, child
  environment only). The daemon starts on first use, restarts if it dies, stops after 10 idle minutes, and dies with the
  app (job object). Remotes are created through `config/create` the first time a destination is used, with vault refs
  in `config.params` resolved at that moment; an existing remote is left alone so refreshed OAuth tokens survive.
  Google Drive requires the user's own client ID and secret and is pinned to scope `drive.file`. Archive copies with
  `_async` rc jobs (`_group` per item), reports bytes from `core/stats`, and cancels with `job/stop`; the copy lands at
  `<path>/<yyyy-mm-dd>/<original path without the drive colon>` and `archive_entry.dest_uri` is
  `rclone:<remote>:<path>`. Server routes: `GET /api/runtime/rclone`, `POST /api/runtime/rclone/install` (a job),
  `POST /api/destinations/rclone/authorize`.

### ACT-ARCHIVE-VERIFY: Archive deletes the source only after the copy verifies
**Applies to:** hoardini
**Verification:** test

An archive shall delete a source file or folder only after every copied file's size and BLAKE3 match the source as read
during the copy, and only while every source file is unchanged since its copy started and no file was added. A copy
that fails verification shall be removed, the item shall fail, and the source shall stay untouched. A re-run shall
reuse copies that still verify. An archive shall never overwrite or add to a copy that an earlier `archive_entry`
records.

**Acceptance criteria:**
- Archiving a 3 MiB file and a nested folder (with an empty subfolder) to a local destination removes both sources; the copy sits under the destination with the original layout (`...\work\docs\model.gguf`), byte-identical, and `archive_entry.blake3` equals BLAKE3 of the original bytes.
- When each finished copy is altered before verification, the file and the folder items fail with "verification failed", both sources are byte-identical to before, no `archive_entry` is written, and the job frees 0 bytes.
- After a run whose second file failed verification, a new archive of the same folder copies only the two files not yet verified and succeeds.
- A file created in the folder while it is copied, or a file appended to after its copy finished, fails the item; the source (with the new data) stays, no `archive_entry` is written, and 0 bytes are freed.
- An archive whose copies fail verification leaves the last-access time of the source file and of a file inside the source folder unchanged (the copy reads the source through the atime-preserving handle).
- Archiving a file and a folder, then new content at the same paths again the same day, gives four entries with four distinct `dest_uri`s, and restoring the first two brings back the first bytes and tree.

### ACT-JOURNAL: Every action is journaled with undo info before it reports success
**Applies to:** hoardini
**Verification:** test

Each executed item shall have an `action` row whose status is `done` and whose `undo_json` is set before the item's
status becomes `done` or a job event reports it. Delete undo info says there is no undo; archive undo info names the
`archive_entry`, destination, and copy path. The vanished total shall equal the sum of journaled freed bytes and survive
a restart.

**Acceptance criteria:**
- At every job event during a run of a two-file delete and a one-file archive, every item already `done` has a `done` action row with `undo_json`.
- Delete rows carry `{"kind":"none"}` and `freed_bytes` equal to the item's allocated size; the archive row carries `{"kind":"restore","archive_entry_id","dest_uri"}` matching its `archive_entry`.
- `vanished_total()` equals the sum of `freed_bytes` and is unchanged after a new `Executor` opens the same database.

### ACT-FOLLOW-UP: A finished job leaves a refreshed card for what was not done
**Applies to:** hoardini
**Verification:** test

When a delete, archive, compress, or online-only job ends (done, partial, failed, or cancelled), the executor shall create a pending
follow-up proposal whose `parent_id` is the job's proposal, in the same conversation, holding every item whose status is
not `done`. Each item starts with the tick it was approved with: an approved item that failed or was skipped comes back
ticked; an item left unticked comes back unticked. The items go through proposal creation again, so path resolution,
the deny list, `alloc`, blocks, caps, in-use status, and the memory note are current: a newly blocked item is unticked,
and a newly denied path is listed in `dropped`. An item whose path no longer exists is left out. Kind, destination,
format, summary, verdicts, and reasons carry over; the follow-up has no `tool_call_id`. Nothing is created when no item
is left, and dedupe jobs get no follow-up. The follow-up row is committed before the job's terminal update is broadcast
(`Executor::subscribe`, so the status stream's `job` frame too). Proposal JSON (`GET /api/proposals/{id}` and the
`proposals` map of `GET /api/thread`) carries `parent_id` and `follow_up_id` (the newest proposal whose `parent_id` is
this one), both null when absent. A job interrupted by a restart (marked failed at startup) gets no follow-up. Tests:
`specs/act/tests/follow_up.rs`; the JSON and stream order are also checked in `specs/srv/tests/srv.rs`.

**Acceptance criteria:**
- Of seven delete items, one done, one grown after the proposal (approved, skipped as changed), one left unticked, one deleted before approval, one opened after the proposal and still open, one open at proposal time (blocked) and closed before approval, and one under a protect memory added before approval: when the job's terminal update arrives the parent already has `follow_up_id`. The follow-up has `parent_id`, status `pending`, kind `delete`, conversation `main`, and the parent's summary, and holds four items. The grown file is ticked with a larger `alloc` and its verdict and reason; the unticked one is unticked; the still-open one is `blocked` and unticked with an `Open in` reason; the formerly blocked one is `pending` and unticked. The done, deleted, and protected files are not items; the protected one is in `dropped` with the memory text.
- A job where every item is done, and one whose only item not done was deleted, leave `follow_up_id` null and no child row. A delete dedupe whose second copy fails leaves none either.
- An archive whose copy fails verification gets a follow-up of kind `archive` with the same destination and the item ticked. An archive cancelled during its first copy ends `cancelled` with the second item ticked and `pending` in the follow-up. An `lzx` compress that fails up front gets a follow-up of kind `compress`, format `lzx`, item ticked.
- Through the server: a two-item delete with one approved streams its terminal `job` frame after `GET /api/proposals/{id}` already shows `follow_up_id`; the follow-up's `parent_id` names the parent, it holds the unapproved item unticked with its memory note, and `GET /api/thread` shows both links.

### ACT-RESTORE: Restore puts back the original path and content
**Applies to:** hoardini
**Verification:** test

Restoring an archive entry from a local destination shall recreate the original path with identical content (and the
file's original mtime), remove the stub, set `restored_ms`, and keep the archived copy. Restore shall refuse when
something exists at the original path, and shall fail without leaving anything at the original path when the archived
copy no longer matches its recorded BLAKE3.

**Acceptance criteria:**
- After archiving `docs\report.pdf` (stub created) and a folder, restoring the file brings back identical bytes and mtime, removes the stub, keeps the copy, and sets `restored_ms`; restoring the folder brings back the same tree.
- A second restore of the same entry is refused with "already exists".
- Restoring an entry whose archived copy was altered fails with a BLAKE3 error, leaves the original path absent, and leaves `restored_ms` unset.

### ACT-DELETE-CONFIRM: Every delete needs hold-to-confirm
**Applies to:** hoardini
**Verification:** manual

A delete proposal card, and a dedupe card whose mode is `delete`, shall say the delete is permanent and cannot be undone,
and shall approve only after the delete button is held for the full hold time (`HoldButton` in
`ui/src/components/ProposalCard.tsx`, 1.2 s). A click, a short press, or releasing early shall not approve. Archive
cards, compress cards, and hardlink dedupe cards approve with one click.

**Verification (manual):**
1. Start the app, ask the agent to propose deleting a scratch file you created (e.g. `D:\scratch\junk.bin`).
2. The card shows the item, its size, verdict badge, and the text "Deleting is permanent and cannot be undone. Nothing goes to the Recycle Bin."
3. Click "Hold to delete" once. The fill bar starts and resets; the card stays pending; the file still exists.
4. Press and release after about half a second. Same result.
5. With keyboard focus on the button, press and release Space quickly. Same result.
6. Press and hold until the bar fills (about 1.2 s). The card switches to approved, a job appears, and the file is gone.
7. Uncheck every item on another delete card: the hold button is disabled.
8. Ask for an archive proposal to a registered destination: its approve button works with a single click. The card shows the job's progress bar and item count, then "Done. Freed …", and the header's "made to vanish" grows.
9. Settings → Destinations → Archived items lists the archived folder; Restore puts it back (the row turns "restored") and the folder reappears in the index.
10. Ask for a dedupe proposal in `delete` mode on two scratch copies: the card shows the permanent-delete warning and "Hold to delete"; a click does nothing, a full hold approves. A `hardlink` dedupe card still approves with one click ("Remove duplicates").
(2026-10-08, sandbox folder, scripted mock model: 1–6 and 8–9 as described; 7 seen on a card whose only item was blocked, which disables the button the same way.)

### ACT-COMPRESS-ESTIMATE: Compression estimates track the data
**Applies to:** hoardini
**Verification:** test

`compress::estimate(path)` shall sample the file (or the largest files of a folder) through the atime-preserving
handle and report, per format (`zip`, `7z`, `tar.zst`, `lzx`, `lznt1`), the compressed/original ratio, expected result
size, savings, and time. All figures are estimates. Data identified as already compressed (media, archives, quantized
weights) shall report ratio 1.0 without being compressed.

**Acceptance criteria:**
- A 4 MiB pseudo-random file: every format's ratio is within 0.97 to 1.0 and savings are under 150 KB.
- A 4 MiB file of zeros: every format's ratio is below 0.1 and savings exceed 3 MiB.
- A `.mp4`-named random file is flagged `already_compressed`, samples 0 bytes, and every format reports ratio 1.0 and 0 saved.

### ACT-COMPRESS-VERIFY: Originals go only after the archive tests clean
**Applies to:** hoardini
**Verification:** test

An archive compress shall delete originals only after the finished archive was re-read and every entry matched the
source's size and BLAKE3. A failed test shall leave the originals untouched, no archive at the final name, and no
partial file. An in-place compress shall leave content identical and reduce allocation on compressible data.

**Acceptance criteria:**
- For each of zip, 7z, tar.zst: a text file and a nested folder (with an empty subfolder) compress to `<name>.<ext>` beside them, both originals are gone, the job frees more than 0 bytes, and the zip's entries equal the folder's file bytes.
- When each finished archive is damaged before its test, the job fails, the file and folder are byte-identical to before, no archive or `.hoardini-partial` exists, and 0 bytes are freed.
- `lzx` and `lznt1` on a 4 MiB text file: content is identical, allocation after is under half of before, `freed_bytes` equals the difference, and the journaled undo is a `compact /u` command.
- `lzx` on a folder of text files: `freed_bytes` equals the files' allocation before minus after.
- `lzx` and `lznt1` on a folder holding a junction to another folder: the item fails naming the link, and neither the junction's target nor the folder's own files change allocation.

### ACT-DEDUPE-VERIFY: Duplicates are removed only after a fresh full-hash match
**Applies to:** hoardini
**Verification:** test

A dedupe shall act on an extra copy only if, at execution, its full BLAKE3 and size equal the kept copy's. The kept copy
shall never be modified or deleted, and a proposal that lists the kept copy for removal shall change nothing.
`hardlink` shall leave the extra's path readable with the kept content; `delete` shall remove only verified extras.

**Acceptance criteria:**
- Keep `a\keep.bin`, extras `b\twin.bin` and `c\lookalike.bin` (the latter changed after proposing, same size and mtime): in hardlink mode the twin has keep's `file_id` and bytes, the lookalike is `failed` ("differs"), unchanged, and not linked; keep is byte- and `file_id`-identical; freed bytes are at least the file size; the journaled undo says "break the link".
- In delete mode the twin is gone, the lookalike remains, keep is unchanged.
- A proposal listing keep among the items fails the job and leaves every file in place.

### ACT-RCLONE-VERIFY: An rclone archive deletes the source only after the remote copy verifies
**Applies to:** hoardini
**Verification:** test

For an rclone destination, every file's size shall match at the remote and so shall a hash the remote supports
(strongest of SHA-256, SHA-1, MD5, Dropbox content hash) against the same hash computed locally while the source is
read without touching its access time. When the remote offers none of those, or leaves one blank, rclone shall stream
the files back (`operations/hashsum`, `blake3`, `download`) and the BLAKE3 must equal the source's. The source is also
re-checked for size and mtime changes after verification. Any failure shall fail the item, keep the source, remove the
bad remote copy, and write no `archive_entry`. Restore copies back, checks BLAKE3 against the archived hash, and only
then renames into place.

**Acceptance criteria:**
- With a remote that lists SHA-256, a 3 MiB file and a nested folder are archived, both sources are removed, `archive_entry.dest_uri` starts with `rclone:fake:hoardini/`, the remote bytes equal the originals, `archive_entry.blake3` equals the file's BLAKE3, every copy was an `_async` call with a `hoardini-a...` group, and the listing asked for `sha256`.
- With a remote that lists no hashes, the same archive succeeds using `operations/hashsum` and no hashed listing.
- When the remote's copy of each file is altered, the file and the folder items fail with "verification failed" (with or without remote hashes), both sources are byte-identical to before, no `archive_entry` exists, the remote holds no files, and 0 bytes are freed.
- When the source file grows after it was hashed, the item fails with "changed while", the source stays, and no `archive_entry` exists.
- Restoring both entries brings back identical bytes and keeps the remote copy.
- A second archive of new content at the same path the same day lands at a different `dest_uri`; the first remote copy keeps its bytes and restores them.

### ACT-RCLONE-PROGRESS: Archive progress comes from the rclone job group, and cancel stops the job
**Applies to:** hoardini
**Verification:** test

While an rclone copy runs, the executor shall poll `job/status` and `core/stats` for the item's `_group` and add the
byte delta to the Job's progress. Cancelling the executor job shall call `job/stop` and leave the source in place.

**Acceptance criteria:**
- A slow fake job (8 polls) produces Job updates with `0 < done_bytes < total_bytes` before the job finishes, and every `core/stats` call names a `hoardini-a...` group.
- Cancelling after two status polls ends the job as `cancelled`, calls `job/stop` once, keeps the source, and writes no `archive_entry`.

### ACT-RCLONE-SECRETS: Cloud credentials live in the vault and reach rclone only when a remote is created
**Applies to:** hoardini
**Verification:** test

A destination config shall hold only `secret://` references for secret-looking fields. The values are read from the
vault when `config/create` is called and never stored in the destination row. A Google Drive destination without a
client ID and secret, or with a scope other than `drive.file`, shall be refused.

**Acceptance criteria:**
- Creating an S3 destination with a plaintext `secret_access_key` fails mentioning `secret://`.
- With vault refs for `secret_access_key` and `access_key_id`, the stored config contains the refs but neither value; testing the destination makes exactly one `config/create` whose parameters carry the vault values and the plain `region`.
- A `drive` destination without client credentials is refused with a message naming the OAuth client.

### ACT-RCLONE-TEST: Testing an rclone destination reports free space or probes with a write
**Applies to:** hoardini
**Verification:** test

`destinations::test` for rclone shall create the remote if needed, report `operations/about` free space when the
backend supports it, and always prove writability by copying a probe file and deleting it. Bad rc credentials fail.

**Acceptance criteria:**
- With `about` available the result is ok with that free space.
- With `about` unsupported the result is ok with no free space, the probe is copied then deleted, and the remote holds no files afterwards.
- Wrong rc credentials give a failed result and the fake sees the rejected request.

### ACT-RCLONE-DAEMON: The rclone daemon is loopback-only, authenticated, and launched only from a verified binary
**Applies to:** hoardini
**Verification:** test

The daemon command line shall bind `127.0.0.1:<port>`, carry a random `--rc-user` and `--rc-pass` (different on every
start), name the config file, and never include `--rc-no-auth`. A binary that fails `verify_before_run` shall not be
launched, and nothing (no config, no vault secret) is created for it.

**Acceptance criteria:**
- Two generated endpoints differ in user and password; the arguments hold `--rc-addr 127.0.0.1:40123`, the user, the password, `--config`, and no `--rc-no-auth`; the endpoint's Debug output omits the password.
- A garbage `rclone.exe` in the runtime folder makes `RcloneDaemon::client` fail with an install-record or SHA-256 error, leaves the daemon not running, and creates neither `rclone.conf` nor the config-password secret.

### ACT-ARCHIVE-RCLONE: Archive and restore through a real rclone
**Applies to:** hoardini
**Verification:** manual

The pinned rclone downloads, runs, and carries an archive and a restore end to end. Last run (2026-10-08, rclone
1.75.1, `local` backend): download of 31,488,864 bytes and its SHA-256 check passed, destination test ok, a 3 MiB file
and a folder archived with SHA-256 verification and again with the read-back BLAKE3 path, sources removed, both
restored byte-identical, no `rclone.exe` left running.

**Verification (manual):**
1. Use an empty scratch data dir: `$env:HOARDINI_DATA_DIR = "D:\scratch\hoardini-data"`; start the app or `hoardini serve`.
2. Open Destinations and add a Cloud destination. The runtime panel shows rclone 1.75.1 (about 30 MB) as not installed. Click Install; the bar fills, then the panel says installed (`GET /api/runtime/rclone` returns `installed: true`).
3. Save a destination with remote `loc`, backend type `local`, folder `D:/scratch/remote-store` (via the API: `POST /api/destinations` with `{"name":"loc","kind":"rclone","config":{"remote":"loc","type":"local","path":"D:/scratch/remote-store"}}`). Press Test: it reports reachable with free space.
4. Create `D:\scratch\src\a.bin` (3 MiB) and a folder `D:\scratch\proj` with a nested subfolder. Ask the agent to archive both to the destination and approve.
5. Both sources disappear (stubs may appear), the job reaches done, and `D:\scratch\remote-store\<yyyy-mm-dd>\D\scratch\...` holds the copies. The archive list shows `rclone:loc:...` addresses.
6. Restore each entry from the archive list: the originals come back byte-identical (`Get-FileHash`), the remote copies stay.
7. While rclone runs, `tasklist | findstr rclone` shows one process; ten idle minutes after the last job, or after quitting the app, none. `D:\scratch\hoardini-data\rclone.conf` starts with `# Encrypted rclone configuration File`.
8. Cloud accounts (Google Drive, Dropbox, OneDrive, S3, B2, R2, SFTP) are not covered by this procedure; for each, Authorize (OAuth types) or enter keys, Test, archive a small file, restore it.

### ACT-ONLINE-ONLY: Online-only frees the local copy of synced files and never deletes
**Applies to:** hoardini
**Verification:** test

An `online_only` proposal (from `propose_online_only` or the review) shall block an item that is not under a registered
sync root ("Not in a cloud-synced folder ...") or that has nothing stored on this PC ("Already online-only"), plus the
usual in-use block. An item's verdict comes from the sync check run when the card is made (EVID-COPY-SYNC): `synced`
when it, or every file in the folder, is in sync, else `review`; the model's verdict is ignored, and only unblocked,
not-small `synced` items start ticked (POLICY-DEFAULT-TICK). At execution each file under an approved item is asked
again: a file the sync engine marks in sync and that still has local data is freed; one that is not in sync, or is no
placeholder, is skipped and counted; one already online-only is left alone. Nothing is ever deleted, moved, or read. A
file item that frees nothing fails ("nothing here is in sync with <provider> yet; nothing was changed"); a folder item
is done when it freed or found at least one in-sync file, its detail saying how many were freed and how many were left on
this PC. Each item is journaled per ACT-JOURNAL with undo `{"kind": "pin", "provider", "how"}`, `freed_bytes` is what
the dehydration released, and the index row is rescanned, not removed (INDEX-ACTION-PATCH). The Win32 calls
(`hoardini_win::free_up_space`, `placeholder_info`) sit behind the `Cloud` trait, so the selection and skip logic is
tested against a fake sync root.

**Acceptance criteria (fake sync root `<work>\OneDrive`, `specs/act/tests/online.rs`):**
- Proposed together: a file outside the root is `blocked` "Not in a cloud-synced folder"; an in-sync file is `synced`, ticked, with copy note "synced to OneDrive"; a file not in sync and a folder holding one in-sync and one not-in-sync file are `review` and unticked.
- Approving the in-sync file, the not-in-sync file, and the folder: the fake is asked to free exactly the two in-sync files; the in-sync file is `done` "1 file(s) now online-only in OneDrive"; the not-in-sync file is `failed` and untouched; the folder is `done` "1 file(s) now online-only in OneDrive; 1 not in sync were left on this PC"; the proposal is `partial`.
- Every file, including the freed ones, still exists with its bytes; the job's freed bytes equal the two freed files' sizes; the two done items have `done` action rows with undo kind `pin` and provider OneDrive, the failed one a `failed` row; the freed files' index rows are still there.

**Verification (manual, live sync engine):**
1. With a scratch sync provider or a scratch folder inside OneDrive that holds only files you created for the test, wait until Explorer shows them as synced (green check).
2. Ask for the scratch folder to be made online-only (`propose_online_only`), approve the card, and wait for the job.
3. Explorer shows the files as online-only (cloud icon); `Get-Item` reports the same lengths, `fsutil file layout` or Properties shows "Size on disk" near 0, and opening one downloads it again with identical bytes (`Get-FileHash` before and after).
4. A file modified just before approving (not yet synced) is reported as left on this PC.
