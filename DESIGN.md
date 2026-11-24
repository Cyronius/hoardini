---
name: Hoardini
description: The world's smartest trashcan, sold as a driveway yard sale of your drive.
colors:
  concrete: "#d8d7d2"
  concrete-2: "#cbcac4"
  ink: "#161616"
  ink-2: "#3a3a38"
  board-yellow: "#fff23d"
  board-pink: "#ff5cad"
  board-orange: "#ff7a1f"
  board-lime: "#a6f04f"
  paper-white: "#fbfbf8"
  masking-tape: "rgba(236, 222, 178, 0.9)"
  sticker-red: "#c92a21"
  sticker-green: "#13793a"
  sticker-blue: "#1f63d6"
  table: "#f3f3ef"
  table-edge: "#b9b8b1"
  apron: "#2d2d2b"
  manila: "#f1d79a"
typography:
  display:
    fontFamily: "Sedgwick Ave Display, Shantell Sans, cursive"
    fontSize: "clamp(3.4rem, 7.4vw, 6rem)"
    fontWeight: 400
    lineHeight: 1.14
    letterSpacing: "-0.01em"
  headline:
    fontFamily: "Sedgwick Ave Display, Shantell Sans, cursive"
    fontSize: "clamp(2.6rem, 5vw, 4.2rem)"
    fontWeight: 400
    lineHeight: 0.95
    letterSpacing: "-0.01em"
  title:
    fontFamily: "Sedgwick Ave Display, Shantell Sans, cursive"
    fontSize: "clamp(1.7rem, 2.6vw, 2.3rem)"
    fontWeight: 400
    lineHeight: 1
  numeral:
    fontFamily: "Sedgwick Ave Display, Shantell Sans, cursive"
    fontSize: "4.6rem"
    fontWeight: 400
    lineHeight: 0.9
  note:
    fontFamily: "Shantell Sans, Comic Sans MS, cursive"
    fontSize: "1rem"
    fontWeight: 600
    lineHeight: 1.35
  label:
    fontFamily: "Shantell Sans, Comic Sans MS, cursive"
    fontSize: "0.95rem"
    fontWeight: 800
    lineHeight: 1
  body:
    fontFamily: "Atkinson Hyperlegible, system-ui, sans-serif"
    fontSize: "1.125rem"
    fontWeight: 400
    lineHeight: 1.55
rounded:
  card: "2px"
  board: "3px"
  tag: "4px"
  table: "6px"
  hangtag: "8px"
  pill: "999px"
  sticker: "50%"
spacing:
  xs: "8px"
  sm: "16px"
  gutter: "24px"
  lg: "48px"
  xl: "56px"
  section-sm: "96px"
  section-lg: "120px"
components:
  button-download:
    backgroundColor: "{colors.board-orange}"
    textColor: "{colors.ink}"
    typography: "{typography.label}"
    rounded: "{rounded.board}"
    padding: "18px 22px 18px 24px"
  button-act:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    typography: "{typography.label}"
    rounded: "{rounded.pill}"
    padding: "9px 14px"
  button-act-hover:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.paper-white}"
  button-act-pressed:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.board-yellow}"
  button-clear-all:
    backgroundColor: "{colors.sticker-red}"
    textColor: "{colors.paper-white}"
    rounded: "{rounded.pill}"
    padding: "14px 22px"
  button-clear-all-hover:
    backgroundColor: "#a8221a"
  nav-download:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.board-yellow}"
    rounded: "{rounded.pill}"
    padding: "8px 16px"
  price-tag:
    backgroundColor: "{colors.paper-white}"
    textColor: "{colors.ink}"
    rounded: "{rounded.tag}"
    padding: "44px 14px 14px"
  dot-sticker:
    textColor: "{colors.paper-white}"
    rounded: "{rounded.sticker}"
    size: "64px"
  poster-board:
    backgroundColor: "{colors.board-yellow}"
    textColor: "{colors.ink}"
    rounded: "{rounded.board}"
    padding: "40px 48px 36px"
  hang-tag:
    backgroundColor: "{colors.manila}"
    textColor: "{colors.ink}"
    rounded: "{rounded.hangtag}"
    padding: "72px 36px 36px"
  speech-tag:
    backgroundColor: "{colors.manila}"
    textColor: "{colors.ink}"
    typography: "{typography.note}"
    padding: "14px 16px"
  chip:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    typography: "{typography.label}"
    rounded: "{rounded.pill}"
    padding: "9px 14px"
---

# Design System: Hoardini

