use gpui::AppContext as _;
use std::collections::{HashMap, VecDeque};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use ranki_model::Store as _;
use ranki_model::{
    Card, CardQueue, CardTemplate, CardType, Deck, DeckStats, Id, NewNote,
    Note, NoteKind, NoteModel, ReviewDecision, StoreError,
};

use crate::Options;

use super::{
    DeckRow, EmptyKind, MaskShape, MouseButton, NormalizedRect, OcclusionDrag,
    OcclusionMask, OcclusionState, Phase, UiAction, View, build_deck_rows,
    collect_tags, compute_visible_deck_rows, deck_meta, due_total,
    duration_label, empty_copy, empty_kind, filter_decks, focused_delta,
    forecast_bar_height, forecast_total, model_index, models_with_stock,
    next_due_deck, next_view, prepare_deck_rows, queue_counts, refreshed_card,
    session_progress, sum_leaf_counts, validate_draft, validate_occlusion,
};
use gpui::{SharedString, point, px, size};

struct ModalStore;

impl ranki_model::Store for ModalStore {
    fn decks(&self) -> Result<Vec<Deck>, StoreError> {
        Ok(vec![Deck {
            id: 1,
            name: String::from("Default"),
            description: String::new(),
        }])
    }

    fn create_deck(&mut self, name: &str) -> Result<Deck, StoreError> {
        if name == "taken" {
            return Err(StoreError::InvalidData(String::from(
                "deck already exists: taken",
            )));
        }
        Ok(Deck {
            id: 2,
            name: name.to_owned(),
            description: String::new(),
        })
    }

    fn models(&self) -> Result<Vec<NoteModel>, StoreError> {
        Ok(ranki_model::stock_models())
    }

    fn find_cards(&self, _: &str) -> Result<Vec<Id>, StoreError> {
        Ok(Vec::new())
    }

    fn due_cards(
        &self,
        _: Option<Id>,
        _: usize,
        _: i64,
    ) -> Result<Vec<Card>, StoreError> {
        Ok(Vec::new())
    }

    fn card(&self, _: Id) -> Result<Option<Card>, StoreError> {
        Ok(None)
    }

    fn note(&self, _: Id) -> Result<Option<Note>, StoreError> {
        Ok(None)
    }

    fn add_note(&mut self, _: &NewNote) -> Result<Vec<Id>, StoreError> {
        Ok(Vec::new())
    }

    fn apply_review(&mut self, _: &ReviewDecision) -> Result<Id, StoreError> {
        Ok(0)
    }

    fn stats(
        &self,
        _: Option<Id>,
        _: i64,
    ) -> Result<ranki_model::DeckStats, StoreError> {
        Ok(ranki_model::DeckStats::default())
    }

    fn collection_modified(&self) -> Result<i64, StoreError> {
        Ok(0)
    }

    fn collection_created(&self) -> Result<i64, StoreError> {
        Ok(0)
    }
}

struct BrowseStore {
    decks: Vec<Deck>,
    models: Vec<NoteModel>,
    cards: HashMap<Id, Card>,
    notes: HashMap<Id, Note>,
    ids: Vec<Id>,
}

impl ranki_model::Store for BrowseStore {
    fn decks(&self) -> Result<Vec<Deck>, StoreError> {
        Ok(self.decks.clone())
    }

    fn models(&self) -> Result<Vec<NoteModel>, StoreError> {
        Ok(self.models.clone())
    }

    fn find_cards(&self, _: &str) -> Result<Vec<Id>, StoreError> {
        Ok(self.ids.clone())
    }

    fn card(&self, id: Id) -> Result<Option<Card>, StoreError> {
        Ok(self.cards.get(&id).cloned())
    }

    fn note(&self, id: Id) -> Result<Option<Note>, StoreError> {
        Ok(self.notes.get(&id).cloned())
    }

    fn create_deck(&mut self, name: &str) -> Result<Deck, StoreError> {
        ModalStore.create_deck(name)
    }

    fn due_cards(
        &self,
        _: Option<Id>,
        _: usize,
        _: i64,
    ) -> Result<Vec<Card>, StoreError> {
        Ok(Vec::new())
    }

    fn add_note(&mut self, _: &NewNote) -> Result<Vec<Id>, StoreError> {
        Ok(Vec::new())
    }

    fn apply_review(&mut self, _: &ReviewDecision) -> Result<Id, StoreError> {
        Ok(0)
    }

    fn stats(
        &self,
        _: Option<Id>,
        _: i64,
    ) -> Result<ranki_model::DeckStats, StoreError> {
        Ok(ranki_model::DeckStats::default())
    }

    fn collection_modified(&self) -> Result<i64, StoreError> {
        Ok(0)
    }

    fn collection_created(&self) -> Result<i64, StoreError> {
        Ok(0)
    }
}

fn browser_test_row(front: String) -> crate::browser_view::BrowserRow {
    let card = Card {
        id: 1,
        note_id: 1,
        deck_id: 1,
        ordinal: 0,
        modified: 0,
        usn: 0,
        card_type: CardType::New,
        queue: CardQueue::New,
        due: 0,
        interval: 0,
        factor: 0,
        reps: 0,
        lapses: 0,
        left: 0,
        original_due: 0,
        original_deck_id: 0,
        flags: 0,
        data: String::new(),
    };
    let note = Note {
        id: 1,
        guid: String::new(),
        model_id: 0,
        modified: 0,
        usn: 0,
        tags: Vec::new(),
        fields: vec![front, String::from("Answer!")],
        sort_field: String::new(),
        checksum: 0,
        flags: 0,
        data: String::new(),
    };
    let decks = [Deck {
        id: 1,
        name: String::from("Default"),
        description: String::new(),
    }];
    crate::browser_view::browser_row(
        &card,
        &note,
        &decks,
        &ranki_model::stock_models(),
    )
    .expect("browser row")
}

#[gpui::test]
fn browse_panels_fill_the_window_for_any_card(cx: &mut gpui::TestAppContext) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(size(px(1280.), px(800.)));

    let mut panel_heights = |front: &str| {
        cx.update_entity(&root, |root, cx| {
            root.view = View::Browse;
            root.browse.rows = vec![browser_test_row(front.to_owned())];
            root.browse.selected = Some(1);
            cx.notify();
        });
        cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
            root.clone()
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        (
            cx.debug_bounds("browse-table").expect("table").size.height,
            cx.debug_bounds("browse-detail")
                .expect("detail")
                .size
                .height,
        )
    };

    let short = panel_heights("Question?");
    let long = panel_heights(&"word ".repeat(300));
    assert_eq!(short, long);
    assert!(
        short.0 > px(600.),
        "table should fill the window: {short:?}"
    );
    assert!(
        short.1 > px(600.),
        "preview should fill the window: {short:?}"
    );
}

