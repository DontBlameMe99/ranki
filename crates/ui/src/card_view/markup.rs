//! Pure parsing of card markup into media, cloze, and occlusion data.

use super::Side;
use ranki_model::{
    cloze_ordinal,
    image_occlusion::{ImageOcclusionField, ShapeKind},
    parse_image_occlusion_field,
};

/// A media reference extracted from a field value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaRef {
    pub kind: MediaKind,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Sound,
}

/// Media references that appear on the answer but not on the question.
#[must_use]
pub fn answer_only_media(
    question: &[MediaRef],
    answer: &[MediaRef],
) -> Vec<MediaRef> {
    answer
        .iter()
        .filter(|media| !question.contains(media))
        .cloned()
        .collect()
}

/// One rendered cloze span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClozeSegment {
    pub text: String,
    pub kind: ClozeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClozeKind {
    Plain,
    ActiveQuestion,
    ActiveAnswer,
    Inactive,
}

/// `MathJax` markup in a field becomes these display spans. Anki stores inline
/// math between `\(` and `\)` and display math between `\[` and `\]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MathSegment {
    pub text: String,
    pub kind: MathKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathKind {
    Text,
    Inline,
    Block,
}

const MATH_DELIMITERS: [(&str, &str, MathKind); 3] = [
    ("\\(", "\\)", MathKind::Inline),
    ("\\[", "\\]", MathKind::Block),
    ("$$", "$$", MathKind::Block),
];

/// Splits card text into plain runs and `MathJax` expressions.
#[must_use]
pub fn math_segments(input: &str) -> Vec<MathSegment> {
    let mut segments = Vec::new();
    let mut cursor = 0usize;
    while let Some(rest) = input.get(cursor..) {
        let mut found: Option<(usize, &str, &str, MathKind)> = None;
        for (open, close, kind) in MATH_DELIMITERS {
            if let Some(at) = rest.find(open)
                && found.is_none_or(|(earliest, ..)| at < earliest)
            {
                found = Some((at, open, close, kind));
            }
        }
        let Some((at, open, close, kind)) = found else {
            break;
        };
        let start = cursor.saturating_add(at);
        let body_start = start.saturating_add(open.len());
        let Some(body) = input.get(body_start..) else {
            break;
        };
        let Some(end) = body.find(close) else {
            push_plain_segment(
                &mut segments,
                input.get(cursor..body_start).unwrap_or(""),
            );
            cursor = body_start;
            continue;
        };
        push_plain_segment(
            &mut segments,
            input.get(cursor..start).unwrap_or(""),
        );
        let expression = body.get(..end).unwrap_or("");
        segments.push(MathSegment {
            text: display_math(expression),
            kind,
        });
        cursor = body_start.saturating_add(end).saturating_add(close.len());
    }
    push_plain_segment(&mut segments, input.get(cursor..).unwrap_or(""));
    segments
}

fn push_plain_segment(segments: &mut Vec<MathSegment>, text: &str) {
    if !text.is_empty() {
        segments.push(MathSegment {
            text: text.to_owned(),
            kind: MathKind::Text,
        });
    }
}

