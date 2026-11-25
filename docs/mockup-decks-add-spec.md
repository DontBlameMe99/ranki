# Decks and add-card mockup implementation spec

Scope: design research only. Both supplied mockups were extracted under a temp
directory, read as HTML, and rendered with `html_preview` at 1280x800. The PNGs
were inspected. No raster image assets exist. Icons are inline SVG. No file was
copied into the repo. No code was changed.
The mockups are Mocha artboards with exact inline values. Values absent from the
HTML are marked "derived" and use existing tokens from `crates/ui/src/theme.rs`.
There are no `box-shadow`, `filter`, `opacity`, or `transition` declarations.
Hover and press rules below are proposed derived states using existing palette
roles.

## 1\. Shared measured tokens

| Role | Mocha hex | Source use |
| --- | --- | --- |
| Canvas / text | `#1e1e2e` / `#cdd6f4` | Root, field background |
| Sidebar / footer | `#11111b` | Left rail, logo ink |
| Raised panel | `#181825` | Sync card, forecast, deck table, history, footer |
| Input / selected fill | `#313244` | Search, selectors, fields, selected row |
| Divider / inactive fill | `#45475a` | Borders, shortcuts, bars |
| Subtext | `#a6adc8` | Secondary copy, labels |
| Muted | `#7f849c` | Chevron, ellipsis, hint, zero count |
| Accent / on accent | `#cba6f7` / `#11111b` | Primary actions, focus, active nav |
| New / learning / review | `#89b4fa` / `#f38ba8` / `#a6e3a1` | Counts and dots |
| Deck icon tints | `#94e2d5`, `#fab387` | Teal and peach icon tiles |

Radii present in the HTML: logo 10, nav 10, shortcut pill 6, accent buttons 12,
Create deck 12, search 12, dashboard cards 24, deck table 20, deck icon 10,
field 16, selector 12, selector option 9, tag chip 12, preview card 24,
history card 16, footer buttons 14, stepper circle 12, toggle 13 and 10,
color dot 4, study button 9, bars 8.
Type present in the HTML: brand 18/700, nav 14/600, shortcut 11/600 or 700,
page title 32/700 with `-0.02em` tracking, date 13/400, search 14/400,
Create deck 14/600, summary caption 13/400, due number 64/700 with
`-0.03em` tracking, Study all 16/600, stat label 12/400, stat value 20/600,
table header 12/600 with 0.08em tracking, deck name 15/600, deck detail
12.5/400, subdeck 14.5/500, count 15/600 with tabular numerals, Study 13/600,
add-card title 28/700, step label 13/600, field label 13/600, focused field
26/500, secondary field 20/400, field note 13/400, preview front 52/500,
preview answer 28/700, footer button 15/600.
Derived interaction rules: hover uses accent at 16% alpha or `surface0`.
Active uses accent at 22% alpha, pressed darkens the fill by 15%, and focus uses
a 1.5 px accent stroke. No control changes size on state change.

## 2\. Decks view layout

