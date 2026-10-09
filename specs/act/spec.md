# Actions, destinations, journal (prefix ACT)

The executor runs approved proposals as background jobs. Canonical design: `.claude/plans/hoardini-design.md` §5.6.
Code: `crates/core/src/{actions,destinations,compress}`. Tests: `specs/act/tests/act.rs` and its modules.

## Model

`Executor::approve(proposal_id, item_ids)` is the only call that changes the filesystem. It accepts a `pending`
proposal and a subset of its selectable item ids, revalidates them (POLICY-REVALIDATE), marks the proposal `approved`,
starts a job on its own thread, and returns `{approved, items, job_id}`. Jobs run one at a time (`queued` → `running` →
`done` | `failed` | `cancelled`); progress (bytes, items, current path) goes to `Executor::subscribe()` and the `job`
table; cancel takes effect between files and between 1 MiB copy chunks. Kinds: delete, archive (local folder,
SMB UNC path), compress, dedupe, system_cleanup (operations Windows' own tools carry out; ACT-WINDOWS-CLEANUP).

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
  One card may hold several files or folders (`propose_compress` takes `paths`); each item runs and is journaled on its own.
- **Dedupe**: items are the extra copies; `keep` (from the proposal) is never listed, opened for write, or changed.
  `hardlink` (same volume only) hashes keep and extra in full at execution, creates the link under a temp name beside
  the extra, then renames it over the extra; `delete` removes the extra. Candidates come from `core::dupes`: same
  logical size, distinct `file_id` (hard links are one file), equal head+tail sample, then equal full BLAKE3, files of
  at least 100 MB by default, never cloud placeholders. Full hashes are reused while size, mtime, and `file_id` hold.
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

### ACT-APPS-RARELY-USED: Big apps not run for 90 days are listed
**Applies to:** hoardini
**Verification:** test

`apps::installed(inputs)` shall turn the uninstall registry and Steam's app manifests into apps with a stable id:
`steam-<appid>` for a Steam game (its folder `<library>\steamapps\common\<installdir>`, uninstalled through
`steam://uninstall/<appid>`), and `reg-<16 hex of BLAKE3 of the lower-cased uninstall key>` for a registry program,
uninstalled by its own `UninstallString` split into program and arguments (`apps::split_command`: a quoted program, else
the text up to the first `.exe` followed by a space or the end, else the first word). A registry program is left out
when it is hidden (`SystemComponent = 1` or an update with `ParentKeyName`), has no `InstallLocation` or
`UninstallString`, has an install location too broad to mean one app (a drive, `Program Files`, `Windows`, `Users`, ...;
`evidence::owner`), or shares its folder with a Steam game or an earlier program. Steam libraries come from
`libraryfolders.vdf` (current and old layouts) under the Steam folder in `HKCU\Software\Valve\Steam\SteamPath`, and
games from each library's `appmanifest_*.acf` (`appid`, `name`, `installdir`, `LastPlayed` in seconds, 0 = never).

`apps::rarely_used(conn, inputs, now)` shall size each app from the index (the allocated size of its folder, minus the
folders of other apps installed inside it, so a launcher is not charged for its games), take its last run as the latest
of Steam's `LastPlayed` and every UserAssist entry that is a run of it, and list, biggest first, the apps of at least
2 GiB (`MIN_APP_BYTES`) whose last run is more than 90 days ago (`IDLE_DAYS`) or unknown, each with `id`, `name`,
`publisher`, `source` (`registry` or `steam`), `path` (as the index spells it), `size`, `last_run_ms`, and
`last_run_from` (`last run of <entry>` or `Steam LastPlayed`). Launches from the taskbar or Start are recorded as a
shortcut or an app id rather than the exe path, so a UserAssist entry is a run of an app when it is an exe in the app's
folder; or its last name (without `.lnk` or `.exe`, letters and digits only, lower case) is at least 5 characters and
is contained in the app's display name squashed the same way, or contains it (`Microsoft.VisualStudioCode` for
"Microsoft Visual Studio Code (User)"); or it is an app id (no backslash) that names one of the `.exe` files the index
holds in the app's folder (`Microsoft.Office.WINWORD.EXE.15`). A path entry outside the folder never counts by its exe
name. Access times are not used: on a measured PC (2026-10-09) a scanner had touched every exe in the Visual Studio
folder within the same minute. An app whose folder is not in the index is not listed. Tests: `specs/act/tests/apps.rs`.