/// Converts the common `MathJax` subset to glyphs a native text run can show.
fn display_math(expression: &str) -> String {
    let mut output = String::with_capacity(expression.len());
    let mut cursor = 0usize;
    while let Some(rest) = expression.get(cursor..) {
        if rest.is_empty() {
            break;
        }
        if rest.starts_with('\\') {
            if let Some((command, glyph)) = math_symbol(rest) {
                output.push_str(glyph);
                cursor = cursor.saturating_add(command.len());
                while expression.get(cursor..).is_some_and(|tail| {
                    tail.starts_with(|character: char| {
                        character.is_ascii_whitespace()
                    })
                }) {
                    cursor = cursor.saturating_add(1);
                }
                continue;
            }
            let fraction = ["\\dfrac", "\\tfrac", "\\frac"]
                .into_iter()
                .find(|command| rest.starts_with(command));
            if let Some(command) = fraction {
                cursor = cursor.saturating_add(command.len());
                let (numerator, next) = math_argument(expression, cursor);
                cursor = next;
                let (denominator, next) = math_argument(expression, cursor);
                cursor = next;
                output.push('(');
                output.push_str(&group_part(&numerator));
                output.push('/');
                output.push_str(&group_part(&denominator));
                output.push(')');
                continue;
            }
            if rest.starts_with("\\sqrt") {
                cursor = cursor.saturating_add("\\sqrt".len());
                let (radicand, next) = math_argument(expression, cursor);
                cursor = next;
                output.push('√');
                output.push_str(&group_part(&radicand));
                continue;
            }
            let name = rest
                .get(1..)
                .unwrap_or("")
                .chars()
                .take_while(char::is_ascii_alphabetic)
                .count();
            let unknown = rest.get(..name.saturating_add(1)).unwrap_or(rest);
            output.push_str(unknown);
            cursor = cursor.saturating_add(unknown.len());
            if expression
                .get(cursor..)
                .is_some_and(|tail| tail.starts_with('{'))
            {
                let (group, next) = raw_group(expression, cursor);
                output.push_str(&group);
                cursor = next;
            }
            continue;
        }
        if rest.starts_with('{') {
            let (group, next) =
                math_group(expression, cursor.saturating_add(1));
            cursor = next;
            output.push_str(&group);
            continue;
        }
        if rest.starts_with('^') || rest.starts_with('_') {
            let superscript = rest.starts_with('^');
            let after = cursor.saturating_add(1);
            let grouped = expression
                .get(after..)
                .is_some_and(|tail| tail.starts_with('{'));
            let (token, next) = math_argument(expression, after);
            cursor = next;
            if let Some(script) = script_glyph(&token, superscript) {
                output.push_str(&script);
            } else {
                output.push(if superscript { '^' } else { '_' });
                if grouped {
                    output.push('(');
                    output.push_str(&token);
                    output.push(')');
                } else {
                    output.push_str(&token);
                }
            }
            continue;
        }
        let Some(character) = rest.chars().next() else {
            break;
        };
        output.push(character);
        cursor = cursor.saturating_add(character.len_utf8());
    }
    output
}

/// Keeps slash fractions and radicals unambiguous for compound operands.
fn group_part(text: &str) -> String {
    let compound = text.chars().any(|character| {
        character.is_whitespace()
            || matches!(character, '+' | '-' | '=' | '/' | '×' | '÷')
    });
    if compound {
        format!("({text})")
    } else {
        text.to_owned()
    }
}

/// Reads a braced group or the next single token as a math argument.
fn math_argument(input: &str, start: usize) -> (String, usize) {
    let mut cursor = start;
    while input.get(cursor..).is_some_and(|rest| {
        rest.starts_with(|character: char| character.is_ascii_whitespace())
    }) {
        cursor = cursor.saturating_add(1);
    }
    if input
        .get(cursor..)
        .is_some_and(|rest| rest.starts_with('{'))
    {
        math_group(input, cursor.saturating_add(1))
    } else {
        let (token, next) = math_token(input, cursor);
        (display_math(&token), next)
    }
}

/// Copies a braced group verbatim, including its braces.
fn raw_group(input: &str, start: usize) -> (String, usize) {
    let mut depth = 0usize;
    let mut cursor = start;
    while let Some(rest) = input.get(cursor..) {
        if rest.is_empty() {
            break;
        }
        if rest.starts_with('\\') {
            cursor = cursor.saturating_add(1);
        }
        if rest.starts_with('{') {
            depth = depth.saturating_add(1);
        } else if rest.starts_with('}') {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                let end = cursor.saturating_add(1);
                return (input.get(start..end).unwrap_or("").to_owned(), end);
            }
        }
        let Some(character) = rest.chars().next() else {
            break;
        };
        cursor = cursor.saturating_add(character.len_utf8());
    }
    (input.get(start..).unwrap_or("").to_owned(), input.len())
}

