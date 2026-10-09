# UX revisions §13: refreshed follow-up card after a job (act)

Design: `.claude/plans/ux-revisions.md` §13 (tick state decided 2026-10-09).

## New requirements

- **ACT-FOLLOW-UP** (test): when a delete, archive, or compress job ends (done, partial, failed, cancelled), the executor creates a pending proposal with `parent_id` = the job's proposal, in the same conversation, holding every item whose status is not `done`, each with `selected` = its approved tick state. Items are re-examined through `proposals::create` (resolve, deny list, alloc, block, cap, memory note); a newly blocked item is unticked; a path that no longer exists is dropped; a denied path goes to `dropped`. Kind, destination, format, summary, verdicts, and reasons carry over. No items left = no follow-up. Dedupe gets none. The follow-up row is committed before the job's terminal update is broadcast. Proposal JSON carries `parent_id` and `follow_up_id` (the newest proposal whose `parent_id` is this one). Schema: `proposal.parent_id` and `proposal_item.memory_note` appended to `schema_v3.sql`.

## Modified requirements

None.

## Removed requirements

None.
