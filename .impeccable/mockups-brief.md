# Hoardini site: five mockups (shared brief)

Development-only. Never copy any of this into a page.

## Job
Five materially different, full-fidelity, static HTML landing-page mockups for Hoardini, so the owner can pick one. The picked one becomes the real site at cyronius.github.io/hoardini/. Mode: Persuade. Code-first (no comps). Each mockup must be good enough to ship.

## Audience and action
Everyday, non-technical Windows users whose C: drive is full. Primary action: download the portable app. Secondary: source on GitHub. Plain, warm words. No jargon in headlines (AppData, WinSxS, NTFS stay out of headlines).

## Facts (only these; never invent users, reviews, ratings, press, or "X GB saved on average" claims)
- Name: Hoardini. Tagline: "The world's smartest trashcan." Free and open source (Apache-2.0). Windows 10/11, 64-bit.
- He works out what every big folder and file on your drive actually is, says it in plain words, and proposes what to do: delete it, shrink it in place (compress, nothing lost), move it to another drive, or keep it.
- Results come as one short screen in three groups: **Junk you won't miss** (temp files, caches, Windows leftovers, Recycle Bin; one press clears it), **Free space, keep the files** (compress in place, or keep cloud files online only; nothing lost), **Your call** (big things only you can judge, biggest first; open one and he says what it is and why he thinks so; Delete, Shrink, Move, Keep, or ask him about it).
- Evidence first: a catalog of known locations, file types, the installed-programs list, and checks for files that are open. Then the AI explains. The web last.
- Safety: Nothing happens until you press. Delete needs the button held. Delete is permanent and skips the Recycle Bin, because the bin frees nothing until it is emptied. Your word wins over his guesses; only what you said to keep is held back. Things you keep stop showing up, with one link to bring them back. Rules in code can still hold any path back.
- Stays out of the way: one window. Keeps a list of the whole drive and updates it quietly in the background, in a low-priority process.
- Brains: **On this PC (recommended)**: Granite 4.1 3B downloads once and runs on the CPU; nothing leaves your PC; works offline. Qwen3.5 9B (smarter, needs a graphics card) and any .gguf file too; LM Studio, Ollama, llama.cpp, vLLM. **Free online**: Google Gemini, OpenRouter, Ollama Cloud, Mistral, Cloudflare Workers AI, Z.ai; the setup screen says how much each allows, whether a card is needed, and whether it may learn from your questions. **Your own provider**: OpenAI, Anthropic, Groq, Together, Cerebras, or any OpenAI-compatible endpoint. Online choices send your questions, and the file and folder names he looks up, to that provider.
- The mascot: Hoardini is a friendly sumo wrestler, "he". **He gets bigger as your drive fills up** (real app behavior: his girth follows how full the drive is) and slims down as you free space.
- Download (current release 0.1.2): portable app, one file, about 39 MB, no installer:
  https://github.com/Cyronius/hoardini/releases/download/v0.1.2/Hoardini-0.1.2-x64-portable.exe
  All downloads / notes: https://github.com/Cyronius/hoardini/releases/latest
  Source: https://github.com/Cyronius/hoardini
  Back to the maker: https://cyronius.github.io/ ("Made by Cyrus Attoun")
- Honest first-run notes (say once, near the download, in plain words, not as a scary banner; do NOT say "alpha"): the app is unsigned, so Windows SmartScreen warns: choose "More info", then "Run anyway". First run sets up a brain.

## Example data (label it "Example" wherever a visitor could take it as their own result)
Drive C: 476 GB, 9 GB free.
- Junk you won't miss, about 41 GB: Recycle Bin 13.6 GB ("Things you deleted, still taking space"); Windows Update leftovers 12.4 GB ("Windows already installed these updates"); Temporary files 8.9 GB ("Leftovers from Windows and your apps, untouched for 12 days"); Chrome cache 6.2 GB ("Web pages Chrome saved; it gets them again as you browse").
- Free space, keep the files, about 38 GB: OneDrive photos 2019-2022, 29.5 GB ("Keep online only. They stay in OneDrive and open when you click them."); Old school projects, 8.7 GB ("Compress in place. Same files, less space.").
- Your call: iPhone backup from 2021, 64.2 GB ("A full backup of an iPhone, last changed March 2021. There is a newer one next to it."); Zoom recordings, 18.3 GB ("Meeting recordings from 2022. Zoom doesn't need them."); Downloads, 22.0 GB ("Mostly installers you already ran.").

## Assets
- `site/mockups/assets/mascot/<state>-<slim|medium|full>.svg`: the real app mascot, baked from the app. States: idle, shuffle, think, tada (arms up, sparkles), look (magnifying glass), carry (holding a box), worry (sweat drop), nap (zzz). slim/medium/full = drive 30% / 60% / 97% full. Inline the SVG markup (copy the file contents) when you want to animate parts or recolor the belt: the belt uses `var(--sumo-belt, #d9604c)`. As `<img>`, the fallback colour applies. viewBox 0 0 64 64, overflow visible.
- `site/mockups/assets/icon.svg`: the app icon (sumo face on a rounded square).
- No app screenshots: show the product as an authored HTML demonstration in your page's own world, with the example data above, labeled Example.

## Rules
- Each mockup is one folder `site/mockups/NN-slug/` with `index.html` (+ optional `style.css`/`script.js`). Paths to shared assets are relative (`../assets/...`). Google Fonts allowed for mockups. No other external hosts. JS only where it earns its place; content visible without JS.
- Load and obey `C:/Users/josha/.claude/skills/impeccable/reference/craft-floor.md` and `C:/Users/josha/.claude/skills/impeccable/reference/mode-persuade.md` before writing. Bans that matter most here: no eyebrow/kicker labels above headings, no same-size icon-card grids as structure, no gradient text, no glass, no emoji as icons, no hero-metric template, no section numbers unless they carry information, no fake testimonials or stats.
- First viewport (1440x900 and 390x844): what it is, why it matters, and a working Download button, plus a demonstration of the mechanism in the world's own form. Then the page continues: the three groups, "what is this?" with evidence, safety, brains/privacy, download again with requirements + source + first-run notes, footer back to cyronius.github.io.
- One authored motion moment, respecting prefers-reduced-motion. Theme selection, focus rings, scrollbars from the palette. Light/dark chosen by the world, not by default.
- Copy: short sentences, plain words, Hoardini is "he". American English.
- Verify: capture 1440-wide full page and 390-wide full page with
  `python -I "C:/Users/josha/AppData/Local/Temp/claude/14d8f24c23fe798e/1629593d-f719-4e10-a140-24ed749782fb/scratchpad/shot1.py" "file:///C:/code/hoardini/site/mockups/NN-slug/index.html" <out.png> <width> <height>`
  into `.impeccable/review/NN-slug-desktop.png` and `-mobile.png`. Look, fix, look once more (two rounds max). No horizontal scroll at 390.
- Do not commit. Do not touch files outside your folder (the shared assets are read-only).
