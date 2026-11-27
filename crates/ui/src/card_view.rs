//! Anki template parsing, cloze and occlusion rendering, and bounded media cache.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

use futures::FutureExt;
use gpui::{
    AnyElement, App, AppContext, Asset, AssetLogger, Div, Entity, FontStyle,
    ImageAssetLoader, ImageCache, ImageCacheError, ImageCacheItem,
    InteractiveElement, IntoElement, ParentElement, PathBuilder, RenderImage,
    Resource, Rgba, SharedString, Styled, StyledImage, StyledText, Window,
    canvas, div, hash, img, point, px, relative,
};
use ranki_model::{Note, NoteModel, image_occlusion::ShapeKind};

use crate::theme::{Palette, SPACE_GROUP, alpha};

mod markup;
mod math;

pub use markup::{
    CardBlock, ClozeSegment, CodeSpan, MaskOverlay, MathKind, MediaKind,
    MediaRef, TextSpan, TextSpanKind, TokenKind, answer_only_media,
    blocks_text, card_blocks, cloze_segments, math_segments, media_refs,
    occlusion_overlays,
};
use markup::{
    ClozeSpan, cloze_spans, is_empty_html, normalize_newlines, styled_text,
};
#[cfg(test)]
pub use markup::{cloze_text, highlight_code};
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

/// One piece of card text with its own color override.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub foreground: Option<Rgba>,
    pub background: Option<Rgba>,
    /// Math source and kind when this run is a math expression.
    pub source: String,
    pub kind: MathKind,
    pub math: bool,
    /// Inline code rendered in the monospace font.
    pub code: bool,
}

/// Result of rendering a template for one card side.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderedCard {
    pub text: String,
    pub runs: Vec<TextRun>,
    pub blocks: Vec<CardBlock>,
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
    let styled = styled_text(&rendered);
    let source_text = styled
        .iter()
        .map(|run| run.text.as_str())
        .collect::<String>();
    let (runs, clozes) = if model.is_cloze() {
        let spans = cloze_spans(&source_text, ordinal, side);
        let clozes = spans.iter().map(|span| span.segment.clone()).collect();
        (cloze_runs(&styled, &spans), clozes)
    } else {
        (styled, Vec::new())
    };
    let runs = apply_math(normalize_runs(runs));
    let blocks = if model.is_cloze() {
        let markup = cloze_segments(&rendered, ordinal, side)
            .into_iter()
            .map(|segment| segment.text)
            .collect::<String>();
        card_blocks(&markup)
    } else {
        card_blocks(&rendered)
    };
    let text = if blocks.is_empty() {
        runs.iter().map(|run| run.text.as_str()).collect::<String>()
    } else {
        normalize_newlines(&blocks_text(&blocks))
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
        text,
        runs,
        blocks,
        media: media_refs(&rendered),
        clozes,
        masks,
    }
}

/// Applies cloze labels to styled runs without losing their colors.
fn cloze_runs(styled: &[TextRun], spans: &[ClozeSpan]) -> Vec<TextRun> {
    let map = RunMap::new(styled);
    let mut result = Vec::new();
    let mut cursor = 0usize;
    for span in spans {
        if span.start > cursor {
            map.push_slice(cursor, span.start, &mut result);
        }
        if span.label_start < span.label_end {
            map.push_mapped_slice(
                span.label_start..span.label_end,
                &span.segment.text,
                &mut result,
            );
        } else if let Some(run) = map.style_at(span.start, span.end) {
            result.push(TextRun {
                text: span.segment.text.clone(),
                foreground: run.foreground,
                background: run.background,
                math: false,
                ..Default::default()
            });
        }
        cursor = cursor.max(span.end);
    }
    map.push_slice(cursor, map.end(), &mut result);
    result
}

/// Byte offsets of each styled run inside the rendered source text.
struct RunMap<'a> {
    runs: &'a [TextRun],
    bounds: Vec<(usize, usize, usize)>,
}

impl<'a> RunMap<'a> {
    fn new(runs: &'a [TextRun]) -> Self {
        let mut bounds = Vec::new();
        let mut cursor = 0usize;
        for (index, run) in runs.iter().enumerate() {
            let end = cursor.saturating_add(run.text.len());
            bounds.push((cursor, end, index));
            cursor = end;
        }
        Self { runs, bounds }
    }

