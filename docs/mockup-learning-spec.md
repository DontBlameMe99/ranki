# Active learning mockup spec, question and answer states

Research only. No code changed. Source: the two supplied design exports,
`Desktop-Study-Front.dc.html` and `Desktop-Study.dc.html`. Both were extracted under
`.tmp/`, rendered with `html_preview` at 1280x800 in dark mode, and inspected as
screenshots. The exports are static artboards. Their inline styles are the source of
truth below. Files from the attachments were not copied into the repo.

The artboard is 1280x800 with background `#1e1e2e`, text `#cdd6f4`, and
`Inter, Noto Sans JP, system-ui, sans-serif`. The exported buttons have no hover or
pressed rules and no `box-shadow` anywhere. Every shadow, hover color, and pressed
color below is an implementation proposal, not a measured value.

## Shared chrome, present in both states

| Element | Measured CSS |
| --- | --- |
| Header bar | height 72, padding 0 32, flex row, `space-between` |
| End session | height 40, padding 0 14, radius 12, fill `#313244`, text 14/600, icon 18x18 stroke 2, gap 10 |
| Key badge | font 11/600, padding 2 6, radius 6, fill `#45475a`, text `#a6adc8` |
| Deck title | font 14/600, centered. Sample `Japanese › Vocabulary N5` |
| Progress track | width 420, height 6, radius 3, fill `#313244`, margin-top 10 |
| Progress fill | width 43% in the sample, height 6, radius 3, fill `#cba6f7` |
| Queue counts | gap 22, font 13, text `#a6adc8`, padding-bottom 4, bottom border 2 transparent |
| Active count | sample Review, number 27/700 `#a6e3a1`, label `#a6e3a1`, border 2 `#a6e3a1` |
| Inactive counts | New 11/700 `#89b4fa`, Learning 3/700 `#f38ba8`, transparent border |
| Main region | flex 1, column, centered on both axes, gap 24 |
| Card stage | width 760, height 440, padding 28, radius 32, fill `#313244` |
| Card top row | `space-between`, vertically centered |
| Tag chip | font 12/500, padding 5 10, radius 10, fill `#45475a`, gap 6 |
| Audio button | height 40, padding 0 12, radius 20, fill `rgba(203,166,247,.16)`, text `#cba6f7`, icon 20x20 stroke 2, gap 8 |
| Footer bar | height 64, padding 0 32, `space-between` |
| Footer action | height 36, padding 0 12, radius 10, font 13/500, text `#a6adc8`, icon 16x16 stroke 2, gap 8 |
| Footer groups | left Undo `Z`, Flag `F`, right Edit `E`, Bury `B`, gap 4 inside each group |

The card top row and footer bar are byte-identical in both exports. The header is
also byte-identical. Nothing outside the card and the primary control band moves on
reveal.

## Question state

Center block inside the card is vertically and horizontally centered.

| Element | Measured CSS |
| --- | --- |
| Eyebrow | `RECALL THE MEANING`, font 11/600, letter-spacing `.1em`, color `#7f849c`, margin-bottom 18 |
| Front | `食べる`, font 84/500, line-height 1.1, color inherited `#cdd6f4` |
| Reading | `たべる · taberu`, font 18, color `#a6adc8`, margin-top 8 |
| Primary button | width 760, height 76, radius 18, fill `#cba6f7`, text `#11111b`, font 17/600, gap 12 |
| Space badge | font 11/700, padding 2 8, radius 6, fill `rgba(17,17,27,.18)` |

Shown: tags, audio affordance, eyebrow, front, reading, Show answer button, header,
footer, progress. Hidden: answer separator, answer text, example block, all four
rating buttons, all interval previews.

## Answer state

The card frame, header, and footer keep their question-state geometry.

| Element | Measured CSS |
| --- | --- |
| Front and reading | unchanged, but the group shifts upward inside the center block as answer content is added |
| Separator row | width 100%, flex row, gap 12, margin `22px 0 16px` |
| Separator lines | two flexible lines, height 1, fill `#45475a` |
| Separator label | `ANSWER`, font 11/600, letter-spacing `.1em`, color `#7f849c` |
| Answer text | `to eat`, font 36/700, color `#cba6f7` |
| Feedback block | last child of the card, padding 14 18, radius 16, fill `#1e1e2e` |
| Feedback sentence | font 17/500, sample `毎日りんごを食べる。` |
| Feedback translation | font 13, color `#a6adc8`, margin-top 4, sample `I eat an apple every day.` |
| Rating row | width 760, CSS grid 4 equal columns, gap 14 |
| Rating button | height 76, radius 18, column layout, gap 4, centered |
| Rating interval | top line, font 12, opacity `.85`, sample `10m`, `6d`, `14d`, `21d` |
| Rating label row | font 16/600, gap 8, label plus key badge |
| Key badge | font 11, padding 2 6, radius 6, fill is 20% alpha of the rating color |

