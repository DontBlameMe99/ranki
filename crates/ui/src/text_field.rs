//! A small keyboard-first text field for GPUI 0.2.2 text input.

use std::{ops::Range, sync::Arc};

use gpui::{
    App, Bounds, Context, Element, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    Render, ShapedLine, SharedString, Style, TextRun, UTF16Selection,
    UnderlineStyle, Window, actions, div, fill, hsla, point, prelude::*, px,
    size,
};

/// Called after each edit so the owning view can refresh derived state.
pub type EditHook = Arc<dyn Fn(&mut App) + Send + Sync>;

actions!(
    text_field,
    [
        FieldBackspace,
        FieldDelete,
        FieldLeft,
        FieldRight,
        FieldSelectLeft,
        FieldSelectRight,
        FieldSelectAll,
        FieldHome,
        FieldEnd,
    ]
);

/// Binds the field's own key context so global study shortcuts stay out of text.
#[must_use]
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("backspace", FieldBackspace, Some("ranki_input")),
        KeyBinding::new("delete", FieldDelete, Some("ranki_input")),
        KeyBinding::new("left", FieldLeft, Some("ranki_input")),
        KeyBinding::new("right", FieldRight, Some("ranki_input")),
        KeyBinding::new("shift-left", FieldSelectLeft, Some("ranki_input")),
        KeyBinding::new("shift-right", FieldSelectRight, Some("ranki_input")),
        KeyBinding::new("cmd-a", FieldSelectAll, Some("ranki_input")),
        KeyBinding::new("ctrl-a", FieldSelectAll, Some("ranki_input")),
        KeyBinding::new("home", FieldHome, Some("ranki_input")),
        KeyBinding::new("end", FieldEnd, Some("ranki_input")),
    ]
}

/// State and platform text input for one field.
pub struct TextField {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    placeholder_color: Hsla,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    selecting: bool,
    edit_hook: Option<EditHook>,
}

/// Wraps `range` in `open`/`close` and returns the new text plus the caret
/// offset. An empty selection gets an empty tag pair with the caret inside.
#[must_use]
pub fn wrap_selection(
    content: &str,
    range: Range<usize>,
    tags: (&str, &str),
) -> (String, usize) {
    let (open, close) = tags;
    let start = range.start.min(content.len());
    let end = range.end.min(content.len()).max(start);
    let mut text = String::with_capacity(
        content.len().saturating_add(open.len() + close.len()),
    );
    text.push_str(content.get(..start).unwrap_or(""));
    text.push_str(open);
    text.push_str(content.get(start..end).unwrap_or(""));
    text.push_str(close);
    text.push_str(content.get(end..).unwrap_or(""));
    let cursor = if start == end {
        start.saturating_add(open.len())
    } else {
        end.saturating_add(open.len()).saturating_add(close.len())
    };
    (text, cursor)
}

/// Byte offset of a UTF-16 offset inside `text`.
fn utf16_offset_in(text: &str, offset: usize) -> usize {
    let mut utf16 = 0usize;
    for (index, character) in text.char_indices() {
        if utf16 >= offset {
            return index;
        }
        utf16 = utf16.saturating_add(character.len_utf16());
    }
    text.len()
}