    fn end(&self) -> usize {
        self.bounds.last().map_or(0, |(_, end, _)| *end)
    }

    fn style_at(&self, start: usize, end: usize) -> Option<&'a TextRun> {
        let (_, _, index) = self
            .bounds
            .iter()
            .find(|(from, to, _)| *from < end && *to > start)?;
        self.runs.get(*index)
    }

    fn push_slice(&self, start: usize, end: usize, output: &mut Vec<TextRun>) {
        for &(from, to, index) in &self.bounds {
            if to <= start || from >= end {
                continue;
            }
            let Some(run) = self.runs.get(index) else {
                continue;
            };
            let from_offset = from.max(start).saturating_sub(from);
            let to_offset = to.min(end).saturating_sub(from);
            if let Some(text) = run.text.get(from_offset..to_offset)
                && !text.is_empty()
            {
                output.push(TextRun {
                    text: text.to_owned(),
                    foreground: run.foreground,
                    background: run.background,
                    math: false,
                    ..Default::default()
                });
            }
        }
    }

    fn push_mapped_slice(
        &self,
        range: std::ops::Range<usize>,
        label: &str,
        output: &mut Vec<TextRun>,
    ) {
        for &(from, to, index) in &self.bounds {
            let start = from.max(range.start);
            let end = to.min(range.end);
            if start >= end {
                continue;
            }
            let Some(run) = self.runs.get(index) else {
                continue;
            };
            let label_from = start.saturating_sub(range.start);
            let label_to = end.saturating_sub(range.start);
            if let Some(text) = label.get(label_from..label_to)
                && !text.is_empty()
            {
                output.push(TextRun {
                    text: text.to_owned(),
                    foreground: run.foreground,
                    background: run.background,
                    math: false,
                    ..Default::default()
                });
            }
        }
    }
}

/// Drops blank lines and merges adjacent runs with the same style.
fn normalize_runs(runs: Vec<TextRun>) -> Vec<TextRun> {
    let lines = split_lines(&runs);
    let mut output = Vec::new();
    for line in lines {
        if !line.iter().any(|run| !run.text.trim().is_empty()) {
            continue;
        }
        if !output.is_empty() {
            output.push(TextRun::default());
            if let Some(break_run) = output.last_mut() {
                break_run.text.push('\n');
            }
        }
        output.extend(line);
    }
    output
}

/// Splits math delimiters into runs that keep the raw TeX for typesetting.
fn apply_math(runs: Vec<TextRun>) -> Vec<TextRun> {
    let mut output: Vec<TextRun> = Vec::new();
    for run in runs {
        for segment in math_segments(&run.text) {
            let math = segment.kind != MathKind::Text;
            if let Some(last) = output.last_mut()
                && last.math == math
                && last.source == segment.source
                && last.kind == segment.kind
                && last.foreground == run.foreground
                && last.background == run.background
            {
                last.text.push_str(&segment.text);
            } else {
                output.push(TextRun {
                    text: segment.text,
                    foreground: run.foreground,
                    background: run.background,
                    source: segment.source,
                    kind: segment.kind,
                    math,
                    code: false,
                });
            }
        }
    }
    output
}

fn split_lines(runs: &[TextRun]) -> Vec<Vec<TextRun>> {
    let mut lines: Vec<Vec<TextRun>> = vec![Vec::new()];
    for run in runs {
        for (index, line) in run.text.split('\n').enumerate() {
            if index > 0 {
                lines.push(Vec::new());
            }
            if line.is_empty() {
                continue;
            }
            if let Some(current) = lines.last_mut() {
                current.push(TextRun {
                    text: line.to_owned(),
                    foreground: run.foreground,
                    background: run.background,
                    source: run.source.clone(),
                    kind: run.kind,
                    math: run.math,
                    code: false,
                });
            }
        }
    }
    lines
}

