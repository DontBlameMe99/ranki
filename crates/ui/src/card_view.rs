//! Anki template parsing, cloze and occlusion rendering, and bounded media cache.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

use futures::FutureExt;
use gpui::{
    AnyElement, App, AppContext, Asset, AssetLogger, Div, Entity,
    ImageAssetLoader, ImageCache, ImageCacheError, ImageCacheItem,
    InteractiveElement, IntoElement, ParentElement, PathBuilder, RenderImage,
    Resource, Rgba, SharedString, Styled, StyledImage, Window, canvas, div,
    hash, img, point, px, relative,
};
use ranki_model::{Note, NoteModel, image_occlusion::ShapeKind};

use crate::theme::{Palette, SPACE_GROUP, alpha};

mod markup;

#[cfg(test)]
pub use markup::cloze_text;
pub use markup::{
    ClozeSegment, MaskOverlay, MediaKind, MediaRef, answer_only_media,
    cloze_segments, media_refs, occlusion_overlays, strip_tags,
};
use markup::{is_empty_html, normalize_newlines};

/// Side of the card being rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Question,
    Answer,
}

/// Parsed template node. Anki templates are a tiny language, not general HTML.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateNode {
    Text(String),
    Field {
        key: String,
        filters: Vec<String>,
    },
    Conditional {
        key: String,
        negated: bool,
        children: Vec<Self>,
    },
}

/// Result of rendering a template for one card side.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderedCard {
    pub text: String,
    pub media: Vec<MediaRef>,
    pub clozes: Vec<ClozeSegment>,
    pub masks: Vec<MaskOverlay>,
}

/// Parses `{{Field}}`, `{{cloze:Text}}` and `{{#Field}}` sections.
#[must_use]
pub fn parse_template(template: &str) -> Vec<TemplateNode> {
    let mut roots = Vec::new();
    let mut stack: Vec<(String, bool, Vec<TemplateNode>)> = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) =
        template.get(cursor..).and_then(|rest| rest.find("{{"))
    {
        let start = cursor.saturating_add(relative);
        let text = template.get(cursor..start).unwrap_or("");
        if !text.is_empty() {
            push_node(
                &mut stack,
                &mut roots,
                TemplateNode::Text(text.to_owned()),
            );
        }
        let Some(after) = template.get(start.saturating_add(2)..) else {
            break;
        };
        let Some(end) = after.find("}}") else {
            push_node(
                &mut stack,
                &mut roots,
                TemplateNode::Text(
                    template.get(start..).unwrap_or("").to_owned(),
                ),
            );
            cursor = template.len();
            break;
        };
        let raw = after.get(..end).unwrap_or("").trim();
        cursor = start
            .saturating_add(2)
            .saturating_add(end)
            .saturating_add(2);
        if let Some(key) = raw.strip_prefix('/') {
            if let Some((owner, negated, children)) = stack.pop() {
                if owner == key {
                    push_node(
                        &mut stack,
                        &mut roots,
                        TemplateNode::Conditional {
                            key: owner,
                            negated,
                            children,
                        },
                    );
                } else {
                    push_node(
                        &mut stack,
                        &mut roots,
                        TemplateNode::Conditional {
                            key: owner,
                            negated,
                            children,
                        },
                    );
                    if let Some((outer, outer_negated, outer_children)) =
                        stack.pop()
                    {
                        stack.push((outer, outer_negated, outer_children));
                    }
                }
            }
            continue;
        }
        if let Some(key) = raw.strip_prefix('#') {
            stack.push((key.to_owned(), false, Vec::new()));
            continue;
        }
        if let Some(key) = raw.strip_prefix('^') {
            stack.push((key.to_owned(), true, Vec::new()));
            continue;
        }
        let (key, filters) = match raw.split_once(':') {
            Some((filter, field))
                if filter.trim().eq_ignore_ascii_case("cloze") =>
            {
                (field.trim().to_owned(), vec![String::from("cloze")])
            }
            _ => {
                let mut parts = raw.split(':');
                let first = parts.next().unwrap_or("").trim().to_owned();
                (first, parts.map(str::trim).map(str::to_owned).collect())
            }
        };
        push_node(&mut stack, &mut roots, TemplateNode::Field { key, filters });
    }
    if let Some(rest) = template.get(cursor..)
        && !rest.is_empty()
    {
        push_node(&mut stack, &mut roots, TemplateNode::Text(rest.to_owned()));
    }
    while let Some((key, negated, children)) = stack.pop() {
        push_node(
            &mut stack,
            &mut roots,
            TemplateNode::Conditional {
                key,
                negated,
                children,
            },
        );
    }
    roots
}