| Rating | Fill | Border 1px | Text | Sample key |
| --- | --- | --- | --- | --- |
| Again | `rgba(243,139,168,.14)` | `rgba(243,139,168,.4)` | `#f38ba8` | `1` |
| Hard | `rgba(250,179,135,.14)` | `rgba(250,179,135,.4)` | `#fab387` | `2` |
| Good | `rgba(166,227,161,.14)` | `rgba(166,227,161,.4)` | `#a6e3a1` | `3` |
| Easy | `rgba(137,180,250,.14)` | `rgba(137,180,250,.4)` | `#89b4fa` | `4` |

Proposal for missing states. Primary hover fill `#d7b8ff`, pressed `#b48bdc`, no
size change. Rating hover fill alpha `.22` and border alpha `.6`, pressed fill alpha
`.30` and border alpha `.8` with the same text color. Focus ring is 2px `#cba6f7`
with 2px offset. No drop shadows. Transitions are instantaneous, matching the
existing "no flip animation" decision in `docs/ux-research.md`.

## Transition semantics

The mockups imply one reveal action and one grading action. They do not define
animation, so treat reveal as a content swap inside a fixed frame.

```text
Question --Space, Enter, Show answer, or first 1..4--> Answer
Answer    --1..4 or rating click--> persist review, load next card, Question
Answer    --R--> Question
Any card  --Esc--> deck list, no write
```

What stays fixed: window chrome, header contents, 420x6 progress, 760x440 card box,
card top row, 64px footer, and the 24px gap between card and control band. What
changes: the center block of the card gains the separator, answer, and feedback
block, the 760x76 primary button is replaced by the 4-column rating row, and the
key badge changes from `Space` to `1` through `4`. The front group moves up inside
the card. No control outside the card changes position.

Current focus behavior is close. `grading()` requests focus on Show answer before
reveal and on Good after reveal. Keep that mapping. On reveal, focus should land on
Good because `3` and `Space` are both good answers in current `dispatch()`. Help
key `?` opens the existing sheet. Footer key badges are hints only until their
actions exist. Store has no undo, flag, edit, or bury primitive, so render those four
controls disabled with a tooltip and ignore their keys. Audio has no playback
callback, so show the affordance only when a sound ref exists and explain the limit
on hover.

## Theme reconciliation

The mockup is Catppuccin Mocha with roles already present in `theme.rs`: base
`#1e1e2e`, surface0 `#313244`, surface1 `#45475a`, text `#cdd6f4`, subtext
`#a6adc8`, mauve `#cba6f7`, red `#f38ba8`, green `#a6e3a1`, blue `#89b4fa`,
crust `#11111b`, and overlay2 `#7f849c`. Hard uses peach `#fab387`, not yellow.
Current `rating_fill(2)` uses `Palette::yellow`, so change code 2 to
`Palette::peach`. Current `rating_ink()` returns crust in dark mode, so change it to
return the solid rating role in dark mode.

Conflicts and resolutions:

1. Mockup 11px eyebrow color `#7f849c` on `#313244` is 3.4:1, below WCAG AA for
   small text. Use subtext `#a6adc8` (5.65:1) for the eyebrow and `ANSWER` label.
2. Mockup uses Inter and Noto Sans JP. The app uses CCL Nerd Font through
   `font_candidates()`. Keep CCL as the first proportional family and add Noto Sans
   JP as a CJK fallback when fontconfig finds it. Do not add font binaries. Keep the
   measured pixel sizes at the default `font_size` of 16.
3. Mockup is dark only. Map light roles as follows: page `LATTE.base #eff1f5`, card
   `LATTE.mantle #e6e9ef`, feedback block `LATTE.surface0 #ccd0da`, accent
   `LATTE.mauve #8839ef` with white ink, inactive chip `LATTE.surface1 #bcc0cc`.
   Use 10% fill and 45% border alpha for light rating buttons. Use darker text
   inks in light mode for AA: Again `#a7163a`, Hard `#8f4300`, Good `#276b1c`,
   Easy `#1755c9`. These give 5.8:1 or better on the light card.
4. Existing `progress()` counts session answers. The mockup shows deck New,
   Learning, and Review counts. Use `selected_stats()` for the three numbers and
   keep session progress only as the bar fraction.

