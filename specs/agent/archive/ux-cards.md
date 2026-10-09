# UX revisions §12: file lists come as proposal cards (agent)

Design: `.claude/plans/ux-revisions.md` §12.

## Modified requirements

- **AGENT-PROMPT-CONTEXT** (test; wording never tested): the system prompt (full and small) carries the card rule. Files or folders the user could act on go in one `propose_*` card with a verdict and one-line reason per item, not a text list; overviews may stay text; the explanation comes before the call. Large ordinary-file backups (phone backups, `WindowsImageBackup`, old `.bak`/archive copies) are included as `review`.
- M6 eval: `biggest-files` may now end in a card (`may_propose`). `grew` stays text-only (an overview of change), so AGENT-EVAL-SCORING's `grew` criterion is unchanged.

## New requirements

None.

## Removed requirements

None.