#[gpui::test]
fn browse_loads_cards_past_the_first_page(cx: &mut gpui::TestAppContext) {
    let mut store = BrowseStore {
        decks: vec![
            Deck {
                id: 10,
                name: String::from("WorldFlags"),
                description: String::new(),
            },
            Deck {
                id: 11,
                name: String::from("personal"),
                description: String::new(),
            },
        ],
        models: ranki_model::stock_models(),
        cards: HashMap::new(),
        notes: HashMap::new(),
        ids: Vec::new(),
    };
    for (deck_id, count) in [(10, 201), (11, 1)] {
        for index in 0..count {
            let id = i64::try_from(store.ids.len() + 1).expect("id");
            store.ids.push(id);
            let _ = store.cards.insert(
                id,
                Card {
                    id,
                    note_id: id,
                    deck_id,
                    ordinal: 0,
                    modified: 0,
                    usn: 0,
                    card_type: CardType::New,
                    queue: CardQueue::New,
                    due: 0,
                    interval: 0,
                    factor: 0,
                    reps: 0,
                    lapses: 0,
                    left: 0,
                    original_due: 0,
                    original_deck_id: 0,
                    flags: 0,
                    data: String::new(),
                },
            );
            let _ = store.notes.insert(
                id,
                Note {
                    id,
                    guid: String::new(),
                    model_id: 0,
                    modified: 0,
                    usn: 0,
                    tags: Vec::new(),
                    fields: vec![format!("Question {index}"), "Answer".into()],
                    sort_field: String::new(),
                    checksum: 0,
                    flags: 0,
                    data: String::new(),
                },
            );
        }
    }
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(store),
                options: Options::default(),
                save_settings: Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.update_entity(&root, super::Root::refresh_browse);
    cx.update_entity(&root, |root, _| {
        assert_eq!(root.browse.rows.len(), 202);
        let groups = root
            .browse
            .entries
            .iter()
            .filter_map(|entry| match entry {
                crate::browser_view::BrowseEntry::Group(group) => {
                    Some((group.path.as_str(), group.count))
                }
                crate::browser_view::BrowseEntry::Card { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(groups, [("WorldFlags", 201), ("personal", 1)]);
    });
}
#[gpui::test]
fn create_deck_modal_closes_only_on_outside_click(
    cx: &mut gpui::TestAppContext,
) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.run_until_parked();
    cx.dispatch_action(super::CreateDeck);
    cx.run_until_parked();
    assert!(cx.update(|_, app| root.read(app).create_modal));

    let center = cx.update(|window, _| {
        let size = window.viewport_size();
        point(
            px(f32::from(size.width) / 2.),
            px(f32::from(size.height) / 2.),
        )
    });
    cx.simulate_click(center, gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(
        cx.update(|_, app| root.read(app).create_modal),
        "clicking inside the modal must keep it open"
    );

    cx.simulate_click(point(px(4.), px(4.)), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(
        !cx.update(|_, app| root.read(app).create_modal),
        "clicking outside the modal must close it"
    );
}
#[gpui::test]
fn escape_closes_the_create_deck_modal(cx: &mut gpui::TestAppContext) {
    cx.update(|app| app.bind_keys(super::key_bindings()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.run_until_parked();
    cx.dispatch_action(super::CreateDeck);
    cx.run_until_parked();
    assert!(cx.update(|_, app| root.read(app).create_modal));

    cx.simulate_keystrokes("escape");

    assert!(!cx.update(|_, app| root.read(app).create_modal));
}
#[gpui::test]
fn create_deck_error_lives_by_the_input_until_the_name_changes(
    cx: &mut gpui::TestAppContext,
) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.run_until_parked();

    cx.update(|window, app| {
        root.update(app, |root, cx| {
            root.open_create_modal(window, cx);
            root.create_field.update(cx, |field, cx| {
                field.set_text("taken", cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|window, app| {
        root.update(app, |root, cx| root.create_deck(window, cx));
    });
    cx.run_until_parked();

    let error = cx.update(|_, app| root.read(app).create_error.clone());
    assert_eq!(
        error.as_deref(),
        Some("Could not create deck: invalid data: deck already exists: taken")
    );
    assert!(cx.update(|_, app| root.read(app).toast.is_none()));

    cx.update(|_, app| {
        root.update(app, |root, cx| {
            root.create_field.update(cx, |field, cx| {
                field.set_text("taken again", cx);
            });
        });
    });
    cx.run_until_parked();
    assert!(cx.update(|_, app| root.read(app).create_error.is_none()));
}

#[gpui::test]
fn dragging_the_divider_resizes_the_rail_between_icons_and_the_cap(
    cx: &mut gpui::TestAppContext,
) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let press = |cx: &mut gpui::VisualTestContext| {
        let bar = cx.debug_bounds("nav-divider").expect("divider bounds");
        assert_eq!(bar.size.width, px(crate::theme::NAV_DIVIDER_WIDTH));
        bar.center()
    };
    let width = |cx: &mut gpui::VisualTestContext| {
        cx.read_entity(&root, |root, _| root.nav_width)
    };

    let start = press(cx);
    assert_eq!(width(cx), crate::theme::NAV_WIDTH);
    let middle = crate::theme::NAV_DIVIDER_WIDTH / 2.0;
    assert_eq!(start.x, px(crate::theme::NAV_WIDTH - middle));

    mouse_drag(
        cx,
        start,
        point(start.x + px(40.), start.y),
        gpui::MouseButton::Left,
    );
    assert_eq!(width(cx), crate::theme::NAV_WIDTH + 40.);

    let start = press(cx);
    mouse_drag(
        cx,
        start,
        point(start.x + px(400.), start.y),
        gpui::MouseButton::Left,
    );
    assert_eq!(width(cx), crate::theme::NAV_WIDTH_MAX);
    assert!(!cx.read_entity(&root, |root, _| root.nav_compact()));

    let start = press(cx);
    mouse_drag(cx, start, point(px(10.), start.y), gpui::MouseButton::Left);
    assert_eq!(width(cx), crate::theme::NAV_WIDTH_MIN);
    assert!(cx.read_entity(&root, |root, _| root.nav_compact()));
    assert_eq!(press(cx).x, px(crate::theme::NAV_WIDTH_MIN - middle));
}

#[gpui::test]
fn deck_picker_floats_above_the_form(cx: &mut gpui::TestAppContext) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.update_entity(&root, |root, cx| {
        root.view = super::View::AddCard;
        cx.notify();
    });
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let button = cx
        .debug_bounds("add-deck-picker")
        .expect("deck picker button bounds");

    let center = button.center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::none());
    cx.simulate_mouse_down(
        center,
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_up(
        center,
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let after = cx
        .debug_bounds("add-deck-picker")
        .expect("deck picker button bounds after opening");
    assert_eq!(button.origin, after.origin);
    assert_eq!(button.size, after.size);

    let popup = cx
        .debug_bounds("add-deck-picker-popup")
        .expect("deck picker popup bounds");
    assert!(
        popup.size.width <= px(360.),
        "popup was {} wide",
        popup.size.width
    );
    assert!(popup.origin.y >= after.bottom());
}

#[gpui::test]
fn long_deck_names_are_truncated_in_the_picker(cx: &mut gpui::TestAppContext) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.update_entity(&root, |root, cx| {
        root.decks = vec![Deck {
            id: 1,
            name: String::from(
                "rbbk::operating_systems_and_networking_fundamentals",
            ),
            description: String::new(),
        }];
        root.stats = vec![ranki_model::DeckStats::default()];
        root.view = super::View::AddCard;
        root.add.deck_picker = true;
        root.add.deck_picker_anchor = point(px(270.), px(224.));
        cx.notify();
    });
    cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
        root.clone()
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let popup = cx
        .debug_bounds("add-deck-picker-popup")
        .expect("deck picker popup bounds");
    let name = cx
        .debug_bounds("add-deck-picker-name")
        .expect("deck picker name bounds");
    let due = cx
        .debug_bounds("add-deck-picker-due")
        .expect("deck picker due bounds");

    assert!(
        name.origin.x + name.size.width <= due.origin.x,
        "name overflowed into the due count"
    );
    assert!(
        due.origin.x + due.size.width <= popup.origin.x + popup.size.width,
        "due count overflowed the popup"
    );
}
#[gpui::test]
fn deck_picker_clicks_do_not_reach_the_editor(cx: &mut gpui::TestAppContext) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.update_entity(&root, |root, cx| {
        root.view = super::View::AddCard;
        root.seed_fields("untouched", cx);
        root.add.deck_picker = true;
        root.add.deck_picker_anchor = point(px(270.), px(224.));
        cx.notify();
    });
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let row = cx
        .debug_bounds("add-deck-0")
        .expect("first deck row bounds");
    let bold = cx.debug_bounds("add-format-B").expect("bold button bounds");
    assert!(
        row.intersects(&bold),
        "row {row:?} must overlap bold {bold:?}"
    );
    let click = point(
        px(f32::midpoint(
            f32::from(row.origin.x.max(bold.origin.x)),
            f32::from(row.bottom_right().x.min(bold.bottom_right().x)),
        )),
        px(f32::midpoint(
            f32::from(row.origin.y.max(bold.origin.y)),
            f32::from(row.bottom_right().y.min(bold.bottom_right().y)),
        )),
    );
    cx.simulate_mouse_move(click, None, gpui::Modifiers::none());
    cx.simulate_mouse_down(
        click,
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_up(
        click,
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    cx.run_until_parked();

    let (text, picker_open) = cx.read_entity(&root, |root, cx| {
        (root.add.fields[0].read(cx).text(), root.add.deck_picker)
    });
    assert!(!picker_open, "selecting a deck must close the picker");
    assert_eq!(text, "untouched");
}

fn click_test_point(
    cx: &mut gpui::VisualTestContext,
    at: gpui::Point<gpui::Pixels>,
) {
    cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
    cx.simulate_mouse_down(
        at,
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_up(at, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

fn mouse_drag(
    cx: &mut gpui::VisualTestContext,
    from: gpui::Point<gpui::Pixels>,
    to: gpui::Point<gpui::Pixels>,
    button: gpui::MouseButton,
) {
    cx.simulate_mouse_move(from, None, gpui::Modifiers::none());
    cx.simulate_mouse_down(from, button, gpui::Modifiers::none());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_move(to, Some(button), gpui::Modifiers::none());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_up(to, button, gpui::Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

#[gpui::test]
fn occlusion_canvas_moves_resizes_and_removes_masks(
    cx: &mut gpui::TestAppContext,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("media directory");
    image::RgbaImage::new(200, 100)
        .save(media.join("diagram.png"))
        .expect("test image");
    let options = Options {
        media_dir: media,
        ..Options::default()
    };
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options,
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.update_entity(&root, |root, cx| {
        root.view = super::View::AddCard;
        let occlusion = root
            .models
            .iter()
            .position(ranki_model::NoteModel::is_image_occlusion)
            .expect("stock occlusion");
        root.select_model(occlusion, cx);
        root.add.occlusion.image = Some(String::from("diagram.png"));
        root.add.occlusion.masks = vec![OcclusionMask {
            shape: MaskShape::Rect,
            rect: NormalizedRect {
                left: 0.2,
                top: 0.2,
                width: 0.4,
                height: 0.4,
            },
        }];
        cx.notify();
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let bounds = cx.read_entity(&root, |root, _| {
        root.add.occlusion.bounds.expect("canvas bounds")
    });
    let at = |x: f32, y: f32| {
        point(
            px(f32::from(bounds.origin.x) + f32::from(bounds.size.width) * x),
            px(f32::from(bounds.origin.y) + f32::from(bounds.size.height) * y),
        )
    };

    mouse_drag(cx, at(0.3, 0.3), at(0.4, 0.35), gpui::MouseButton::Left);
    let moved =
        cx.read_entity(&root, |root, _| root.add.occlusion.masks[0].rect);
    assert!((moved.left - 0.3).abs() < 1e-5, "{moved:?}");
    assert!((moved.top - 0.25).abs() < 1e-5, "{moved:?}");
    assert_eq!(
        cx.read_entity(&root, |root, _| root.add.occlusion.masks.len()),
        1
    );

    mouse_drag(cx, at(0.35, 0.3), at(0.25, 0.2), gpui::MouseButton::Right);
    let resized =
        cx.read_entity(&root, |root, _| root.add.occlusion.masks[0].rect);
    assert!((resized.left - 0.25).abs() < 1e-5, "{resized:?}");
    assert!((resized.top - 0.2).abs() < 1e-5, "{resized:?}");
    assert!((resized.width - 0.45).abs() < 1e-5, "{resized:?}");
    assert_eq!(
        cx.read_entity(&root, |root, _| root.add.occlusion.masks.len()),
        1
    );

    let click = at(0.475, 0.425);
    cx.simulate_mouse_move(click, None, gpui::Modifiers::none());
    cx.simulate_mouse_down(
        click,
        gpui::MouseButton::Right,
        gpui::Modifiers::none(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_up(
        click,
        gpui::MouseButton::Right,
        gpui::Modifiers::none(),
    );
    cx.run_until_parked();
    assert_eq!(
        cx.read_entity(&root, |root, _| root.add.occlusion.masks.len()),
        0
    );
}

#[gpui::test]
fn deck_picker_dismisses_on_outside_click(cx: &mut gpui::TestAppContext) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(px(1280.), px(800.)));
    cx.update_entity(&root, |root, cx| {
        root.view = super::View::AddCard;
        cx.notify();
    });
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let button = cx
        .debug_bounds("add-deck-picker")
        .expect("deck picker button bounds")
        .center();

    click_test_point(cx, button);
    assert!(cx.read_entity(&root, |root, _| root.add.deck_picker));

    click_test_point(cx, point(px(1100.), px(600.)));
    assert!(!cx.read_entity(&root, |root, _| root.add.deck_picker));

    click_test_point(cx, button);
    assert!(cx.read_entity(&root, |root, _| root.add.deck_picker));

    click_test_point(cx, button);
    assert!(
        !cx.read_entity(&root, |root, _| root.add.deck_picker),
        "the dismissing press must not reopen the picker"
    );
}

#[gpui::test]
fn escape_closes_the_deck_picker(cx: &mut gpui::TestAppContext) {
    cx.update(|app| app.bind_keys(super::key_bindings()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(px(1280.), px(800.)));
    cx.update_entity(&root, |root, cx| {
        root.view = super::View::AddCard;
        root.add.deck_picker = true;
        cx.notify();
    });
    cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
        root.clone()
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    cx.simulate_keystrokes("escape");

    let state =
        cx.read_entity(&root, |root, _| (root.view, root.add.deck_picker));
    assert_eq!(state.0, super::View::AddCard);
    assert!(!state.1);
}
#[gpui::test]
fn deck_picker_closes_when_the_form_scrolls(cx: &mut gpui::TestAppContext) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(px(1280.), px(800.)));
    cx.update_entity(&root, |root, cx| {
        root.view = super::View::AddCard;
        root.add.deck_picker = true;
        root.add.deck_picker_anchor = point(px(270.), px(224.));
        cx.notify();
    });
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let popup = cx
        .debug_bounds("add-deck-picker-popup")
        .expect("deck picker popup bounds");
    let scroll = |cx: &mut gpui::VisualTestContext, at| {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: at,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-20.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.run_until_parked();
    };

    scroll(
        cx,
        point(popup.origin.x + px(160.), popup.origin.y + px(70.)),
    );
    assert!(
        cx.read_entity(&root, |root, _| root.add.deck_picker),
        "scrolling the deck list must keep the picker open"
    );

    scroll(cx, point(px(1100.), px(600.)));
    assert!(!cx.read_entity(&root, |root, _| root.add.deck_picker));
}

#[gpui::test]
fn deck_scroll_animates_toward_the_wheel_target(cx: &mut gpui::TestAppContext) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(size(px(1280.), px(800.)));
    cx.update_entity(&root, |root, cx| {
        root.decks = (0..200i64)
            .map(|index| Deck {
                id: index.saturating_add(1),
                name: format!("Deck {index:03}"),
                description: String::new(),
            })
            .collect();
        root.stats = vec![DeckStats::default(); root.decks.len()];
        root.rebuild_deck_rows();
        cx.notify();
    });
    cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
        root.clone()
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(
        cx.debug_bounds("deck-row-Deck 199").is_none(),
        "offscreen decks must not be built"
    );
    let row = cx
        .debug_bounds("deck-row-Deck 000")
        .expect("first deck row")
        .center();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: row,
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-240.))),
        modifiers: gpui::Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });

    let mut target =
        cx.read_entity(&root, |root, _| root.decks_scroll.target());
    let offset = cx.read_entity(&root, |root, _| root.decks_scroll.offset());
    assert!(target >= 240.0, "wheel step should move the target");
    assert!(offset < 20.0, "the wheel must not jump to the target");

    cx.update_entity(&root, |_, cx| cx.notify());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let first_frame =
        cx.read_entity(&root, |root, _| root.decks_scroll.offset());
    assert!(
        first_frame > 0.0 && first_frame < target,
        "one frame should be partway through the transition"
    );

    cx.update_entity(&root, |root, cx| {
        root.options.reduce_motion = Some(true);
        cx.notify();
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(
        !cx.read_entity(&root, |root, _| root.decks_scroll.animating()),
        "reduced motion must stop the animation loop"
    );
    cx.update_entity(&root, |root, cx| {
        root.options.reduce_motion = Some(false);
        root.decks_scroll.wheel(240.0);
        cx.notify();
    });
    target = cx.read_entity(&root, |root, _| root.decks_scroll.target());

    cx.update_entity(&root, |root, _| root.decks_scroll.settle());
    let settled = cx.read_entity(&root, |root, _| root.decks_scroll.offset());
    assert_eq!(
        settled, target,
        "the animation should settle exactly ({settled} vs {target})"
    );

    cx.update_entity(&root, |root, cx| {
        root.decks_scroll
            .handle()
            .set_offset(point(px(0.), px(-20_000.)));
        cx.notify();
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(
        cx.debug_bounds("deck-row-Deck 199").is_some(),
        "the last deck must appear when scrolled to the bottom"
    );
}

#[test]
fn session_progress_never_fills_an_empty_queue() {
    assert_eq!(session_progress(0, 0), 0.0);
    assert_eq!(session_progress(0, 20), 0.0);
    assert_eq!(session_progress(10, 20), 0.5);
    assert_eq!(session_progress(20, 20), 1.0);
    assert_eq!(session_progress(21, 20), 1.0);
}

#[gpui::test]
fn repeated_hard_answers_leave_progress_unchanged(
    cx: &mut gpui::TestAppContext,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("collection.anki2");
    let mut store =
        ranki_storage::Collection::create(&path).expect("collection");
    let _ = store
        .add_note(&NewNote {
            model_id: 1,
            deck_id: 1,
            fields: vec![String::from("Question?"), String::from("Answer!")],
            tags: Vec::new(),
        })
        .expect("note");
    let options = Options {
        collection: path,
        ..Options::default()
    };
    let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(store),
                options,
                save_settings,
            },
            window,
            cx,
        )
    });
    cx.update_entity(&root, super::Root::study_all);

    let mut grade = |rating| {
        cx.update_entity(&root, |root, cx| {
            root.review.phase = Phase::Answer;
            root.review.last_grade_at = None;
            root.grade(rating, cx);
        });
        cx.read_entity(&root, |root, _| {
            (root.progress_fraction(), root.review.answered)
        })
    };
    assert_eq!(grade(ranki_model::Rating::Hard), (0.0, 1));
    assert_eq!(grade(ranki_model::Rating::Hard), (0.0, 2));
    assert_eq!(grade(ranki_model::Rating::Easy), (1.0, 3));
}

#[gpui::test]
fn finishing_a_session_starts_one_quiet_sync(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().expect("temp dir");
    let collection = dir.path().join("collection.anki2");
    let mut store =
        ranki_storage::Collection::create(&collection).expect("collection");
    for front in ["First?", "Second?"] {
        let _ = store
            .add_note(&NewNote {
                model_id: 1,
                deck_id: 1,
                fields: vec![front.to_owned(), String::from("Answer!")],
                tags: Vec::new(),
            })
            .expect("note");
    }
    let options = Options {
        collection,
        sync: ranki_model::SyncSettings {
            enabled: true,
            ..Default::default()
        },
        ..Options::default()
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let (root, cx) = cx.add_window_view(|window, cx| {
        let mut root = super::Root::new(
            super::Launch {
                store: Box::new(store),
                options,
                save_settings: Arc::new(|_| Ok(())),
            },
            window,
            cx,
        );
        root.sync.signed_in = true;
        root.sync_run = Arc::new(move |_, _, _, _| {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok(ranki_sync::SyncOutcome::Uploaded)
        });
        root
    });
    cx.update_entity(&root, super::Root::study_all);

    cx.update_entity(&root, |root, cx| {
        root.review.phase = Phase::Answer;
        root.review.last_grade_at = None;
        root.grade(ranki_model::Rating::Easy, cx);
    });
    cx.run_until_parked();
    assert_eq!(calls.load(Ordering::Relaxed), 0);

    cx.update_entity(&root, |root, cx| {
        root.review.phase = Phase::Answer;
        root.review.last_grade_at = None;
        root.grade(ranki_model::Rating::Easy, cx);
    });
    cx.run_until_parked();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    cx.read_entity(&root, |root, _| assert!(root.toast.is_none()));
}
#[test]
fn duration_labels_read_like_a_session_summary() {
    assert_eq!(duration_label(Duration::from_secs(0)), "0s");
    assert_eq!(duration_label(Duration::from_secs(59)), "59s");
    assert_eq!(duration_label(Duration::from_secs(60)), "1m 0s");
    assert_eq!(duration_label(Duration::from_secs(125)), "2m 5s");
    assert_eq!(duration_label(Duration::from_mins(65)), "1h 5m");
}

#[test]
fn queue_counts_include_the_current_card() {
    let make = |id: Id, kind| Card {
        id,
        note_id: 1,
        deck_id: 1,
        ordinal: 0,
        modified: 0,
        usn: 0,
        card_type: kind,
        queue: ranki_model::CardQueue::New,
        due: 0,
        interval: 0,
        factor: 0,
        reps: 0,
        lapses: 0,
        left: 0,
        original_due: 0,
        original_deck_id: 0,
        flags: 0,
        data: String::new(),
    };
    let mut queue = VecDeque::new();
    queue.push_back(make(2, CardType::Learning));
    queue.push_back(make(3, CardType::Review));
    let mut pending = VecDeque::new();
    pending.push_back(make(4, CardType::Learning));
    let stats = queue_counts(Some(&make(1, CardType::New)), &queue, &pending);
    assert_eq!(stats.new_cards, 1);
    assert_eq!(stats.learning_cards, 2);
    assert_eq!(stats.review_cards, 1);
    assert_eq!(stats.total_cards, 4);
    let empty = queue_counts(None, &VecDeque::new(), &VecDeque::new());
    assert_eq!(empty, ranki_model::DeckStats::default());
}

#[test]
fn review_uses_anki_fsrs_defaults() {
    let scheduler = super::from_settings(&Options::default());
    assert_eq!(scheduler.params(), &ranki_core::FsrsParams::default());
    assert_eq!(scheduler.preset().desired_retention, 0.9);
}

#[test]
fn grading_uses_the_stored_card_revision() {
    let current = Card {
        id: 7,
        note_id: 1,
        deck_id: 1,
        ordinal: 0,
        modified: 100,
        usn: -1,
        card_type: CardType::Learning,
        queue: CardQueue::Learning,
        due: 101_000,
        interval: -60,
        factor: 2_500,
        reps: 1,
        lapses: 0,
        left: 1_001,
        original_due: 0,
        original_deck_id: 0,
        flags: 0,
        data: String::new(),
    };
    let mut saved = current.clone();
    saved.usn = 4;
    assert_eq!(refreshed_card(&current, |_| Some(saved.clone())).usn, 4);
    assert_eq!(refreshed_card(&current, |_| None), current);
}

#[test]
fn config_options_keep_their_defaults() {
    let options = Options::default();
    assert_eq!(options.font_size, 16.0);
    assert_eq!(options.desired_retention, 0.9);
    assert_eq!(options.review_batch_size, 20);
    assert!(options.show_keyboard_hints);
}

#[expect(
    clippy::too_many_arguments,
    reason = "test fixture with one argument per queue"
)]
fn deck(
    id: i64,
    name: &str,
    new: usize,
    learning: usize,
    review: usize,
) -> (Deck, ranki_model::DeckStats) {
    (
        Deck {
            id,
            name: name.to_owned(),
            description: String::new(),
        },
        ranki_model::DeckStats {
            new_cards: new,
            learning_cards: learning,
            review_cards: review,
            total_cards: new + learning + review,
        },
    )
}

#[test]
fn deck_tree_builds_stable_indented_rows() {
    let decks = [
        deck(1, "A", 1, 1, 4),
        deck(2, "A::B", 2, 0, 3),
        deck(3, "Z", 0, 0, 0),
    ];
    let stats = decks.iter().map(|(_, stats)| *stats).collect::<Vec<_>>();
    let rows = build_deck_rows(
        &decks.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(),
        &stats,
        &std::collections::HashSet::default(),
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].path, "A");
    assert!(!rows[0].virtual_group);
    assert_eq!(rows[0].stats.review_cards, 4);
    assert_eq!(rows[1].path, "Z");
}

#[test]
fn expanded_tree_shows_children() {
    let decks = [deck(1, "A", 0, 0, 0), deck(2, "A::B", 1, 2, 3)];
    let stats = decks.iter().map(|(_, stats)| *stats).collect::<Vec<_>>();
    let mut expanded = std::collections::HashSet::new();
    let _ = expanded.insert(String::from("A"));
    let rows = build_deck_rows(
        &decks.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(),
        &stats,
        &expanded,
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].depth, 1);
    assert_eq!(rows[1].name, "B");
}
#[test]
fn review_keys_are_bound_where_they_are_advertised() {
    let mut keymap = gpui::Keymap::default();
    keymap.add_bindings(super::key_bindings());
    let root = [gpui::KeyContext::parse("ranki").expect("ranki context")];
    for (key, expected) in [("enter", 1), ("r", 1), ("f5", 1)] {
        let stroke = gpui::Keystroke::parse(key).expect("keystroke");
        let (bindings, _) = keymap.bindings_for_input(&[stroke], &root);
        assert_eq!(bindings.len(), expected, "{key}");
    }
}

#[gpui::test]
fn space_ends_a_completed_session(cx: &mut gpui::TestAppContext) {
    cx.update(|app| app.bind_keys(super::key_bindings()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: std::sync::Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.update_entity(&root, |root, cx| {
        root.view = super::View::Review;
        root.review.current = None;
        root.review.answered = 1;
        cx.notify();
    });
    cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
        root.clone()
    });
    cx.simulate_keystrokes("enter");
    assert_eq!(
        cx.read_entity(&root, |root, _| root.view),
        super::View::Review
    );
    cx.simulate_keystrokes("space");
    assert_eq!(
        cx.read_entity(&root, |root, _| root.view),
        super::View::Decks
    );
}

#[gpui::test]
fn space_ends_a_caught_up_session(cx: &mut gpui::TestAppContext) {
    cx.update(|app| app.bind_keys(super::key_bindings()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.update_entity(&root, |root, cx| root.study_deck(0, cx));
    cx.run_until_parked();
    cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
        root.clone()
    });
    assert_eq!(cx.update(|_, cx| root.read(cx).view), View::Review);
    assert!(cx.update(|_, cx| root.read(cx).review.current.is_none()));

    cx.simulate_keystrokes("space");
    assert_eq!(cx.update(|_, cx| root.read(cx).view), View::Decks);
}

#[gpui::test]
fn space_after_grading_waits_for_debounce(cx: &mut gpui::TestAppContext) {
    cx.update(|app| app.bind_keys(super::key_bindings()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.update_entity(&root, |root, cx| {
        root.view = View::Review;
        root.review.current = None;
        root.review.answered = 1;
        root.review.last_grade_at = Some(std::time::Instant::now());
        cx.notify();
    });
    cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
        root.clone()
    });
    cx.simulate_keystrokes("space");
    assert_eq!(cx.update(|_, cx| root.read(cx).view), View::Review);

    cx.update_entity(&root, |root, _| root.review.last_grade_at = None);
    cx.simulate_keystrokes("space");
    assert_eq!(cx.update(|_, cx| root.read(cx).view), View::Decks);
}

#[test]
fn tab_moves_focus_in_and_out_of_text_inputs() {
    let mut keymap = gpui::Keymap::default();
    keymap.add_bindings(super::key_bindings());
    let root = [gpui::KeyContext::parse("ranki").expect("ranki context")];
    let typing = [
        gpui::KeyContext::parse("ranki").expect("ranki context"),
        gpui::KeyContext::parse("ranki_input").expect("input context"),
    ];
    let dialog = [
        gpui::KeyContext::parse("ranki").expect("ranki context"),
        gpui::KeyContext::parse("ranki_dialog").expect("dialog context"),
    ];
    for key in ["tab", "shift-tab"] {
        let stroke = gpui::Keystroke::parse(key).expect("keystroke");
        let (from_root, _) =
            keymap.bindings_for_input(std::slice::from_ref(&stroke), &root);
        let (from_input, _) =
            keymap.bindings_for_input(std::slice::from_ref(&stroke), &typing);
        assert_eq!(from_root.len(), 1, "{key}");
        assert_eq!(from_input.len(), 1, "{key}");
        let (in_dialog, _) =
            keymap.bindings_for_input(std::slice::from_ref(&stroke), &dialog);
        assert!(in_dialog.is_empty(), "{key}");
    }
}

#[test]
fn study_bindings_stay_out_of_text_input() {
    let mut keymap = gpui::Keymap::default();
    keymap.add_bindings(super::key_bindings());
    let typing = [
        gpui::KeyContext::parse("ranki").expect("ranki context"),
        gpui::KeyContext::parse("ranki_input").expect("input context"),
    ];
    let elsewhere = [gpui::KeyContext::parse("ranki").expect("ranki context")];
    let keystroke = gpui::Keystroke::parse("a").expect("keystroke");
    let (in_input, _) =
        keymap.bindings_for_input(std::slice::from_ref(&keystroke), &typing);
    assert!(in_input.is_empty());
    let (outside, _) =
        keymap.bindings_for_input(std::slice::from_ref(&keystroke), &elsewhere);
    assert_eq!(outside.len(), 1);
}

#[gpui::test]
fn typing_in_settings_input_does_not_change_view(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|app| app.bind_keys(super::key_bindings()));
    let mut options = Options::default();
    options.sync.enabled = true;
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options,
                save_settings: Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.update_entity(&root, |root, cx| {
        root.view = super::View::Settings;
        cx.notify();
    });
    let focus = cx.read_entity(&root, |root, cx| {
        root.sync.username.read(cx).focus_handle().clone()
    });
    cx.update(|window, _| window.focus(&focus));
    cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
        root.clone()
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    cx.simulate_keystrokes("a");

    let (view, text) = cx.read_entity(&root, |root, cx| {
        (root.view, root.sync.username.read(cx).text())
    });
    assert_eq!(view, super::View::Settings);
    assert_eq!(text, "a");
}

#[test]
fn theme_has_no_number_shortcuts() {
    let mut keymap = gpui::Keymap::default();
    keymap.add_bindings(super::key_bindings());
    let settings = [
        gpui::KeyContext::parse("ranki").expect("ranki context"),
        gpui::KeyContext::parse("ranki_settings").expect("settings context"),
    ];
    for key in ["1", "2", "3"] {
        let keystroke = gpui::Keystroke::parse(key).expect("keystroke");
        let (in_settings, _) = keymap
            .bindings_for_input(std::slice::from_ref(&keystroke), &settings);
        assert!(in_settings.is_empty());
    }
}

#[test]
fn statistics_key_is_only_bound_in_statistics() {
    let mut keymap = gpui::Keymap::default();
    keymap.add_bindings(super::key_bindings());
    let keystroke = gpui::Keystroke::parse("t").expect("keystroke");
    let stats = [gpui::KeyContext::parse(View::Statistics.key_context())
        .expect("stats context")];
    let review = [gpui::KeyContext::parse(View::Review.key_context())
        .expect("review context")];
    let typing = [
        gpui::KeyContext::parse(View::Statistics.key_context())
            .expect("stats context"),
        gpui::KeyContext::parse("ranki_input").expect("input context"),
    ];
    let (on_stats, _) =
        keymap.bindings_for_input(std::slice::from_ref(&keystroke), &stats);
    let (in_review, _) =
        keymap.bindings_for_input(std::slice::from_ref(&keystroke), &review);
    let (in_input, _) =
        keymap.bindings_for_input(std::slice::from_ref(&keystroke), &typing);
    assert_eq!(on_stats.len(), 1);
    assert!(in_review.is_empty());
    assert!(in_input.is_empty());
}

#[test]
fn colon_only_deck_names_build_one_row() {
    let decks = [deck(1, ":", 0, 0, 1)];
    let stats = decks.iter().map(|(_, stats)| *stats).collect::<Vec<_>>();
    let rows = build_deck_rows(
        &decks.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(),
        &stats,
        &std::collections::HashSet::default(),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path, ":");
}

#[test]
fn child_listed_before_parent_replaces_the_virtual_group() {
    let decks = [deck(1, "A::B", 1, 0, 0), deck(2, "A", 0, 0, 0)];
    let stats = decks.iter().map(|(_, stats)| *stats).collect::<Vec<_>>();
    let mut expanded = std::collections::HashSet::new();
    let _ = expanded.insert(String::from("A"));
    let rows = build_deck_rows(
        &decks.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(),
        &stats,
        &expanded,
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].path, "A");
    assert!(!rows[0].virtual_group);
    assert_eq!(rows[1].path, "A::B");
}
#[test]
fn leaf_totals_keep_cards_owned_by_a_parent_deck() {
    let decks = [deck(1, "A", 1, 0, 1), deck(2, "A::B", 1, 0, 0)];
    let stats = [
        ranki_model::DeckStats {
            new_cards: 2,
            total_cards: 2,
            ..Default::default()
        },
        ranki_model::DeckStats {
            new_cards: 1,
            total_cards: 1,
            ..Default::default()
        },
    ];
    let deck_list = decks
        .iter()
        .map(|(deck, _)| deck.clone())
        .collect::<Vec<_>>();
    let total = super::leaf_deck_totals(&deck_list, &stats);
    assert_eq!(due_total(total), 2);
    assert_eq!(total.total_cards, 2);
}

#[test]
fn explicit_parent_stats_are_not_added_to_children_twice() {
    let decks = [deck(1, "A", 1, 0, 0), deck(2, "A::B", 1, 0, 0)];
    let stats = [
        ranki_model::DeckStats {
            new_cards: 2,
            total_cards: 2,
            ..Default::default()
        },
        ranki_model::DeckStats {
            new_cards: 1,
            total_cards: 1,
            ..Default::default()
        },
    ];
    let rows = build_deck_rows(
        &decks
            .iter()
            .map(|(deck, _)| deck.clone())
            .collect::<Vec<_>>(),
        &stats,
        &std::collections::HashSet::default(),
    );
    assert_eq!(rows[0].stats.new_cards, 2);
    assert_eq!(rows[0].stats.total_cards, 2);
}

#[test]
fn leaf_sum_does_not_double_count_parents() {
    let decks = [deck(1, "A", 1, 1, 1), deck(2, "A::B", 2, 0, 3)];
    let stats = decks.iter().map(|(_, stats)| *stats).collect::<Vec<_>>();
    let mut expanded = std::collections::HashSet::new();
    let _ = expanded.insert(String::from("A"));
    let rows = build_deck_rows(
        &decks.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(),
        &stats,
        &expanded,
    );
    let total = sum_leaf_counts(&rows);
    assert_eq!(due_total(total), 5);
}

fn row(id: i64, path: &str, depth: usize) -> DeckRow {
    DeckRow {
        id: Some(id),
        path: path.into(),
        name: path.rsplit("::").next().unwrap_or(path).into(),
        depth,
        stats: ranki_model::DeckStats::default(),
        virtual_group: false,
        has_children: false,
        expanded: false,
        meta: String::new(),
        child_count: 0,
        deck_index: None,
        key: SharedString::default(),
        chevron_key: SharedString::default(),
        study_key: SharedString::default(),
        menu_key: SharedString::default(),
    }
}

#[test]
fn search_keeps_ancestors_and_sorts_stable() {
    let rows = vec![
        row(1, "A", 0),
        row(2, "A::B", 1),
        row(3, "A::B::C", 2),
        row(4, "A::X", 1),
        row(5, "Z", 0),
    ];
    let paths = |query: &str| {
        filter_decks(&rows, query)
            .iter()
            .map(|row| row.path.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(paths("b"), ["A", "A::B", "A::B::C"]);
    assert_eq!(paths("x"), ["A", "A::X"]);
    assert_eq!(paths("nothing"), Vec::<String>::new());
    assert_eq!(paths("").len(), 5);
}

#[test]
fn draft_validation_blocks_empty_and_cloze_without_marker() {
    let (deck, _) = deck(1, "Default", 0, 0, 0);
    let model = NoteModel {
        id: 1,
        name: "Cloze".into(),
        fields: vec!["Text".into()],
        templates: vec![CardTemplate::default()],
        css: String::new(),
        sort_field_index: 0,
        kind: NoteKind::Cloze,
        original_stock_kind: None,
        field_tags: vec![],
        template_ordinals: vec![0],
        raw_config: None,
    };
    assert!(
        validate_draft(&deck, &model, &[String::new()]).contains_key("fields")
    );
    assert!(
        validate_draft(&deck, &model, &[String::from("plain text")])
            .contains_key("fields")
    );
    assert!(
        validate_draft(&deck, &model, &[String::from("{{c1::answer}}")])
            .is_empty()
    );
}

#[test]
fn empty_states_are_distinct() {
    assert_eq!(empty_kind(0, 0), EmptyKind::NoDecks);
    assert_eq!(empty_kind(1, 1), EmptyKind::None);
    assert_eq!(empty_kind(1, 0), EmptyKind::NoMatch);
    assert_eq!(empty_kind(1, 3), EmptyKind::None);
    assert_eq!(empty_copy(EmptyKind::NoDecks).0, "No decks yet");
}

#[test]
fn view_transitions_return_to_decks() {
    assert_eq!(next_view(View::Review, View::Decks), View::Decks);
    assert_eq!(next_view(View::Decks, View::AddCard), View::AddCard);
}

#[test]
fn phase_and_action_are_copy_comparable() {
    assert_ne!(Phase::Question, Phase::Answer);
    assert_ne!(UiAction::OpenAdd, UiAction::OpenDecks);
}

#[test]
fn next_due_deck_skips_selection() {
    let decks = [deck(1, "A", 0, 0, 0), deck(2, "B", 1, 0, 0)];
    let stats = decks.iter().map(|(_, s)| *s).collect::<Vec<_>>();
    let list = decks
        .iter()
        .map(|(deck, _)| deck.clone())
        .collect::<Vec<_>>();
    assert_eq!(next_due_deck(&list, &stats, Some(1)), None);
}

#[test]
fn focused_time_is_capped_and_ignores_blur() {
    let elapsed = std::time::Duration::from_secs(61);
    assert_eq!(
        focused_delta(elapsed, false, std::time::Duration::ZERO),
        std::time::Duration::ZERO
    );
    assert_eq!(
        focused_delta(elapsed, true, std::time::Duration::ZERO),
        std::time::Duration::from_secs(60)
    );
}

#[test]
fn deck_meta_counts_children_and_cards() {
    let row = DeckRow {
        id: None,
        path: "Japanese".into(),
        name: "Japanese".into(),
        depth: 0,
        stats: ranki_model::DeckStats {
            total_cards: 1284,
            ..Default::default()
        },
        virtual_group: true,
        has_children: true,
        expanded: false,
        meta: String::new(),
        child_count: 0,
        deck_index: None,
        key: SharedString::default(),
        chevron_key: SharedString::default(),
        study_key: SharedString::default(),
        menu_key: SharedString::default(),
    };
    let child = DeckRow {
        id: Some(2),
        path: "Japanese::Vocabulary N5".into(),
        name: "Vocabulary N5".into(),
        depth: 1,
        stats: ranki_model::DeckStats::default(),
        virtual_group: false,
        has_children: false,
        expanded: false,
        meta: String::new(),
        child_count: 0,
        deck_index: None,
        key: SharedString::default(),
        chevron_key: SharedString::default(),
        study_key: SharedString::default(),
        menu_key: SharedString::default(),
    };
    assert_eq!(
        deck_meta(&[row.clone(), child], &row),
        "1 subdeck · 1284 cards"
    );
    let solo = DeckRow {
        path: String::from("Solo"),
        stats: ranki_model::DeckStats {
            total_cards: 1,
            ..Default::default()
        },
        ..row
    };
    assert_eq!(deck_meta(std::slice::from_ref(&solo), &solo), "1 card");
}

#[test]
fn deck_rows_cache_display_metadata() {
    let entries = [deck(1, "A", 0, 0, 2), deck(2, "A::B", 0, 0, 3)];
    let decks = entries
        .iter()
        .map(|(deck, _)| deck.clone())
        .collect::<Vec<_>>();
    let stats = entries.iter().map(|(_, stats)| *stats).collect::<Vec<_>>();
    let expanded = ["A".to_owned()].into_iter().collect();
    let mut rows = build_deck_rows(&decks, &stats, &expanded);
    prepare_deck_rows(&decks, &mut rows);
    assert_eq!(rows[0].meta, "1 subdeck · 2 cards");
    assert_eq!(rows[0].child_count, 1);
    assert_eq!(rows[0].deck_index, Some(0));
    assert_eq!(rows[1].meta, "3 cards");
    assert_eq!(rows[1].child_count, 0);
    assert_eq!(rows[1].deck_index, Some(1));

    let filtered = compute_visible_deck_rows(&decks, &stats, &rows, "B");
    assert_eq!(filtered[0].path, "A");
    assert_eq!(filtered[0].meta, rows[0].meta);
    assert_eq!(filtered[0].child_count, rows[0].child_count);
    assert_eq!(filtered[0].deck_index, rows[0].deck_index);
    assert_eq!(filtered[1].meta, rows[1].meta);
}

#[test]
fn deck_element_keys_stay_stable_through_tree_changes() {
    let prepare = |decks: &[Deck],
                   stats: &[ranki_model::DeckStats],
                   expanded: &[&str]| {
        let expanded = expanded
            .iter()
            .map(|path| (*path).to_owned())
            .collect::<std::collections::HashSet<_>>();
        let mut rows = build_deck_rows(decks, stats, &expanded);
        prepare_deck_rows(decks, &mut rows);
        rows
    };
    let check_unique = |rows: &[DeckRow]| {
        let mut paths = std::collections::HashSet::new();
        let mut chevrons = std::collections::HashSet::new();
        let mut studies = std::collections::HashSet::new();
        let mut menus = std::collections::HashSet::new();
        for row in rows {
            assert!(paths.insert(row.key.as_str()), "duplicate row key");
            assert!(
                chevrons.insert(row.chevron_key.as_str()),
                "duplicate chevron key"
            );
            assert!(
                studies.insert(row.study_key.as_str()),
                "duplicate study key"
            );
            if row.deck_index.is_some() {
                assert!(
                    menus.insert(row.menu_key.as_str()),
                    "duplicate menu key"
                );
            }
        }
    };
    let key = |rows: &[DeckRow], path: &str| {
        rows.iter()
            .find(|row| row.path == path)
            .expect("deck row")
            .key
            .to_string()
    };

    let entries = [
        deck(1, "Topic", 0, 0, 2),
        deck(2, "Topic::Child", 0, 0, 3),
        deck(3, "Zed", 0, 0, 1),
    ];
    let decks = entries
        .iter()
        .map(|(deck, _)| deck.clone())
        .collect::<Vec<_>>();
    let stats = entries.iter().map(|(_, stats)| *stats).collect::<Vec<_>>();
    let collapsed = prepare(&decks, &stats, &[]);
    let expanded = prepare(&decks, &stats, &["Topic"]);
    check_unique(&collapsed);
    check_unique(&expanded);
    assert_eq!(key(&collapsed, "Topic"), key(&expanded, "Topic"));
    assert_eq!(key(&collapsed, "Zed"), key(&expanded, "Zed"));

    let visible = compute_visible_deck_rows(&decks, &stats, &expanded, "Child");
    check_unique(&visible);
    let paths = visible
        .iter()
        .map(|row| row.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["Topic", "Topic::Child"]);
    assert_eq!(key(&visible, "Topic"), key(&expanded, "Topic"));
    assert_eq!(
        key(&visible, "Topic::Child"),
        key(&expanded, "Topic::Child")
    );

    let renamed_entries = [
        deck(1, "Topic", 0, 0, 2),
        deck(2, "Topic::Renamed", 0, 0, 3),
        deck(3, "Zed", 0, 0, 1),
    ];
    let renamed_decks = renamed_entries
        .iter()
        .map(|(deck, _)| deck.clone())
        .collect::<Vec<_>>();
    let renamed_stats = renamed_entries
        .iter()
        .map(|(_, stats)| *stats)
        .collect::<Vec<_>>();
    let renamed = prepare(&renamed_decks, &renamed_stats, &["Topic"]);
    check_unique(&renamed);
    assert_eq!(key(&renamed, "Zed"), key(&expanded, "Zed"));
    assert_eq!(key(&renamed, "Topic"), key(&expanded, "Topic"));
    assert_ne!(
        key(&renamed, "Topic::Renamed"),
        key(&expanded, "Topic::Child")
    );
    assert_eq!(key(&renamed, "Topic::Renamed"), "deck-row-Topic::Renamed");
    assert_eq!(
        renamed
            .iter()
            .find(|row| row.path == "Topic::Renamed")
            .expect("renamed row")
            .menu_key
            .as_str(),
        "decks-menu-2"
    );
}

#[test]
fn due_totals_add_all_queues() {
    let (_, stats) = deck(1, "A", 2, 3, 4);
    assert_eq!(due_total(stats), 9);
}

#[test]
fn forecast_bars_scale_to_the_busiest_day() {
    let days = [5, 2, 0, 3, 1, 0, 4];
    assert_eq!(forecast_total(&days), 15);
    assert_eq!(forecast_total(&[]), 0);
    assert_eq!(forecast_bar_height(0, 5), 6.0);
    assert_eq!(forecast_bar_height(2, 0), 6.0);
    assert_eq!(forecast_bar_height(1, 4), 28.5);
    assert_eq!(forecast_bar_height(4, 4), 60.0);
}

#[test]
fn stock_types_fill_in_missing_note_models() {
    let models = models_with_stock(Vec::new());
    let names = models
        .iter()
        .map(|model| model.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "Basic",
            "Basic (and reversed card)",
            "Cloze",
            "Image Occlusion"
        ]
    );
    assert!(models.iter().all(|model| model.id == 0));
    let installed = ranki_model::stock_models()
        .into_iter()
        .next()
        .map(|mut model| {
            model.id = 7;
            model
        })
        .expect("stock basic");
    let merged = models_with_stock(vec![installed]);
    assert_eq!(merged.len(), 4);
    assert_eq!(merged[0].id, 7);
}

#[test]
fn occlusion_drag_creates_one_mask_on_release() {
    let mut state = OcclusionState {
        bounds: Some(gpui::Bounds {
            origin: point(px(100.), px(50.)),
            size: size(px(200.), px(100.)),
        }),
        ..OcclusionState::default()
    };
    let start = state.pointer((125., 75.)).expect("start point");
    let end = state.pointer((175., 125.)).expect("end point");
    state.drag = Some(OcclusionDrag::Draw { start, end });
    let rect = state.drag_rect().expect("drag preview");
    assert_eq!((rect.left, rect.top), (0.125, 0.25));
    assert_eq!((rect.width, rect.height), (0.25, 0.5));
    state.finish_drag();
    assert_eq!(state.masks.len(), 1);
    assert_eq!(state.drag, None);
}

#[test]
fn left_drag_moves_the_mask_under_the_press() {
    let mut state = OcclusionState {
        image: Some("diagram.png".into()),
        bounds: Some(gpui::Bounds {
            origin: point(px(100.), px(50.)),
            size: size(px(200.), px(100.)),
        }),
        masks: vec![OcclusionMask {
            shape: MaskShape::Rect,
            rect: NormalizedRect {
                left: 0.2,
                top: 0.2,
                width: 0.3,
                height: 0.3,
            },
        }],
        ..OcclusionState::default()
    };
    assert!(state.press((160., 80.), MouseButton::Left));
    assert!(matches!(
        state.drag,
        Some(OcclusionDrag::Move { index: 0, .. })
    ));
    assert!(state.drag_to((180., 95.)));
    let moved = state.masks[0].rect;
    assert_eq!((moved.left, moved.width, moved.height), (0.3, 0.3, 0.3));
    assert!((moved.top - 0.35).abs() < 1e-5, "{moved:?}");
    assert!(!state.finish_drag());
    assert_eq!(state.masks.len(), 1);
}

#[test]
fn right_drag_resizes_from_the_pressed_corner() {
    let mut state = OcclusionState {
        image: Some("diagram.png".into()),
        bounds: Some(gpui::Bounds {
            origin: point(px(100.), px(50.)),
            size: size(px(200.), px(100.)),
        }),
        masks: vec![OcclusionMask {
            shape: MaskShape::Rect,
            rect: NormalizedRect {
                left: 0.2,
                top: 0.2,
                width: 0.4,
                height: 0.4,
            },
        }],
        ..OcclusionState::default()
    };
    assert!(state.press((150., 75.), MouseButton::Right));
    assert!(matches!(
        state.drag,
        Some(OcclusionDrag::Resize { index: 0, .. })
    ));
    assert!(state.drag_to((120., 60.)));
    let resized = state.masks[0].rect;
    assert!((resized.left - 0.1).abs() < 1e-5, "{resized:?}");
    assert!((resized.top - 0.1).abs() < 1e-5, "{resized:?}");
    assert!((resized.width - 0.5).abs() < 1e-5, "{resized:?}");
    assert!((resized.height - 0.5).abs() < 1e-5, "{resized:?}");
    assert!(state.drag_to((220., 115.)));
    let clamped = state.masks[0].rect;
    assert!((clamped.left - 0.59).abs() < 1e-5, "{clamped:?}");
    assert!((clamped.top - 0.59).abs() < 1e-5, "{clamped:?}");
    assert!((clamped.width - 0.01).abs() < 1e-5, "{clamped:?}");
    assert!((clamped.height - 0.01).abs() < 1e-5, "{clamped:?}");
    assert!(!state.finish_drag());
    assert_eq!(state.masks.len(), 1);
}

#[test]
fn right_click_without_dragging_removes_the_mask() {
    let mut state = OcclusionState {
        image: Some("diagram.png".into()),
        bounds: Some(gpui::Bounds {
            origin: point(px(100.), px(50.)),
            size: size(px(200.), px(100.)),
        }),
        masks: vec![OcclusionMask {
            shape: MaskShape::Rect,
            rect: NormalizedRect {
                left: 0.2,
                top: 0.2,
                width: 0.4,
                height: 0.4,
            },
        }],
        ..OcclusionState::default()
    };
    assert!(state.press((150., 75.), MouseButton::Right));
    assert!(!state.drag_to((151., 75.)));
    assert_eq!(state.masks[0].rect.left, 0.2);
    assert!(state.finish_drag());
    assert_eq!(state.masks.len(), 0);
    assert_eq!(state.drag, None);
}

#[test]
fn occlusion_pointer_uses_the_fitted_image_area() {
    let state = OcclusionState {
        image_size: Some((200.0, 100.0)),
        bounds: Some(gpui::Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(400.), px(400.)),
        }),
        masks: vec![OcclusionMask {
            shape: MaskShape::Rect,
            rect: NormalizedRect {
                left: 0.0,
                top: 0.0,
                width: 0.2,
                height: 1.0,
            },
        }],
        ..OcclusionState::default()
    };
    let rect = state.image_rect().expect("fitted image");
    assert_eq!((rect.top, rect.height), (0.25, 0.5));
    assert_eq!(state.pointer((200.0, 100.0)), Some((0.5, 0.0)));
    assert_eq!(state.pointer((200.0, 300.0)), Some((0.5, 1.0)));
    let outside = state.raw_pointer((40.0, 50.0)).expect("raw point");
    assert!(outside.1 < 0.0);
    assert_eq!(state.mask_at(state.pointer((40.0, 50.0)).unwrap()), Some(0));
    assert_eq!(state.mask_at(outside), None);
}

#[test]
fn mask_hit_test_finds_the_newest_overlap_and_clears_with_the_image() {
    let mask = |left, top| OcclusionMask {
        shape: MaskShape::Rect,
        rect: NormalizedRect {
            left,
            top,
            width: 0.4,
            height: 0.4,
        },
    };
    let mut state = OcclusionState {
        image: Some("diagram.png".into()),
        masks: vec![mask(0.1, 0.1), mask(0.3, 0.3)],
        ..OcclusionState::default()
    };
    assert_eq!(state.mask_at((0.2, 0.2)), Some(0));
    assert_eq!(state.mask_at((0.35, 0.35)), Some(1));
    assert_eq!(state.mask_at((0.9, 0.9)), None);
    assert!(state.remove_mask(1));
    assert_eq!(state.masks.len(), 1);
    assert!(!state.remove_mask(5));
    state.clear_image();
    assert_eq!(state.image, None);
    assert_eq!(state.masks.len(), 0);
}

#[test]
fn occlusion_validation_needs_an_image_and_a_mask() {
    let deck = Deck {
        id: 1,
        name: "Default".into(),
        description: String::new(),
    };
    let mut model = ranki_model::StockModelKind::ImageOcclusion.definition();
    model.id = 5;
    let errors = validate_occlusion(&deck, &model, None, &[]);
    assert_eq!(
        errors.get("occlusion").map(String::as_str),
        Some("Upload an image first")
    );
    assert_eq!(
        errors.get("masks").map(String::as_str),
        Some("Draw at least one mask on the image")
    );
    let mask = crate::occlusion::default_mask(0);
    let errors = validate_occlusion(&deck, &model, Some("a.png"), &[mask]);
    assert!(errors.is_empty(), "{errors:?}");
    let placeholder = models_with_stock(Vec::new())
        .into_iter()
        .find(ranki_model::NoteModel::is_image_occlusion)
        .expect("stock occlusion");
    let errors =
        validate_occlusion(&deck, &placeholder, Some("a.png"), &[mask]);
    assert_eq!(
        errors.get("model").map(String::as_str),
        Some("This note type is not in the collection")
    );
}

#[gpui::test]
fn saving_an_occlusion_note_clears_the_chosen_image(
    cx: &mut gpui::TestAppContext,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("collection.anki2");
    let store =
        Box::new(ranki_storage::Collection::create(&path).expect("collection"));
    let options = Options {
        collection: path,
        ..Options::default()
    };
    let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store,
                options,
                save_settings,
            },
            window,
            cx,
        )
    });
    cx.update_entity(&root, |root, cx| {
        root.view = super::View::AddCard;
        let occlusion = root
            .models
            .iter()
            .position(ranki_model::NoteModel::is_image_occlusion)
            .expect("stock occlusion");
        root.select_model(occlusion, cx);
        root.add.occlusion.image = Some(String::from("diagram.png"));
        root.add.occlusion.masks = vec![crate::occlusion::default_mask(0)];
        cx.notify();
    });
    assert!(
        cx.read_entity(&root, |root, _| root.add.occlusion.image.is_some())
    );

    cx.update_entity(&root, super::Root::save_note);

    let state = cx.read_entity(&root, |root, _| {
        (
            root.add.occlusion.image.clone(),
            root.add.occlusion.masks.len(),
        )
    });
    assert_eq!(state.0, None);
    assert_eq!(state.1, 0);
}