/// Reads one brace-balanced group and renders its contents.
fn math_group(input: &str, start: usize) -> (String, usize) {
    let mut depth = 0usize;
    let mut cursor = start;
    while let Some(rest) = input.get(cursor..) {
        if rest.is_empty() {
            break;
        }
        if rest.starts_with('\\') {
            cursor = cursor.saturating_add(1);
            continue;
        }
        if rest.starts_with('{') {
            depth = depth.saturating_add(1);
        } else if rest.starts_with('}') {
            if depth == 0 {
                let group = input.get(start..cursor).unwrap_or("");
                return (display_math(group), cursor.saturating_add(1));
            }
            depth = depth.saturating_sub(1);
        }
        let Some(character) = rest.chars().next() else {
            break;
        };
        cursor = cursor.saturating_add(character.len_utf8());
    }
    (display_math(input.get(start..).unwrap_or("")), input.len())
}

/// Reads a command or a single character used as a script.
fn math_token(input: &str, start: usize) -> (String, usize) {
    let Some(rest) = input.get(start..) else {
        return (String::new(), input.len());
    };
    if rest.starts_with('\\') {
        let length = rest
            .get(1..)
            .unwrap_or("")
            .chars()
            .take_while(char::is_ascii_alphabetic)
            .count()
            .saturating_add(1);
        let token = rest.get(..length).unwrap_or(rest);
        return (token.to_owned(), start.saturating_add(token.len()));
    }
    let Some(character) = rest.chars().next() else {
        return (String::new(), input.len());
    };
    (
        character.to_string(),
        start.saturating_add(character.len_utf8()),
    )
}

/// Maps `MathJax` commands to Unicode symbols. Longest commands come first.
fn math_symbol(input: &str) -> Option<(&'static str, &'static str)> {
    const SYMBOLS: &[(&str, &str)] = &[
        ("\\Leftrightarrow", "⇔"),
        ("\\Longrightarrow", "⟹"),
        ("\\qquad", "  "),
        ("\\rightarrow", "→"),
        ("\\leftarrow", "←"),
        ("\\Rightarrow", "⇒"),
        ("\\varepsilon", "ε"),
        ("\\varnothing", "∅"),
        ("\\ldots", "…"),
        ("\\partial", "∂"),
        ("\\nabla", "∇"),
        ("\\notin", "∉"),
        ("\\subset", "⊂"),
        ("\\alpha", "α"),
        ("\\beta", "β"),
        ("\\gamma", "γ"),
        ("\\delta", "δ"),
        ("\\epsilon", "ε"),
        ("\\theta", "θ"),
        ("\\lambda", "λ"),
        ("\\sigma", "σ"),
        ("\\omega", "ω"),
        ("\\Gamma", "Γ"),
        ("\\Delta", "Δ"),
        ("\\Theta", "Θ"),
        ("\\Lambda", "Λ"),
        ("\\Sigma", "Σ"),
        ("\\Omega", "Ω"),
        ("\\times", "×"),
        ("\\cdot", "⋅"),
        ("\\sum", "∑"),
        ("\\prod", "∏"),
        ("\\int", "∫"),
        ("\\infty", "∞"),
        ("\\approx", "≈"),
        ("\\neq", "≠"),
        ("\\sim", "∼"),
        ("\\leq", "≤"),
        ("\\geq", "≥"),
        ("\\forall", "∀"),
        ("\\exists", "∃"),
        ("\\cup", "∪"),
        ("\\cap", "∩"),
        ("\\dots", "…"),
        ("\\quad", " "),
        ("\\left", ""),
        ("\\right", ""),
        ("\\text", ""),
        ("\\pi", "π"),
        ("\\mu", "μ"),
        ("\\phi", "φ"),
        ("\\div", "÷"),
        ("\\pm", "±"),
        ("\\le", "≤"),
        ("\\ge", "≥"),
        ("\\ne", "≠"),
        ("\\in", "∈"),
        ("\\to", "→"),
        ("\\,", ""),
        ("\\;", ""),
        ("\\!", ""),
        ("\\ ", " "),
    ];
    SYMBOLS
        .iter()
        .find(|(command, _)| {
            if !input.starts_with(command) {
                return false;
            }
            let letter = command
                .chars()
                .last()
                .is_some_and(|character| character.is_ascii_alphabetic());
            let next = input
                .get(command.len()..)
                .and_then(|tail| tail.chars().next());
            !letter
                || !next
                    .is_some_and(|character| character.is_ascii_alphabetic())
        })
        .copied()
}