fn text_lines(card: &RenderedCard) -> Vec<Vec<TextRun>> {
    if card.runs.is_empty() {
        return card
            .text
            .lines()
            .map(|line| {
                vec![TextRun {
                    text: line.to_owned(),
                    foreground: None,
                    background: None,
                    math: false,
                    ..Default::default()
                }]
            })
            .collect();
    }
    split_lines(&card.runs)
}

/// Draws one line of card text, keeping per-run color overrides.
#[must_use]
pub fn rendered_line(runs: &[TextRun], color: Rgba, size_px: f32) -> Div {
    let plain = runs.iter().map(|run| run.text.as_str()).collect::<String>();
    let mut line = div()
        .w_full()
        .text_center()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_center()
        .text_size(px(size_px))
        .font_family(crate::theme::font_for_text(&plain));
    for run in runs {
        let mut part = div()
            .debug_selector(|| "card-text-run".to_owned())
            .min_w(px(0.))
            .text_color(run.foreground.unwrap_or(color));
        if run.code {
            part = part.font_family(crate::theme::mono_font_family());
        }
        if let Some(background) = run.background {
            part = part.bg(background).px(px(2.)).rounded(px(4.));
        }
        let text = run.text.clone();
        let content = if run.math && !run.source.is_empty() {
            math::math_element(
                &run.source,
                run.kind,
                run.foreground.unwrap_or(color),
                size_px,
            )
        } else if run.math {
            StyledText::new(text.clone())
                .with_highlights([(0..text.len(), FontStyle::Italic.into())])
                .into_any_element()
        } else {
            text.into_any_element()
        };
        line = line.child(part.child(content));
    }
    line
}

/// Draws cloze and occlusion content as a compact text block.
#[must_use]
pub fn rendered_text(
    card: &RenderedCard,
    colors: Palette,
    color: Rgba,
    size_px: f32,
) -> Div {
    let mut block = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(SPACE_GROUP))
        .text_size(px(size_px))
        .text_color(color);
    if card.blocks.is_empty() {
        for line in text_lines(card) {
            block = block.child(rendered_line(&line, color, size_px));
        }
        return block;
    }
    for content in &card.blocks {
        match content {
            CardBlock::Text(lines) => {
                for line in lines {
                    let mut row = div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .justify_center()
                        .text_size(px(size_px));
                    for span in &line.spans {
                        match span.kind {
                            TextSpanKind::Plain => {
                                let runs = apply_math(vec![TextRun {
                                    text: span.text.clone(),
                                    foreground: span.foreground,
                                    background: span.background,
                                    source: String::new(),
                                    kind: MathKind::Text,
                                    math: false,
                                    code: false,
                                }]);
                                row = row.child(rendered_line(
                                    &runs, color, size_px,
                                ));
                            }
                            TextSpanKind::Code => {
                                let (ink, background) =
                                    inline_code_style(span, colors.text);
                                let mut code = div()
                                    .font_family(
                                        crate::theme::mono_font_family(),
                                    )
                                    .text_color(ink);
                                if let Some(background) = background {
                                    code = code
                                        .bg(background)
                                        .px(px(2.))
                                        .rounded(px(4.));
                                }
                                row = row.child(code.child(span.text.clone()));
                            }
                        }
                    }
                    block = block.child(row);
                }
            }
            CardBlock::Code { language, lines } => {
                block = block.child(code_block(
                    language.as_deref(),
                    lines,
                    colors,
                    size_px,
                ));
            }
        }
    }
    block
}

/// Styled runs for the last visible line of a rendered card.
#[must_use]
pub fn preview_runs(rendered: &RenderedCard) -> Vec<TextRun> {
    let mut lines = text_lines(rendered);
    let last = lines
        .iter()
        .rposition(|line| line.iter().any(|run| !run.text.trim().is_empty()))
        .unwrap_or(0);
    lines.get_mut(last).cloned().unwrap_or_default()
}

/// Draws one preview line with its color overrides.
#[must_use]
pub fn preview_line(rendered: &RenderedCard, color: Rgba, size_px: f32) -> Div {
    let runs = preview_runs(rendered);
    if runs.is_empty() {
        div()
            .text_size(px(size_px))
            .text_color(color)
            .child(preview_text(rendered))
    } else {
        rendered_line(&runs, color, size_px)
    }
}

