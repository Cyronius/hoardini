# INDEX — persistent disk index

Canonical spec for the indexer (`hoardini-core::index`, `hoardini index`). Design: `.claude/plans/hoardini-design.md` §5.1.

Tests: `specs/index/tests/index.rs` and its modules `journal.rs` and `compact.rs` (`cargo test -p hoardini-specs --test index`). Most run
against `index::FakeFs`, an in-memory volume with a fake change journal; the `real_*` tests run the real lister
(`index::WinLister`) and, for the journal, the real C: change journal (`index::WinJournal`) on a temp directory.

## Terms

- **Row**: a `node` table entry. Every directory and every link has one; a file has one when
  `max(allocated, logical) >= threshold`. A directory row keeps its modification time only (INDEX-COMPACT).
- **Counted size** of a file: its allocated bytes, or 0 when it is a hard-link duplicate (see INDEX-HARDLINK-ONCE).
- **Subtree totals** of a directory (`sub_alloc`, `sub_logical`, `sub_count`): its folded small files plus the counted
  size of every child file row plus the subtree totals of every child directory. Links count 0.
- **Refresh**: one full walk of a volume (`index::refresh`). A first build is a refresh of an empty index.
- **Journal refresh**: an update driven by the volume's NTFS change journal (`index::journal_refresh`), read without
  admin rights; it re-lists only the directories the journal names.

## Requirements

### INDEX-ALLOC-SIZE: Allocated-size accounting
**Applies to:** hoardini
**Verification:** test

Sizes shall be allocated bytes. A file whose listing reports 0 allocated bytes, a logical size above 0, no sparse
attribute, and no cloud-placeholder attribute (the measured signature of a WOF-compressed file) shall be sized by one
open that reads its true allocation (`alloc_size_by_open`); when that open fails the logical size is used. A sparse
file keeps its listed allocation. A cloud placeholder counts its listed allocation (0 when cloud-only) and is never
opened. Logical size is kept for display.

**Acceptance criteria:**
- File listed alloc 8 MiB / logical 5 MiB → row size 8 MiB, logical 5 MiB.
- WOF file listed alloc 0 / logical 4 MiB, true allocation 20 KiB → size 20 KiB, logical 4 MiB.
- Sparse file listed alloc 1 MiB / logical 10 GiB → size 1 MiB; sparse file listed alloc 0 → size 0, not opened.
- Cloud placeholder listed alloc 0 / logical 5 GiB → size 0, logical 5 GiB, `placeholder` flag set, never opened.
- Exactly the WOF file is opened; the parent's subtree size is the sum of the above plus folded small files.
- Real lister on a temp dir: a 2 MiB + 1 B file has logical 2 MiB + 1 B and allocated ≥ logical and < logical + 1 MiB;
  file counts and logical totals of the dir and the root are exact.

### INDEX-HARDLINK-ONCE: Hard links count once per volume
**Applies to:** hoardini
**Verification:** test

A file with several hard links shall count once per volume, keyed by its 128-bit file ID. Every link at or above the
threshold keeps a row; exactly one is counted and the others carry the `hardlink_dup` flag with counted size 0. The
counted link is the one with the smallest path (case-insensitive), independent of walk order or worker count. Links
below the threshold fold once, into the directory of the first link the walk meets. A file ID of all zeros means
"unknown" (the portable lister) and is never merged. On NTFS the dedupe set is a bitset over MFT record numbers;
other file systems use an exact hash set.

**Acceptance criteria:**
- One 200 MiB file with links `C:\b\model.gguf`, `C:\a\copy.gguf`, `C:\c\third.gguf`: the volume counts 200 MiB once,
  `C:\a\copy.gguf` is counted, the other two are flagged, `C:\a` = 200 MiB, `C:\b` = 0 — on every run with 4 workers,
  for both the NTFS (bitset) and the non-NTFS (hash set) path.
- A 100 KiB file linked from two directories adds 100 KiB in total to their small totals.
- Two files with all-zero IDs both count.
- Real lister: a 2 MiB file plus a hard link in another dir counts once when the lister reports file IDs, twice when
  it reports zero IDs.

### INDEX-NO-FOLLOW: Junctions and symlinks are never traversed
**Applies to:** hoardini
**Verification:** test