Artboard: 1280x800. Root is a horizontal flex with a 232 px fixed sidebar and
a main column padded `32px 40px` with 24 px vertical gaps.
Sidebar: 232 px wide, background `#11111b`, padding `24px 14px 20px`,
6 px child gap. Brand row has a 32x32 logo with radius 10, accent fill, dark
"A", then "Ranki" at 18/700. Primary "Add card" button is 44 px high, radius
12, accent fill, dark ink, padding `0 14px`, 8 px gap, 18 px plus icon, label
14/600, and an 11/700 "A" pill with radius 6. Nav buttons are 40 px high,
radius 10, padding `0 12px`, gap 12, 18 px icon, label 14/600, and a 11/600
pill. Active nav uses accent text on 16% accent fill. Inactive nav uses
`#a6adc8` on transparent. Order: Decks D active, Browse B, Statistics T,
Settings comma. Browse, Statistics, and Settings stay visible but disabled with
"Not implemented" tooltips. The bottom card is 12/14 padding, radius 14,
`#181825`, 13 px subtext, with an 8 px dot. Because Ranki has no sync, replace
"Synced" with "Local, updated 2 min ago" derived from
`Store::collection_modified`. Use review green when `external_change` is false
and red when true.
Header: left shows a 13 px date line then "Decks" at 32/700. Right has a
300x44 search field with radius 12, `#313244`, padding `0 14px`, 18 px icon,
14 px text, and a "/" pill, then a 44 px "Create deck" button with radius 12,
1 px `#45475a` border, padding `0 16px`, 14/600 label, and 18 px plus icon.
Summary row: two cards, 20 px gap. Left card has flex 1.6, radius 24,
`#313244`, padding 24. It shows "Due today" at 13 px, a 64/700 mauve due
count, and a 52 px pill button, radius 26, padding `0 24px`, 16/600, 18 px
play icon, "Study all". The 16 px spaced stats strip has a 1 px top border and
three equal columns. Each column has a colored 8 px dot, 12 px label, and 20/600
value 2 px below. Right card has flex 1, radius 24, `#181825`, 1 px
`#313244` border, padding 24, "Next 7 days" at 14/600, a 12/600 mauve pill with
4/10 padding and radius 10, and seven 8 px radius bars in a 10 px gap. The mock
Each bar shows the count and weekday from the store's daily due forecast,
starting with today. Overdue and currently due learning cards join today's
bucket. A store without the query keeps a weak "7-day forecast unavailable"
line. Do not show a streak or estimated minutes.
Deck table: radius 20, `#181825`, 1 px `#313244` border, clipped. Header and
rows use five cells: `1fr 72px 72px 72px 108px`. Header padding is `12px 20px`,
label 12/600 with 0.08em tracking. New, Learning, and Review are right aligned
and colored. Rows use `0 20px` padding and a 1 px top border. A parent row is 60 px
high with a 18 px chevron in `#7f849c`, a 36x36 radius 10 icon tile with 16%
tint, name 15/600, detail 12.5 px, three right-aligned counts at 15/600
tabular, and an ellipsis in the last cell. A subdeck row is about 45 px high.
Its first cell has `margin-left: 35px`, a 2 px left border in `#45475a`, and
`padding: 12px 0 12px 18px`, with 14.5/500 text. Inline "2 subdecks" text is
12 px subtext with 10 px left margin. A zero count uses muted `#7f849c`.
Selected rows use `#313244` across all five cells. The selected row's last cell
shows a 30 px Study button, radius 9, padding `0 14px`, 13/600 accent fill and
dark ink. Other rows show the ellipsis. Show Study on hover as a derived rule.
Subdecks are display-only grouping of `Deck.name` split on `::`. A real parent
deck uses its own `Store::stats(id)`, which already includes descendants in
`storage::Collection::deck_subtree`. A missing parent becomes a virtual group
header with summed child stats and no Study button. The global due count sums
leaf deck stats only. This is the minimal reconciliation with the standing
"no nested subdecks" note in `docs/ux-research.md`. No new Store method or
write path is needed, only an indented view of existing names.
Empty states use the same table shell and tokens. No decks: centered 24 px
panel with a 32/700 title "No decks yet", 14 px subtext "Create a deck to
start studying", and a 44 px accent "Create deck" action. Search with no match:
"No decks match" plus a secondary "Clear search" button. A deck with zero due
cards keeps its row and replaces Study with a muted "0 due". Study all is
disabled when the leaf sum is zero.

## 3\. Add-card view layout