fn code_block(
    language: Option<&str>,
    lines: &[Vec<CodeSpan>],
    colors: Palette,
    size_px: f32,
) -> Div {
    let code_size = (size_px * 0.68).clamp(13.0, 24.0);
    let background = if colors.base.r > 0.5 {
        colors.mantle
    } else {
        colors.crust
    };
    let mut block = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(2.))
        .p(px(SPACE_GROUP))
        .rounded(px(10.))
        .bg(background)
        .border_1()
        .border_color(colors.surface1)
        .text_size(px(code_size));
    if let Some(language) = language.filter(|value| !value.is_empty()) {
        block = block.child(
            div()
                .text_size(px(10.))
                .text_color(colors.overlay)
                .child(language.to_owned()),
        );
    }
    for line in lines {
        let mut row = div()
            .flex()
            .whitespace_nowrap()
            .font_family(crate::theme::mono_font_family());
        if line.is_empty() {
            row = row.child(" ");
        }
        for span in line {
            row = row.child(
                div()
                    .text_color(token_color(span.kind, colors))
                    .child(span.text.clone()),
            );
        }
        block = block.child(row);
    }
    block
}

fn inline_code_style(span: &TextSpan, fallback: Rgba) -> (Rgba, Option<Rgba>) {
    (span.foreground.unwrap_or(fallback), span.background)
}