/// Converts scripts into Unicode where the alphabet has matching glyphs.
fn script_glyph(token: &str, superscript: bool) -> Option<String> {
    let digit = if superscript {
        "⁰¹²³⁴⁵⁶⁷⁸⁹"
    } else {
        "₀₁₂₃₄₅₆₇₈₉"
    };
    let mut output = String::with_capacity(token.len());
    for character in token.chars() {
        let glyph = match (character, superscript) {
            ('0'..='9', _) => character
                .to_digit(10)
                .and_then(|value| usize::try_from(value).ok())
                .and_then(|index| digit.chars().nth(index))
                .unwrap_or(character),
            ('+', true) => '⁺',
            ('+', false) => '₊',
            ('-', true) => '⁻',
            ('-', false) => '₋',
            ('=', true) => '⁼',
            ('=', false) => '₌',
            ('(', true) => '⁽',
            ('(', false) => '₍',
            (')', true) => '⁾',
            (')', false) => '₎',
            ('n', false) => 'ₙ',
            ('i', false) => 'ᵢ',
            ('j', false) => 'ⱼ',
            ('n', true) => 'ⁿ',
            ('i', true) => 'ⁱ',
            (other, _) => other,
        };
        if glyph == character && !character.is_ascii_digit() {
            return None;
        }
        output.push(glyph);
    }
    Some(output)
}

/// Occlusion overlay with image-relative coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct MaskOverlay {
    pub shape: ShapeKind,
    pub ordinals: Vec<u32>,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub points: Vec<(f32, f32)>,
    pub text: Option<String>,
    pub occlude_inactive: bool,
    pub active: bool,
}

pub(super) fn is_empty_html(value: &str) -> bool {
    strip_tags(value).trim().is_empty() && media_refs(value).is_empty()
}

/// Converts the supported markup subset to plain text with line breaks.
#[must_use]
pub fn strip_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    while cursor < input.len() {
        let rest = input.get(cursor..).unwrap_or("");
        if rest.starts_with("<script") || rest.starts_with("<style") {
            let closing = if rest.starts_with("<script") {
                "</script>"
            } else {
                "</style>"
            };
            if let Some(end) = rest.find(closing) {
                cursor =
                    cursor.saturating_add(end).saturating_add(closing.len());
            } else {
                break;
            }
            continue;
        }
        if rest.starts_with("<button") {
            if let Some(end) = rest.find("</button>") {
                cursor = cursor
                    .saturating_add(end)
                    .saturating_add("</button>".len());
            } else {
                break;
            }
            continue;
        }
        if rest.starts_with('<')
            && rest
                .get(1..)
                .and_then(|tail| tail.chars().next())
                .is_some_and(|character| {
                    character.is_ascii_alphabetic()
                        || matches!(character, '/' | '!' | '?')
                })
            && let Some(end) = rest.find('>')
        {
            let tag = rest.get(..end.saturating_add(1)).unwrap_or("");
            if tag.starts_with("<br")
                || tag.starts_with("<hr")
                || tag.starts_with("</div")
                || tag.starts_with("</p")
            {
                output.push('\n');
            }
            cursor = cursor.saturating_add(end).saturating_add(1);
            continue;
        }
        let Some(character) = rest.chars().next() else {
            break;
        };
        output.push(character);
        cursor = cursor.saturating_add(character.len_utf8());
    }
    normalize_newlines(&decode_entities(&output))
}

/// Decodes the HTML entities Anki fields use for displayed characters.
fn decode_entities(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    while let Some(rest) = input.get(cursor..) {
        let Some(at) = rest.find('&') else {
            output.push_str(rest);
            break;
        };
        output.push_str(rest.get(..at).unwrap_or(""));
        cursor = cursor.saturating_add(at).saturating_add(1);
        let tail = input.get(cursor..).unwrap_or("");
        let Some(end) = tail.find(';').filter(|end| *end <= 10) else {
            output.push('&');
            continue;
        };
        let entity = tail.get(..end).unwrap_or("");
        if let Some(decoded) = decode_entity(entity) {
            output.push_str(&decoded);
            cursor = cursor.saturating_add(end).saturating_add(1);
        } else {
            output.push('&');
        }
    }
    output
}