fn push_node(
    stack: &mut [(String, bool, Vec<TemplateNode>)],
    roots: &mut Vec<TemplateNode>,
    node: TemplateNode,
) {
    if let Some((_, _, children)) = stack.last_mut() {
        children.push(node);
    } else {
        roots.push(node);
    }
}

/// Field names and values, parallel slices straight from the note.
#[derive(Clone, Copy, Debug)]
pub struct FieldValues<'a> {
    pub names: &'a [String],
    pub values: &'a [String],
}

impl<'a> FieldValues<'a> {
    #[must_use]
    pub const fn new(names: &'a [String], values: &'a [String]) -> Self {
        Self { names, values }
    }
}

/// Renders a template with field values, conditionals, and `FrontSide` support.
#[must_use]
pub fn render_template(
    template: &str,
    fields: FieldValues<'_>,
    front_side: &str,
) -> String {
    render_nodes(&parsed_template(template), fields, front_side)
}

const TEMPLATE_CACHE_LIMIT: usize = 128;

thread_local! {
    static TEMPLATES: RefCell<HashMap<String, Rc<[TemplateNode]>>> =
        RefCell::new(HashMap::new());
}

fn parsed_template(source: &str) -> Rc<[TemplateNode]> {
    TEMPLATES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(nodes) = cache.get(source) {
            return Rc::clone(nodes);
        }
        let nodes = Rc::from(parse_template(source));
        if cache.len() >= TEMPLATE_CACHE_LIMIT {
            cache.clear();
        }
        let _ = cache.insert(source.to_owned(), Rc::clone(&nodes));
        nodes
    })
}

fn render_nodes(
    nodes: &[TemplateNode],
    fields: FieldValues<'_>,
    front_side: &str,
) -> String {
    let mut output = String::new();
    for node in nodes {
        match node {
            TemplateNode::Text(text) => output.push_str(text),
            TemplateNode::Field { key, .. } => {
                if key.eq_ignore_ascii_case("FrontSide") {
                    output.push_str(front_side);
                } else if let Some(field) = key.strip_prefix("cloze:") {
                    if let Some(value) = find_field(fields, field) {
                        output.push_str(value);
                    }
                } else if let Some(value) = find_field(fields, key) {
                    output.push_str(value);
                }
            }
            TemplateNode::Conditional {
                key,
                negated,
                children,
            } => {
                let value = find_field(fields, key).unwrap_or("");
                let present = !value.trim().is_empty() && !is_empty_html(value);
                if present != *negated {
                    output
                        .push_str(&render_nodes(children, fields, front_side));
                }
            }
        }
    }
    output
}

fn find_field<'a>(fields: FieldValues<'a>, key: &str) -> Option<&'a str> {
    fields
        .names
        .iter()
        .zip(fields.values.iter())
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.as_str())
}

/// One card side plus whether the answer repeats the front.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardRequest {
    pub ordinal: u16,
    pub side: Side,
    pub include_front: bool,
}