Scope: this governs the product website in `site/` only. The desktop app in `ui/` keeps its own separate "Marshmallow" look and is not governed here.

## Overview

**Creative North Star: "The Driveway Yard Sale"**

The site is a Saturday-morning yard sale of your hard drive. The ground is light, grained concrete. Fluorescent poster boards hang slightly askew from masking tape. Everything on the drive sits on folding tables as a white price tag with a round dot sticker, and the price is gigabytes. The sumo mascot runs the sale, and his girth follows how full the drive is. Nothing is sleek and nothing is dark. The site is handmade on purpose, the way a careful neighbour's sale looks, because it has to persuade non-technical people that this is friendly and that they decide.

Density is relaxed: big hand-lettered signs, generous section breathing room (96 to 120px), and one strict 12-column grid underneath so the tilt reads as hand-hung rather than broken. Character comes from the objects: tape, stickers, tags, the table with its dark front and legs. Shapes, gradients and glass do none of the work. State is shown with a mark. A marker strike means something was freed, and a strip of tape reading HOLD · MINE means it was kept.

The site rejects the category default of a dark utility page with a treemap screenshot and three matching feature cards.

**Key Characteristics:**
- Grained light concrete ground, never white or dark.
- Fluorescent boards taped up at small tilts, always with ink text.
- Three hands of type: marker signs, handwritten notes, a legible reading face.
- Red, green and blue dot stickers that carry meaning and are never decoration.
- Physical props (folding table, hang tag, speech tag, tape) instead of cards and panels.
- State is a mark: strike-through for freed, tape for held.
- One quiet motion moment: signs swing up into place on load.

## Colors

The palette sets fluorescent poster stock and three sticker primaries against neutral concrete and ink.

### Primary
- **Poster Yellow** (board-yellow): the hero sign, the "Free space, keep the files" table sign, text selection, the skip link, and the text color on ink pills (nav Download, pressed Keep, numbered steps). This is the site's signature color and its theme color.

### Secondary
- **Fluorescent Pink** (board-pink): a poster board for the junk table sign and the closing "Take him home" sign.
- **Safety Orange** (board-orange): the download button board and the "Your call" table sign.
- **Highlighter Lime** (board-lime): the free-space tally board and the recommended brain card.

### Tertiary (the sticker code)
- **Junk Red** (sticker-red): dot stickers on junk-you-won't-miss items, and the Clear-the-whole-table button.
- **Keep-Files Green** (sticker-green): dot stickers on free-space-keep-the-files items, and the "files kept" note on a struck row.
- **Your-Call Blue** (sticker-blue): dot stickers on items only the user can judge.

### Neutral
- **Driveway Concrete** (concrete): the page ground, always under the fractal-noise grain overlay. It also shows through the hang-tag eyelet.
- **Shadowed Concrete** (concrete-2): the scrollbar track and the deeper concrete tone.
- **Marker Ink** (ink): all text, pill borders, focus outlines, the strike mark, and the footer ground.
- **Faded Ink** (ink-2): secondary text (row descriptions, download meta, table footnotes) and the scrollbar thumb.
- **Price-Tag White** (paper-white): price tags, plain brain cards, the House rules board, and the example card.
- **Masking Tape** (masking-tape): the tape strips holding boards up, and the HOLD · MINE mark.
- **Tabletop** (table) and **Table Edge** (table-edge): the folding-table surface and its visible thickness.
- **Apron** (apron): the dark table front that carries the example card and the sticker legend, plus the table rails.
- **Manila** (manila): the hang tag and the sumo's speech tag.

### Named Rules
**The Sticker Code Rule.** Red, green and blue appear only as dot stickers (and their legend), and they always mean junk, free-space-keep-files, and your call. The poster fluorescents carry no meaning; any board color may sign any table.

**The Fluorescent Board Rule.** Yellow, pink, orange and lime are paper stock. They fill whole boards with ink text on top, and they are never used as a text color on concrete. Yellow-on-ink is the one exception: the text inside ink pills.

**The Concrete Ground Rule.** The page ground is grained concrete. Sections that need separation use a 28% white wash with a faint 2px ink border, never a new hue.

## Typography

**Display Font:** Sedgwick Ave Display (fallback Shantell Sans, cursive)
**Body Font:** Atkinson Hyperlegible (fallback system-ui, sans-serif)
**Label/Note Font:** Shantell Sans (fallback Comic Sans MS, cursive)

