# Ranki review UI design direction

Research date: 2026-11-22. Scope: the egui/eframe review screen only. No code was
changed.

## Evidence from the running app

A screenshot was obtained on Hyprland with `grim`. The UI was launched from the
existing debug binary with a temporary copy of `WorldFlags.apkg` (204 cards) and
a throwaway collection under `.tmp/`. The imported model had no templates, so
the temp collection JSON was patched to copy the Basic template before capture.
That patch was research scaffolding, not a product finding.

Observed in the answer state:

- The card content occupies the top 15% of the central panel. The remaining
  space is empty background.
- The question/answer caption, media caption, reveal button, four rating
  buttons, and full shortcut sentence all sit in the same narrow band near the
  top.
- Rating buttons are 74x34 px, uniform gray, and disabled before reveal.
- The progress bar spans the whole central panel and carries the text
  "0 of 204 reviewed".
- The imported image is not rendered. The card area instead shows
  `Media  Image: Abkhazia.png (available)` as ordinary text.

For image-only notes, a review screen must show the image. The current
`card_view::render` collects media references but only prints their names
(`crates/ui/src/card_view.rs:233-270`). Rendering the resolved image with
`egui::Image::from_path` inside the card frame is the single largest visual fix.
Check the API in the pinned egui source before implementation; the research did
not verify image widget details beyond the 0.37 widget set.

## What current review apps actually do

Anki 25.x: the study flow is question, reveal, then four ratings mapped to
`1` through `4`. Each answer button shows the next interval. Anki also supports
a two-button Again/Good mode. Preferences include hiding top and bottom bars,
minimalist mode, reduced motion, showing remaining count, and showing next
review time above answer buttons. Sources:
<https://docs.ankiweb.net/studying.html>,
<https://docs.ankiweb.net/manual/preferences>.

AnkiDroid: 2.23 ships a redesigned study screen with a better toolbar, improved
gestures, and tablet/desktop support. Early feedback on that screen highlights
colored answer buttons, two-button mode, larger card margins, and complaints
about low-contrast answer-button text. That is evidence for color plus adequate
text contrast, not color alone. Sources:
<https://forums.ankiweb.net/t/ankidroid-2-23-0-changelog/67744>,
<https://forums.ankiweb.net/t/new-study-screen/63230>.

RemNote: one front side, `Reveal Answer`, then large rating buttons at the
bottom with next intervals. The queue counter sits at the top. `Skip Card` is a
top-right toolbar action, and ratings can be swiped on mobile. Sources:
<https://help.remnote.com/en/articles/6022755-getting-started-with-spaced-repetition>.

Mochi: uses a two-phase flow with Forgot/Remembered, spaced repetition, and
dark/light documentation screenshots. Cards are Markdown documents, including
cloze and diagram cards. Source:
<https://mochi.cards/docs/getting-started/reviewing-cards/>,
<https://www.mochi.cards/docs/cards/>.

Knowt: flashcards flip on click, navigation is by arrows or keyboard, and
study controls sit under the card. Source:
<https://help.knowt.com/en/articles/10298062-how-can-i-use-flashcards-mode>.

Obsidian spaced repetition: the review view shows note heading context above a
card and exposes edit, reset, info, skip, and delete as discrete toolbar
actions. Source:
<https://stephenmwangi.com/obsidian-spaced-repetition/flashcards/reviewing/>.

General constraints: WCAG 2.2 AA requires 4.5:1 contrast for normal text and
3:1 for large text. Body line length of 45 to 75 characters is a long-standing
readability recommendation. Review loops repeat hundreds of times per session,
so control reach and instant reveal matter more than decoration. Sources:
<https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html>,
<https://webtypography.net/2.1.2>,
<https://yazu.app/blog/designing-a-review-screen/>.

## Current code audit

- `crates/ui/src/theme.rs:19-27`: Heading 24, Body 16, Button 15, item spacing
  (10, 8), button padding (12, 7). The type scale has only three roles.
- `crates/ui/src/theme.rs:38-59`: dark and light fills and text colors are
  defined, but there are no semantic colors for ratings.
- `crates/ui/src/app.rs:80-81`: window starts at 1024x768 with an 800x600
  minimum, so 800x600 is a real target.
- `crates/ui/src/app.rs:373-407`: sidebar uses a 210 px default width and
  three-line deck labels with all counts inline.
- `crates/ui/src/app.rs:440-464`: deck heading, FSRS(T) button, full-width
  progress bar, and two separators surround the card.
- `crates/ui/src/app.rs:476-491`: the card is a weak caption plus raw markup.
- `crates/ui/src/app.rs:500-518`: reveal and all four ratings share one
  `horizontal_wrapped` row directly under the card.
- `crates/ui/src/app.rs:522-528`: the entire shortcut list is a body-size
  label inside the card flow.
- `crates/ui/src/card_view.rs:236`: text wraps at full available width, up to
  the panel width, with no measure cap.

## Actionable spec

All APIs named below exist in the pinned egui 0.37 source.

### Must

1. Give the card a stage. In `current_card`, wrap `card_view::render` in
   `egui::Frame::NONE.fill(visuals.faint_bg_color).inner_margin(egui::Margin::same(24)).corner_radius(12.0)`.
   Use `ui.vertical_centered_justified` for the question and answer. Reserve at
   least 280 px height at 800x600 with `ui.add_space` or a min-size container.
   Remove the two `ui.separator()` calls at `app.rs:422` and `app.rs:432`.