#[gpui::test]
fn image_button_removes_the_chosen_image(cx: &mut gpui::TestAppContext) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(size(px(1280.), px(800.)));
    cx.update_entity(&root, |root, cx| {
        root.view = super::View::AddCard;
        let occlusion = root
            .models
            .iter()
            .position(ranki_model::NoteModel::is_image_occlusion)
            .expect("stock occlusion");
        root.add.model_index = occlusion;
        root.add.occlusion.image = Some(String::from("diagram.png"));
        root.add.occlusion.masks = vec![crate::occlusion::default_mask(0)];
        cx.notify();
    });
    cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
        root.clone()
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let button = cx
        .debug_bounds("io-image")
        .expect("unified image button bounds");
    click_test_point(cx, button.center());

    let state = cx.read_entity(&root, |root, _| {
        (
            root.add.occlusion.image.clone(),
            root.add.occlusion.masks.len(),
        )
    });
    assert_eq!(state, (None, 0));
}

#[test]
fn installing_a_stock_type_keeps_its_tab_selected() {
    let mut installed = ranki_model::StockModelKind::Cloze.definition();
    installed.id = 77;
    let models = models_with_stock(vec![installed]);
    assert_eq!(model_index(&models, "Cloze"), Some(0));
    assert_eq!(models[0].id, 77);
    assert_eq!(model_index(&models, "cloze"), Some(0));
    assert_eq!(model_index(&models, "Nope"), None);
    assert_eq!(models.len(), 4);
    let missing = models_with_stock(Vec::new());
    assert_eq!(model_index(&missing, "Cloze"), Some(2));
    assert_eq!(missing[2].id, 0);
}

