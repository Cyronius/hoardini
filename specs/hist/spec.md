# HIST — refresh history and hotspots

Canonical spec for `hoardini-core::history` and `hoardini changes`. Design: `.claude/plans/hoardini-design.md` §5.2.

Tests: `specs/hist/tests/hist.rs` (`cargo test -p hoardini-specs --test hist`).

## Terms

- **Scan**: the `scan` row a completed refresh writes, plus its `scan_dir` and `scan_file` rows (the summary).
- Sizes are allocated bytes; 256 MB and 100 MB mean 256 MiB and 100 MiB, matching the binary units shown everywhere.

## Requirements

### HIST-SUMMARY: A completed refresh stores a compact summary
**Applies to:** hoardini
**Verification:** test

Each completed refresh shall write one `scan` row (volume, generation, mode, start and finish time, total, used, free,
indexed bytes, unaccounted bytes, file, dir, and denied counts) plus one `scan_dir` row for every directory whose
subtree is ≥ 256 MiB (path, size, file count) and one `scan_file` row for every counted file ≥ 100 MiB (path, size,
mtime). Hard-link duplicates are not listed. An interrupted refresh writes nothing.

**Acceptance criteria:**
- Dirs of 300 MiB, exactly 256 MiB, and 200 MiB − 4 KiB: the first two and the root are stored with exact sizes and
  file counts; the third is not.
- Files of 300 MiB, 200 MiB, exactly 100 MiB, 100 MiB − 4 KiB, and 56 MiB: the first three are stored with an mtime.
- A hard link to the 300 MiB file in another dir does not add a second `scan_file` row.
- The scan's indexed bytes equal the root total; file count 5, dir count 5; used and unaccounted follow from the
  volume's total and free space.
- A cancelled refresh adds no scan.

### HIST-CHANGES: Diff between two refreshes
**Applies to:** hoardini
**Verification:** test

`history::changes(conn, volume, since, limit)` and `hoardini changes --since <scan id | Nd | previous>` shall compare
the latest scan with an earlier one (the previous scan; a given scan id; or the newest scan at least N days old,
falling back to the oldest) and report: `grew` and `shrank` (in both summaries, change ≥ 1 MiB), `new` (only in the
latest summary), and `removed` (only in the earlier one), for dirs and files, sorted by size of change, each capped at
`limit` with the omitted count reported, plus the free-space and indexed-total deltas. To keep the list short, a dir
whose change one child entry explains (≥ 90%) is dropped in favour of the child, and a new or removed dir under a new
or removed parent is dropped in favour of the parent. Files are never dropped.

The `changes` agent tool result shall also carry, when the range holds at least two scans with a known free
space, one free-space line (`history::free_summary`) giving the refresh count, start, end, and lowest free space with
their dates, and the net change; it never carries the raw points. The tool takes an optional `drive` (default: the selected drive once it has an index, INDEX-VOLUMES). The result ends with a machine-readable line
`range_ms: <earlier finished_ms>..<latest finished_ms> <volume root>` that the chat card uses to fetch the chart's points.

**Acceptance criteria:**
- After adding a 2 GiB file to `models`, deleting the 500 MiB dir `C:\old`, deleting a 600 MiB file and shrinking a
  424 MiB file to 300 MiB in `cache`, adding the 1 GiB dir `C:\new\sub`, and growing a log by 0.5 MiB:
  - `new` holds the 2 GiB file and `C:\new` (not `C:\new\sub`); `models` and its ancestors are not in `grew`.
  - `removed` holds `C:\old` (−500 MiB), its 400 MiB file, and the 600 MiB file; `shrank` holds the shrunk file
    (−124 MiB) and `cache` (−724 MiB, not explained by a single child).
  - The 0.5 MiB growth is not reported; the free delta is −2 GiB.
- `since` as the first scan's id gives the same result; `30d` with no older scan falls back to the oldest.
- `limit = 1` keeps one `removed` entry and reports the rest as omitted.
- Scans finished on days 10, 20, 30, and 40 with 100, 80, 60, and 90 GiB free: the summary over all four reads
  `Free space over 4 refreshes: start 100 GB (1970-01-11), end 90.0 GB (1970-02-10), low 60.0 GB (1970-01-31), net -10.0 GB.`;
  over days 20 to 30 only, start 80, end and low 60, net -20.0 GB; a range holding one scan gives no summary.

### HIST-HOTSPOT: Hotspot ranking
**Applies to:** hoardini
**Verification:** test

Each summarized dir shall get a hotspot score over the last N = 10 scans:
`ln(1 + size_GiB) + 3·ln(1 + growth_GiB) + recurrence + 2·refilled`, where size is the latest size, growth is the sum
of increases between consecutive scans, recurrence is the number of scan-to-scan steps in which the dir was among the
top 10 growers, and refilled means it dropped by half or more from ≥ 256 MiB and then regained at least half of the
drop. A dir missing from a scan counts as 0 there. The volume root is never a hotspot. `history::hotspots` returns
dirs by descending score; among the top candidates a dir that has a candidate below it is dropped (the deeper dir is
the hotspot).

**Acceptance criteria:**
- Over 11 synthetic scans (only the last 10 count):
  - steady growth of 1 GiB per step outranks one 9 GiB spike to the same size (recurrence);
  - the spike outranks a static 50 GiB dir (growth outweighs size);
  - the same spike on a 20 GiB dir outranks it on a 2 GiB dir (size);
  - a dir cleaned to 0 and refilled outranks one with the same size, growth, and recurrence but no cleaning;
  - growth only before the window scores the same as a flat dir of the same size;
  - the steady dir reports growth 9 GiB, recurrence 9, size 11 GiB, and ranks first; the root is absent.

### HIST-HOTSPOT-FIRST: Refresh lists hotspots first
**Applies to:** hoardini
**Verification:** manual

A refresh shall list the top 16 hotspot dirs that grew within the window (`history::hotspot_dirs`), and finish their
subtrees, before it lists the volume root and the rest of the volume. Progress lines report phase `hotspots` until the
root is listed, then `walk`, then `finalize`. Each dir is still listed once. (In-crate test
`index::query::tests::hotspots_are_listed_first` checks the order with one worker on the fake volume.)

**Verification (manual):**
1. Index C: with a temp `--db`, copy a ≥ 1 GB file into `~\Downloads`, and index again (history now shows growth).
2. Start Process Monitor with filters Process Name is `hoardini.exe` and Operation is `QueryDirectory`.
3. Run a third refresh with `--mode background --workers 1`.
4. The first directories queried are `~\Downloads` and its subdirectories; `C:\` comes after them. The first progress
   lines show `"phase":"hotspots"`.
5. Delete the temp database.

### HIST-RETENTION: History keeps the last 20 refreshes and one per month
**Applies to:** hoardini
**Verification:** test

After each completed refresh, history for that volume shall keep the newest 20 scans plus the newest scan of every
calendar month (UTC) and delete every other scan with its `scan_dir` and `scan_file` rows.

**Acceptance criteria:**
- With 1 recent scan and 10 synthetic scans in each of April, May, and June 2020, the next refresh leaves 21 scans:
  the 2 recent ones, all 10 of June, the newest 8 of May, and only the newest of April; a deleted scan's `scan_dir`
  rows are gone and the kept April scan still has its row.
