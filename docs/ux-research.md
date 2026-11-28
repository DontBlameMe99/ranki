# Ranki review UX spec

Research note for one focused UI worktree. It describes the current state machine, external patterns, and a prioritized change list. It does not propose a rewrite of the scheduler, storage, or transports.

This note is scoped to the desktop review worktree. The separate mobile UI path in `crates/bin/src/mobile` intentionally adds touch layout, browse, add-note and sync screens; see `docs/mobile.md`.

## Internal baseline

`crates/ui/src/app.rs` owns all review UI state. `ReviewApp::new` loads decks, models, and per-deck stats, selects deck 0, then `reset_queue` clears the queue and calls `ensure_current`. Each frame `eframe::App::ui` refills the queue, loads the next card if needed, handles all unmodified key events in `handle_key`, and draws a sidebar plus central panel.

Review loop today:

1. `ensure_current` pops one card, sets `shown_at`, clears `revealed`, loads the note and renders front/back templates from `card_view`.
2. Space or Enter calls `reveal`; R calls `toggle_reveal`. Buttons use the same calls.
3. Keys 1 to 4 call `answer` even while the question is shown, but `answer` returns early unless `revealed`. Rating buttons are disabled until reveal.
4. `answer` measures `shown_at.elapsed()`, asks the active `Scheduler` for a decision, writes `revlog.time_ms`, calls `Store::apply_review`, then clears the current card and immediately calls `ensure_current`. There is no transition state, next-interval label, or session summary.
5. `seen` prevents a card from returning in the same session, so learning cards with a future due time disappear until reload.
6. `selected` deck changes, F5, and refresh all call `reset_queue`, which discards session counts without warning.
7. `self.error` is one persistent colored label. It is overwritten by the next error and never cleared on success.

Constraints from the repo:

- `Store` in `crates/model/src/lib.rs` has `decks`, `models`, `create_deck`, `add_note`, `apply_review`, `stats`, `collection_modified`, and `collection_created`. It has no note update, delete, or undo method.
- `ranki_ui::Options` in `crates/ui/src/lib.rs` carries only title, media directory, and theme. Import and export live in `crates/bin/src/main.rs` and `ranki_format`; the GUI has no callback into them.
- `crates/bin/src/connect.rs` opens its own `Collection` and writes through `addNote`, `answerCards`, `createDeck`, `suspend`, and `changeDeck`. `crates/bin/src/mcp.rs` opens its own collection behind a mutex and writes through `add_note`, `answer_card`, `import_apkg`, and `export_apkg`. Neither notifies the GUI.
- `docs/architecture.md` fixes the `Store` boundary. A true undo or note edit needs a model change outside this worktree, so the UI contract is specified here without changing that crate.

## External patterns

