//! Small, safe renderer for Anki-style note templates.

use std::{
    iter::Peekable,
    mem,
    path::{Path, PathBuf},
    str::Chars,
};

use egui::{
    FontId, Label, RichText, TextWrapMode,
    text::{LayoutJob, TextFormat},
};

const MAX_ENTITY_LEN: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emphasis {
    Plain,
    Bold,
    Italic,
    BoldItalic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub emphasis: Emphasis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Sound,
    Image,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaRef {
    pub kind: MediaKind,
    pub name: String,
    pub path: Option<PathBuf>,
}

/// Replaces `{{Field}}` placeholders, matching names case-insensitively.
#[expect(
    clippy::excessive_nesting,
    reason = "the template scan replaces placeholders in one pass"
)]
pub fn substitute_fields(
    template: &str,
    field_names: &[String],
    values: &[String],
) -> String {
    let mut parts = template.split("{{");
    let Some(first) = parts.next() else {
        return String::new();
    };
    let mut output = String::with_capacity(template.len());
    output.push_str(first);
    for part in parts {
        if let Some((raw_key, tail)) = part.split_once("}}") {
            let key = raw_key.trim();
            let replacement = field_names
                .iter()
                .zip(values.iter())
                .find(|(field, _)| field.eq_ignore_ascii_case(key))
                .map(|(_, field_value)| field_value.as_str());
            if let Some(field_value) = replacement {
                output.push_str(field_value);
            } else {
                output.push_str("{{");
                output.push_str(key);
                output.push_str("}}");
            }
            output.push_str(tail);
        } else {
            output.push_str("{{");
            output.push_str(part);
        }
    }
    output
}

/// Converts the supported markup subset into styled text segments.
#[expect(
    clippy::excessive_nesting,
    reason = "the HTML subset is parsed in a single scanning loop"
)]
pub fn parse_markup(input: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut bold = false;
    let mut italic = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '<' => {
                let mut raw_tag = String::new();
                for next in chars.by_ref() {
                    if next == '>' {
                        break;
                    }
                    raw_tag.push(next);
                }
                let tag = raw_tag.trim().to_ascii_lowercase();
                if tag == "br" || tag == "br/" {
                    current.push('\n');
                } else if is_tag(&tag, "b") || is_tag(&tag, "strong") {
                    flush(
                        &mut segments,
                        &mut current,
                        emphasis((bold, italic)),
                    );
                    bold = true;
                } else if is_tag(&tag, "/b") || is_tag(&tag, "/strong") {
                    flush(
                        &mut segments,
                        &mut current,
                        emphasis((bold, italic)),
                    );
                    bold = false;
                } else if is_tag(&tag, "i") || is_tag(&tag, "em") {
                    flush(
                        &mut segments,
                        &mut current,
                        emphasis((bold, italic)),
                    );
                    italic = true;
                } else if is_tag(&tag, "/i") || is_tag(&tag, "/em") {
                    flush(
                        &mut segments,
                        &mut current,
                        emphasis((bold, italic)),
                    );
                    italic = false;
                } else if is_block_tag(&tag) {
                    current.push('\n');
                }
            }
            '&' => current.push_str(&read_entity(&mut chars)),
            _ => current.push(ch),
        }
    }
    flush(&mut segments, &mut current, emphasis((bold, italic)));
    segments
}

#[cfg(test)]
pub fn plain_text(input: &str) -> String {
    parse_markup(input)
        .into_iter()
        .map(|segment| segment.text)
        .collect()
}

/// Resolves a media file name to an existing path inside `media_dir`.
pub fn media_path(media_dir: Option<&Path>, name: &str) -> Option<PathBuf> {
    let trimmed = name.trim();
    let safe = !trimmed.is_empty()
        && !trimmed.contains('/')
        && !trimmed.contains('\\')
        && trimmed != "."
        && trimmed != "..";
    if !safe {
        return None;
    }
    let path = media_dir?.join(trimmed);
    path.is_file().then_some(path)
}