fn decode_entity(entity: &str) -> Option<String> {
    match entity {
        "amp" => return Some(String::from("&")),
        "lt" => return Some(String::from("<")),
        "gt" => return Some(String::from(">")),
        "quot" => return Some(String::from("\"")),
        "apos" | "#39" => return Some(String::from("'")),
        "nbsp" => return Some(String::from(" ")),
        _ => {}
    }
    let digits = entity.strip_prefix('#')?;
    let code = digits.strip_prefix(['x', 'X']).map_or_else(
        || digits.parse::<u32>().ok(),
        |hex| u32::from_str_radix(hex, 16).ok(),
    )?;
    char::from_u32(code).map(String::from)
}

pub(super) fn normalize_newlines(text: &str) -> String {
    let mut lines = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty());
    let mut result = lines.next().unwrap_or("").to_owned();
    for line in lines {
        result.push('\n');
        result.push_str(line);
    }
    result.trim().to_owned()
}

/// Extracts `<img src>` and `[sound:...]` references without changing fields.
#[must_use]
pub fn media_refs(input: &str) -> Vec<MediaRef> {
    let mut refs = Vec::new();
    let mut cursor = 0usize;
    while let Some(rest) = input.get(cursor..) {
        let image = rest.find("<img ").map(|at| (at, MediaKind::Image));
        let sound = rest.find("[sound:").map(|at| (at, MediaKind::Sound));
        let chosen = match (image, sound) {
            (Some(left), Some(right)) => {
                Some(if left.0 <= right.0 { left } else { right })
            }
            (Some(image), None) => Some(image),
            (None, Some(sound)) => Some(sound),
            (None, None) => None,
        };
        let Some((at, kind)) = chosen else {
            break;
        };
        cursor = cursor.saturating_add(at);
        let rest = input.get(cursor..).unwrap_or("");
        match kind {
            MediaKind::Image => {
                let tag_end = rest
                    .find('>')
                    .map_or(rest.len(), |end| end.saturating_add(1));
                let tag = rest.get(..tag_end).unwrap_or(rest);
                if let Some(after) = tag.find("src=") {
                    let tail = tag.get(after.saturating_add(4)..).unwrap_or("");
                    let quote = tail.chars().next();
                    let name = if quote == Some('"') || quote == Some('\'') {
                        let mark = quote.unwrap_or('"');
                        tail.get(1..)
                            .and_then(|value| {
                                value.find(mark).map(|end| value.get(..end))
                            })
                            .flatten()
                    } else {
                        tail.split(|c: char| c.is_whitespace() || c == '>')
                            .next()
                    };
                    if let Some(name) = name {
                        refs.push(MediaRef {
                            kind,
                            name: decode_entities(name),
                        });
                    }
                }
                cursor = cursor.saturating_add(tag_end);
            }
            MediaKind::Sound => {
                let tail = rest.get(7..).unwrap_or("");
                if let Some(end) = tail.find(']') {
                    let name = tail.get(..end).unwrap_or("");
                    refs.push(MediaRef {
                        kind,
                        name: name.to_owned(),
                    });
                    cursor = cursor
                        .saturating_add(7)
                        .saturating_add(end)
                        .saturating_add(1);
                } else {
                    break;
                }
            }
        }
    }
    refs
}