The indexer shall not list the contents of a junction or symbolic link. Any reparse point whose tag is a name
surrogate (junction, symlink, WSL symlink), and any reparse directory whose tag the lister cannot read (tag 0), is
recorded as a `link` row with size 0. A reparse directory with a non-surrogate tag (for example a cloud-files folder)
is an ordinary directory and is traversed.

**Acceptance criteria:**
- A junction, a directory symlink, and a tagless reparse directory, each "containing" a 5 GiB file, are never listed;
  each has a `link` row of size 0; no row exists beneath them; the root total excludes the 5 GiB files.
- A file symlink is a `link` row of size 0.
- A directory with tag `0x9000601A` (cloud files) is listed and its 2 MiB file counts.
- Real lister: a `mklink /J` junction inside a temp dir is a `link` row; its target's file is counted once, via the
  target only.

### INDEX-SMALL-FOLD: Small files fold into their parent
**Applies to:** hoardini
**Verification:** test

Files with `max(allocated, logical)` below the threshold shall get no row; their count, allocated bytes, and logical
bytes add to the parent directory's `small_count`, `small_bytes`, `small_logical`. For every directory, subtree totals
shall equal its small totals plus its counted children (`index::verify_totals` returns no problems).

**Acceptance criteria:**
- For a generated tree of 144 files around the 1 MiB threshold: every file ≥ threshold has a row with its allocated
  size, every smaller file has none, and each directory's `small_count`/`small_bytes` equal the sum of its small files.
- The root's allocated, logical, and file-count totals equal the sums over all files.
- `verify_totals` is empty.

### INDEX-THRESHOLD: File-row threshold is a fixed 1 MiB
**Applies to:** hoardini
**Verification:** test

The file-row threshold shall be a fixed 1 MiB (1,048,576 bytes), tuned by measurement (C: indexes to about 132 MiB).
There is no setting; a `meta.threshold_bytes` left by an older build is ignored. `hoardini index --threshold`
overrides it for one run, for testing. The threshold used by the last completed refresh is stored on the volume; a
journal refresh keeps it, and an `auto` refresh whose stored threshold differs runs full. A full refresh at any
threshold shall produce the same index as a fresh build at that threshold.

**Acceptance criteria:**
- With `meta.threshold_bytes` set to 64 KiB, the completed refresh records threshold 1,048,576; a file of exactly
  1 MiB has a row, one of 1 MiB − 1 does not.
- Refreshing with an override of 64 KiB, then 4 MiB, then none, each snapshot equals a fresh build at that
  threshold, totals verify, and the volume ends at 1,048,576.

### INDEX-REFRESH-EQUIV: A refresh equals a fresh build
**Applies to:** hoardini
**Verification:** test

After any mix of changes on the volume, a completed refresh of an existing index shall produce the same index as a
fresh build of the same tree: same rows (by path and kind), sizes, small totals, subtree totals, times, attributes,
reparse tags, file IDs, and flags (`index::snapshot`). Hotspot-first ordering and the worker count do not change the
result. Exception: for a hard-linked file below the threshold whose links sit in different directories, which
directory's small totals hold its bytes may differ (their common ancestors' totals do not).

**Acceptance criteria:**
- Starting from a sample tree, two rounds of changes — added, deleted, grown, shrunk, and renamed files and dirs; a
  file crossing the threshold both ways; a file replaced by a same-named dir; a dir replaced by a junction and a
  junction by a dir; a denied dir becoming readable and a new denied dir; a hard-link owner removed and a new link
  added; a WOF file's allocation changed; an mtime change — each refresh with 3 workers yields a snapshot equal to a
  fresh build. A further refresh with 1 worker does too.

### INDEX-JOURNAL-EQUIV: A journal refresh equals a full refresh
**Applies to:** hoardini
**Verification:** test

Every completed full refresh shall store the volume's change-journal ID and next USN, queried before its walk starts;
a full refresh clears them when it starts, so an unfinished one leaves none. A journal refresh (`index::journal_refresh`,
`hoardini index --kind journal`) shall read every record from the stored USN to the journal's current end and re-list
only: the parent of each changed entry, every directory holding a row with a changed file's ID (hard-link siblings), a
directory whose permissions changed, and the parent of each directory whose set of entries changed (so its own times
update). A renamed or moved directory that stays inside the indexed root keeps its rows and is re-parented, with its
subtree totals moved from the old ancestors to the new ones; a directory that is new to the index is walked. Hard-link
ownership is re-resolved, and the new next USN is stored. The result shall equal a fresh build of the same tree under
the rules and exceptions of INDEX-REFRESH-EQUIV.