**Acceptance criteria:**
- With fake registry, UserAssist, and Steam inputs over a fake `D:\`: Old Game (Steam, 5 GiB, played 100 days ago), Never Opened (4 GiB, no run; a `setup.exe` in its folder and a run of `C:\Users\me\Downloads\setup.exe` yesterday do not count), Big Tool (3 GiB, run 200 days ago), and Unplayed (Steam, 2.5 GiB, never) are listed in that order, and nothing else: not a 3 GiB app run 10 days ago, a 1 GiB app, a hidden one, one without an uninstaller, one at `D:\Program Files`, one not in the index, the Steam client (1 GiB once its three games are taken out, though run 300 days ago), New Game (UserAssist 200 days ago, Steam 3 days ago), "Microsoft Visual Studio Code (User)" (app id `Microsoft.VisualStudioCode` 2 days ago), an app holding `WINWORD.EXE` (app id `Microsoft.Office.WINWORD.EXE.15` 12 days ago), or "Shortcut App" (`{Programs}\Shortcut App.lnk` 20 days ago). The registry entry "Steam App 1001" for Old Game's folder is not listed separately; Old Game's id is `steam-1001`, a registry id is `reg-` plus 16 hex digits, and a second call gives the same ids.
- Old Game uninstalls through `steam://uninstall/1001`; Never Opened through `D:\Apps\Never\Uninstall Never.exe` with `/x`.
- `split_command`: a quoted program keeps its arguments apart; an unquoted `C:\Program Files\My App\Uninstall My App.exe --remove now` splits after `.exe`; `MsiExec.exe /X{...}` splits after `MsiExec.exe`; `C:\Tools\app.exe.old\run.exe` stays whole; blank text and an unterminated quote give nothing.
- `libraryfolders.vdf` in the current layout gives both library paths with single backslashes; the old layout gives its numbered path; an `appmanifest` gives the app id, name, folder under the library, and `LastPlayed` in ms (0 = none); one without `installdir` gives nothing.

### ACT-UNINSTALL: Uninstall runs the app's own uninstaller and never deletes its folder
**Applies to:** hoardini
**Verification:** manual

`POST /api/apps/{id}/uninstall` (SRV-APPS) shall look the id up among the installed apps (ACT-APPS-RARELY-USED) and start
that app's registered uninstaller through the shell (`ShellExecuteExW`, so an uninstaller that needs admin rights shows
its own UAC prompt), or `steam://uninstall/<appid>` for a Steam game, and return at once. The uninstaller asks for its
own confirmation. Hoardini never deletes an app's folder or files itself, and the request body is never read.