/// Renders a cloze field for a one-based card number and side.
#[must_use]
pub fn cloze_segments(
    text: &str,
    ordinal: u16,
    side: Side,
) -> Vec<ClozeSegment> {
    let number = ordinal.saturating_add(1);
    let mut segments = Vec::new();
    let mut cursor = 0usize;
    while let Some(rest) = text.get(cursor..) {
        let Some(start) = rest.find("{{c") else {
            push_plain(&mut segments, rest);
            break;
        };
        let start = cursor.saturating_add(start);
        push_plain(&mut segments, text.get(cursor..start).unwrap_or(""));
        let Some(rest) = text.get(start.saturating_add(2)..) else {
            break;
        };
        let Some(separator) = rest.find("::") else {
            push_plain(&mut segments, text.get(start..).unwrap_or(""));
            break;
        };
        let numbers = rest
            .get(..separator)
            .unwrap_or("")
            .trim()
            .trim_start_matches('c');
        let active = numbers
            .split(',')
            .filter_map(|value| value.trim().parse::<u32>().ok())
            .any(|value| u32::from(number) == value);
        let body_start = start
            .saturating_add(2)
            .saturating_add(separator)
            .saturating_add(2);
        let Some(body) =
            text.get(body_start..).and_then(|value| value.find("}}"))
        else {
            break;
        };
        let content = text
            .get(body_start..body_start.saturating_add(body))
            .unwrap_or("");
        let (answer, hint) = content.split_once("::").unwrap_or((content, ""));
        let kind = if active {
            match side {
                Side::Question => ClozeKind::ActiveQuestion,
                Side::Answer => ClozeKind::ActiveAnswer,
            }
        } else {
            ClozeKind::Inactive
        };
        let label = match kind {
            ClozeKind::ActiveQuestion if hint.is_empty() => "[...]".to_owned(),
            ClozeKind::ActiveQuestion => format!("[{hint}]"),
            ClozeKind::ActiveAnswer | ClozeKind::Inactive => answer.to_owned(),
            ClozeKind::Plain => answer.to_owned(),
        };
        segments.push(ClozeSegment { text: label, kind });
        cursor = body_start.saturating_add(body).saturating_add(2);
    }
    segments
}

fn push_plain(segments: &mut Vec<ClozeSegment>, text: &str) {
    if !text.is_empty() {
        segments.push(ClozeSegment {
            text: text.to_owned(),
            kind: ClozeKind::Plain,
        });
    }
}

/// Flattens cloze segments into the text shown on a card.
#[must_use]
#[cfg(test)]
pub fn cloze_text(text: &str, ordinal: u16, side: Side) -> String {
    cloze_segments(text, ordinal, side)
        .into_iter()
        .map(|segment| segment.text)
        .collect::<String>()
}

/// Parses image-occlusion shapes for a card side.
#[must_use]
pub fn occlusion_overlays(
    occlusion: &str,
    ordinal: u16,
    side: Side,
) -> Vec<MaskOverlay> {
    let ImageOcclusionField { shapes, .. } =
        parse_image_occlusion_field(occlusion);
    shapes
        .into_iter()
        .map(|mask| {
            let active = mask
                .ordinals
                .iter()
                .any(|number| cloze_ordinal(*number) == Some(ordinal));
            let left = parse_ratio(mask.properties.get("left"), 0.0);
            let top = parse_ratio(mask.properties.get("top"), 0.0);
            let width = parse_ratio(mask.properties.get("width"), 0.2);
            let height = parse_ratio(mask.properties.get("height"), 0.1);
            let points = mask
                .properties
                .get("points")
                .map(|value| parse_points(value, width, height))
                .unwrap_or_default();
            let text = mask.properties.get("text").cloned();
            let occlude_inactive =
                mask.properties.get("oi").is_some_and(|v| v == "1");
            MaskOverlay {
                shape: mask.shape,
                ordinals: mask.ordinals,
                left,
                top,
                width,
                height,
                points,
                text,
                occlude_inactive,
                active,
            }
        })
        .filter(|mask| match side {
            Side::Question => true,
            Side::Answer => mask.active || mask.occlude_inactive,
        })
        .collect()
}

