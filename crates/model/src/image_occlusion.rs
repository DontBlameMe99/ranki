//! Parsing and preservation of Anki image-occlusion field values.

use std::collections::BTreeMap;

use crate::cloze::cloze_markers;

/// Shape encoded in an image-occlusion cloze marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Rect,
    Ellipse,
    Polygon,
    Text,
}

impl ShapeKind {
    /// Shape name used in Anki's `image-occlusion:` marker.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rect => "rect",
            Self::Ellipse => "ellipse",
            Self::Polygon => "polygon",
            Self::Text => "text",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "rect" => Some(Self::Rect),
            "ellipse" => Some(Self::Ellipse),
            "polygon" => Some(Self::Polygon),
            "text" => Some(Self::Text),
            _ => None,
        }
    }
}

/// One parsed image-occlusion shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeMask {
    pub shape: ShapeKind,
    /// Normalized coordinates and options. Unknown keys are kept as written.
    pub properties: BTreeMap<String, String>,
    /// One-based cloze numbers associated with this shape.
    pub ordinals: Vec<u32>,
    /// The exact marker body, preserved for Anki round trips.
    pub raw: String,
}

/// Parsed contents of an `Occlusion` field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageOcclusionField {
    pub shapes: Vec<ShapeMask>,
    /// Original field text, including `<br>` separators and escaping.
    pub raw: String,
}

/// Parses Anki's image-occlusion cloze markers without rewriting them.
#[must_use]
pub fn parse_image_occlusion_field(text: &str) -> ImageOcclusionField {
    let mut shapes = Vec::new();
    for marker in cloze_markers(text) {
        let Some(mask) = parse_shape(marker.body, &marker.numbers) else {
            continue;
        };
        shapes.push(mask);
    }
    ImageOcclusionField {
        shapes,
        raw: text.to_owned(),
    }
}

fn parse_shape(body: &str, ordinals: &[u32]) -> Option<ShapeMask> {
    let mut parts = split_unescaped(body, ':').into_iter();
    if parts.next()? != "image-occlusion" {
        return None;
    }
    let shape = ShapeKind::parse(parts.next()?)?;
    let mut properties = BTreeMap::new();
    for part in parts {
        let mut pair = split_unescaped(part, '=').into_iter();
        let key = unescape(pair.next()?);
        // Anki keeps everything after the first `=` in the value.
        let value = unescape(&pair.collect::<Vec<_>>().join("="));
        let _ = properties.insert(key, value);
    }
    Some(ShapeMask {
        shape,
        properties,
        ordinals: ordinals.to_vec(),
        raw: body.to_owned(),
    })
}

fn split_unescaped(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut escaped = false;
    for (index, character) in text.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == separator {
            parts.push(text.get(start..index).unwrap_or(""));
            start = index.saturating_add(character.len_utf8());
        }
    }
    parts.push(text.get(start..).unwrap_or(""));
    parts
}

#[expect(
    clippy::excessive_nesting,
    reason = "escaping is clearer in a single character pass"
)]
fn unescape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            if let Some(next) = characters.next() {
                output.push(next);
            }
        } else {
            output.push(character);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::ShapeKind;
    use super::parse_image_occlusion_field;

    #[test]
    fn parses_all_stock_shapes_and_keeps_the_field() {
        let text = concat!(
            "{{c1::image-occlusion:rect:left=.2325:top=.3261:width=.202:height=.0975}}<br>",
            "{{c2::image-occlusion:ellipse:left=.1:top=.2:width=.3:height=.4:rx=.15:ry=.2}}<br>",
            "{{c3::image-occlusion:polygon:points=0,0 10,10 20,0}}<br>",
            "{{c4::image-occlusion:text:text=label:fs=14:scale=1:angle=0}}",
        );
        let field = parse_image_occlusion_field(text);
        assert_eq!(field.raw, text);
        assert_eq!(field.shapes.len(), 4);
        assert_eq!(field.shapes[0].shape, ShapeKind::Rect);
        assert_eq!(
            field.shapes[0].properties.get("left").map(String::as_str),
            Some(".2325")
        );
        assert_eq!(field.shapes[1].shape, ShapeKind::Ellipse);
        assert_eq!(
            field.shapes[1].properties.get("rx").map(String::as_str),
            Some(".15")
        );
        assert_eq!(field.shapes[2].shape, ShapeKind::Polygon);
        assert_eq!(
            field.shapes[2].properties.get("points").map(String::as_str),
            Some("0,0 10,10 20,0")
        );
        assert_eq!(field.shapes[3].shape, ShapeKind::Text);
        assert_eq!(field.shapes[3].ordinals, [4]);
        assert_eq!(
            field.shapes[3].properties.get("fs").map(String::as_str),
            Some("14")
        );
    }

    #[test]
    fn preserves_escapes_and_multiple_ordinals() {
        let text = r"{{c1,2::image-occlusion:text:text=my\:label:oi=1}}";
        let field = parse_image_occlusion_field(text);
        assert_eq!(field.shapes.len(), 1);
        assert_eq!(field.shapes[0].ordinals, [1, 2]);
        assert_eq!(
            field.shapes[0].properties.get("text").map(String::as_str),
            Some("my:label")
        );
        assert_eq!(
            field.shapes[0].raw,
            r"image-occlusion:text:text=my\:label:oi=1"
        );
    }

    #[test]
    fn keeps_equals_signs_inside_property_values() {
        let text = "{{c1::image-occlusion:text:text=x=2:fs=14}}";
        let field = parse_image_occlusion_field(text);
        assert_eq!(
            field.shapes[0].properties.get("text").map(String::as_str),
            Some("x=2")
        );
        assert_eq!(
            field.shapes[0].properties.get("fs").map(String::as_str),
            Some("14")
        );
    }
}
