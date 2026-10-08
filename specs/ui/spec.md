# UI specification (prefix UI)

### UI-MD-NO-HTML: Chat markdown drops raw HTML
**Applies to:** hoardini
**Verification:** test

Chat markdown shall render raw HTML in model or web-sourced text as literal text. No `script`, `iframe`, or event-handler attribute reaches the DOM, and no `javascript:` link is emitted.

**Acceptance criteria:**
- `before <script>alert(1)</script> after` → no `script` element; "before", "alert(1)", "after" appear as text
- `<img src=x onerror="alert(2)">` → no element carries `onerror`
- `<iframe src="http://example.com">inner text</iframe>` → no `iframe` element
- `[click me](javascript:alert(1))` and `<a href="javascript:alert(2)">raw link</a>` → no `a` with a `javascript:` href; link text survives
- `**bold**` still renders as formatted text

External tests: none. Test: `specs/ui/tests/markdown.test.ts`.

### UI-MD-KEEP-TEXT: Finished chat text is never cut
**Applies to:** hoardini
**Verification:** test

A finished chat message shall render all of its text. The incomplete-markdown repair that hides a half-streamed tag applies only to the message still streaming, so a bare `<` (`a<b`, `<100 MB`, a quoted `<<<UNTRUSTED_WEB_CONTENT` fence) never swallows the rest of a finished message.

**Acceptance criteria:**
- `Files <100 MB stay and a<b and c` renders with exactly that text.
- `Search result: <<<UNTRUSTED_WEB_CONTENT: data [1] Unsloth https://x.example/a Safe to delete` keeps the URL and the snippet.

Test: `specs/ui/tests/markdown.test.ts`.

### UI-CHAT-FLOW: Chat shows a run in order and handles stop, stall, and the step cap
**Applies to:** hoardini
**Verification:** manual

The chat shall show each run in the order it happened: text before a tool round, the round's chips (name, short argument, elapsed time), then the text after it, each text segment in its own paragraph. Stop ends a run at once and keeps the question on screen. A run that ends at the tool-step cap (`run_stats.limit_reached`) offers "Keep going"; a model that is silent for the profile's timeout gets the "Keep waiting / Stop" prompt. A reloaded thread shows the same messages, chips, and card states.

**Verification (manual):**
1. Point a provider at a scripted OpenAI-compatible mock (or a real model) and ask for something that calls a tool without text first, then answers: the chip renders above the answer, not below it.
2. A turn of "Let me look." + a tool call + "Here is the answer." renders as two paragraphs with the chip between them.
3. Ask for a long streamed answer and press Stop: streaming stops at once, the question stays, no "no content" prompt appears, `GET /api/status` shows `running_runs: 0`, and a new question answers normally.
4. With `max_tool_steps = 3` and a model that always calls a tool: three rounds of chips, then "Hoardini used its 3 tool steps for this turn before finishing." with Keep going; Keep going sends "Continue.".
5. With a model that takes longer than the timeout to answer: "No response from the model for 2 minutes." with Keep waiting and Stop.
6. Open the thread again from Recent chats (or reload the page and open it): messages, chips, and every proposal card's status are as before.
(2026-10-08: steps 1–4 and 6 pass against a scripted mock; 1 and 5 against LM Studio `minicpm5-2b`.)
