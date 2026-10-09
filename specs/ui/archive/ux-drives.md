# UX revisions, WP-DRIVES: drive selector (ui)

Design: `.claude/plans/ux-revisions.md` §11.

## New requirements

- **UI-DRIVE-SELECTOR** (manual): the header shows only the selected drive's free-space bar; its drive letter is a dropdown button (`C: ▾`). The menu lists every fixed drive with its label and a free-space bar. Watched drives have a check, and clicking one selects it. Other drives have **Add**. Each watched drive has **Stop watching** behind a confirm step, disabled for the last watched drive. Unplugged drives are greyed out as "not connected". FAT and exFAT drives say "no change journal: every refresh is a full scan". Keyboard: the button opens it with focus on the selected drive, arrows move between drives, Escape or an outside click closes it and focus returns to the button.

## Modified requirements

- **UI-MASCOT-FULLNESS** (manual): girth follows the selected drive (else `C:`, else the first volume), and so does the tooltip's "D: is N% full". The welcome-back card (UI-WELCOME-BACK) also follows the selected drive.

## Removed requirements

None.