#[test]
fn tag_field_commits_words_into_chips() {
    let mut chips = Vec::new();
    assert_eq!(collect_tags(&mut chips, "japan"), "japan");
    assert_eq!(chips.len(), 0);
    assert_eq!(collect_tags(&mut chips, "japan "), "");
    assert_eq!(chips, ["japan"]);
    assert_eq!(collect_tags(&mut chips, "japan verb"), "verb");
    assert_eq!(chips, ["japan"]);
    assert_eq!(collect_tags(&mut chips, "verb "), "");
    assert_eq!(chips, ["japan", "verb"]);
    assert_eq!(collect_tags(&mut chips, " 	 "), "");
    assert_eq!(chips, ["japan", "verb"]);
}

#[test]
fn learn_ahead_pulls_learning_cards_within_twenty_minutes() {
    let now = 1_700_000_000;
    let card = |id, queue, due| Card {
        id,
        note_id: 0,
        deck_id: 0,
        ordinal: 0,
        modified: now,
        usn: -1,
        card_type: CardType::Learning,
        queue,
        due,
        interval: -600,
        factor: 2_500,
        reps: 1,
        lapses: 0,
        left: 1_001,
        original_due: 0,
        original_deck_id: 0,
        flags: 0,
        data: String::new(),
    };
    let mut pending = VecDeque::new();
    pending.push_back(card(1, CardQueue::Learning, (now + 10 * 60) * 1_000));
    pending.push_back(card(2, CardQueue::Review, (now + 60) * 1_000));
    pending.push_back(card(4, CardQueue::Learning, (now + 60) * 1_000));
    pending.push_back(card(3, CardQueue::Learning, (now + 21 * 60) * 1_000));
    let picked =
        super::Root::due_learning_card(&pending, now + super::LEARN_AHEAD_SECS);
    assert_eq!(picked.map(|card| card.id), Some(4));
}