The journal refresh shall change nothing and report "needs a full refresh" when no position is stored, the volume has
no journal, the journal ID changed (reset), or the stored USN is older than the journal's first USN (wrapped).
`hoardini index --kind auto` (the default) then runs a full refresh; it also runs one when the threshold setting differs
from the stored threshold, which only a full refresh can apply. Records whose parent is not indexed and resolves to a
path outside the indexed root are ignored. A directory re-listed only for changes to rows, subdirectories, or links
keeps its stored small-file fold, plus small files created since; otherwise its small files are recounted. An
interrupted journal refresh keeps subtree totals consistent and clears the stored position, so the next refresh is
full. A completed journal refresh records a `scan` row of kind `journal` with history summaries; journal refreshes
within a day of the first one after a full refresh fold into one row.

Exceptions, beyond INDEX-REFRESH-EQUIV's: a hard-linked file below the threshold whose other link sits in a directory
that is not re-listed counts again when its own directory's small files are recounted, until the next full refresh. On
a live volume, directory times and access times are not stable between two walks (listing updates a directory's access
time, and NTFS refreshes a directory's times in its parent's entry lazily), so the real-volume test masks them.

**Acceptance criteria:**
- Fake volume, journal read 7 records per page, parent times updated on every entry change: from the INDEX-REFRESH-EQUIV
  sample tree, its round one of changes plus a dir with a subtree moved under another existing dir, a dir moved into a
  new dir, two sibling dirs swapped, a new four-level subtree, and a new small file beside two same-dir links — one
  journal refresh yields a snapshot equal to a fresh build and totals verify; it read more than one page; the moved
  dirs and their children are not listed; an untouched dir is not listed; no dir is listed twice.
- Then round two of changes plus a dir mtime change, then denying and re-allowing a new dir: each journal refresh
  equals a fresh build; afterwards a journal refresh with no new records lists nothing, and the history holds one
  `journal` row after the `full` one.
- A 100 KiB file linked from `C:\c` and `C:\d`: after new big files in both dirs, a new subdir, and a rename, both dirs
  are re-listed and keep their small totals; a new 8 KiB file adds to `C:\d`; the root row equals a fresh build's.
- Journal reset: `NeedsFull(JournalChanged)` and the snapshot is unchanged; auto runs a full refresh equal to a fresh
  build. Journal wrapped: `--kind journal` reports `NeedsFull(Wrapped)` and indexes nothing; auto falls back and equals
  a fresh build; the next auto run is a journal refresh. No journal: `NeedsFull(NoJournal)`. A full refresh without a
  journal source: `NeedsFull(NoPosition)`. Threshold setting changed: a journal refresh still runs; auto falls back with
  `ThresholdChanged` and equals a fresh build.
- A journal refresh cancelled after 3 listings returns `completed = false` and no scan id, totals verify, the next
  journal refresh returns `NeedsFull(NoPosition)`, and auto then equals a fresh build.
- Real lister and the real C: journal on a temp dir: after a new big file, a small file grown past the threshold, a
  rename, a subdir moved to another parent, a deleted file and dir tree, a new subtree, and a new hard link, a journal
  refresh equals a fresh build of the temp dir (directory times and access times masked), and the moved subdir's child
  was not listed.

### INDEX-JOURNAL-UNPRIVILEGED: The unprivileged journal read sees what the index needs
**Applies to:** hoardini
**Verification:** manual

The journal refresh reads with `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL`, without admin rights. For a change to a file the
user cannot open, inside a directory the user can list, the read shall still return a record, so the journal refresh
re-lists that directory. If the read skips such records, the gap shall be recorded here; a full refresh corrects it.

**Verification (manual):**
1. Elevated prompt: `mkdir C:\jtest`, `fsutil file createnew C:\jtest\locked.bin 2000000`, then
   `icacls C:\jtest\locked.bin /inheritance:r /grant:r Administrators:F` (the user can list `C:\jtest` but not open the file).
2. As the user: `hoardini index --db %TEMP%\h-j.db --volume C:\jtest --kind full`.
3. Elevated: `fsutil file seteof C:\jtest\locked.bin 6000000`.
4. As the user: `hoardini index --db %TEMP%\h-j.db --volume C:\jtest --kind journal`; the `done` line shows
   `"kind":"journal"` and `records` ≥ 1.