impl CardRequest {
    #[must_use]
    pub const fn question(ordinal: u16) -> Self {
        Self {
            ordinal,
            side: Side::Question,
            include_front: true,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub const fn answer(ordinal: u16) -> Self {
        Self {
            ordinal,
            side: Side::Answer,
            include_front: true,
        }
    }

    /// Answer template without the repeated front, for the revealed stage.
    #[must_use]
    pub const fn answer_only(ordinal: u16) -> Self {
        Self {
            ordinal,
            side: Side::Answer,
            include_front: false,
        }
    }
}

/// Renders the front or back text of a card using the selected template.
#[must_use]
pub fn render_card(
    model: &NoteModel,
    note: &Note,
    request: CardRequest,
) -> RenderedCard {
    let CardRequest {
        ordinal,
        side,
        include_front,
    } = request;
    let Some(template) = model.template_for(ordinal) else {
        return RenderedCard {
            text: String::from("This note has no template."),
            ..RenderedCard::default()
        };
    };
    let source = match side {
        Side::Question => template.question_format.as_str(),
        Side::Answer => template.answer_format.as_str(),
    };
    let source = if model.is_image_occlusion() {
        source.replace("{{cloze:Occlusion}}", "")
    } else {
        source.to_owned()
    };
    let fields = FieldValues::new(&model.fields, &note.fields);
    let front = if include_front {
        let question = if model.is_image_occlusion() {
            template.question_format.replace("{{cloze:Occlusion}}", "")
        } else {
            template.question_format.clone()
        };
        render_template(&question, fields, "")
    } else {
        String::new()
    };
    let rendered = render_template(&source, fields, &front);
    let text = strip_tags(&rendered);
    let (text, clozes) = if model.is_cloze() {
        let clozes = cloze_segments(&text, ordinal, side);
        let text = clozes
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<String>();
        (text, clozes)
    } else {
        (text, Vec::new())
    };
    let masks = if model.is_image_occlusion() {
        note.fields
            .iter()
            .find(|field| field.contains("image-occlusion:"))
            .map(|field| occlusion_overlays(field, ordinal, side))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    RenderedCard {
        text: normalize_newlines(&text),
        media: media_refs(&rendered),
        clozes,
        masks,
    }
}

/// Draws cloze and occlusion content as a compact text block.
#[must_use]
pub fn rendered_text(card: &RenderedCard, color: Rgba, size_px: f32) -> Div {
    let mut block = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(SPACE_GROUP))
        .text_size(px(size_px))
        .text_color(color);
    for line in card.text.lines() {
        block = block.child(
            div()
                .font_family(crate::theme::font_for_text(line))
                .child(line.to_owned()),
        );
    }
    block
}

/// GPUI overlays for parsed image-occlusion masks.
#[must_use]
pub fn occlusion_layer(
    masks: &[MaskOverlay],
    colors: Palette,
    answer: bool,
) -> Div {
    let mut layer = div().absolute().inset_0();
    for mask in masks {
        let fill = if answer {
            alpha(colors.red, 0.10)
        } else if mask.active || mask.occlude_inactive {
            alpha(colors.red, 0.72)
        } else {
            continue;
        };
        let border = if answer {
            alpha(colors.red, 0.7)
        } else {
            colors.crust
        };
        if mask.shape == ShapeKind::Polygon && !mask.points.is_empty() {
            layer = layer.child(polygon_overlay(mask, colors, answer));
            continue;
        }
        let mut shape = div()
            .absolute()
            .left(relative(mask.left))
            .top(relative(mask.top))
            .w(relative(mask.width))
            .h(relative(mask.height))
            .bg(fill)
            .border_1()
            .border_color(border);
        if mask.shape == ShapeKind::Ellipse {
            shape = shape.rounded_full();
        } else if mask.shape == ShapeKind::Text {
            shape = shape.rounded(px(4.));
            if answer && let Some(text) = mask.text.as_ref() {
                shape = shape
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(14.))
                    .text_color(colors.text)
                    .child(text.clone());
            }
        } else {
            shape = shape.rounded(px(3.));
        }
        layer = layer.child(shape);
    }
    layer
}

fn polygon_overlay(
    mask: &MaskOverlay,
    colors: Palette,
    answer: bool,
) -> AnyElement {
    let points = mask.points.clone();
    let fill = if answer {
        alpha(colors.red, 0.10)
    } else {
        alpha(colors.red, 0.72)
    };
    let border = if answer {
        alpha(colors.red, 0.7)
    } else {
        colors.crust
    };
    canvas(
        move |bounds, _, _| {
            let mut fill_path = PathBuilder::fill();
            let mut stroke_path = PathBuilder::stroke(px(1.));
            let Some(first) = points.first() else {
                return (None, None);
            };
            let to_point = |(x, y): (f32, f32)| {
                point(
                    px(f32::from(bounds.left())
                        + x * f32::from(bounds.size.width)),
                    px(f32::from(bounds.top())
                        + y * f32::from(bounds.size.height)),
                )
            };
            fill_path.move_to(to_point(*first));
            stroke_path.move_to(to_point(*first));
            for point in points.iter().skip(1) {
                fill_path.line_to(to_point(*point));
                stroke_path.line_to(to_point(*point));
            }
            fill_path.close();
            stroke_path.close();
            (fill_path.build().ok(), stroke_path.build().ok())
        },
        move |_, (fill_path, stroke_path), window, _| {
            if let Some(path) = fill_path {
                window.paint_path(path, fill);
            }
            if let Some(path) = stroke_path {
                window.paint_path(path, border);
            }
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

/// Bounded LRU image cache backed by GPUI's async image decoder.
pub struct LruImageCache {
    items: HashMap<u64, ImageCacheItem>,
    order: VecDeque<u64>,
    capacity: usize,
}

impl LruImageCache {
    pub const CAPACITY: usize = 24;

    #[must_use]
    pub fn new(cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|_| Self {
            items: HashMap::new(),
            order: VecDeque::new(),
            capacity: Self::CAPACITY,
        });
        cx.observe_release(&entity, |cache, cx| {
            for (_, mut item) in std::mem::take(&mut cache.items) {
                if let Some(Ok(image)) = item.get() {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();
        entity
    }

    fn touch(&mut self, key: u64) {
        self.order.retain(|item| *item != key);
        self.order.push_back(key);
    }

    fn evict(&mut self, cx: &mut App) {
        while self.order.len() > self.capacity
            && let Some(oldest) = self.order.pop_front()
        {
            if let Some(mut item) = self.items.remove(&oldest)
                && let Some(Ok(image)) = item.get()
            {
                cx.drop_image(image, None);
            }
        }
    }

    /// Loads an image resource, keeping at most `capacity` decoded entries.
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the ImageCache trait signature"
    )]
    pub fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let key = hash(resource);
        if self.items.contains_key(&key) {
            self.touch(key);
            if let Some(item) = self.items.get_mut(&key) {
                return item.get();
            }
        }
        let future =
            AssetLogger::<ImageAssetLoader>::load(resource.clone(), cx);
        let task = cx.background_executor().spawn(future).shared();
        let _ = self
            .items
            .insert(key, ImageCacheItem::Loading(task.clone()));
        self.order.push_back(key);
        self.evict(cx);
        let view = window.current_view();
        window
            .spawn(cx, async move |cx| {
                let _ = task.await;
                cx.on_next_frame(move |_, cx| cx.notify(view));
            })
            .detach();
        None
    }
}

impl ImageCache for LruImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        Self::load(self, resource, window, cx)
    }
}

/// Resolves a media name inside the collection media directory.
///
/// Names come from imported card content, so absolute paths and parent
/// segments are rejected instead of escaping `media_dir`.
pub fn media_path(
    media_dir: &std::path::Path,
    name: &str,
) -> Option<std::path::PathBuf> {
    let path = std::path::Path::new(name);
    let mut components = path.components();
    let single =
        matches!(components.next(), Some(std::path::Component::Normal(_)))
            && components.next().is_none();
    single.then(|| media_dir.join(path))
}

/// Convenience wrapper for a media image with a missing-file caption.
#[must_use]
pub fn media_image(
    path: Option<std::path::PathBuf>,
    colors: Palette,
    height: f32,
) -> AnyElement {
    let Some(path) = path else {
        return unavailable_media(colors, height);
    };
    let id = SharedString::from(path.to_string_lossy().into_owned());
    img(path)
        .id(id)
        .h(px(height))
        .max_w(px(520.))
        .object_fit(gpui::ObjectFit::Contain)
        .with_fallback(move || unavailable_media(colors, height))
        .into_any_element()
}

/// Image with its occlusion masks aligned to the rendered picture.
#[must_use]
pub fn occlusion_media(
    media_dir: &std::path::Path,
    name: &str,
    masks: &[MaskOverlay],
    colors: Palette,
    answer: bool,
    height: f32,
    natural_size: Option<(f32, f32)>,
) -> Div {
    let path = media_path(media_dir, name);
    let natural = natural_size.or_else(|| {
        path.as_ref()
            .and_then(|path| image::image_dimensions(path).ok())
            .map(|(width, height)| {
                (
                    f32::from(u16::try_from(width).unwrap_or(u16::MAX)),
                    f32::from(u16::try_from(height).unwrap_or(u16::MAX)),
                )
            })
    });
    let (width, fitted_height) =
        natural.map_or((520.0, height), |(width, natural_height)| {
            let width = width.max(1.0);
            let natural_height = natural_height.max(1.0);
            let scale = (520.0 / width).min(height / natural_height);
            (width * scale, natural_height * scale)
        });
    div()
        .relative()
        .w(px(width))
        .h(px(fitted_height))
        .child(media_image(path, colors, fitted_height))
        .child(occlusion_layer(masks, colors, answer))
}

fn unavailable_media(colors: Palette, height: f32) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(height))
        .text_size(px(12.))
        .text_color(colors.subtext)
        .child("Media unavailable")
        .into_any_element()
}

