# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Product site: static HTML/CSS in `site/`, no build step, published by a GitHub Pages workflow to cyronius.github.io/hoardini/ (same pattern as the based repo). The app itself is a Tauri desktop shell around a React UI; that stack is already set by the codebase.

## Users

Everyday Windows users whose drive is full and who don't know why. They are not developers. They want the space back without breaking anything and without learning what `AppData` or `WinSxS` means. Secondary: builders who arrive from cyronius.github.io and may open the source.

## Product Purpose

Hoardini works out what every big folder and file on the drive actually is, explains it in plain words, and proposes how to get the space back: delete it, shrink it in place, move it, or keep it. Success is a user who frees real space in one sitting and trusts what they deleted.

## Positioning

Not another disk-usage treemap and not a "PC cleaner". Hoardini answers "what is this?" for each big item, from evidence first (a catalog of known locations, file types, installed programs, open-file checks), then an AI to explain, then the web. Results come as three short groups: junk you won't miss, space you can free and keep the files, and your call.

## Operating Context

Windows 10/11 x64, NTFS drives. Portable app, no installer. The user picks a "brain" on first run: a small local model that runs on the CPU (recommended, offline, private), a free online provider, or their own provider. Online choices send questions and the file and folder names Hoardini looks up to that provider.

## Capabilities and Constraints

- Delete is permanent and skips the Recycle Bin, on purpose; a delete needs the button held.
- Nothing happens until the user presses; code rules can still hold any path back.
- Things the user keeps stop showing up, with one link to bring them back.
- Index stays fresh from the NTFS change journal, in a low-priority process.
- Binaries are unsigned; SmartScreen warns on first run.
- Downloads: GitHub releases (portable .exe; zip with the app and the `hoardini` CLI). Current release 0.1.2.

## Brand Commitments

- Name: Hoardini. Tagline: "The world's smartest trashcan."
- Mascot: a friendly sumo wrestler (`app/icon-src/icon.svg`, `icon.png`): round face, topknot, rosy cheeks, plum outline.
- Hoardini is "he" in product copy ("why he thinks so", "ask him about it").
- Voice: plain, warm, short sentences, no jargon. Owner: do not call the product "extreme alpha" any more.

## Evidence on Hand

- README copy and feature list.
- Mascot artwork in `app/icon-src/`.
- The real UI, runnable against a mock server (`cd ui; bun run mock`) for screenshots.
- No user counts, testimonials, reviews, press, or measured space-saved figures exist. Do not invent them. Demonstration data on the site must be labeled as an example.

## Product Principles

- The answer is the action: every explanation ends in a button that does it.
- Your word wins over the AI's guess.
- Evidence before AI, AI before the web.
- Plain words for people who aren't technical.
- Stay out of the way: one window, no tray icon, no notifications.