## egui 0.37 implementation plan

Current locations from `crates/ui/src/`.

- `app.rs:600 review()`: replace the stacked header, progress, and card flow with
  the three-band layout. Keep `Panel::bottom("grading")`, but make the footer a
  separate 64px row after it.
- `app.rs:619 review_header()` and `app.rs:639 progress()`: collapse into one 72px
  header. Left is End session. Center is deck title plus a fixed 420x6
  `ProgressBar::new()` with `.desired_height(6.0)` and `.desired_width(420.0)`.
  Right is the three counts with a 2px bottom stroke on the active one. Use
  `RichText::size()` at 14, 13, and 11. Remove the `answered / remaining` label.
- `app.rs:727 current_card()`: change `Margin::same(24)` to `same(28)`,
  `corner_radius(12.0)` to `32.0`, and `STAGE_INNER_HEIGHT` from 232 to 440 at
  1280x800. Keep `Frame::NONE.fill(faint_bg_color)` and keep the media notice
  caption below the stage. Add the separator row, answer label, answer text, and
  feedback block only when revealed. Reuse `card_view::render` for front, answer,
  and images.
- `card_view.rs:375 render()`: keep it unchanged except for text size arguments.
  Add `theme::study_question_size()`, `study_answer_size()`, and
  `study_eyebrow_size()` that return 84, 36, and 11 at base 16 and scale with the
  user font size. Keep `LayoutJob` wrapping at `clamp(320.0, 720.0)`.
- `theme.rs:111 apply()`: add `study_*` sizes and semantic constants for the
  eyebrow color, rating fills, rating borders, rating inks, and hover and pressed
  variants. Extend the existing palette tests with exact color assertions.
- `app.rs:846 grading()`: question branch becomes one centered 760x76
  `Button::new("Show answer").min_size(vec2(760.0, 76.0)).corner_radius(18.0)`
  with a child badge. Answer branch keeps `columns_const::<4, _>()`, but allocates
  a 760px row, sets `item_spacing.x = 14.0`, and gives each button fill, 1px
  stroke, radius 18, height 76, interval label, rating label, and key badge.
  Keep `rating_previews()` and `scheduler.preview()` so intervals are computed.
- `app.rs:1259 action_for_key()`: add `Key::Escape` to a new End session action and
  ignore `Z`, `F`, `E`, and `B` until their store primitives exist. Replace the
  `bool revealed` with `enum Phase { Question, Answer }` in `ReviewApp` and
  `dispatch()`. Keep the 120ms grade debounce and the first-digit-reveals rule.
- Keep: sidebar, `ImageCache`, toast, external-change banner, dialogs, `?` help,
  focus requests, and status line. Remove: inline rating key text from
  `show_keyboard_hints`, the current `0 / remaining` progress label, the 180x40
  reveal button, and the 92x40 single-line rating buttons.

At 800x600, scale the stage to available width with a 760px cap and 480px floor,
and scale height with a 440px cap and 280px floor. Keep all radii and paddings.

## Acceptance criteria

Must

- At 1280x800, a screenshot shows a 72px header, 760x440 card, 24px gap, and 76px
  primary control. Tag, audio, front, eyebrow, reading, and badge sizes match the
  tables above.
- Question state shows no answer text, separator, feedback block, rating button, or
  interval. Answer state shows all five. Header and footer pixel positions are
  identical between states.
- All four rating buttons are 76px tall with 14px gaps and computed intervals from
  `scheduler.preview()`. Mock values are 10m, 6d, 14d, and 21d for the sample card.
- Unit tests assert the exact dark and light color constants, including the Hard
  peach fix, the eyebrow contrast fix, and AA light rating inks.
- Keyboard-only test reveals with Space, grades with 3, loads the next card in
  Question state, and keeps focus visible. Escape ends the session with no write.

Should

- The active queue count has a 2px underline and the progress bar is 420x6 with a
  mauve fill. Session counts no longer appear in the bar label.
- Hover and pressed styles change fill and border only. No control moves by more
  than 0px between states, verified by two screenshots at the same window size.
- Light theme maps roles as specified and keeps AA contrast for all 11px and 12px
  labels.
- A `[sound:...]` reference produces the pill audio control, and a missing file
  produces a caption without breaking the stage.

Nice

- Visible focus ring is 2px mauve with 2px offset.
- Footer actions render disabled with tooltips when their Store primitive is absent.
- A help sheet lists Esc, Space, 1 to 4, R, Z, F, E, and B with the same badge style.