5. `hoardini --db %TEMP%\h-j.db top --files --under C:\jtest` shows `locked.bin` at about 6 MB (5.7 MB). If it still
   shows 1.9 MB, the unprivileged read skipped the record: note it here.
6. Elevated, to compare: `fsutil usn readjournal C: csv | findstr /i locked.bin` lists the records an admin read sees.
7. Clean up: elevated `rmdir /s /q C:\jtest`; delete `%TEMP%\h-j.db*`.

### INDEX-ACTION-PATCH: Executed actions patch the index at once
**Applies to:** hoardini
**Verification:** test

The executor shall update the index right after an action, without a refresh: `index::patch_removed(conn, path)` for a
deleted or archived path that had a row, and `index::patch_rescan(conn, lister, path)` for anything else (a removed
small file, a compressed file, a replaced or hard-linked file, a new path). Both update the row(s) and every ancestor's
subtree totals in one transaction, and re-resolve hard-link ownership.

**Acceptance criteria:**
- Deleting a 300 MiB file + `patch_removed`: the row is gone and its parent, grandparent, … root each shrink by exactly
  its size.
- Deleting a dir subtree + `patch_removed` removes every row beneath it.
- Compressing a file (alloc 500 → 100 MiB) + `patch_rescan` returns the node with size 100 MiB.
- Deleting a small file + `patch_rescan` lowers the parent's `small_count` by 1.
- Replacing a copy with a hard link + `patch_rescan` counts the pair once (the smallest path owns it); removing the
  owner + `patch_removed` promotes the remaining link.
- `patch_rescan` of a removed directory returns `None`; of a new directory indexes its contents.
- After every step the snapshot equals a fresh build and totals verify.

### INDEX-COMPACT: The index stores only what is read
**Applies to:** hoardini
**Verification:** test

To keep the index small (INDEX-MEMORY-CAP), a directory row shall store its modification time but no access or
creation time; nothing reads them, and listing a directory moves its access time. A file ID whose high 8 bytes are
zero (NTFS) shall be stored as an 8-byte integer, a wider ID (ReFS) as its 16 bytes; both read back as the same
128-bit ID (`index::FileId`). The size index holds file rows only, and the subtree-size index only directories of at
least 256 MiB (the history summary floor). A first build of an empty index walks with a narrow parent index in place
of the parent/name and file-ID indexes and builds both, sorted, when it completes, so their pages are packed. Upgrading an index written under schema v1 converts it in
place.

**Acceptance criteria:**
- After a build, every directory row has a modification time and no access or creation time; a file row keeps all
  three.
- A file and a directory with NTFS-style IDs, including one whose low half has the sign bit set, are stored as
  integers and read back equal to their listed IDs; a hard-linked file with a 128-bit ID is stored as a 16-byte blob
  and counts once.
- A schema v1 index of the same tree (16-byte blob IDs, directory access and creation times) opens as v2 with a
  snapshot equal to a fresh build, only the wide IDs left as blobs; a refresh then deletes nothing and still equals
  a fresh build.

### INDEX-INTERRUPT-SAFE: An interrupted refresh leaves a valid index
**Applies to:** hoardini
**Verification:** test

A refresh that stops early (cancelled, killed, or failed) shall delete no rows and leave every committed batch
consistent: rows it reached are new, the rest are old, and subtree totals equal the sum of their parts. Rows not seen
in a generation are deleted only when that refresh completes. An interrupted refresh records no history entry. The next
refresh starts over.

**Acceptance criteria:**
- After a completed build and a round of changes, refreshes cancelled after 1, 4, and 9 listings each return
  `completed = false` and no scan id; every path present before is still present; `verify_totals` is empty; the
  scan count is unchanged.
- The next completed refresh equals a fresh build.

### INDEX-NO-HYDRATE: Cloud-only files are never read
**Applies to:** hoardini
**Verification:** manual

No component shall open or read the data of a file with `RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN`, or `OFFLINE`
attributes. Listing a directory and reading attributes is allowed. (M0 covers the indexer; the evidence engine, hashing,
compression, and archive add their own checks against this requirement.)

**Verification (manual):**
1. In OneDrive (or SVI Dropbox), pick a folder with at least one file over 100 MB and choose "Free up space" so every
   file shows the cloud-only icon.