#[gpui::test]
fn clicking_create_button_saves_the_deck(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("collection.anki2");
    let store =
        Box::new(ranki_storage::Collection::create(&path).expect("collection"));
    let options = Options {
        collection: path,
        ..Options::default()
    };
    let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store,
                options,
                save_settings,
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.update_entity(&root, |root, cx| {
        root.create_modal = true;
        root.create_field.update(cx, |field, cx| {
            field.set_text("Click Deck", cx);
        });
        cx.notify();
    });
    let focus = cx.read_entity(&root, |root, cx| {
        root.create_field.read(cx).focus_handle().clone()
    });
    cx.update(|window, _| window.focus(&focus));
    cx.draw(
        gpui::point(gpui::px(0.), gpui::px(0.)),
        gpui::size(gpui::px(1280.), gpui::px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let button = cx
        .debug_bounds("create-deck-primary")
        .expect("create button bounds");
    cx.simulate_mouse_move(button.center(), None, gpui::Modifiers::none());
    cx.simulate_mouse_down(
        button.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_up(
        button.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );

    cx.run_until_parked();
    let names = cx.read_entity(&root, |root, _| {
        root.decks
            .iter()
            .map(|deck| deck.name.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(names, ["Click Deck"]);
}
#[gpui::test]
fn clicking_a_deck_row_studies_that_deck(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("collection.anki2");
    let mut store =
        ranki_storage::Collection::create(&path).expect("collection");
    let cards = store
        .add_note(&NewNote {
            model_id: 1,
            deck_id: 1,
            fields: vec![String::from("Question?"), String::from("Answer!")],
            tags: Vec::new(),
        })
        .expect("note");
    assert_eq!(cards.len(), 1);
    let options = Options {
        collection: path,
        ..Options::default()
    };
    let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(store),
                options,
                save_settings,
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let row = cx
        .debug_bounds("deck-row-Default")
        .expect("deck row bounds");
    cx.simulate_click(row.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    let state = cx.read_entity(&root, |root, _| {
        (
            root.view,
            root.selected,
            root.review.scope_all,
            root.review.session_total,
            root.review.current.is_some(),
        )
    });
    assert_eq!(state.0, View::Review);
    assert_eq!(state.1, Some(0));
    assert!(!state.2);
    assert_eq!(state.3, 1);
    assert!(state.4);
}

#[gpui::test]
fn space_reveals_after_clicking_a_deck_row(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("collection.anki2");
    let mut store =
        ranki_storage::Collection::create(&path).expect("collection");
    store
        .add_note(&NewNote {
            model_id: 1,
            deck_id: 1,
            fields: vec![String::from("Question?"), String::from("Answer!")],
            tags: Vec::new(),
        })
        .expect("note");
    let options = Options {
        collection: path,
        ..Options::default()
    };
    let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
    cx.update(|cx| cx.bind_keys(super::key_bindings()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(store),
                options,
                save_settings,
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let row = cx
        .debug_bounds("deck-row-Default")
        .expect("deck row bounds");
    cx.simulate_click(row.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    cx.simulate_keystrokes("space");

    let phase = cx.read_entity(&root, |root, _| root.review.phase);
    assert_eq!(phase, Phase::Answer);
}

#[gpui::test]
fn deck_row_controls_do_not_start_a_session(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("collection.anki2");
    let mut store =
        ranki_storage::Collection::create(&path).expect("collection");
    let _ = store.create_deck("Topic::Child").expect("deck");
    let _ = store
        .add_note(&NewNote {
            model_id: 1,
            deck_id: 1,
            fields: vec![String::from("Question?"), String::from("Answer!")],
            tags: Vec::new(),
        })
        .expect("note");
    let options = Options {
        collection: path,
        ..Options::default()
    };
    let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(store),
                options,
                save_settings,
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let chevron = cx
        .debug_bounds("decks-chevron-Topic")
        .expect("chevron bounds");
    cx.simulate_click(chevron.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    let (view, expanded) = cx.read_entity(&root, |root, _| {
        (root.view, root.expanded.contains("Topic"))
    });
    assert_eq!(view, View::Decks);
    assert!(expanded);

    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let menu = cx.debug_bounds("decks-menu-1").expect("menu bounds");
    cx.simulate_click(menu.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    let (view, menu_open) =
        cx.read_entity(&root, |root, _| (root.view, root.deck_menu.is_some()));
    assert_eq!(view, View::Decks);
    assert!(menu_open);
}
#[gpui::test]
fn right_clicking_a_deck_opens_its_actions_menu(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("collection.anki2");
    let mut store =
        ranki_storage::Collection::create(&path).expect("collection");
    let cards = store
        .add_note(&NewNote {
            model_id: 1,
            deck_id: 1,
            fields: vec![String::from("Question?"), String::from("Answer!")],
            tags: Vec::new(),
        })
        .expect("note");
    assert_eq!(cards.len(), 1);
    let options = Options {
        collection: path,
        ..Options::default()
    };
    let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(store),
                options,
                save_settings,
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let row = cx
        .debug_bounds("deck-row-Default")
        .expect("deck row bounds");
    cx.simulate_mouse_down(
        row.center(),
        gpui::MouseButton::Right,
        gpui::Modifiers::none(),
    );
    cx.simulate_mouse_up(
        row.center(),
        gpui::MouseButton::Right,
        gpui::Modifiers::none(),
    );
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let state = cx.read_entity(&root, |root, _| {
        (root.view, root.deck_menu.map(|menu| menu.id))
    });
    assert_eq!(state.0, View::Decks);
    assert_eq!(state.1, Some(1));
    assert!(cx.debug_bounds("deck-actions-menu").is_some());
}
#[gpui::test]
fn exporting_a_deck_asks_for_a_save_path(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("collection.anki2");
    let mut store =
        ranki_storage::Collection::create(&path).expect("collection");
    let _ = store
        .add_note(&NewNote {
            model_id: 1,
            deck_id: 1,
            fields: vec![String::from("Question?"), String::from("Answer!")],
            tags: Vec::new(),
        })
        .expect("note");
    let options = Options {
        collection: path,
        ..Options::default()
    };
    let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(store),
                options,
                save_settings,
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
        root.clone()
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.update_entity(&root, |root, cx| {
        root.deck_menu = Some(super::DeckMenu {
            id: 1,
            anchor: point(px(100.), px(100.)),
        });
        cx.notify();
    });
    cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
        root.clone()
    });

    let export = cx
        .debug_bounds("decks-export")
        .expect("deck export menu item bounds");
    cx.simulate_click(export.center(), gpui::Modifiers::none());

    assert!(cx.did_prompt_for_new_path());
    assert!(cx.read_entity(&root, |root, _| root.deck_menu.is_none()));
}

#[test]
fn export_file_name_is_safe_for_deck_paths() {
    assert_eq!(
        super::Root::export_file_name("Visual Deck"),
        "Visual Deck.apkg"
    );
    assert_eq!(
        super::Root::export_file_name("Language::French/1"),
        "Language - French_1.apkg"
    );
    assert_eq!(super::Root::export_file_name("::"), "deck.apkg");
}
#[gpui::test]
fn escape_closes_a_deck_actions_menu(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("collection.anki2");
    let mut store =
        ranki_storage::Collection::create(&path).expect("collection");
    let _ = store
        .add_note(&NewNote {
            model_id: 1,
            deck_id: 1,
            fields: vec![String::from("Question?"), String::from("Answer!")],
            tags: Vec::new(),
        })
        .expect("note");
    let options = Options {
        collection: path,
        ..Options::default()
    };
    let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
    cx.update(|cx| cx.bind_keys(super::key_bindings()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(store),
                options,
                save_settings,
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
        root.clone()
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.update_entity(&root, |root, cx| {
        root.deck_menu = Some(super::DeckMenu {
            id: 1,
            anchor: point(px(100.), px(100.)),
        });
        cx.notify();
    });
    cx.draw(point(px(0.), px(0.)), size(px(1280.), px(800.)), |_, _| {
        root.clone()
    });

    cx.simulate_keystrokes("escape");

    assert!(cx.read_entity(&root, |root, _| root.deck_menu.is_none()));
}
#[gpui::test]
fn clicking_the_rename_field_keeps_the_decks_view(
    cx: &mut gpui::TestAppContext,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("collection.anki2");
    let mut store =
        ranki_storage::Collection::create(&path).expect("collection");
    let _ = store
        .add_note(&NewNote {
            model_id: 1,
            deck_id: 1,
            fields: vec![String::from("Question?"), String::from("Answer!")],
            tags: Vec::new(),
        })
        .expect("note");
    let options = Options {
        collection: path,
        ..Options::default()
    };
    let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(store),
                options,
                save_settings,
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.update_entity(&root, |root, cx| {
        let deck = root.decks[0].clone();
        root.start_rename(&deck, cx);
    });
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let field = cx
        .debug_bounds("deck-rename-field")
        .expect("rename field bounds");
    cx.simulate_click(field.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    let (view, editing) =
        cx.read_entity(&root, |root, _| (root.view, root.deck_edit.is_some()));
    assert_eq!(view, View::Decks);
    assert!(editing);
}
#[gpui::test]
fn popup_actions_do_not_study_the_deck_below(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("collection.anki2");
    let mut store =
        ranki_storage::Collection::create(&path).expect("collection");
    for name in ["aa", "bb", "cc", "dd", "ee"] {
        let _ = store.create_deck(name).expect("deck");
    }
    let options = Options {
        collection: path,
        ..Options::default()
    };
    let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(store),
                options,
                save_settings,
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let row = cx.debug_bounds("deck-row-bb").expect("deck row bounds");
    let deck_id = cx.read_entity(&root, |root, _| root.decks[1].id);
    cx.update_entity(&root, |root, cx| {
        root.deck_menu = Some(super::DeckMenu {
            id: deck_id,
            anchor: point(row.center().x, row.origin.y - px(40.)),
        });
        cx.notify();
    });
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let overlap = |a: gpui::Bounds<gpui::Pixels>,
                   b: gpui::Bounds<gpui::Pixels>|
     -> Option<gpui::Point<gpui::Pixels>> {
        let left = a.origin.x.max(b.origin.x);
        let right = a.bottom_right().x.min(b.bottom_right().x);
        let top = a.origin.y.max(b.origin.y);
        let bottom = a.bottom_right().y.min(b.bottom_right().y);
        if left < right && top < bottom {
            Some(point(
                px(f32::midpoint(f32::from(left), f32::from(right))),
                px(f32::midpoint(f32::from(top), f32::from(bottom))),
            ))
        } else {
            None
        }
    };
    let rows = [
        "deck-row-aa",
        "deck-row-bb",
        "deck-row-cc",
        "deck-row-dd",
        "deck-row-ee",
    ];
    let remove = cx.debug_bounds("decks-remove").expect("remove bounds");
    let click = rows
        .iter()
        .filter_map(|selector| cx.debug_bounds(selector))
        .find_map(|bounds| overlap(remove, bounds))
        .expect("remove must overlap a deck row");
    click_test_point(cx, click);
    let (view, confirmation) =
        cx.read_entity(&root, |root, _| (root.view, root.confirmation.clone()));
    assert_eq!(view, View::Decks, "menu click must not study a deck");
    assert!(
        matches!(confirmation, Some(super::Confirmation::RemoveDeck { .. })),
        "menu click must open the remove confirmation"
    );

    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let confirm = cx.debug_bounds("confirm-primary").expect("confirm bounds");
    let click = rows
        .iter()
        .filter_map(|selector| cx.debug_bounds(selector))
        .find_map(|bounds| overlap(confirm, bounds))
        .expect("confirmation must overlap a deck row");
    click_test_point(cx, click);
    let (view, confirmation) =
        cx.read_entity(&root, |root, _| (root.view, root.confirmation.clone()));
    assert_eq!(view, View::Decks, "dialog click must not study a deck");
    assert!(
        confirmation.is_none(),
        "dialog click must still confirm the removal"
    );
}

fn recording_saver() -> (crate::SettingsSaver, Arc<Mutex<Option<Options>>>) {
    let latest = Arc::new(Mutex::new(None));
    let for_saver = Arc::clone(&latest);
    let saver: crate::SettingsSaver = Arc::new(move |settings| {
        *for_saver.lock().expect("settings lock") = Some(settings.clone());
        Ok(())
    });
    (saver, latest)
}

#[gpui::test]
fn saved_window_layout_is_restored(cx: &mut gpui::TestAppContext) {
    let mut options = Options::default();
    let state = &mut options.ui_state;
    state.view = ranki_model::UiView::Browse;
    state.previous_view = ranki_model::UiView::AddCard;
    state.theme = Some(ranki_model::ThemeChoice::Light);
    state.nav_width = Some(140.0);
    state.selected_deck = Some(1);
    state.expanded_decks = vec![String::from("Default")];
    state.deck_query = String::from("deck term");
    state.browse_query = String::from("card term");
    state.review_all_decks = true;

    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options,
                save_settings: Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    let restored = cx.read_entity(&root, |root, cx| {
        (
            root.view,
            root.previous_view,
            root.options.theme,
            root.nav_width,
            root.selected,
            root.expanded.contains("Default"),
            root.deck_search.read(cx).text(),
            root.browse.search.read(cx).text(),
            root.review.scope_all,
        )
    });
    assert_eq!(restored.0, View::Browse);
    assert_eq!(restored.1, View::AddCard);
    assert_eq!(restored.2, ranki_model::ThemeChoice::Light);
    assert_eq!(restored.3, 140.0);
    assert_eq!(restored.4, Some(0));
    assert!(restored.5);
    assert_eq!(restored.6, "deck term");
    assert_eq!(restored.7, "card term");
    assert!(restored.8);
}

#[gpui::test]
fn layout_changes_are_saved_and_restored(cx: &mut gpui::TestAppContext) {
    let (saver, latest) = recording_saver();
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: saver,
            },
            window,
            cx,
        )
    });
    cx.update(|window, cx| {
        root.update(cx, |root, cx| {
            root.dispatch(UiAction::OpenSettings, window, cx);
            root.toggle_deck(String::from("Topic"), cx);
            root.set_theme(ranki_model::ThemeChoice::Dark, cx);
        });
    });

    let recorded = latest
        .lock()
        .expect("settings lock")
        .clone()
        .expect("recorded settings");
    assert_eq!(recorded.ui_state.view, ranki_model::UiView::Settings);
    assert!(
        recorded
            .ui_state
            .expanded_decks
            .contains(&String::from("Topic"))
    );
    assert_eq!(
        recorded.ui_state.theme,
        Some(ranki_model::ThemeChoice::Dark)
    );

    let (restarted, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: recorded,
                save_settings: Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    let restored = cx.read_entity(&restarted, |root, _| {
        (
            root.view,
            root.options.theme,
            root.expanded.contains("Topic"),
        )
    });
    assert_eq!(restored.0, View::Settings);
    assert_eq!(restored.1, ranki_model::ThemeChoice::Dark);
    assert!(restored.2);
}

#[gpui::test]
fn search_queries_are_saved_on_edit(cx: &mut gpui::TestAppContext) {
    let (saver, latest) = recording_saver();
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: saver,
            },
            window,
            cx,
        )
    });
    cx.update_entity(&root, |root, cx| {
        root.deck_search
            .update(cx, |field, cx| field.insert_text("deck term", cx));
        root.browse
            .search
            .update(cx, |field, cx| field.insert_text("card term", cx));
    });
    cx.run_until_parked();

    let recorded = latest
        .lock()
        .expect("settings lock")
        .clone()
        .expect("recorded settings");
    assert_eq!(recorded.ui_state.deck_query, "deck term");
    assert_eq!(recorded.ui_state.browse_query, "card term");
}

#[gpui::test]
fn editing_a_browse_card_saves_the_add_card_view(
    cx: &mut gpui::TestAppContext,
) {
    let (saver, latest) = recording_saver();
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: saver,
            },
            window,
            cx,
        )
    });
    cx.update(|window, cx| {
        root.update(cx, |root, cx| {
            root.browse.rows =
                vec![browser_test_row(String::from("Question?"))];
            root.start_card_edit(1, window, cx);
        });
    });

    let recorded = latest
        .lock()
        .expect("settings lock")
        .clone()
        .expect("recorded settings");
    assert_eq!(recorded.ui_state.view, ranki_model::UiView::AddCard);
    assert_eq!(recorded.ui_state.previous_view, ranki_model::UiView::Browse);
}

#[gpui::test]
fn reloading_the_collection_saves_the_reset_view(
    cx: &mut gpui::TestAppContext,
) {
    let (saver, latest) = recording_saver();
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: saver,
            },
            window,
            cx,
        )
    });
    cx.update_entity(&root, |root, cx| {
        root.view = View::Statistics;
        root.reload_collection(cx);
    });

    let recorded = latest
        .lock()
        .expect("settings lock")
        .clone()
        .expect("recorded settings");
    assert_eq!(recorded.ui_state.view, ranki_model::UiView::Decks);
}

