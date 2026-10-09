# UX revisions §12: smart default ticks and memory notes per item (policy)

Design: `.claude/plans/ux-revisions.md` §12.

## New requirements

- **POLICY-DEFAULT-TICK** (test): a new item starts ticked only when it is not blocked, its stored (capped) verdict is `safe` or `redownload` (no verdict counts as `review`), and it is not small. Small = `alloc < 50 MiB` and `alloc < 2%` of the proposal's `total_alloc`. Each item's JSON carries `small: boolean`. The deterministic rule is `proposals::is_small(alloc, total)` and `proposals::default_tick(blocked, verdict, small)`.
- **POLICY-ITEM-MEMORY** (test): each item carries `memory_note`, the text of the most specific non-protect, non-forgotten memory covering its path (MEM-RECALL-ANCESTRY order), or null. Attaching it marks that note used (MEM-USAGE). A protect memory still refuses the path (POLICY-PROTECT).

## Modified requirements

- Model section: "Selectable" items start ticked per POLICY-DEFAULT-TICK instead of "unless `keep`".

## Removed requirements

None.