/// Renders field value media into a column of images and sound captions.
#[must_use]
pub fn media_column(
    refs: &[MediaRef],
    media_dir: &std::path::Path,
    colors: Palette,
    height: f32,
) -> Option<Div> {
    if refs.is_empty() {
        return None;
    }
    let mut column =
        div().flex().flex_col().items_center().gap(px(SPACE_GROUP));
    for media in refs {
        column = column.child(match media.kind {
            MediaKind::Image => {
                let path = media_path(media_dir, &media.name);
                media_image(path, colors, height)
            }
            MediaKind::Sound => div()
                .flex()
                .items_center()
                .gap(px(SPACE_GROUP))
                .text_size(px(12.))
                .text_color(colors.mauve)
                .child(icon_placeholder("[sound]", colors))
                .child(media.name.clone())
                .into_any_element(),
        });
    }
    Some(column)
}

fn icon_placeholder(label: &str, colors: Palette) -> Div {
    div()
        .text_size(px(12.))
        .text_color(colors.subtext)
        .child(label.to_owned())
}

/// Renders note fields through the selected template into a preview block.
#[must_use]
pub fn preview_rendered(
    model: &NoteModel,
    fields: &[String],
    side: Side,
) -> RenderedCard {
    let note = Note {
        id: 0,
        guid: String::new(),
        model_id: model.id,
        modified: 0,
        usn: 0,
        tags: Vec::new(),
        fields: fields.to_vec(),
        sort_field: String::new(),
        checksum: 0,
        flags: 0,
        data: String::new(),
    };
    let ordinal = model.card_ordinals(fields).first().copied().unwrap_or(0);
    let request = match side {
        Side::Question => CardRequest::question(ordinal),
        Side::Answer => CardRequest::answer_only(ordinal),
    };
    render_card(model, &note, request)
}