The sidebar is identical except that "Add card" is active and "Decks" is
inactive. Main content is a column. The scrollable region has `32px 40px 0`
padding and a 20 px gap. The footer is fixed, `16px 40px 22px` padding,
`#181825`, 1 px top border, 14 px child gap.
Header: h1 "New card" at 28/700. The stepper has three 24 px circles with
2 px borders and 8 px label gaps. Active circle uses accent fill and dark ink
with a 14 px check. Current circle uses accent text and border. Future circle
uses `#313244` and subtext. Connectors are 24x1 in accent or `#45475a`. Labels
are 13/600: Setup, Content, Finish.
Setup row: 14 px gap. Deck selector is 340 px wide, `#313244`, radius 14,
padding `10px 12px`, gap 6. It shows an 11/600 uppercase "DECK" label, the
selected deck path at 14/600 with a `›` separator, and a 16 px chevron. The
note type selector is a `#313244` container with 4 px padding, radius 12,
2 px gaps, and 32 px options with `0 14px` padding, radius 9, 13/600. Options
are Basic, Reversed, Cloze. The active option uses 22% accent fill and accent
text. Resolve models by name from `Store::models`. Never hardcode ids. Map
names containing "reversed" and "cloze" case-insensitively. Otherwise use the
first model.
Form row: 32 px gap. Left column flexes, right rail is 360 px. Field labels are
13/600 subtext with `0 2px 8px` margins. The focused field has a 1.5 px accent
border, radius 16, `#313244`, padding `16px 16px 8px`, and 26/500 text with a
2x28 px caret. Its toolbar has a 1 px top border in `#45475a`, 10 px top
margin, 8 px top padding, 2 px gaps, and 36x36 radius 10 icon buttons. A `[…]`
button has `0 10px` padding and 13/600. The right hint is 12 px muted.
Unfocused fields use a 1.5 px `#45475a` border, `#1e1e2e` fill, 16 px padding,
20 px primary text, and 13 px subtext 6 px below. Fields are 18 px apart and
come from `selected_model.fields`. Tags use 8 px gaps. Chips are 13/500 with
`7px 8px 7px 12px` padding, radius 12, `#313244`, and a 14 px close icon. The
Add tag control repeats that shape with a 1.5 px dashed accent border and
accent text. In the wide layout, one trailing row 18 px under the columns holds
the tags field and the session history side by side at the same 32 px gap: each
is a 13/600 subtext label with an 8 px gap over a control at least 46 px tall,
so the two line up instead of the history starting from its own margin. Narrow
and image-occlusion layouts stack tags, and then the history, in the form
column at the same 18 px gaps.
Preview rail: radius 24, `#313244`, padding 20, 14 px gaps. Header has an
11/600 spaced "LIVE PREVIEW" label and a Front/Back toggle using the note type
option style. The preview is centered, 52/500 front with 1.15 line height,
14 px subtext reading, 1 px `#45475a` divider, 28/700 mauve answer, and a
14 px `#1e1e2e` box with `10px 14px` padding and radius 14. Render it through
the existing template substitution and `card_view::render`. The history card is
radius 16, `#181825`, 1 px border. Each row has `11px 14px` padding, a 16 px
green check, 15/500 name, 13 px subtext, and 12 px muted time. Keep this as UI
state only.
Footer: the toggle is 44x26, radius 13, accent fill when on, with a 20x20
radius 10 dot and a 13 px "Keep open after adding" label. Spacer flexes.
Cancel is 48 px high, radius 14, 1 px `#45475a` border, padding `0 20px`,
15/600. Add card is 48 px high, radius 14, accent fill, dark ink, padding
`0 22px`, 15/600, 18 px plus icon, and an 11/700 shortcut pill marked
Ctrl+Enter on non-macOS and Cmd+Enter on macOS.
Validation: save is blocked and no `Store::add_note` call occurs when no field
has text, when no deck or model is selected, or when the Cloze model has no
`{{cN::...}}` marker. Mark the offending field with a 1.5 px red border, show
a 13 px red line below it, and keep focus. Store failures use the existing
5 second toast. Success closes the view unless Keep open is on, clears only the
field values, adds a history row, refreshes stats, and reports "Added N cards".

## 4\. egui 0.37 implementation plan

In `crates/ui/src/app.rs`, add `View { Decks, Review, AddCard }` and a `view`
field. Keep `store: Box<dyn Store>` private and unchanged. Add UI state for
`deck_query: String`, `expanded_decks: HashSet<String>`, `deck_rows: Vec<DeckRow>`,
`add_draft`, `field_errors`, and `added_this_session`. `DeckRow` is a pure view
model with id, depth, name, counts, has\_children, and virtual flag. Add actions
for OpenDecks, OpenAdd, SetDeckQuery, ToggleDeck, CreateDeck, SelectDeck,
SetModel, SetDeck, SaveNote, and ToggleKeepOpen to the existing `dispatch`.
Replace `sidebar()` with a two-part layout: a shared `nav::render` and a
decks-specific `decks_view::render`. Keep `review()` and the grading code as
the Review view. Replace `add_note_dialog()` and `Dialog::AddNote` with the
full AddCard view. Change `EmptyKind::EmptyDeck` to set `view = AddCard`.
In `eframe::App::ui`, draw the nav `Panel::left` with `default_size(232.0)`,
`min_size(200.0)`, `max_size(280.0)`, then draw the selected view in the
central panel. The footer of the AddCard view is a bottom panel inside that
central panel. At widths below 1000 px, hide the 360 px preview rail and let
the form use the full width. This is a derived responsive rule.
Extract these modules under `crates/ui/src`: `nav.rs` for the shared rail,
`decks_view.rs` for header, summary cards, table, and empty states,
`add_card_view.rs` for setup, fields, preview, history, and footer, and
`components.rs` for reusable primary, secondary, nav, pill, search, dot, and
bordered-field widgets. Keep `card_view.rs` unchanged except for calls that
render preview markup.
Put pure functions beside their views and unit test them with the existing
`MemoryStore`: `build_deck_rows(decks, stats)`, `sum_leaf_counts(rows)`,
`filter_decks(rows, query)`, `resolve_model(models, tab)`, and
`validate_draft(draft, model)`. Reuse `reload()` for writes. `create_deck`,
`models`, `decks`, `stats`, and `add_note` are the only Store methods needed.
In `theme.rs`, add role helpers for `muted`, `selection_fill`, `accent_soft`,
`border`, and `on_accent`, plus a derived `press` helper. Do not add new color
constants. Make `font_candidates` put `CCL Nerd Font` first, then the matched
system sans, then `Noto Sans JP` for Japanese text. Reuse `accent_fill` and
`accent_ink` for all primary buttons. At 800x600, use the current min window
size and collapse the forecast card and preview rail.