**Verification (manual):**
1. Install a small throwaway app that registers an uninstaller (e.g. an Inno Setup or NSIS installer) into `D:\scratch\app`, then pad its folder past 2 GiB: `fsutil file createnew D:\scratch\app\pad.bin 2147483648`.
2. With `D:\scratch` indexed (`hoardini index --volume D:\` or a watched folder) and the app never started, `GET /api/apps/rarely-used` lists it with `last_run_ms: null`.
3. `POST /api/apps/<its id>/uninstall`: the app's own uninstaller window opens (with a UAC prompt first if it needs admin). Cancel it: the folder and `pad.bin` are unchanged.
4. Repeat and confirm: the uninstaller removes what it installed; `pad.bin`, which it did not install, is still there, so Hoardini removed nothing itself.
5. For a Steam game listed by the same route, the POST opens Steam's own "Uninstall <game>?" dialog.
6. `POST /api/apps/reg-0000000000000000/uninstall` answers 404 and opens nothing.
(2026-10-09, partial: `hoardini serve --no-refresh` on a copy of this PC's index, real registry, UserAssist and Steam: `GET /api/apps/rarely-used` listed Visual Studio Community 2022, HIP SDK Core, Visual Studio Build Tools 2022 and LLVM, each with no recorded run; step 6 answered 404. Steps 1 and 3–5 were not run: they install and uninstall software.)

### ACT-WINDOWS-CLEANUP-DETECT: Cleanups only Windows can do are detected without elevation
**Applies to:** hoardini
**Verification:** test

`syscleanup::candidates(conn, facts)` shall offer, in this order, each operation the index and `SystemFacts` (read
without elevation by `SystemFacts::gather`: the system drive, `SystemRoot`, physical RAM,
`HKLM\SYSTEM\CurrentControlSet\Control\Power\HibernateEnabled`, the Recycle Bin size across drives, the WSL 2 distros
in `HKCU\...\Lxss` with their `BasePath\VhdFileName`, and `%LOCALAPPDATA%\Docker\wsl`) show to be worthwhile:
- `component_store` when the index holds `<SystemRoot>\WinSxS` at 2 GiB or more (the bytes not hard-linked into Windows
  itself, INDEX-HARDLINK-ONCE); estimate unknown, size = that folder.
- `restore_points` for the system drive when its latest refresh's unaccounted gap (INDEX-UNACCOUNTED) is 2 GiB or more;
  target `System Volume Information`, estimate unknown, size = the gap.
- `hibernation` when hibernation is not known to be off and the index holds `hiberfil.sys` at least 1 GiB larger than a
  fifth of RAM (the reduced size); estimate = the difference.
- `compact_vhdx` for each WSL distro disk and each `.vhdx` under Docker's `wsl` folder that the index holds at 5 GiB or
  more, once per path; estimate unknown, size = the file.
- `recycle_bin` when the Recycle Bin holds 1 GiB or more; estimate = its size.
Each candidate has a target path, an optional estimate, a size, and a one-line note. `proposals::create_system_cleanup`
stores them as a pending `system_cleanup` proposal whose items are the operations: the path is the target, `alloc` the
estimate (0 when unknown), the verdict `safe` (component store, hibernation, compaction) or `review` (restore points,
Recycle Bin), so ticks follow POLICY-DEFAULT-TICK with nothing counted small, and each item's JSON carries `op`
(`{op, volume?|path?, label, explain, elevated, irreversible, estimate_known, size}`). A `compact_vhdx` whose path is
not an existing `.vhdx` file, or a `restore_points` whose volume is not a drive letter, goes to `dropped`; a request
with nothing left fails. `syscleanup::propose(db, facts, wanted, ...)` keeps the wanted operation names (all when
empty) and lists a wanted one that was not detected in `dropped`. A `system_cleanup` request through
`proposals::create` (paths) is refused. Tests: `specs/act/tests/system.rs`.

**Acceptance criteria:**
- On a fake `C:\` with a 6 GiB WinSxS, a 25 GiB `hiberfil.sys`, a 60 GiB Ubuntu disk, a 40 GiB `docker_data.vhdx`, a 1 GiB Docker `ext4.vhdx`, and over 2 GiB unaccounted, with 64 GiB RAM, hibernation on, and 3 GiB in 12 Recycle Bin items: the candidates are, in order, component store (size 6 GiB), restore points on `C:` (size = the gap), hibernation (estimate 25 GiB − 12.8 GiB), the Ubuntu disk, the Docker data disk, and the Recycle Bin (3 GiB, note starting "12 items").
- With a 1 GiB WinSxS, a gap under 2 GiB, 128 GiB RAM, a 500 MiB Recycle Bin, and no WSL or Docker disks there are none, also with 16 GiB RAM and hibernation off; with hibernation unknown, the 25 GiB file is offered at 25 − 3.2 GiB. `propose` then fails with "nothing Windows can clean up".
- Proposing restore points, hibernation, Recycle Bin, and compaction gives three items in that order with ticks off, on, off, the irreversible flag on restore points and the Recycle Bin, `elevated` off only for the Recycle Bin, `estimate_known` off only for restore points, `small` off, `total_alloc` = the two estimates, and both vhdx files (absent on disk) in `dropped`. The JSON has `kind: "system_cleanup"` and `op: {op: "restore_points", volume: "C:", explain: "...cannot be brought back."}`.

### ACT-WINDOWS-CLEANUP-RUN: One administrator prompt per job, each operation journaled
**Applies to:** hoardini
**Verification:** test

Approving a `system_cleanup` proposal (no file revalidation: the items are operations) starts a job that writes a
`running` action row for every approved operation first, then runs the ones that belong to the user (the Recycle Bin,
`SHEmptyRecycleBinW` with no confirmation; an empty bin is reported done with nothing freed) in-process, then hands every other operation to
one `SystemRunner::elevated` call: `WindowsRunner` runs `wsl --shutdown` as the user when a disk is to be compacted,
then relaunches the current exe once with the `runas` verb (one UAC prompt) as
`system-cleanup --result "<temp file>" "<op>"...`, where each op is `component-store`, `restore-points=<X:>`,
`hibernation-reduced`, or `compact-vhdx=<absolute local .vhdx path>` (`syscleanup::helper_params`); the elevated side
(`syscleanup::run_helper`, wired into both the CLI and the app's `main`) accepts nothing else
(`syscleanup::parse_helper_args`), runs each operation, and appends one JSON result per operation (`ok`, `freed` = the
volume's free space after minus before, `error`, `detail`) that the job reads as they arrive. A declined prompt fails
every elevated item with "The administrator prompt was declined; nothing ran". Each item ends `done` with its action
row's `freed_bytes` and `undo_json` written first (ACT-JOURNAL; hibernation's undo is `powercfg /h /type full`, the
others say there is no undo), or `failed` with the error; the proposal ends `done`, `partial`, or `failed` as other
jobs do, and gets no follow-up. Cancel takes effect before the prompt; once the elevated helper runs, the Windows tools
run to their end. Restore points are counted from `vssadmin list shadows` by their
`\Device\HarddiskVolumeShadowCopyN` device paths, which are not localized (`syscleanup::count_shadows`).
`Executor::set_system_runner` replaces `WindowsRunner` in tests. Tests: `specs/act/tests/system.rs`.

**Acceptance criteria:**
- With a fake runner, approving all five operations calls the user side once with the Recycle Bin and the elevated side exactly once with component store, restore points on `C:`, hibernation, and the vhdx, in that order; the results (3, 5, 12 GiB, and a diskpart failure) leave four items `done` (with their details) and the vhdx `failed` with the error; the proposal is `partial`, the job `done` with 21 GiB freed, five action rows carry 3, 5, 12, 0, and 1 GiB, the restore-point undo says they cannot be brought back, hibernation's undo is `["powercfg.exe","/h","/type","full"]`, the failed row has none, and there is no follow-up. At every job event, every `done` item already has a `done` row with undo info.
- A declined prompt fails the four elevated items with the declined message (four failed action rows), the Recycle Bin still runs, and the job frees 1 GiB.
- Approving only the Recycle Bin never calls the elevated side.
- The helper command line for four operations is exactly `system-cleanup --result "<file>" "component-store" "restore-points=D:" "hibernation-reduced" "compact-vhdx=C:\wsl\Ubuntu Disk\ext4.vhdx"` and parses back to the same operations; the Recycle Bin cannot be put on it; `recycle-bin`, `format-c`, `restore-points=C:\`, a non-`.vhdx` path, a relative path, a UNC path, and a relative result file are refused.
- A `vssadmin` listing with two shadow device paths (one under a German label) counts 2; "No items found" counts 0.

### ACT-WINDOWS-CLEANUP: Windows' own tools free the space, behind one admin prompt
**Applies to:** hoardini
**Verification:** manual

Each approved operation runs the Windows tool that owns it: component store, `Dism.exe /Online /Cleanup-Image
/StartComponentCleanup /Quiet /NoRestart` (exit 3010 = done, restart pending); restore points, every shadow copy of the
drive except the newest via `vssadmin delete shadows /for=<X:> /oldest /quiet`, repeated (deleting old points is chosen
over capping shadow storage, because a cap can only be set blind without elevation and may grow the allowance instead);
hibernation, `powercfg /h /type reduced` (reduced rather than off: Fast Startup keeps working, the Hibernate option goes
away, and `powercfg /h /type full` undoes it); WSL and Docker disks, `wsl --shutdown` as the user, then `diskpart`
(`select vdisk`, `attach vdisk readonly`, `compact vdisk`, and always a separate `detach vdisk`), which needs no Hyper-V
module, chosen over `wsl --manage --set-sparse`, which WSL itself flags as experimental and which only frees space as
Linux trims it later; the Recycle Bin, `SHEmptyRecycleBinW` as the user. Child tools run without a console window. The
card (UI-SYSTEM-CLEANUP-CARD) says what each operation does and, on the approve step, that deleted restore points
cannot be brought back. The freed bytes reported are the volume's free space after minus before.

**Verification (manual):**
1. On a test PC (or VM) with an index of `C:` and some restore points, a WSL distro, and a few GB in the Recycle Bin, ask "what can Windows clean up?"; the model calls `propose_system_cleanup` and the card lists the detected operations with their notes.
2. Tick everything and hold to clean up: exactly one UAC prompt appears (for `hoardini.exe system-cleanup ...`). Decline it: every operation except the Recycle Bin fails with "The administrator prompt was declined; nothing ran", and the Recycle Bin is empty.
3. Ask again, approve and accept the prompt: WSL stops (`wsl -l -v` shows every distro Stopped), DISM runs (Task Manager), and the job ends with each item done or failed with the tool's message.
4. `vssadmin list shadows /for=C:` (as admin) shows one shadow copy left; `powercfg /a` shows hibernation unavailable with Fast Startup available and `hiberfil.sys` at about a fifth of RAM; the vhdx file is smaller and the distro starts with its files intact.
5. The collapsed card reads "Cleaned up N items · freed X"; X matches the drop in used space (`fsutil volume diskfree C:` before and after, within what else changed).
6. `SELECT kind, status, freed_bytes, undo_json, error FROM action WHERE kind = 'system_cleanup'` shows one row per operation with its undo info.
(2026-10-09, partial: none of steps 1–6 were run on this PC, which must not be cleaned up. Run instead: `hoardini system-cleanup --result <scratch file>` with no operations exited 0 and wrote an empty file; with `format-c`, `recycle-bin`, or a relative result file it exited 2 and ran nothing. Detection on a copy of this PC's index (read-only facts) offered component store (19.2 GB WinSxS), restore points (91.2 GB unaccounted), hibernation (35.1 GB file, about 17.5 GB to free with 87.6 GB RAM), and three WSL/Docker disks of 7.5, 121, and 6.1 GB; the Recycle Bin held 40 KB and was not offered.)

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

When a delete, archive, or compress job ends (done, partial, failed, or cancelled), the executor shall create a pending
follow-up proposal whose `parent_id` is the job's proposal, in the same conversation, holding every item whose status is
not `done`. Each item starts with the tick it was approved with: an approved item that failed or was skipped comes back
ticked; an item left unticked comes back unticked. The items go through proposal creation again, so path resolution,
the deny list, `alloc`, blocks, caps, in-use status, and the memory note are current: a newly blocked item is unticked,
and a newly denied path is listed in `dropped`. An item whose path no longer exists is left out. Kind, destination,
format, summary, verdicts, and reasons carry over; the follow-up has no `tool_call_id`. Nothing is created when no item
is left, and dedupe and system_cleanup jobs get no follow-up. The follow-up row is committed before the job's terminal update is broadcast
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

### ACT-COMPRESS-CANDIDATES: Folders worth compressing in place are found from the index
**Applies to:** hoardini
**Verification:** test

`compress::candidates(conn, env, root, now)` shall list folders under `root` of at least 2 GiB allocated (`MIN_FOLDER`),
biggest first, that compressing in place can help, using only the index: walking down from `root` through folders of
at least 2 GiB, a folder with two or more such children, or one child holding at least 90% of it, is opened instead of
listed; a deny-listed folder (POLICY-DENY) is opened too; a folder whose catalog category gets a delete card instead
(cache, browser, package or installer cache, build output, temp, crash dumps, Windows leftovers, system, VM disk,
downloads) is skipped with everything under it. Any other folder is listed unless one of these holds, in which case its
children are tried instead: something in it (a folder or a file row) was modified in the last 30 days (`QUIET_DAYS`;
writes undo WOF compression); it holds a link or a cloud-only file; the folder has the NTFS compressed attribute; half or
more of its file-row bytes are already NTFS-compressed or allocated under 90% of their size (WOF hides its tag from
listings); half or more of its bytes (INDEX-EXT-MIX) are media, archive, package or quantized-weight types; or it holds a
virtual disk (`vhdx`, `vhd`, `avhdx`, `vmdk`, `vdi`, `qcow2`), which WSL and Hyper-V refuse to run compressed. At most
400 folders are examined per call.

Each candidate carries the cached `lzx` estimate (`compress_estimate` table, schema v6) while its size and newest
modification time are unchanged. `compress::estimate_top(db, candidates, limit)` runs `compress::estimate`
(ACT-COMPRESS-ESTIMATE) for the first `limit` candidates without a current estimate and stores each; the review runs it
for the top 10 (`ESTIMATE_TOP`) on a background thread (`compress::spawn_estimates`). A candidate is proposable when
the estimate saves at least 1 GiB and at least 20% of its size (`compress::proposable(saved, logical)`); the card is an
in-place `lzx` compress of those folders. Tests: `specs/act/tests/candidates.rs`.

**Acceptance criteria:**
- On a fake `C:\` (now = a year after the files were written): `steamapps\common\GameB` (4.1 GiB), `C:\data\corpus` (3.5 GiB), `C:\Users\me\Datasets\set`, `steamapps\common\GameA` (3 GiB each), and `C:\proj\old` (2.5 GiB) are listed in that order, and nothing else: not a folder with a file touched 5 days ago, not one allocated at 5 of 8 GiB, not one with the NTFS compressed attribute, not one holding a junction, not one of `.mp4` files, not a 1 GiB folder, not the folder holding `disk.vhdx`, not `C:\Windows\big`, not a folder under `%TEMP%`, and not `C:\games`, `steamapps`, `common`, `C:\data`, or `C:\proj` (whose newest file is a day old) themselves.
- A stored estimate for `C:\data\corpus` at its size and newest time rides along (2 GiB saved: proposable); one stored at a different size does not.
- `proposable`: 1 GiB of 5 GiB, and 2 GiB of 10 GiB, are; 1 GiB − 1 byte of 4 GiB, 1.5 GiB of 8 GiB, and 2 GiB − 1 byte of 10 GiB are not.
- `estimate_top` with limit 1 over two 4 MiB text folders estimates only the first (saved over 2 MiB, ratio under 0.5); a second call estimates nothing; limit 2 estimates the second; a changed newest time estimates the first again.

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
- One `lzx` card holding two folders: both items end `done`, each folder's files shrink below half their allocation with content unchanged, and each item's action row carries its own folder's saving.
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