/// Picks the last non-empty line of a rendered card for a compact preview.
#[must_use]
pub fn preview_text(rendered: &RenderedCard) -> String {
    rendered
        .text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .to_owned()
}

#[cfg(test)]
/// Renders note fields through the selected template into a preview block.
#[must_use]
pub fn preview_card(
    model: &NoteModel,
    fields: &[String],
    side: Side,
) -> String {
    preview_text(&preview_rendered(model, fields, side))
}

#[cfg(test)]
mod tests {
    use super::{
        CardRequest, MediaKind, MediaRef, Side, TemplateNode,
        answer_only_media, cloze_text, markup::ClozeKind, media_refs,
        occlusion_overlays, parse_template, preview_card, render_card,
        strip_tags,
    };
    use ranki_model::{CardTemplate, Note, NoteKind};

    fn model(
        fields: &[&str],
        question: &str,
        answer: &str,
    ) -> ranki_model::NoteModel {
        ranki_model::NoteModel {
            id: 1,
            name: String::from("Basic"),
            fields: fields.iter().map(|value| (*value).to_owned()).collect(),
            templates: vec![CardTemplate {
                name: String::from("Card 1"),
                question_format: question.to_owned(),
                answer_format: answer.to_owned(),
                ord: 0,
            }],
            css: String::new(),
            sort_field_index: 0,
            kind: NoteKind::Normal,
            original_stock_kind: None,
            field_tags: Vec::new(),
            template_ordinals: vec![0],
            raw_config: None,
        }
    }