## 5\. Standing requirements reconciliation

Palette: no conflict. Every mockup color is an exact Catppuccin Mocha role.
For Latte map canvas to `base`, rail and footer to `mantle`, panels and inputs
to `surface0`, borders and bars to `surface1`, text to `text`, labels to
`subtext`, accent to `mauve`, on-accent to white, and count colors to `blue`,
`red`, and `green`. Muted `#7f849c` maps to `subtext` at 72% alpha. Teal and
peach icon tints map to 16% `teal` and `peach`.
Font: direct conflict. The mockups load Inter and Noto Sans JP. Ranki requires
CCL Nerd Font. Substitute CCL Nerd Font for every Inter role and keep Noto Sans
JP only as a CJK fallback. Use the mockup sizes and weights unchanged.
Data: direct conflicts exist for sync status, 7-day history, streak, and true
nested deck records. Replace sync with a local collection timestamp, read the
forecast from card dues, drop the streak, and render `::` names as a view only.
Do not add a sync feature or deck hierarchy write.

## 6\. Acceptance criteria

Must:

1. At 1280x800, the app shows the 232 px rail, Decks active, the 64 px due
   number, both summary cards, and the five-column deck table. Test visually.
2. Counts equal `Store::stats` for each real deck and leaf counts equal the
   global due total without double counting parents. Unit test
   `build_deck_rows` and `sum_leaf_counts`.
3. Search filters by deck name case-insensitively and shows the no-match empty
   state. Unit test `filter_decks`.
4. Study all and each row Study button select the intended deck and enter
   Review. Unit test selection and queue reset with `MemoryStore`.
5. Add card is reachable from the rail and from the empty deck state. Its
   fields match the selected model and its deck list matches `Store::decks`.
   Unit test `resolve_model` and draft initialization.
6. Save calls `Store::add_note` with the chosen model id, deck id, fields, and
   whitespace-split tags, then refreshes stats. Unit test with a recording
   mock.
7. Empty fields, missing deck or model, and Cloze without a marker each block
   the write and show an inline error. Unit test `validate_draft`.
8. Dark mode uses Mocha and light mode uses Latte role mapping, with CCL Nerd
   Font first in the candidate list. Unit test palette and font helpers.

Should:

1. `::` deck names render as indented rows with 35 px margin, 2 px rule, and
   18 px padding. Virtual parents show aggregate counts and no Study button.
   Unit test row depth and virtual flag.
2. Hover, active, pressed, and focus states use the derived rules and cause no
   layout shift. Verify visually at 800x600 and 1280x800.
3. Front/Back preview changes text using the selected model templates.
   Unit test substitution from `card_view`.
4. Keep open preserves the form and clears only field values. Session history
   lists each successful add. Unit test state transitions.
5. Browse, Statistics, and Settings nav items are visibly disabled and explain
   why in a tooltip. Verify visually.

Nice:

1. Ctrl+Enter or Cmd+Enter saves from any field. A and D switch Add and Decks
   outside text fields. Unit test key routing.
2. The forecast card shows real bars when the daily dues query reaches `Store`.
   Test the view model with fixed daily counts.
3. At widths below 1000 px the preview rail moves below the form without
   clipping. Verify visually.
