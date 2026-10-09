# UX revisions, WP-CONV: one continuous conversation (ui)

Design: `.claude/plans/ux-revisions.md` §1, §2, §4 (welcome-back card); contract: `.claude/plans/ux-revisions-impl.md` (Conversation block).

## New requirements

- **UI-CONTEXT-BOUNDARY** (manual): the "Hoardini doesn't remember above this line" divider before `window_start_id` (or at the end when it is null and `has_hidden`), and the header's Start fresh button in New chat's old slot.
- **UI-WELCOME-BACK** (manual): the greeting becomes a welcome-back card at the end of the scrollback when `fresh` is true, with a third starter, "Free space over time".
- **UI-CLEAR-HISTORY** (manual): `ClearHistorySection` in Settings asks for confirmation, deletes the chat text only, and the chat empties.

## Modified requirements

- **UI-CHAT-FLOW**: steps 6 and 7 use a page reload (the conversation opens itself) instead of Recent chats.

## Removed requirements

None.