**Character:** A felt-marker sign-writer's hand for the signs, a friendly handwritten voice for anything written on a tag or note, and a face built for low-vision legibility for everything people actually read. All three are self-hosted.

### Hierarchy
- **Display** (400, clamp(3.4rem, 7.4vw, 6rem), 1.14): the hero sign headline only.
- **Headline** (400, clamp(2.6rem, 5vw, 4.2rem), 0.95): section headlines and the closing sign.
- **Title** (400, clamp(1.7rem, 2.6vw, 2.3rem), 1): table signs, brain card heads (2rem), and hang-tag heads (2.2rem).
- **Numeral** (400, 4.6rem, 0.9): the live free-GB tally and the "Free." on the download board (2.4rem).
- **Note** (Shantell 600, 1rem to 1.6rem, 1.3 to 1.45): sign sublines, speech tags, the hang-tag quote, table footnotes, and captions.
- **Label** (Shantell 800, 0.95 to 1.3rem, 1 to 1.2): buttons, chips, tag names, sticker numerals, and rule leads.
- **Body** (Atkinson 400, 1.125rem, 1.55; 1.0625rem under 720px): all reading copy, kept to 38 to 60ch.

### Named Rules
**The Three Hands Rule.** Marker is for signs and never runs below 1.7rem or into a paragraph. Shantell is for what is written on tags, notes and buttons. Atkinson is for anything someone reads to understand.

## Layout

One 12-column grid with a 24px gutter, inside a container of min(1320px, 100% - 64px) (100% - 32px under 720px). Compositions are asymmetric 7/5 or 6/6 splits: a sign on the left, explanation on the right. Section heads put the marker headline in columns 1 to 7 and a short paragraph in 8 to 12, bottom-aligned. Section vertical padding runs 96 to 120px (72 to 80px on mobile). Spacing steps are 8, 16, 24, 48 and 56px.

Breakpoints: at 1180px every split stacks to 12 columns, the hero tag row drops to three tags, and the sumo moves beside its caption. At 720px the nav keeps only Download, tags become full-width rows with the sticker on the left, table rows go to two columns (sticker, then text with actions below), and the speech tag sits beside a smaller sumo.

## Elevation & Depth

Depth is physical. Paper objects hang slightly off the wall on soft drop shadows that are tight at the top and fall off below. Tape and stickers sit just proud of the paper. The folding table shows its thickness as a solid, zero-blur edge, because a table has an edge. There is no glassmorphism, no glow, and no tonal-layer system.

### Shadow Vocabulary
- **Hung board** (`box-shadow: 0 14px 22px -12px rgba(25,25,20,0.45), 0 2px 4px rgba(25,25,20,0.14)`): poster boards, brain cards, the hang tag, and the speech tag.
- **Board lift** (`box-shadow: 0 20px 26px -12px rgba(25,25,20,0.5), 0 2px 4px rgba(25,25,20,0.14)`): the download board on hover.
- **Price tag** (`box-shadow: 0 10px 14px -10px rgba(25,25,20,0.5), 0 1px 2px rgba(25,25,20,0.12)`): price tags on the hero table.
- **Tape** (`box-shadow: 0 1px 2px rgba(0,0,0,0.12)`): tape strips and the HOLD mark.
- **Sticker** (`box-shadow: inset 0 -3px 0 rgba(0,0,0,0.14), 0 2px 3px rgba(0,0,0,0.18)`): dot stickers.
- **Table edge** (`box-shadow: 0 4px 0 #b9b8b1, 0 22px 28px -18px rgba(0,0,0,0.4)`): sale tables and the hero tabletop (3px edge there).

### Named Rules
**The Taped-Up Rule.** Paper lifts on soft, blurred shadow. The only zero-blur offset in the system is the folding table's edge; never put a hard offset shadow on a board, card or button.

## Shapes

Paper corners are barely softened: 2px on the example card, 3px on boards, 4px on tags and cards, 6px on tables and the speech tag, and 8px to 10px on the hang tag. Everything you press is a full pill (999px) or a perfect circle (stickers, numbered steps, rule bullets). Hand-hung objects tilt by small amounts (about 0.6 to 2.2 degrees) and alternate direction between neighbours. Stickers sit at -8 degrees and tape at ±38 degrees on board corners, or -3 degrees centred on top. Hover or press straightens a tilted object to 0 and lifts it.

**The Hand-Hung Tilt Rule.** Tilt stays small and alternates between neighbours. Only paper tilts. Copy set directly on concrete or on a tabletop (lede, section paragraphs, table rows) stays level.

## Components