- Anki 25 shows only the question until Space or Show Answer, then exposes Again/Hard/Good/Easy with 1 to 4; after reveal, Space or Enter selects Good. Each button shows the next interval. Shortcuts are discoverable in menus and button tooltips. Preferences include remaining card count, next review time above buttons, optional Space answers, and reduced motion ([Studying](https://docs.ankiweb.net/manual/studying), [Preferences](https://docs.ankiweb.net/manual/preferences)).
- Anki's deck screen shows New, Learn, and Due counts before Study Now; statistics separate today's reviews from history and report review time and answer-button counts ([Getting Started](https://docs.ankiweb.net/manual/getting-started), [Statistics](https://docs.ankiweb.net/manual/stats)).
- AnkiDroid puts the three counts at the top of the study screen and allows gestures for each answer button, undo, edit, and other actions ([Study Screen](https://docs.ankidroid.org/manual.html), [Gestures](https://krmanik.github.io/ankidroiddocs/gestures.html)).
- RemNote exposes due counts in the sidebar and a practice button for a document or the global queue. Its `/` menu lets users invoke commands by name without memorizing shortcuts ([Flashcard Basics](https://help.remnote.com/en/articles/8663109-flashcard-basics), [Keyboard Shortcuts](https://help.remnote.com/en/articles/7893440-keyboard-shortcuts)).
- Mochi uses two ratings, Remembered (`Space`) and Forgot (`F`), and explains that the interval grows or shrinks and that a forgotten review enters a re-review queue ([Reviewing cards](https://mochi.cards/docs/getting-started/reviewing-cards/)).
- Knowt's flashcard mode uses click-to-flip plus arrows or keyboard navigation, with options applied immediately. Its SRS mode rates Again, Hard, Good, or Easy ([Flashcards mode](https://help.knowt.com/en/articles/10298062-how-can-i-use-flashcards-mode), [Spaced Repetition](https://help.knowt.com/en/articles/10714645-how-do-i-use-spaced-repetition-mode)).
- Keyboard-first UX needs a persistent visible focus indicator and no content-imposed time limit ([WCAG 2.4.7](https://www.w3.org/WAI/WCAG21/Understanding/focus-visible.html), [WCAG 2.2.1](https://www.w3.org/WAI/WCAG21/Understanding/timing-adjustable.html)).

## Interaction decisions

Micro interactions:

- Reveal is one action. Space or Enter reveals the answer. After reveal, Space or Enter grades Good. The first 1 to 4 press on the question side also reveals, matching AnkiDroid; the second grades. No hidden action silently does nothing.
- Answer buttons show key, label, and next interval. Keep order fixed: Again, Hard, Good, Easy. Use a four-column grid, at least 44 px tall, with stable layout when card text wraps.
- After a valid grade, show a quiet one-line status: `Rated Good, next in 3 days`. Swap text in place. Do not animate the card or move the sidebar.
- Measure time only while the window is focused and the card is visible. Cap at 60 seconds, the default collection `maxTaken`. Count time on both question and answer sides.
- Undo is one step, Ctrl+Z, available until another write happens. It restores the prior card state and removes the last review log entry when storage gains that primitive.
- Focus starts in the review panel on session start and returns there after grading. Global shortcuts are ignored while a text field or select has focus. Keyboard focus always has a visible ring.
- Hover changes background or border only. Press darkens the button. Disabled controls explain why in a tooltip. No layout shift on any state change.
- Errors are non-blocking toasts at the top of the central panel with a 5 second timeout and a dismiss action. Success clears the current error. Retry actions stay visible for failures.
- Progress is `answered this session` plus current remaining due count, recomputed after each review. An empty deck shows `0 due`, never a full bar.
- Debounce grading for 120 ms after a card loads or a grade is written so one held key cannot rate two cards.

Macro interactions:

- First run distinguishes no decks, an empty deck, and a caught-up deck. Each state has one primary action: create deck, add note, or study another deck. The caught-up state includes session counts.
- Deck selection shows name plus New, Learning, and Review counts. Study starts the session; End session returns to the deck list and writes nothing.
- Add note is a small modal with model, deck, one field per model field, and tags. Save calls `Store::add_note`, reports card count, refreshes stats, and keeps the deck selected. Editing needs a new Store method and is out of scope here.
- Import and export are visible actions that delegate to existing CLI/format code with a path field. Import reports notes, cards, and media. Export reports the package path and refuses an existing target unless the user confirms overwrite.
- Scheduler choice is a labeled control, not a silent T toggle. Switching mid-session asks for confirmation and is marked session only. Anki treats the algorithm as a deck preset and keeps rescheduling a separate choice ([Deck Options](https://docs.ankiweb.net/manual/deck-options)).
- Stats shows current deck counts and today's session counts. Historical graphs are out of scope; `ranki stats` and MCP already provide collection counts, and this Store has no revlog query.
- GUI writes banner on external changes. Poll `Store::collection_modified`; when it changes, show `Collection changed outside Ranki. Refresh to see new cards`. Refresh after answers asks for confirmation because it ends the session. Do not merge concurrent writes.

## Prioritized changes

Acceptance criteria use the existing `MemoryStore` test harness in `app.rs` unless noted.

| Priority | Current location | Change | Acceptance criterion |
| --- | --- | --- | --- |
| must | `app.rs` `reload`, `review`, `progress` | Add three explicit empty states and a 0 due label | Fixtures for no decks, empty deck, and due=0 render distinct copy; progress never reads full at 0/0 |
| must | `app.rs` `answer`, `rating_buttons`, `ensure_current` | Compute interval previews from pure `Scheduler::schedule`; show a quiet post-grade status | For a fixed mock scheduler, button labels match the decision for each rating; one status line appears and no animation state is added |
| must | `app.rs` `ui`, `handle_key` | Route keys through a single action dispatcher; first 1 to 4 reveals; ignore keys in text fields; set initial focus | Keyboard-only test reveals, grades, and returns focus; a focused text field receives digits instead of grading |
| must | `app.rs` `toggle_scheduler`, `review_header`, `hint_bar` | Replace silent T toggle with confirmed, labeled session-only choice | T opens confirmation; cancel leaves `scheduler_kind` unchanged; header text names active scheduler |
| must | `app.rs` `set_error`, `review` | Replace persistent label with toast queue | Store error shows toast; toast disappears after 5 s or dismiss; next success clears any remaining toast |
| must | `app.rs` `new`, `reload` plus `Store::collection_modified` | Poll external modification and show refresh banner | Mock modification bump raises banner within 1 s; refresh reloads decks and stats; no silent merge |
| must | `app.rs` `shown_at`, `answer` | Track visible focused time and cap at 60 s | Mock clock with unfocused interval records only focused milliseconds; 61 s records 60000 |
| must | `app.rs` `rating_buttons` | Fixed grid, 44 px targets, debounce | Two rating keys within 120 ms produce one review; button positions do not change when question text wraps |
| should | `app.rs` `answer` plus new Store primitive | Add one-step undo with Ctrl+Z | Undo restores prior card fields and removes last revlog in a mock; redo is absent |
| should | `app.rs` `review`, `progress`, `reset_queue` | Add End session and a summary panel | Last answer shows answered, again count, and elapsed time; End session returns to decks without a write |
| should | `app.rs` `sidebar` plus `Store::add_note` | Add note modal with model and deck fields | Save calls `add_note` with entered fields and tags; stats update; empty-field save shows inline error |
| should | `crates/bin/src/main.rs`, `ranki_ui::Options`, `app.rs` | Pass import/export callbacks into UI | Import shows counts and refreshes queue; export refuses existing path unless confirmed |
| should | `app.rs` `refresh_stats`, `sidebar` | Add deck stats panel | Panel matches `Store::stats` for selected deck and session counters; no new Store query needed |
| should | `app.rs` `hint_bar`, `review_header` | Move hints next to controls and add `?` shortcut sheet | Every visible action has a key label or tooltip; `?` opens and closes the sheet |
| nice | `app.rs` `rating_buttons`, `theme.rs` `visuals` | Color-code ratings and define hover/press styles | Unit test checks contrast constants; manual pass confirms no layout shift or motion |
| nice | `card_view.rs` `render` | Mark available sound refs with a replay affordance when an audio callback exists | Missing media stays labeled missing; no audio dependency added in this worktree |

## Do not add

- Card flip animation, confetti, or auto-advance timers.
- Note browser, tag manager, template editor, or full note editing.
- Persistent scheduler presets, rescheduling, or algorithm parameter editing.
- Swipe gestures, touch layout, whiteboard, or image occlusion.
- Keyboard rebinding UI or a command palette.
- Sync, shared decks, AI card generation, or decks from the internet.
- Audio recording or new media decoding dependencies.
- Automatic merge of MCP or AnkiConnect writes into an open card.
- A directory tree or nested subdecks beyond the existing flat deck list.

## Sources

Internal files read: `crates/ui/src/app.rs`, `crates/ui/src/card_view.rs`, `crates/ui/src/lib.rs`, `crates/ui/src/theme.rs`, `crates/core/src/{lib,fsrs,sm2,shared}.rs`, `crates/model/src/lib.rs`, `crates/storage/src/{lib,schema}.rs`, `crates/bin/src/{main,connect,mcp}.rs`, `docs/architecture.md`, `docs/mcp-design.md`, `README.md`.

External links are inline above. Research date: 2026-11-22.