impl TextField {
    #[must_use]
    pub fn new(
        cx: &mut Context<Self>,
        placeholder: impl Into<SharedString>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: String::new().into(),
            placeholder: placeholder.into(),
            placeholder_color: gpui::hsla(0., 0., 0.5, 1.0),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            selecting: false,
            edit_hook: None,
        }
    }

    #[must_use]
    pub fn text(&self) -> String {
        self.content.to_string()
    }

    pub fn set_text(
        &mut self,
        text: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.content = text.into();
        self.selected_range = self.content.len()..self.content.len();
        self.marked_range = None;
        self.notify_edit(cx);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_text("", cx);
    }

    /// Seeds an initial value without firing the edit hook.
    pub fn seed(
        &mut self,
        text: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.content = text.into();
        self.selected_range = self.content.len()..self.content.len();
        self.marked_range = None;
        cx.notify();
    }
    pub fn set_edit_hook(&mut self, hook: EditHook) {
        self.edit_hook = Some(hook);
    }

    /// Wraps the selection in markup, or drops an empty pair at the caret.
    pub fn wrap_markup(&mut self, tags: (&str, &str), cx: &mut Context<Self>) {
        let (text, cursor) =
            wrap_selection(&self.content, self.selected_range.clone(), tags);
        self.content = text.into();
        self.selected_range = cursor..cursor;
        self.notify_edit(cx);
        cx.notify();
    }

    /// Placeholder ink follows the palette so it keeps subtext contrast.
    pub fn set_placeholder_color(&mut self, color: Hsla) {
        self.placeholder_color = color;
    }

    fn notify_edit(&self, cx: &mut Context<Self>) {
        if let Some(hook) = self.edit_hook.as_ref() {
            hook(cx);
        }
    }

    pub const fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    const fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .char_indices()
            .rev()
            .find_map(|(index, _)| (index < offset).then_some(index))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .char_indices()
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or_else(|| self.content.len())
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0usize;
        let mut utf16 = 0usize;
        for character in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 = utf16.saturating_add(character.len_utf16());
            utf8 = utf8.saturating_add(character.len_utf8());
        }
        utf8.min(self.content.len())
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16 = 0usize;
        let mut utf8 = 0usize;
        for character in self.content.chars() {
            if utf8 >= offset {
                break;
            }
            utf8 = utf8.saturating_add(character.len_utf8());
            utf16 = utf16.saturating_add(character.len_utf16());
        }
        utf16
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range =
                self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn index_for_point(&self, point: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) =
            (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if point.y < bounds.top() {
            return 0;
        }
        if point.y > bounds.bottom() {
            return self.content.len();
        }
        line.closest_index_for_x(point.x - bounds.left())
            .min(self.content.len())
    }

    fn backspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let start = self.previous_boundary(self.cursor_offset());
            self.select_to(start, cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let end = self.next_boundary(self.selected_range.end);
            self.selected_range.end = end;
            cx.notify();
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn left(&mut self, cx: &mut Context<Self>) {
        let target = if self.selected_range.is_empty() {
            self.previous_boundary(self.cursor_offset())
        } else {
            self.selected_range.start
        };
        self.move_to(target, cx);
    }

    fn right(&mut self, cx: &mut Context<Self>) {
        let target = if self.selected_range.is_empty() {
            self.next_boundary(self.selected_range.end)
        } else {
            self.selected_range.end
        };
        self.move_to(target, cx);
    }

    fn select_left(&mut self, cx: &mut Context<Self>) {
        let target = self.previous_boundary(self.cursor_offset());
        self.select_to(target, cx);
    }

    fn select_right(&mut self, cx: &mut Context<Self>) {
        let target = self.next_boundary(self.cursor_offset());
        self.select_to(target, cx);
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }
}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.offset_from_utf16(range_utf16.start)
            ..self.offset_from_utf16(range_utf16.end);
        *adjusted = Some(
            self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end),
        );
        self.content.get(range).map(str::to_owned)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.offset_to_utf16(self.selected_range.start)
                ..self.offset_to_utf16(self.selected_range.end),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|range| {
            self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
        })
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| {
                self.offset_from_utf16(range.start)
                    ..self.offset_from_utf16(range.end)
            })
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let mut text = self.content.get(..range.start).unwrap_or("").to_owned();
        text.push_str(new_text);
        text.push_str(self.content.get(range.end..).unwrap_or(""));
        self.content = text.into();
        let end = range
            .start
            .saturating_add(new_text.len())
            .min(self.content.len());
        self.selected_range = end..end;
        self.marked_range = None;
        self.notify_edit(cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let replacement = range_utf16
            .or_else(|| {
                self.marked_range.as_ref().map(|range| {
                    self.offset_to_utf16(range.start)
                        ..self.offset_to_utf16(range.end)
                })
            })
            .unwrap_or_else(|| {
                self.offset_to_utf16(self.selected_range.start)
                    ..self.offset_to_utf16(self.selected_range.end)
            });
        let start = self.offset_from_utf16(replacement.start);
        self.replace_text_in_range(Some(replacement), new_text, window, cx);
        if !new_text.is_empty() {
            self.marked_range =
                Some(start..start.saturating_add(new_text.len()));
        }
        if let Some(range) = new_selected_range_utf16 {
            let offset = |position: usize| {
                start.saturating_add(utf16_offset_in(new_text, position))
            };
            self.selected_range = offset(range.start)..offset(range.end);
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        let start = self.offset_from_utf16(range_utf16.start);
        let end = self.offset_from_utf16(range_utf16.end);
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(start), bounds.top()),
            point(bounds.left() + line.x_for_index(end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let line = self.last_layout.as_ref()?;
        let index = line.index_for_x(line_point.x)?.min(self.content.len());
        Some(self.offset_to_utf16(index))
    }
}

impl Render for TextField {
    fn render(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focus = self.focus_handle.clone();
        let placeholder = self.placeholder.clone();
        let text = self.content.clone();
        let selected = self.selected_range.clone();
        let marked = self.marked_range.clone();
        div()
            .id("ranki-text-input")
            .track_focus(&focus)
            .key_context("ranki_input")
            .on_action(cx.listener(|this, _: &FieldBackspace, window, cx| {
                this.backspace(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FieldDelete, window, cx| {
                this.delete(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FieldLeft, _, cx| this.left(cx)))
            .on_action(cx.listener(|this, _: &FieldRight, _, cx| {
                this.right(cx);
            }))
            .on_action(cx.listener(|this, _: &FieldSelectLeft, _, cx| {
                this.select_left(cx);
            }))
            .on_action(cx.listener(|this, _: &FieldSelectRight, _, cx| {
                this.select_right(cx);
            }))
            .on_action(cx.listener(|this, _: &FieldSelectAll, _, cx| {
                this.select_all(cx);
            }))
            .on_action(cx.listener(|this, _: &FieldHome, _, cx| this.home(cx)))
            .on_action(cx.listener(|this, _: &FieldEnd, _, cx| this.end(cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.selecting = true;
                    let index = this.index_for_point(event.position);
                    if event.modifiers.shift {
                        this.select_to(index, cx);
                    } else {
                        this.move_to(index, cx);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.selecting = false;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.selecting = false;
                }),
            )
            .on_mouse_move(cx.listener(
                |this, event: &MouseMoveEvent, _, cx| {
                    if this.selecting {
                        let index = this.index_for_point(event.position);
                        this.select_to(index, cx);
                    }
                },
            ))
            .child(TextElement {
                input: cx.entity(),
                placeholder,
                placeholder_color: self.placeholder_color,
                text,
                selected,
                marked,
            })
    }
}

struct TextElement {
    input: Entity<TextField>,
    placeholder: SharedString,
    placeholder_color: Hsla,
    text: SharedString,
    selected: Range<usize>,
    marked: Option<Range<usize>>,
}

struct PrepaintState {
    line: ShapedLine,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(
        &self,
    ) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let style = window.text_style();
        let (display, color) = if self.text.is_empty() {
            (self.placeholder.clone(), self.placeholder_color)
        } else {
            (self.text.clone(), style.color)
        };
        let mut font = style.font();
        font.family = crate::theme::font_for_text(&display).into();
        let run = TextRun {
            len: display.len(),
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked) = self.marked.as_ref() {
            vec![
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.end.saturating_sub(marked.start),
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display.len().saturating_sub(marked.end),
                    ..run
                },
            ]
            .into_iter()
            .filter(|item| item.len > 0)
            .collect()
        } else {
            vec![run]
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display, font_size, &runs, None);
        let cursor_x = line.x_for_index(self.selected.end.min(line.len()));
        let focus = self.input.read(cx).focus_handle().clone();
        let cursor = focus.is_focused(window).then(|| {
            fill(
                Bounds::new(
                    point(bounds.left() + cursor_x, bounds.top()),
                    size(px(2.), bounds.size.height),
                ),
                style.color,
            )
        });
        let selection = if self.selected.start < self.selected.end {
            Some(fill(
                Bounds::from_corners(
                    point(
                        bounds.left() + line.x_for_index(self.selected.start),
                        bounds.top(),
                    ),
                    point(
                        bounds.left() + line.x_for_index(self.selected.end),
                        bounds.bottom(),
                    ),
                ),
                hsla(0.75, 0.4, 0.7, 0.3),
            ))
        } else {
            None
        };
        PrepaintState {
            line,
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus_handle().clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }
        if let Err(error) =
            prepaint
                .line
                .paint(bounds.origin, window.line_height(), window, cx)
        {
            eprintln!("ranki: cannot paint text field: {error}");
        }
        if let Some(cursor) = prepaint.cursor.take() {
            window.paint_quad(cursor);
        }
        self.input.update(cx, |input, _| {
            input.last_layout = Some(prepaint.line.clone());
            input.last_bounds = Some(bounds);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{key_bindings, wrap_selection};

    #[test]
    fn input_context_defines_navigation_keys() {
        let bindings = key_bindings();
        assert_eq!(bindings.len(), 10);
        assert!(!bindings.is_empty());
    }

    #[test]
    fn markup_wraps_selection_or_inserts_an_empty_pair() {
        let (text, cursor) = wrap_selection("問い", 0..6, ("<b>", "</b>"));
        assert_eq!(text, "<b>問い</b>");
        assert_eq!(cursor, 13);
        let (text, cursor) = wrap_selection("abc", 1..2, ("<b>", "</b>"));
        assert_eq!(text, "a<b>b</b>c");
        assert_eq!(&text[cursor..], "c");
        let (text, cursor) = wrap_selection("abc", 3..3, ("<b>", "</b>"));
        assert_eq!(text, "abc<b></b>");
        assert_eq!(&text[cursor..], "</b>");
    }
}