### Buttons
Buttons feel like things you'd grab off a sale table: thick ink outlines, pill shapes, hand lettering.
- **Shape:** full pill (999px), except the download board, which is a tilted orange board with a 3px ink border.
- **Download board:** board-orange, 3px ink border, a marker "Free." then a Shantell label and a hand-drawn arrow, tilted 1.2 degrees. On hover it straightens and lifts 3px with the board-lift shadow, and on press it drops 1px. It has a smaller sibling for the closing sign.
- **Act (Clear / Free it / Delete / Keep):** transparent with a 2px ink border and Shantell 800 text. On hover it fills ink with paper-white text. When pressed (aria-pressed) it fills ink with yellow text.
- **Clear the whole table:** a sticker-red pill with white text that darkens to #a8221a on hover. It appears only on the junk table.
- **Nav Download:** an ink pill with yellow text that goes to pure black on hover.
- **Focus:** a 3px ink outline offset 4px everywhere (a yellow outline on the ink footer).

### Chips
- **Style:** the act shape (2px ink pill, Shantell 800), shown as static examples on the hang tag. The hold-to-delete chip is filled ink with white text.

### Price Tags
White tag cards with a dot sticker overlapping the top-left corner and the item name in Shantell 800 at the bottom. Heights are staggered and tilts alternate. Hover or selection straightens and raises the tag 8px, and selection adds a 3px ink outline. Picking a tag rewrites the sumo's speech tag.

### Dot Stickers
64px circles (86px large, 56px on mobile) tilted -8 degrees. They show the GB figure in Shantell 800 white with a small "GB" under it, filled with the sticker code color.

### Sale Tables
The tabletop with its table-edge thickness, a dark apron rail drawn under it, and two metal legs. A poster sign is taped across the top-left edge. Rows are a sticker, a name with a plain description, and actions, separated by 2px dashed faint-ink rules.

### State Marks (signature)
**The State Is a Mark Rule.** Freeing an item draws a marker strike from the sticker across the text over 0.55s (cubic-bezier(0.65, 0, 0.35, 1)), and the row fades to 55%. On the keep-files table the strike crosses the sticker only and the text reads "· files kept" in green. Keeping an item slaps a tilted strip of tape reading HOLD · MINE over the sticker. State is never shown by a toggle, checkbox or color change alone.

### Hang Tag and Speech Tag
- **Hang tag:** a manila tag on a string through a concrete-colored eyelet, tilted 2.2 degrees, carrying a large sticker, a marker title, a handwritten quote, check-mark reasons (drawn with CSS borders), and chips.
- **Speech tag:** a manila note with one square corner and a tail pointing at the sumo, set in Shantell 600. It updates live and politely.

### Navigation
A plain row of Atkinson 700 links (1rem) that underline on hover, next to the marker wordmark and app icon, ending in the ink Download pill. Under 720px only the wordmark and Download remain.

### Mascot
Baked SVG poses of the app's sumo (carry, worry, think, idle, tada) in full, medium and slim girths. Girth always tracks drive fullness, and the tally swaps the pose as space is freed.

## Do's and Don'ts

### Do:
- **Do** set every page on grained concrete, separating sections with a 28% white wash and a 2px faint ink border.
- **Do** hang fluorescent boards with masking-tape strips at the corners or top centre, at a small alternating tilt.
- **Do** reserve red, green and blue for dot stickers that mean junk, keep-files, and your call.
- **Do** show freed and kept state as a marker strike and a HOLD · MINE tape mark.
- **Do** keep marker type at 1.7rem or larger, on signs only; set reading copy in Atkinson Hyperlegible at 1.125rem / 1.55.
- **Do** label any demonstration data as an example on a white example card.
- **Do** keep motion to the load-time sign swing (0.9s, cubic-bezier(0.16, 1, 0.3, 1)) and small state transitions, all disabled under reduced motion.

### Don't:
- **Don't** build a dark utility page, show a treemap screenshot, or use three equal feature cards; vary card size, color and tilt.
- **Don't** put a hard zero-blur offset shadow on boards, cards or buttons; that belongs only to the table edge.
- **Don't** use fluorescent colors as text on concrete, or use the sticker colors decoratively.
- **Don't** tilt copy that sits directly on concrete or a tabletop, or tilt any object more than about 2.2 degrees.
- **Don't** set paragraphs in Sedgwick Ave Display or Shantell Sans.
- **Don't** use icon fonts or emoji glyphs; draw icons as inline SVG strokes in ink (2.2 to 3.4 stroke width).