/// Collects `[sound:...]` and `<img src="...">` references.
pub fn media_refs(input: &str, media_dir: Option<&Path>) -> Vec<MediaRef> {
    let mut refs = Vec::new();
    for part in input.split("[sound:").skip(1) {
        if let Some((name, _)) = part.split_once(']')
            && let Some(media) = make_media(MediaKind::Sound, name, media_dir)
        {
            refs.push(media);
        }
    }
    for tag in input.split('<').skip(1) {
        let Some((body, _)) = tag.split_once('>') else {
            continue;
        };
        let Some(rest) = body.trim_start().strip_prefix("img") else {
            continue;
        };
        if !rest
            .chars()
            .next()
            .is_none_or(|ch| ch.is_ascii_whitespace() || ch == '/')
        {
            continue;
        }
        let Some(source) = rest
            .split_whitespace()
            .find_map(|attribute| attribute.strip_prefix("src="))
        else {
            continue;
        };
        let name = source.trim_matches(|ch| matches!(ch, '"' | '\'' | '/'));
        if let Some(media) = make_media(MediaKind::Image, name, media_dir) {
            refs.push(media);
        }
    }
    refs
}

#[expect(
    clippy::excessive_nesting,
    reason = "egui lays out the card and its media in one callback chain"
)]
pub fn render(ui: &mut egui::Ui, html: &str, media_dir: Option<&Path>) {
    let color = ui.visuals().text_color();
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width().max(160.0);
    for segment in parse_markup(html) {
        let size = match segment.emphasis {
            Emphasis::Bold | Emphasis::BoldItalic => 19.0,
            Emphasis::Plain | Emphasis::Italic => 17.0,
        };
        let mut format = TextFormat::simple(FontId::proportional(size), color);
        format.italics =
            matches!(segment.emphasis, Emphasis::Italic | Emphasis::BoldItalic);
        job.append(&segment.text, 0.0, format);
    }
    ui.add(
        Label::new(job)
            .selectable(true)
            .wrap_mode(TextWrapMode::Wrap),
    );

    let refs = media_refs(html, media_dir);
    if refs.is_empty() {
        return;
    }
    ui.add_space(10.0);
    ui.horizontal_wrapped(|row| {
        row.label(RichText::new("Media").weak());
        for media in refs {
            let kind = match media.kind {
                MediaKind::Sound => "Sound",
                MediaKind::Image => "Image",
            };
            let state = if media.path.is_some() {
                "available"
            } else {
                "missing"
            };
            row.label(format!("{kind}: {} ({state})", media.name));
        }
    });
}

const fn emphasis(style: (bool, bool)) -> Emphasis {
    match style {
        (true, true) => Emphasis::BoldItalic,
        (true, false) => Emphasis::Bold,
        (false, true) => Emphasis::Italic,
        (false, false) => Emphasis::Plain,
    }
}

fn flush(
    segments: &mut Vec<Segment>,
    current: &mut String,
    emphasis: Emphasis,
) {
    if current.is_empty() {
        return;
    }
    segments.push(Segment {
        text: mem::take(current),
        emphasis,
    });
}

fn is_tag(tag: &str, name: &str) -> bool {
    tag == name
        || tag
            .strip_prefix(name)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
}

fn is_block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "/p"
            | "div"
            | "/div"
            | "li"
            | "ul"
            | "/ul"
            | "ol"
            | "/ol"
            | "blockquote"
            | "/blockquote"
            | "tr"
            | "/tr"
    )
}