2. Run `hoardini index --db %TEMP%\h-hydrate.db --volume C:\ --mode fast`.
3. Confirm the files still show the cloud-only icon and the sync client shows no download for them.
4. `hoardini --db %TEMP%\h-hydrate.db top --files --under "<folder>"` lists them at 0 B with "(cloud only)".
5. Delete `%TEMP%\h-hydrate.db*`.

### INDEX-UNACCOUNTED: Each refresh reports the unaccounted gap
**Applies to:** hoardini
**Verification:** manual

Each completed refresh of a volume root shall report `unaccounted = (total − free) − indexed total` in its `done` line,
its `scan` row, `index_status`, and the prompt summary. It is omitted when the indexed root is not a volume root or
volume space is unknown.

**Verification (manual):**
1. Run `hoardini index --db %TEMP%\h-gap.db --volume C:\ --mode fast` and note `indexed_bytes` and `unaccounted_bytes`
   from the `done` line.
2. Run `fsutil volume diskfree C:` at about the same time; used = total − free.
3. Check `unaccounted_bytes` ≈ used − `indexed_bytes` (within what changed between the two readings).
4. `vssadmin list shadowstorage` shows the shadow-copy share of the gap.
5. Delete `%TEMP%\h-gap.db*`.

### INDEX-POLITE: Background mode is visibly polite
**Applies to:** hoardini
**Verification:** manual

In background mode the indexer shall run with process background mode (low CPU, very low I/O and memory priority) and
EcoQoS, with 2 workers. Fast mode runs at normal priority with one lister per two logical CPUs, at most 12: a single
writer applies every listing, and more listers only add kernel contention. `--workers` overrides the count for testing.
The Settings tab's Fast / Lean switch (`index_speed`) picks the mode for the launch refresh, timer-driven journal
refreshes, and Rescan now: Lean is background mode, Fast is fast mode, and the very first full scan is always fast
(SRV-ONE-REFRESH). If the OS calls fail, the refresh logs a warning and continues.

**Verification (manual):**
1. Start `hoardini index --db %TEMP%\h-polite.db --volume C:\ --mode background`.
2. Task Manager → Processes: `hoardini.exe` shows the Efficiency mode leaf.
3. Resource Monitor → Disk → Processes with Disk Activity: `hoardini.exe` I/O priority is "Very Low".
4. Repeat with `--mode fast`: no leaf, I/O priority "Normal".
5. Delete `%TEMP%\h-polite.db*`.

### INDEX-MEMORY-CAP: Background refresh stays small
**Applies to:** hoardini
**Verification:** manual

A background refresh of C: shall keep the indexer's working set under 100 MB, and the index database of C: at the
1 MiB threshold shall stay under 150 MB on disk. A Job Object caps the process at a fixed 256 MB in both modes as a
backstop (`--memory-cap-mb` overrides it for testing; measured peaks are 46–56 MB). Bounded channels and a depth-first
work stack keep the pending tree small; the SQLite page cache is 16 MB.

**Verification (manual):**
1. Start `hoardini index --db %TEMP%\h-mem.db --volume C:\ --mode background`.
2. In PowerShell, sample every 2 s until it exits:
   `while ($p = Get-Process hoardini -ErrorAction SilentlyContinue) { $p.WorkingSet64 / 1MB; Start-Sleep 2 }`.
3. The peak stays under 100 MB. Record the peak, the duration, `dirs`/`files` from the `done` line, and the DB size;
   the DB (`h-mem.db`, after the run) is under 150 MB.
4. Delete `%TEMP%\h-mem.db*`.

### INDEX-STALE-AGE: Answers from the stored index state its age
**Applies to:** hoardini
**Verification:** manual

Every answer drawn from the stored index shall state how old it is: `index_status` returns `age_ms` and
`refresh_incomplete`; `summary_for_prompt` starts with "Index of <volume> as of <age>"; `hoardini top` prints the same
header and notes a refresh that has not finished.

**Verification (manual):**
1. Index C: with a temp `--db`, wait at least two minutes.
2. Start a second refresh; while it runs, run `hoardini --db <db> top`.
3. The header reads "Index of C:\ as of N minutes ago (refresh 1), a newer refresh has not finished".
4. After it completes, `top` reads "as of just now (refresh 2)" with no unfinished note.
