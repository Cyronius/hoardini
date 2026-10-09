# UX revisions WP-SET: auto-saving Settings, Rescan now, presence (ui)

Source: `.claude/plans/ux-revisions.md` §6, §8.

## New requirements

- **UI-SETTINGS-AUTOSAVE** (manual): the Settings tab has no Save button. The Fast / Lean switch saves on change with a "Saved" tick beside it or an inline error; text fields save on blur or after 500 ms idle. It shows the switch with its hints and the read-only index size, nothing else of the old settings. The header has one "Rescan now" control when idle; while refreshing, the progress bar (with the leaf in Lean) takes its place. The UI posts presence on focus, blur, and `visibilitychange`, and closes the status stream while the page is hidden.

## Modified requirements

None.

## Removed requirements

None.