2. Make the card text the largest text. In `card_view.rs`, use 26 px for the
   question and 20 px for the answer instead of 17/19. Cap wrapping at
   `ui.available_width().min(720.0).max(320.0)` to keep roughly 50-70
   characters per line. Keep `TextWrapMode::Wrap`.
3. Render media inside the stage. Resolve `[sound:...]` and `<img src=...>`
   with the existing `media_path`, then show images with egui's image widget at
   a max width of 520 px. Move missing-media notices to a 12 px weak caption
   below the stage. Do not leave `Media  Image: X (available)` in the card.
4. Move grading to a bottom panel. Use
   `egui::TopBottomPanel::bottom("grading").show_inside(ui, ...)` in the
   central panel. Put `Reveal answer` there before reveal. After reveal, show
   four `Button`s with `min_size(egui::vec2(92.0, 40.0))`, 8 px gaps, and
   `Button::fill` from the theme roles below. Hide the ratings before reveal
   instead of showing disabled gray buttons.
5. Add semantic rating colors in `theme.rs` as `Color32` constants:
   Again `#BE474D`, Hard `#B6791F`, Good `#356BBE`, Easy `#2B7A52`. Use white
   text on Again/Good/Easy and dark ink `#10141B` on Hard. These pairs give
   5.0:1, 5.05:1, 5.25:1, and 5.24:1 text contrast, and at least 3.48:1
   button-to-page contrast in both current themes.
6. Quiet the chrome. Change the progress bar to `desired_height(6.0)` and put
   `0 / 204` in a 12 px weak label beside it. Remove `Due today` from the
   sidebar. Move the scheduler toggle out of the review header into the
   sidebar or a menu, or show `FSRS` as 12 px weak text.
7. Fix the 800x600 case. Keep the sidebar at `default_size(200.0)`,
   `min_size(160.0)`, `max_size(260.0)`. Replace three-line deck labels with a
   strong 15 px name and a 12 px weak count line. The four 92 px rating buttons
   plus 8 px gaps fit in a 600 px central panel.

### Should

- Show the next interval on each rating button, for example `Good  1d`, using
  the scheduler's own interval calculation. Do not hardcode intervals.
- Replace the hint bar with short labels on the controls (`Space`, `1` through
  `4`) and one `?` help button showing the full list in a tooltip or popup.
- Add a small `context` label from note headings or tags above the card title,
  as Obsidian's reviewer does. Keep it 12 px weak text.
- Add a `Skip` action for cards that were revealed too early. It must not write
  a review.
- Keep the reveal instantaneous. Avoid reveal animations; the research source
  treats flip latency as a core cost in repeated review loops.
- Use `style.spacing.button_padding = egui::vec2(16.0, 9.0)` so buttons reach
  40 px height without per-button padding.
- Raise `TextStyle::Body` to 18 px and keep `TextStyle::Button` at 15 px with
  `RichText::strong()` for the primary action.

### Nice

- Offer a two-button Again/Good mode using the same colored roles. Anki and
  AnkiDroid both support this.
- Add a compact session summary after the last card before returning to the
  empty state, rather than the current bare `No cards due` message at
  `app.rs:428`.
- Add optional swipe or gesture grading as an alternate path, never the only
  path. AnkiDroid and RemNote both keep buttons.
- Let the card stage use a slightly raised fill in light mode and a slightly
  lighter fill in dark mode, derived from `visuals.faint_bg_color`.

## Before and after tied to code

1. `app.rs:476-491` and `card_view.rs:233-270`. Before: `Question` in weak
   text, an image-only card rendered as `Media  Image: Abkhazia.png (available)`, no boundary, most of the panel is empty. After: a 24 px padded stage
   with the image centered at up to 520 px wide, a 26 px question above it,
   and a 12 px media-status line below the stage.
2. `app.rs:500-518`. Before: `Reveal answer (Space)` followed by `1 Again`,
   `2 Hard`, `3 Good`, `4 Easy` in one 74x34 gray row directly under the card.
   After: a bottom panel with `Reveal  Space` centered before reveal, then four
   92x40 filled buttons with next intervals. Hard uses dark ink on amber so
   the label stays legible.
3. `app.rs:440-464`. Before: `WorldFlags`, `FSRS (T)`, and a full-width blue
   bar reading `0 of 204 reviewed` stacked above the card, with separators.
   After: a 20 px deck title, a 6 px progress line with `0 / 204` in 12 px
   weak text, no separators, and the scheduler control in the sidebar.

## Sources

- Anki studying manual: <https://docs.ankiweb.net/studying.html>
- Anki preferences: <https://docs.ankiweb.net/manual/preferences>
- AnkiDroid manual: <https://docs.ankidroid.org/>
- AnkiDroid 2.23 changelog:
  <https://forums.ankiweb.net/t/ankidroid-2-23-0-changelog/67744>
- AnkiDroid new study screen feedback:
  <https://forums.ankiweb.net/t/new-study-screen/63230>
- RemNote spaced repetition:
  <https://help.remnote.com/en/articles/6022755-getting-started-with-spaced-repetition>
- Mochi reviewing cards:
  <https://mochi.cards/docs/getting-started/reviewing-cards/>
- Knowt flashcards mode:
  <https://help.knowt.com/en/articles/10298062-how-can-i-use-flashcards-mode>
- Obsidian spaced repetition reviewer:
  <https://stephenmwangi.com/obsidian-spaced-repetition/flashcards/reviewing/>
- WCAG 2.2 contrast:
  <https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html>
- Line length: <https://webtypography.net/2.1.2>
- Review screen friction: <https://yazu.app/blog/designing-a-review-screen/>