fn parse_ratio(value: Option<&String>, fallback: f32) -> f32 {
    let Some(text) = value else {
        return fallback;
    };
    let text = text.trim();
    let (digits, percent) = text
        .strip_suffix('%')
        .map_or((text, false), |digits| (digits, true));
    let Ok(number) = digits.trim().parse::<f32>() else {
        return fallback;
    };
    if !number.is_finite() {
        return fallback;
    }
    let ratio = if percent || number > 1.0 {
        number / 100.0
    } else {
        number
    };
    ratio.clamp(0.0, 1.0)
}
fn parse_points(text: &str, width: f32, height: f32) -> Vec<(f32, f32)> {
    let scale = |value: f32, size: f32| {
        if value > 1.0 && size > 0.0 && size <= 1.0 {
            value / 100.0
        } else {
            value
        }
    };
    text.split_whitespace()
        .filter_map(|pair| {
            let (x, y) = pair.split_once(',')?;
            let x = x.parse::<f32>().ok()?;
            let y = y.parse::<f32>().ok()?;
            Some((
                scale(x, width).clamp(0.0, 1.0),
                scale(y, height).clamp(0.0, 1.0),
            ))
        })
        .collect()
}

#[cfg(test)]
mod math_tests {
    use super::{MathKind, math_segments, strip_tags};

    #[test]
    fn inline_math_uses_anki_delimiters() {
        let segments = math_segments("Area \\(\\pi r^2\\) here");
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "Area ");
        assert_eq!(segments[0].kind, MathKind::Text);
        assert_eq!(segments[1].text, "πr²");
        assert_eq!(segments[1].kind, MathKind::Inline);
        assert_eq!(segments[2].text, " here");
    }

    #[test]
    fn display_math_stays_on_its_own_span() {
        let segments = math_segments("\\[\\frac{a}{b}\\]");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "(a/b)");
        assert_eq!(segments[0].kind, MathKind::Block);
    }

    #[test]
    fn incomplete_math_stays_visible() {
        let segments = math_segments("\\(x + y");
        let text = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<String>();
        assert_eq!(text, "\\(x + y");
        assert!(
            segments
                .iter()
                .all(|segment| segment.kind == MathKind::Text)
        );
    }

    #[test]
    fn common_math_renders_as_unicode() {
        let segments = math_segments("\\(\\sqrt{x_1} \\leq \\alpha_{n}\\)");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "√x₁ ≤αₙ");
    }

    #[test]
    fn unknown_commands_are_kept() {
        let segments = math_segments("\\(\\operatorname{sin} x\\)");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "\\operatorname{sin} x");
        let segments = math_segments("\\(\\text{bold}\\)");
        assert_eq!(segments[0].text, "bold");
        let segments = math_segments("\\(\\textbf{bold}\\)");
        assert_eq!(segments[0].text, "\\textbf{bold}");
    }

    #[test]
    fn fraction_keeps_compound_parts_grouped() {
        let segments = math_segments("\\(\\frac{a+b}{c}\\)");
        assert_eq!(segments[0].text, "((a+b)/c)");
    }

    #[test]
    fn a_later_pair_survives_an_unclosed_opener() {
        let segments = math_segments("a \\(b $$x^2$$");
        let texts = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(texts, ["a \\(", "b ", "x²"]);
    }

    #[test]
    fn strip_tags_keeps_math_inequalities() {
        assert_eq!(strip_tags("x \\(a < b\\) y"), "x \\(a < b\\) y");
    }

    #[test]
    fn compound_radicals_keep_their_scope() {
        let segments = math_segments("\\(\\sqrt{x+1}\\)");
        assert_eq!(segments[0].text, "√(x+1)");
        let segments = math_segments("\\(\\sqrt{x_1}\\)");
        assert_eq!(segments[0].text, "√x₁");
    }

    #[test]
    fn unsupported_grouped_scripts_keep_grouping() {
        let segments = math_segments("\\(x^{a+b}\\)");
        assert_eq!(segments[0].text, "x^(a+b)");
    }

    #[test]
    fn whitespace_before_math_arguments_is_skipped() {
        let segments = math_segments("\\(\\frac {a}{b}\\)");
        assert_eq!(segments[0].text, "(a/b)");
    }

    #[test]
    fn unknown_command_group_ends_at_its_brace() {
        let segments = math_segments("\\(\\operatorname{sin} x^2 + \\alpha\\)");
        assert_eq!(segments[0].text, "\\operatorname{sin} x² + α");
    }

    #[test]
    fn strip_tags_removes_html_formatting_inside_math() {
        assert_eq!(strip_tags("\\(x<b>+y</b>\\)"), "\\(x+y\\)");
    }
}