fn read_entity(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut entity = String::from("&");
    for _ in 0..MAX_ENTITY_LEN {
        match chars.peek().copied() {
            Some(';') => {
                chars.next();
                entity.push(';');
                return decode_entity(&entity)
                    .map_or(entity, |decoded| decoded.to_string());
            }
            Some(ch) if ch.is_ascii_alphanumeric() || ch == '#' => {
                chars.next();
                entity.push(ch);
            }
            _ => return entity,
        }
    }
    entity
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "&amp;" => return Some('&'),
        "&lt;" => return Some('<'),
        "&gt;" => return Some('>'),
        "&quot;" => return Some('"'),
        "&apos;" => return Some('\''),
        "&nbsp;" => return Some('\u{a0}'),
        _ => {}
    }
    if let Some(hex) =
        entity.strip_prefix("&#x").and_then(|s| s.strip_suffix(';'))
    {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    if let Some(decimal) =
        entity.strip_prefix("&#").and_then(|s| s.strip_suffix(';'))
    {
        return decimal.parse::<u32>().ok().and_then(char::from_u32);
    }
    None
}

fn make_media(
    kind: MediaKind,
    name: &str,
    media_dir: Option<&Path>,
) -> Option<MediaRef> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(MediaRef {
        kind,
        name: trimmed.to_owned(),
        path: media_path(media_dir, trimmed),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        Emphasis, MediaKind, Segment, media_path, media_refs, parse_markup,
        plain_text, substitute_fields,
    };

    fn names() -> Vec<String> {
        vec!["Front".to_owned(), "Back".to_owned()]
    }

    fn values() -> Vec<String> {
        vec!["Question?".to_owned(), "Answer!".to_owned()]
    }

    fn segment(text: &str, emphasis: Emphasis) -> Segment {
        Segment {
            text: text.to_owned(),
            emphasis,
        }
    }

    #[test]
    fn substitute_fields_replaces_placeholders() {
        let result =
            substitute_fields("{{Front}} / {{Back}}", &names(), &values());
        assert_eq!(result, "Question? / Answer!");
    }

    #[test]
    fn substitute_fields_ignores_case_and_keeps_unknown() {
        let result =
            substitute_fields("{{front}} {{Missing}}", &names(), &values());
        assert_eq!(result, "Question? {{Missing}}");
    }

    #[test]
    fn markup_strips_unknown_tags() {
        assert_eq!(plain_text("a<script>bad()</script>c"), "abad()c");
    }

    #[test]
    fn markup_decodes_entities() {
        assert_eq!(
            plain_text("A &amp; B &lt;C&gt; &#65; &#x42; &quot;D&quot;"),
            "A & B <C> A B \"D\""
        );
    }

    #[test]
    fn markup_reports_bold_and_italic_segments() {
        assert_eq!(
            parse_markup("plain <b>bold</b> <i>italic</i>"),
            vec![
                segment("plain ", Emphasis::Plain),
                segment("bold", Emphasis::Bold),
                segment(" ", Emphasis::Plain),
                segment("italic", Emphasis::Italic),
            ]
        );
    }

    #[test]
    fn markup_turns_breaks_and_blocks_into_newlines() {
        assert_eq!(
            plain_text("one<br>two<div>three</div>"),
            "one\ntwo\nthree\n"
        );
    }

    #[test]
    fn media_path_resolves_existing_files() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(
            media_path(Some(dir), "Cargo.toml"),
            Some(dir.join("Cargo.toml"))
        );
    }

    #[test]
    fn media_path_rejects_traversal() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(media_path(Some(dir), "../Cargo.toml"), None);
        assert_eq!(media_path(Some(dir), "sub/file.png"), None);
    }

    #[test]
    fn media_path_ignores_missing_files() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(media_path(Some(dir), "missing.mp3"), None);
        assert_eq!(media_path(None, "Cargo.toml"), None);
    }

    #[test]
    fn media_refs_find_sounds_and_images() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let refs =
            media_refs("[sound:clip.mp3] <img src=\"Cargo.toml\">", Some(dir));
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].kind, MediaKind::Sound);
        assert_eq!(refs[0].name, "clip.mp3");
        assert_eq!(refs[0].path, None);
        assert_eq!(refs[1].kind, MediaKind::Image);
        assert_eq!(refs[1].path, Some(dir.join("Cargo.toml")));
    }
}
