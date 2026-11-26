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
        if rest.starts_with('<') {
            let Some(end) = rest.find('>') else {
                break;
            };
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