fn token_color(kind: TokenKind, colors: Palette) -> Rgba {
    match kind {
        TokenKind::Plain => colors.text,
        TokenKind::Comment => colors.overlay,
        TokenKind::Keyword => colors.mauve,
        TokenKind::Type => colors.blue,
        TokenKind::Function => colors.teal,
        TokenKind::String => colors.green,
        TokenKind::Number => colors.peach,
        TokenKind::Operator => colors.red,
        TokenKind::Punctuation => colors.subtext,
    }
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

    /// Drops cached load failures so files that arrive later are retried.
    pub fn forget_failed(&mut self) {
        self.items
            .retain(|_, item| !matches!(item.get(), Some(Err(_))));
        let items = &self.items;
        self.order.retain(|key| items.contains_key(key));
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
    math_segments(&rendered.text)
        .into_iter()
        .map(|segment| segment.text)
        .collect::<String>()
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
    use super::markup::{ClozeKind, cloze_segments, strip_tags, styled_text};
    use super::{
        CardBlock, CardRequest, CodeSpan, MathKind, MediaKind, MediaRef, Side,
        TemplateNode, TextSpan, TextSpanKind, TokenKind, answer_only_media,
        blocks_text, card_blocks, cloze_text, highlight_code, media_refs,
        occlusion_overlays, parse_template, preview_card, render_card,
    };
    use gpui::{ParentElement, Styled};

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
            raw_json: None,
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
            cloze_segments("{{c1::one}} {{c2::two}}", 0, Side::Answer);
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
        let _ = super::rendered_text(
            &card,
            crate::theme::MOCHA,
            crate::theme::MOCHA.text,
            84.0,
        );
        let _ = crate::theme::MOCHA;
    }
    #[test]
    fn math_runs_stay_inline_with_surrounding_text() {
        let runs = super::apply_math(vec![super::TextRun {
            text: String::from("Find \\(x^2\\) today"),
            ..Default::default()
        }]);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "Find ");
        assert_eq!(runs[0].kind, MathKind::Text);
        assert_eq!(runs[1].text, "x²");
        assert_eq!(runs[1].source, "x^2");
        assert_eq!(runs[1].kind, MathKind::Inline);
        assert_eq!(runs[2].text, " today");
        assert_eq!(runs[2].kind, MathKind::Text);
    }

    #[test]
    fn empty_math_does_not_absorb_following_text() {
        let runs = super::apply_math(vec![super::TextRun {
            text: String::from("\\(\\) then"),
            ..Default::default()
        }]);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].kind, MathKind::Inline);
        assert_eq!(runs[1].text, " then");
        assert_eq!(runs[1].kind, MathKind::Text);
    }

    #[test]
    fn compact_preview_hides_math_delimiters() {
        let model = model(&["Front"], "{{Front}}", "{{Front}}");
        let rendered = render_card(
            &model,
            &note(&["Area \\(\\pi r^2\\)"]),
            CardRequest::question(0),
        );
        assert_eq!(super::preview_text(&rendered), "Area πr²");
    }

    #[test]
    fn inline_colors_survive_template_rendering() {
        let form = model(
            &["Front", "Back"],
            "{{Front}}",
            "{{FrontSide}}<hr id=answer>{{Back}}",
        );
        let card = render_card(
            &form,
            &note(&[
                "<span style=\"color: #f38ba8; background-color: #1e1e2e\">Pink</span> plain",
                "Answer",
            ]),
            CardRequest::question(0),
        );
        assert_eq!(card.text, "Pink plain");
        assert_eq!(card.runs.len(), 2);
        assert_eq!(card.runs[0].text, "Pink");
        assert_eq!(card.runs[0].foreground, Some(crate::theme::MOCHA.red));
        assert_eq!(card.runs[0].background, Some(crate::theme::MOCHA.base));
        assert_eq!(card.runs[1].text, " plain");
        assert_eq!(card.runs[1].foreground, None);
        assert_eq!(card.runs[1].background, None);
    }

    #[test]
    fn nested_color_spans_inherit_unset_properties() {
        let runs = styled_text(
            "<span style=\"background-color: #f9e2af\">base<span style=\"color: #f38ba8\">colored</span></span>",
        );
        assert_eq!(runs[0].text, "base");
        assert_eq!(
            runs[0].background,
            crate::theme::parse_hex_color("#f9e2af")
        );
        assert_eq!(runs[0].foreground, None);
        assert_eq!(runs[1].text, "colored");
        assert_eq!(runs[1].foreground, Some(crate::theme::MOCHA.red));
        assert_eq!(
            runs[1].background,
            crate::theme::parse_hex_color("#f9e2af")
        );
        let inherited = styled_text(
            "<span style=\"color:#f38ba8\">one<span style=\"color:inherit\"> two</span></span>",
        );
        assert_eq!(inherited[0].text, "one two");
        assert_eq!(inherited[0].foreground, Some(crate::theme::MOCHA.red));
    }

    #[test]
    fn color_runs_follow_cloze_question_and_answer_labels() {
        let mut form = model(&["Text"], "{{cloze:Text}}", "{{cloze:Text}}");
        form.kind = NoteKind::Cloze;
        let note = note(&[
            "<span style=\"color:#a6e3a1\">{{c1::Paris}} is nice</span>",
            "",
        ]);
        let question = render_card(&form, &note, CardRequest::question(0));
        assert_eq!(question.text, "[...] is nice");
        assert!(
            question.runs[0].text.starts_with("[...]"),
            "question label keeps its place in the styled line"
        );
        assert!(
            question
                .runs
                .iter()
                .all(|run| run.foreground == Some(crate::theme::MOCHA.green))
        );
        let answer = render_card(&form, &note, CardRequest::answer_only(0));
        assert_eq!(answer.text, "Paris is nice");
        assert!(
            answer.runs[0].text.starts_with("Paris"),
            "answer label keeps its place in the styled line"
        );
        assert!(
            answer
                .runs
                .iter()
                .all(|run| run.foreground == Some(crate::theme::MOCHA.green))
        );
    }

    #[test]
    fn cloze_labels_keep_colors_from_each_source_run() {
        let mut form = model(&["Text"], "{{cloze:Text}}", "{{cloze:Text}}");
        form.kind = NoteKind::Cloze;
        let note = note(&[
            "<span style=\"color:#f38ba8\">{{c1::one</span><span style=\"color:#a6e3a1\"> two}}</span>",
            "",
        ]);
        let answer = render_card(&form, &note, CardRequest::answer_only(0));
        assert_eq!(answer.text, "one two");
        assert_eq!(answer.runs[0].text, "one");
        assert_eq!(answer.runs[0].foreground, Some(crate::theme::MOCHA.red));
        assert_eq!(answer.runs[1].text, " two");
        assert_eq!(answer.runs[1].foreground, Some(crate::theme::MOCHA.green));
        assert_eq!(answer.runs.len(), 2);
    }

    #[test]
    fn html_code_block_keeps_language_indentation_and_blanks() {
        let card = render_card(
            &model(&["Front"], "{{Front}}", "{{Front}}"),
            &note(&[
                "<pre data-language=\"rust\"><code>fn main() {\n    println!(\"hi\");\n\n}</code></pre>",
            ]),
            CardRequest::question(0),
        );
        assert!(matches!(
            card.blocks.first(),
            Some(CardBlock::Code {
                language: Some(language),
                ..
            }) if language == "rust"
        ));
        assert_eq!(
            blocks_text(&card.blocks),
            "fn main() {\n    println!(\"hi\");\n\n}"
        );
        let Some(CardBlock::Code { lines, .. }) = card.blocks.first() else {
            return;
        };
        assert!(lines.iter().any(|line| {
            line.iter().any(|span| {
                span.kind == TokenKind::Keyword && span.text == "fn"
            })
        }));
        assert!(lines.iter().any(|line| {
            line.iter().any(|span| {
                span.kind == TokenKind::String && span.text == "\"hi\""
            })
        }));
    }

    #[test]
    fn answer_only_code_is_rendered_after_the_front() {
        let card = render_card(
            &model(
                &["Front", "Back"],
                "{{Front}}",
                "{{FrontSide}}<hr id=answer>{{Back}}",
            ),
            &note(&[
                "Question?",
                "<pre data-language=\"python\">print(\"hi\")</pre>",
            ]),
            CardRequest::answer_only(0),
        );
        assert!(
            card.blocks
                .iter()
                .any(|block| matches!(block, CardBlock::Code { .. }))
        );
        assert!(!card.text.contains("Question?"));
    }

    #[test]
    fn fenced_code_is_a_block_without_fence_markers() {
        let card = render_card(
            &model(&["Front"], "{{Front}}", "{{Front}}"),
            &note(&["```python\ndef add(a):\n    return a + 1 # note\n```"]),
            CardRequest::question(0),
        );
        assert!(matches!(
            card.blocks.first(),
            Some(CardBlock::Code {
                language: Some(language),
                ..
            }) if language == "python"
        ));
        assert!(!card.text.contains("```"));
        assert!(card.text.contains("def add(a):"));
    }

    #[test]
    fn inline_code_is_a_code_span() {
        let blocks = card_blocks("Use `<b>` here.");
        let Some(CardBlock::Text(lines)) = blocks.first() else {
            return;
        };
        let spans: &[TextSpan] =
            lines.first().map_or(&[], |line| line.spans.as_slice());
        assert_eq!(spans.len(), 3);
        assert_eq!(blocks_text(&blocks), "Use <b> here.");
        assert!(spans.first().is_some_and(|span| span.text.ends_with(' ')));
        assert!(spans.last().is_some_and(|span| span.text.starts_with(' ')));
        assert_eq!(
            spans.get(1).map(|span| span.kind),
            Some(TextSpanKind::Code)
        );
        assert_eq!(spans.get(1).map(|span| span.text.as_str()), Some("<b>"));
    }

    #[test]
    fn code_tokens_are_colored_by_kind() {
        let lines =
            highlight_code("fn main() { let n = 42; // done", Some("rust"));
        let spans: &[CodeSpan] = lines.first().map_or(&[], Vec::as_slice);
        assert!(
            spans.iter().any(
                |span| span.kind == TokenKind::Keyword && span.text == "fn"
            )
        );
        assert!(spans.iter().any(
            |span| span.kind == TokenKind::Function && span.text == "main"
        ));
        assert!(spans.iter().any(|span| span.kind == TokenKind::Number && span.text == "42"));
        assert!(
            spans.iter().any(|span| span.kind == TokenKind::Comment
                && span.text == "// done")
        );
    }
    #[test]
    fn cloze_across_inline_code_is_resolved() {
        let mut form = ranki_model::StockModelKind::Cloze.definition();
        form.id = 1;
        let note = note(&["Use {{c1::`inline`}} here", ""]);
        let question = render_card(&form, &note, CardRequest::question(0));
        let answer = render_card(&form, &note, CardRequest::answer_only(0));
        assert_eq!(question.text, "Use [...] here");
        assert_eq!(answer.text, "Use inline here");
        assert!(!answer.text.contains("{{c1::"));
    }

    #[test]
    fn fenced_code_has_no_trailing_blank_row() {
        let card = render_card(
            &model(&["Front"], "{{Front}}", "{{Front}}"),
            &note(&["```python\ndef add(a):\n    return a + 1\n```"]),
            CardRequest::question(0),
        );
        let Some(CardBlock::Code { lines, .. }) = card.blocks.first() else {
            return;
        };
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn inline_code_decodes_entities() {
        let blocks = card_blocks("Use `&lt;` here.");
        let Some(CardBlock::Text(lines)) = blocks.first() else {
            return;
        };
        let spans: &[TextSpan] =
            lines.first().map_or(&[], |line| line.spans.as_slice());
        assert_eq!(spans.get(1).map(|span| span.text.as_str()), Some("<"));
    }

    #[test]
    fn triple_quoted_strings_are_one_string_span() {
        let lines = highlight_code("\"\"\"hi\"\"\"", Some("python"));
        let spans: &[CodeSpan] = lines.first().map_or(&[], Vec::as_slice);
        assert!(spans.iter().all(|span| span.kind == TokenKind::String));
        assert_eq!(
            spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>(),
            "\"\"\"hi\"\"\""
        );
    }
    #[test]
    fn inline_code_can_contain_pre_tags() {
        let blocks = card_blocks("Use `<pre>foo</pre>` here.");
        assert!(
            !blocks
                .iter()
                .any(|block| matches!(block, CardBlock::Code { .. }))
        );
        let Some(CardBlock::Text(lines)) = blocks.first() else {
            return;
        };
        let spans: &[TextSpan] =
            lines.first().map_or(&[], |line| line.spans.as_slice());
        assert_eq!(
            spans.get(1).map(|span| span.text.as_str()),
            Some("<pre>foo</pre>")
        );
    }
    #[test]
    fn closed_block_comment_keeps_remaining_code() {
        let lines =
            highlight_code("let a = 1; /* note */ let b = 2;", Some("rust"));
        let text = lines
            .first()
            .map(|line| {
                line.iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>()
            })
            .unwrap_or_default();
        assert_eq!(text, "let a = 1; /* note */ let b = 2;");
    }

    #[test]
    fn hidden_markup_inside_inline_code_is_kept() {
        let blocks = card_blocks("Use `<button>OK</button>` here.");
        assert_eq!(blocks_text(&blocks), "Use <button>OK</button> here.");
    }

    #[test]
    fn crlf_fenced_code_has_no_stray_markers() {
        let blocks = card_blocks("```rust\r\nlet x = 1;\r\n```");
        assert!(matches!(
            blocks.first(),
            Some(CardBlock::Code { lines, .. }) if lines.len() == 1
        ));
        assert_eq!(blocks_text(&blocks), "let x = 1;");
    }

    #[test]
    fn invalid_closing_fence_stays_in_code() {
        let blocks = card_blocks("```rust\nx\n```not-a-close\ny\n```");
        let text = blocks_text(&blocks);
        assert!(text.contains("```not-a-close"));
        assert!(!text.contains("\ny\n```"));
    }

    #[test]
    fn colored_cards_keep_styled_runs() {
        let card = render_card(
            &model(
                &["Front"],
                "<span style=\"color:#f38ba8\">Red</span> <pre><code>x</code></pre>",
                "{{Front}}",
            ),
            &note(&["unused"]),
            CardRequest::question(0),
        );
        assert!(
            card.blocks
                .iter()
                .any(|block| matches!(block, CardBlock::Code { .. }))
        );
        assert_eq!(card.runs[0].foreground, Some(crate::theme::MOCHA.red));
        let Some(CardBlock::Text(lines)) = card.blocks.first() else {
            return;
        };
        assert_eq!(
            lines
                .first()
                .and_then(|line| line.spans.first())
                .map(|span| span.foreground),
            Some(Some(crate::theme::MOCHA.red))
        );
    }
    #[test]
    fn hidden_elements_are_skipped_before_block_markers() {
        let blocks = card_blocks(
            "<script>const x = \"<pre>hidden</pre>\";</script>Visible",
        );
        assert_eq!(blocks_text(&blocks), "Visible");
        assert!(
            !blocks
                .iter()
                .any(|block| matches!(block, CardBlock::Code { .. }))
        );
    }
    #[test]
    fn styles_survive_inline_code_spans() {
        let blocks = card_blocks(
            "<span style=\"color:#f38ba8; background-color:#1e1e2e\">before `x` after</span>",
        );
        let Some(CardBlock::Text(lines)) = blocks.first() else {
            return;
        };
        let spans = lines.first().map_or(&[][..], |line| line.spans.as_slice());
        assert_eq!(spans.len(), 3);
        assert_eq!(
            spans.first().map(|span| &span.kind),
            Some(&TextSpanKind::Plain)
        );
        assert_eq!(
            spans.get(1).map(|span| &span.kind),
            Some(&TextSpanKind::Code)
        );
        assert_eq!(
            spans.get(2).map(|span| &span.kind),
            Some(&TextSpanKind::Plain)
        );
        assert!(
            spans
                .iter()
                .all(|span| span.foreground == Some(crate::theme::MOCHA.red))
        );
        assert!(
            spans
                .iter()
                .all(|span| span.background == Some(crate::theme::MOCHA.base))
        );
    }
    #[test]
    fn inline_code_style_uses_span_colors_and_defaults() {
        let colored = TextSpan {
            text: "x".into(),
            kind: TextSpanKind::Code,
            foreground: Some(crate::theme::MOCHA.red),
            background: Some(crate::theme::MOCHA.base),
        };
        assert_eq!(
            super::inline_code_style(&colored, crate::theme::MOCHA.text),
            (crate::theme::MOCHA.red, Some(crate::theme::MOCHA.base))
        );
        let plain = TextSpan {
            text: "x".into(),
            kind: TextSpanKind::Code,
            foreground: None,
            background: None,
        };
        assert_eq!(
            super::inline_code_style(&plain, crate::theme::MOCHA.text),
            (crate::theme::MOCHA.text, None)
        );
    }

    #[test]
    fn fenced_code_may_contain_hidden_elements() {
        let blocks =
            card_blocks("Example:\n```html\n<script>alert(1)</script>\n```\n");
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block, CardBlock::Code { .. }))
        );
        let Some((language, lines)) =
            blocks.iter().find_map(|block| match block {
                CardBlock::Code { language, lines } => Some((language, lines)),
                CardBlock::Text(_) => None,
            })
        else {
            return;
        };
        assert_eq!(language.as_deref(), Some("html"));
        assert_eq!(lines.len(), 1);
        assert!(blocks_text(&blocks).contains("<script>alert(1)</script>"));
    }
    #[test]
    fn inline_code_spans_are_skipped_before_block_markers() {
        let blocks =
            card_blocks("<code><pre>hello</pre><script>keep</script></code>");
        assert!(
            !blocks
                .iter()
                .any(|block| matches!(block, CardBlock::Code { .. }))
        );
        assert!(blocks_text(&blocks).contains("<script>keep</script>"));
    }
    #[gpui::test]
    fn long_card_text_stays_inside_its_box(cx: &mut gpui::TestAppContext) {
        struct Probe;

        impl gpui::Render for Probe {
            fn render(
                &mut self,
                _: &mut gpui::Window,
                _: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                gpui::div()
                    .w(gpui::px(240.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(super::rendered_line(
                        &[super::TextRun {
                            text: String::from(
                                "<span style=\"background-color: i",
                            ),
                            ..Default::default()
                        }],
                        crate::theme::MOCHA.text,
                        52.,
                    ))
            }
        }

        let (probe, cx) = cx.add_window_view(|_, _| Probe);
        cx.draw(
            gpui::point(gpui::px(0.), gpui::px(0.)),
            gpui::size(gpui::px(240.), gpui::px(400.)),
            |_, _| probe.clone(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let run = cx.debug_bounds("card-text-run").expect("run bounds");
        assert!(run.right() <= gpui::px(240.), "{run:?}");
    }
}