    fn note(fields: &[&str]) -> ranki_model::Note {
        ranki_model::Note {
            id: 1,
            guid: String::new(),
            model_id: 1,
            modified: 0,
            usn: 0,
            tags: Vec::new(),
            fields: fields.iter().map(|value| (*value).to_owned()).collect(),
            sort_field: String::new(),
            checksum: 0,
            flags: 0,
            data: String::new(),
        }
    }

    #[test]
    fn cloze_without_colon_is_an_ordinary_field() {
        let nodes = parse_template("{{cloze}}");
        assert!(matches!(
            nodes[0],
            TemplateNode::Field { ref key, .. } if key == "cloze"
        ));
    }

    #[test]
    fn parses_fields_and_front_side() {
        let nodes = parse_template("hi {{Front}} {{FrontSide}}");
        assert_eq!(nodes.len(), 4);
        assert!(matches!(nodes[0], TemplateNode::Text(_)));
        assert!(matches!(
            nodes[1],
            TemplateNode::Field { ref key, .. } if key == "Front"
        ));
    }

    #[test]
    fn parses_nested_conditionals() {
        let nodes = parse_template("{{#Front}}{{^Back}}x{{/Back}}{{/Front}}");
        assert!(matches!(
            nodes[0],
            TemplateNode::Conditional { negated: false, .. }
        ));
        let TemplateNode::Conditional { children, .. } = &nodes[0] else {
            panic!("conditional expected");
        };
        assert!(matches!(
            children[0],
            TemplateNode::Conditional { negated: true, .. }
        ));
    }

    #[test]
    fn renders_basic_question_and_answer() {
        let form = model(
            &["Front", "Back"],
            "{{Front}}",
            "{{FrontSide}}<hr id=answer>{{Back}}",
        );
        let rendered = render_card(
            &form,
            &note(&["Question?", "Answer!"]),
            CardRequest::answer(0),
        );
        assert_eq!(rendered.text, "Question?\nAnswer!");
    }

    #[test]
    fn reversed_card_uses_ordinal_one() {
        let mut form = model(
            &["Front", "Back"],
            "{{Front}}",
            "{{Front}}<hr id=answer>{{Back}}",
        );
        form.templates.push(CardTemplate {
            name: String::from("Card 2"),
            question_format: String::from("{{Back}}"),
            answer_format: String::from("{{Back}}<hr id=answer>{{Front}}"),
            ord: 1,
        });
        form.template_ordinals.push(1);
        let rendered = render_card(
            &form,
            &note(&["Front text", "Back text"]),
            CardRequest::question(1),
        );
        assert_eq!(rendered.text, "Back text");
    }

    #[test]
    fn missing_template_has_a_helpful_fallback() {
        let mut form = model(&["Front"], "{{Front}}", "{{Front}}");
        form.templates.clear();
        let rendered =
            render_card(&form, &note(&["x"]), CardRequest::question(0));
        assert_eq!(rendered.text, "This note has no template.");
    }

    #[test]
    fn conditionals_hide_empty_fields() {
        let form = model(
            &["Front", "Add Reverse", "Back"],
            "{{Front}}{{#Add Reverse}} + {{Back}}{{/Add Reverse}}",
            "{{Front}}",
        );
        let empty = render_card(
            &form,
            &note(&["One", "", "Two"]),
            CardRequest::question(0),
        );
        let filled = render_card(
            &form,
            &note(&["One", "yes", "Two"]),
            CardRequest::question(0),
        );
        assert_eq!(empty.text, "One");
        assert_eq!(filled.text, "One + Two");
    }

    #[test]
    fn cloze_question_hides_answer_and_shows_hint() {
        let text = "{{c1::one}} {{c2::two::hint}}";
        assert_eq!(cloze_text(text, 0, Side::Question), "[...] two");
        assert_eq!(cloze_text(text, 1, Side::Question), "one [hint]");
    }