#[gpui::test]
fn color_picker_applies_markup_to_the_editor_field(
    cx: &mut gpui::TestAppContext,
) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.update(|window, cx| {
        root.update(cx, |root, cx| {
            root.view = View::AddCard;
            root.seed_fields("Paris", cx);
            let field = root.add.fields[0].clone();
            root.toggle_color_picker(
                crate::add_card_view::ColorTarget::Text,
                field.clone(),
                point(px(300.), px(360.)),
                window,
                cx,
            );
            assert!(root.add.color_picker.is_some());
            root.apply_color(
                crate::add_card_view::ColorTarget::Text,
                field,
                "#f38ba8",
                window,
                cx,
            );
            root.apply_color(
                crate::add_card_view::ColorTarget::Text,
                root.add.fields[0].clone(),
                "cba6f7",
                window,
                cx,
            );
            assert!(
                root.add.fields[0]
                    .read(cx)
                    .focus_handle()
                    .is_focused(window),
                "applying a color keeps the editor focused"
            );
        });
    });
    let (text, open) = cx.read_entity(&root, |root, cx| {
        (
            root.add.fields[0].read(cx).text(),
            root.add.color_picker.is_some(),
        )
    });
    assert_eq!(
        text,
        "Paris<span style=\"color: #f38ba8\"><span style=\"color: #cba6f7\"></span></span>"
    );
    assert!(!open, "applying a color closes the picker");
}

#[gpui::test]
fn scrolling_the_form_closes_the_color_picker(cx: &mut gpui::TestAppContext) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        super::Root::new(
            super::Launch {
                store: Box::new(ModalStore),
                options: Options::default(),
                save_settings: Arc::new(|_| Ok(())),
            },
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(px(1280.), px(800.)));
    cx.update(|window, cx| {
        root.update(cx, |root, cx| {
            root.view = View::AddCard;
            let field = root.add.fields[0].clone();
            root.toggle_color_picker(
                crate::add_card_view::ColorTarget::Text,
                field,
                point(px(300.), px(360.)),
                window,
                cx,
            );
        });
    });
    cx.draw(
        point(px(0.), px(0.)),
        gpui::size(px(1280.), px(800.)),
        |_, _| root.clone(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: point(px(1100.), px(600.)),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-20.))),
        modifiers: gpui::Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let (open, focused) = cx.update(|window, cx| {
        let state = root.read(cx);
        (
            state.add.color_picker.is_some(),
            state.focus.is_focused(window),
        )
    });
    assert!(!open, "scrolling the form closes the color picker");
    assert!(focused, "closing the picker restores root focus");
}