    #[test]
    fn cloze_answer_reveals_active_and_inactive() {
        let segments =
            super::cloze_segments("{{c1::one}} {{c2::two}}", 0, Side::Answer);
        assert_eq!(segments[0].kind, ClozeKind::ActiveAnswer);
        assert_eq!(segments[2].kind, ClozeKind::Inactive);
        assert_eq!(
            cloze_text("{{c1::one}} {{c2::two}}", 0, Side::Answer),
            "one two"
        );
    }

    #[test]
    fn multi_ordinal_cloze_is_active_on_both_cards() {
        let text = "{{c1,2::shared}}";
        assert_eq!(cloze_text(text, 0, Side::Question), "[...]");
        assert_eq!(cloze_text(text, 1, Side::Question), "[...]");
        assert_eq!(cloze_text("{{c0::never}}", 0, Side::Question), "never");
    }

    #[test]
    fn media_refs_are_extracted_from_images_and_sound() {
        let refs = media_refs("<img src=\"diagram.png\"> and [sound:word.mp3]");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].kind, MediaKind::Image);
        assert_eq!(refs[0].name, "diagram.png");
        assert_eq!(refs[1].kind, MediaKind::Sound);
        assert_eq!(refs[1].name, "word.mp3");
    }

    #[test]
    fn scripts_and_html_tags_are_stripped() {
        assert_eq!(
            strip_tags("<div>Hello<br>world<script>bad()</script></div>"),
            "Hello\nworld"
        );
    }

    #[test]
    fn answer_media_skips_images_already_shown_on_the_question() {
        let shared = MediaRef {
            kind: MediaKind::Image,
            name: "diagram.png".into(),
        };
        let extra = MediaRef {
            kind: MediaKind::Sound,
            name: "hint.mp3".into(),
        };
        let shown = [shared.clone()];
        let rendered = [shared, extra.clone()];
        assert_eq!(answer_only_media(&shown, &rendered), [extra]);
    }
    #[test]
    fn occlusion_masks_use_normalized_coordinates() {
        let text = "{{c1::image-occlusion:rect:left=.2:top=.3:width=.4:height=.1}}<br>{{c2::image-occlusion:ellipse:left=.1:top=.1:width=.2:height=.2:oi=1}}";
        let question = occlusion_overlays(text, 0, Side::Question);
        let answer = occlusion_overlays(text, 0, Side::Answer);
        assert_eq!(question.len(), 2);
        assert!(question[0].active);
        assert_eq!(question[0].left, 0.2);
        assert_eq!(question[0].width, 0.4);
        assert_eq!(answer.len(), 2);
        assert!(answer[1].occlude_inactive);
    }

    #[test]
    fn preview_uses_the_selected_template() {
        let form = model(&["Front", "Back"], "{{Front}}", "{{Front}} {{Back}}");
        assert_eq!(
            preview_card(
                &form,
                &["Q".to_owned(), "A".to_owned()],
                Side::Question
            ),
            "Q"
        );
        assert_eq!(
            preview_card(
                &form,
                &["Q".to_owned(), "A".to_owned()],
                Side::Answer
            ),
            "Q A"
        );
    }
    #[test]
    fn stock_cloze_renders_through_the_full_template() {
        let mut form = ranki_model::StockModelKind::Cloze.definition();
        form.id = 1;
        let note = note(&["The {{c1::capital}} is large.", "A detail"]);
        let question = render_card(&form, &note, CardRequest::question(0));
        let answer = render_card(&form, &note, CardRequest::answer_only(0));
        assert_eq!(question.text, "The [...] is large.");
        assert_eq!(answer.text, "The capital is large.\nA detail");
    }
    #[test]
    fn preview_uses_the_first_real_cloze_ordinal() {
        let mut form = ranki_model::StockModelKind::Cloze.definition();
        form.id = 1;
        let fields =
            vec![String::from("The {{c2::answer}} is hidden."), String::new()];
        assert_eq!(
            preview_card(&form, &fields, Side::Question),
            "The [...] is hidden."
        );
        assert_eq!(
            preview_card(&form, &fields, Side::Answer),
            "The answer is hidden."
        );
    }

    #[test]
    fn stock_image_occlusion_keeps_markers_out_of_card_text() {
        let mut form = ranki_model::StockModelKind::ImageOcclusion.definition();
        form.id = 1;
        let fields = crate::occlusion::image_occlusion_fields(
            "Map",
            "Country",
            "",
            "map.png",
            &[crate::occlusion::default_mask(0)],
        );
        let note = Note {
            id: 1,
            guid: String::new(),
            model_id: form.id,
            modified: 0,
            usn: 0,
            tags: Vec::new(),
            fields,
            sort_field: String::new(),
            checksum: 0,
            flags: 0,
            data: String::new(),
        };
        let question = render_card(&form, &note, CardRequest::question(0));
        let answer = render_card(&form, &note, CardRequest::answer_only(0));
        assert_eq!(question.text, "Map");
        assert_eq!(answer.text, "Map\nCountry");
        assert_eq!(question.media[0].name, "map.png");
        assert_eq!(question.masks.len(), 1);
        assert!(question.masks[0].active);
    }

    #[test]
    fn percent_suffix_sets_occlusion_units() {
        let mask = occlusion_overlays(
            "{{c1::image-occlusion:rect:left=1%:top=50%:width=1:height=.5}}",
            0,
            Side::Question,
        );
        assert_eq!(mask.len(), 1);
        assert!((mask[0].left - 0.01).abs() < 1e-6);
        assert!((mask[0].top - 0.5).abs() < 1e-6);
        assert!((mask[0].width - 1.0).abs() < 1e-6);
        assert!((mask[0].height - 0.5).abs() < 1e-6);
    }
    #[test]
    fn polygon_points_stay_in_image_coordinates() {
        let mask = occlusion_overlays(
            "{{c1::image-occlusion:polygon:left=.1:top=.2:width=.6:height=.5:points=0.1,0.2 0.7,0.2 0.4,0.8}}",
            0,
            Side::Question,
        );
        assert_eq!(mask.len(), 1);
        assert_eq!(mask[0].points, [(0.1, 0.2), (0.7, 0.2), (0.4, 0.8)]);
    }

    #[test]
    fn text_occlusion_keeps_its_label_for_the_answer() {
        let mask = occlusion_overlays(
            "{{c1::image-occlusion:text:left=.1:top=.2:width=.3:height=.1:text=Paris}}",
            0,
            Side::Answer,
        );
        assert_eq!(mask.len(), 1);
        assert_eq!(mask[0].text.as_deref(), Some("Paris"));
    }

    #[test]
    fn image_src_stays_inside_its_own_tag() {
        let refs = media_refs("<img alt=x>[sound:beep.wav]<img src=\"b.png\">");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].kind, MediaKind::Sound);
        assert_eq!(refs[0].name, "beep.wav");
        assert_eq!(refs[1].kind, MediaKind::Image);
        assert_eq!(refs[1].name, "b.png");
    }

    #[test]
    fn entity_encoded_text_and_media_names_are_decoded() {
        assert_eq!(strip_tags("A &amp; B &#65;"), "A & B A");
        let refs = media_refs("<img src=\"a&quot;b.png\">");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "a\"b.png");
    }

    #[test]
    fn unmatched_template_suffix_is_kept_once() {
        let empty = super::FieldValues {
            names: &[],
            values: &[],
        };
        assert_eq!(
            super::render_template("before {{partial", empty, ""),
            "before {{partial"
        );
    }

    #[test]
    fn media_paths_stay_inside_the_media_directory() {
        let dir = std::path::Path::new("/media");
        assert_eq!(
            super::media_path(dir, "a.png"),
            Some(std::path::PathBuf::from("/media/a.png"))
        );
        assert_eq!(super::media_path(dir, "../secret.png"), None);
        assert_eq!(super::media_path(dir, "/etc/passwd"), None);
        assert_eq!(super::media_path(dir, "sub/a.png"), None);
    }

    #[test]
    fn rendered_text_accepts_the_shared_palette() {
        let card = render_card(
            &model(&["Front"], "{{Front}}", "{{Front}}"),
            &note(&["Hello"]),
            CardRequest::question(0),
        );
        let _ = super::rendered_text(&card, crate::theme::MOCHA.text, 84.0);
        let _ = crate::theme::MOCHA;
    }
}
