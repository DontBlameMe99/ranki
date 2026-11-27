//! Pure parsing of card markup into media, cloze, and occlusion data.

use gpui::Rgba;

use super::{Side, TextRun};
use crate::theme::parse_hex_color;
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

/// A rendered card is a sequence of text and code blocks.
#[derive(Clone, Debug, PartialEq)]
pub enum CardBlock {
    Text(Vec<CardLine>),
    Code {
        language: Option<String>,
        lines: Vec<Vec<CodeSpan>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CardLine {
    pub spans: Vec<TextSpan>,
}

impl CardLine {
    #[must_use]
    pub fn text(&self) -> String {
        self.spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    pub text: String,
    pub kind: TextSpanKind,
    pub foreground: Option<Rgba>,
    pub background: Option<Rgba>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSpanKind {
    Plain,
    Code,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeSpan {
    pub text: String,
    pub kind: TokenKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Comment,
    Keyword,
    Type,
    Function,
    String,
    Number,
    Operator,
    Punctuation,
}

/// Flattens structured card content into the plain text used by previews.
#[must_use]
pub fn blocks_text(blocks: &[CardBlock]) -> String {
    let mut output = String::new();
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        let lines = match block {
            CardBlock::Text(lines) => {
                lines.iter().map(CardLine::text).collect::<Vec<_>>()
            }
            CardBlock::Code { lines, .. } => lines
                .iter()
                .map(|line| {
                    line.iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>(),
        };
        output.push_str(&lines.join("\n"));
    }
    output
}

/// Parses Anki and fenced code blocks, leaving the rest as text spans.
#[must_use]
pub fn card_blocks(input: &str) -> Vec<CardBlock> {
    let source = input;
    let mut blocks = Vec::new();
    let mut pending = String::new();
    let mut cursor = 0usize;
    while let Some(rest) = source.get(cursor..) {
        let Some((at, fenced)) = next_block_marker(source, cursor) else {
            pending.push_str(rest);
            break;
        };
        pending.push_str(rest.get(..at).unwrap_or(""));
        flush_text(&mut blocks, &pending);
        pending.clear();
        let start = cursor.saturating_add(at);
        let parsed = if fenced {
            parse_fence(source, start)
        } else {
            parse_pre(source, start)
        };
        if let Some((block, end)) = parsed {
            blocks.push(block);
            cursor = end;
        } else if fenced {
            pending.push_str("```");
            cursor = start.saturating_add(3);
        } else {
            pending.push_str("<pre");
            cursor = start.saturating_add(4);
        }
    }
    flush_text(&mut blocks, &pending);
    blocks
}

fn next_block_marker(input: &str, cursor: usize) -> Option<(usize, bool)> {
    let rest = input.get(cursor..)?;
    let mut offset = 0usize;
    while let Some(tail) = rest.get(offset..) {
        let fence = tail.find("```").map(|at| (at, 0u8, ""));
        let pre = tail.find("<pre").map(|at| (at, 1, ""));
        let tick = tail.find('`').map(|at| (at, 2, ""));
        let hidden = hidden_at(tail).map(|(at, tag)| (at, 3, tag));
        let code_tag = tail.find("<code").map(|at| (at, 4, ""));
        let (at, kind, marker) = [fence, pre, tick, hidden, code_tag]
            .into_iter()
            .flatten()
            .min_by_key(|(at, kind, _)| (*at, *kind))?;
        match kind {
            0 => return Some((offset.saturating_add(at), true)),
            1 => return Some((offset.saturating_add(at), false)),
            2 => {
                let after = tail.get(at.saturating_add(1)..)?;
                if let Some(end) = after.find('`') {
                    offset = offset
                        .saturating_add(at)
                        .saturating_add(end)
                        .saturating_add(2);
                } else {
                    break;
                }
            }
            4 => {
                let after = tail.get(at.saturating_add("<code".len())..)?;
                if let Some(end) = after.find("</code>") {
                    offset = offset
                        .saturating_add(at)
                        .saturating_add("<code".len())
                        .saturating_add(end)
                        .saturating_add("</code>".len());
                } else {
                    break;
                }
            }
            _ => {
                let closing = hidden_closing(marker)?;
                if let Some(end) =
                    tail.get(at..).and_then(|hidden| hidden.find(closing))
                {
                    offset = offset
                        .saturating_add(at)
                        .saturating_add(end)
                        .saturating_add(closing.len());
                } else {
                    break;
                }
            }
        }
    }
    None
}

fn hidden_at(rest: &str) -> Option<(usize, &'static str)> {
    ["<script", "<style", "<button"]
        .into_iter()
        .filter_map(|tag| rest.find(tag).map(|at| (at, tag)))
        .min_by_key(|(at, _)| *at)
}

fn earliest(
    first: Option<(usize, bool)>,
    second: Option<(usize, bool)>,
) -> Option<(usize, bool)> {
    match (first, second) {
        (Some(left), Some(right)) => {
            Some(if left.0 <= right.0 { left } else { right })
        }
        (Some(found), None) | (None, Some(found)) => Some(found),
        (None, None) => None,
    }
}

fn flush_text(blocks: &mut Vec<CardBlock>, text: &str) {
    let mut lines = Vec::new();
    let mut plain = String::new();
    let mut cursor = 0usize;
    while let Some(rest) = text.get(cursor..) {
        let hidden = hidden_at(rest);
        let tag = rest.find("<code").map(|at| (at, false));
        let tick = rest.find('`').map(|at| (at, true));
        let code = earliest(tag, tick);
        if let Some((at, marker)) = hidden
            && code.is_none_or(|(code_at, _)| at <= code_at)
        {
            plain.push_str(rest.get(..at).unwrap_or(""));
            let tail = rest.get(at..).unwrap_or("");
            let closing = hidden_closing(marker).unwrap_or("");
            if let Some(end) = tail.find(closing) {
                cursor = cursor
                    .saturating_add(at)
                    .saturating_add(end)
                    .saturating_add(closing.len());
            } else {
                break;
            }
            continue;
        }
        let Some((at, backtick)) = code else {
            plain.push_str(rest);
            break;
        };
        plain.push_str(rest.get(..at).unwrap_or(""));
        let start = cursor.saturating_add(at);
        if backtick {
            let after = text.get(start.saturating_add(1)..).unwrap_or("");
            if let Some(end) = after.find('`') {
                plain.push_str(
                    rest.get(at..at.saturating_add(end).saturating_add(2))
                        .unwrap_or(""),
                );
                cursor = start.saturating_add(end).saturating_add(2);
            } else {
                plain.push('`');
                cursor = start.saturating_add(1);
            }
        } else {
            let after = text.get(start..).unwrap_or("");
            let Some(tag_end) = after.find('>') else {
                plain.push_str("<code");
                cursor = start.saturating_add(5);
                continue;
            };
            let body_start = start.saturating_add(tag_end).saturating_add(1);
            let body = text.get(body_start..).unwrap_or("");
            if let Some(end) = body.find("</code>") {
                let code_end = body_start
                    .saturating_add(end)
                    .saturating_add("</code>".len());
                plain.push_str(text.get(start..code_end).unwrap_or(""));
                cursor = code_end;
            } else {
                plain.push_str(after.get(..=tag_end).unwrap_or(""));
                cursor = body_start;
            }
        }
    }
    push_spans(&mut lines, &plain, TextSpanKind::Plain);
    if !lines.is_empty() {
        blocks.push(CardBlock::Text(lines));
    }
}

fn hidden_closing(rest: &str) -> Option<&'static str> {
    if rest.starts_with("<script") {
        Some("</script>")
    } else if rest.starts_with("<style") {
        Some("</style>")
    } else if rest.starts_with("<button") {
        Some("</button>")
    } else {
        None
    }
}

fn push_spans(lines: &mut Vec<CardLine>, text: &str, kind: TextSpanKind) {
    match kind {
        TextSpanKind::Plain => {
            for run in styled_text(text) {
                let kind = if run.code {
                    TextSpanKind::Code
                } else {
                    TextSpanKind::Plain
                };
                push_parts(
                    lines,
                    &run.text,
                    kind,
                    run.foreground,
                    run.background,
                );
            }
        }
        TextSpanKind::Code => {
            push_parts(lines, text, kind, None, None);
        }
    }
}

fn push_parts(
    lines: &mut Vec<CardLine>,
    text: &str,
    kind: TextSpanKind,
    foreground: Option<Rgba>,
    background: Option<Rgba>,
) {
    let mut parts = text.split('\n');
    let first = parts.next().unwrap_or("");
    append_span(lines, first, kind, foreground, background);
    for part in parts {
        lines.push(CardLine { spans: Vec::new() });
        append_span(lines, part, kind, foreground, background);
    }
}

fn append_span(
    lines: &mut Vec<CardLine>,
    text: &str,
    kind: TextSpanKind,
    foreground: Option<Rgba>,
    background: Option<Rgba>,
) {
    if text.is_empty() {
        return;
    }
    if lines.is_empty() {
        lines.push(CardLine { spans: Vec::new() });
    }
    if let Some(line) = lines.last_mut() {
        line.spans.push(TextSpan {
            text: text.to_owned(),
            kind,
            foreground,
            background,
        });
    }
}

fn parse_pre(input: &str, start: usize) -> Option<(CardBlock, usize)> {
    let rest = input.get(start..)?;
    let tag_end = rest.find('>')?;
    let open_tag = rest.get(..=tag_end)?;
    let body = rest.get(tag_end.saturating_add(1)..)?;
    let close = body.find("</pre>")?;
    let end = start
        .saturating_add(tag_end)
        .saturating_add(1)
        .saturating_add(close)
        .saturating_add("</pre>".len());
    let mut inner = body.get(..close)?;
    let mut language = language_from_tag(open_tag);
    if let Some(code_at) = inner.find("<code") {
        let code = inner.get(code_at..)?;
        if let Some(code_end) = code.find('>') {
            language =
                language.or_else(|| language_from_tag(code.get(..=code_end)?));
            let source = code.get(code_end.saturating_add(1)..)?;
            inner = source.get(..source.find("</code>")?).unwrap_or(source);
        }
    }
    let raw =
        decode_entities(&inner.replace("<br>", "\n").replace("<br/>", "\n"));
    let source = raw.strip_suffix('\n').unwrap_or(&raw);
    let lines = highlight_code(source, language.as_deref());
    Some((CardBlock::Code { language, lines }, end))
}

fn parse_fence(input: &str, start: usize) -> Option<(CardBlock, usize)> {
    let marker_len = input
        .get(start..)
        .map_or(3, |tail| tail.chars().take_while(|c| *c == '`').count())
        .max(3);
    let rest = input.get(start.saturating_add(marker_len)..)?;
    let info_end = rest.find('\n')?;
    let language = language_from_info(rest.get(..info_end)?);
    let body_start = start
        .saturating_add(marker_len)
        .saturating_add(info_end)
        .saturating_add(1);
    let body = input.get(body_start..)?;
    let mut offset = 0usize;
    let close = body.split_inclusive('\n').find_map(|line| {
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        let count = trimmed.chars().take_while(|c| *c == '`').count();
        let found = (count >= marker_len
            && trimmed
                .get(count..)
                .is_some_and(|tail| tail.trim().is_empty()))
        .then_some(offset);
        offset = offset.saturating_add(line.len());
        found
    })?;
    let raw = body.get(..close)?;
    let source = raw
        .strip_suffix("\r\n")
        .or_else(|| raw.strip_suffix('\n'))
        .unwrap_or(raw);
    let tail = body.get(close..)?;
    let end = body_start.saturating_add(close).saturating_add(
        tail.find('\n')
            .map_or(tail.len(), |at| at.saturating_add(1)),
    );
    let lines = highlight_code(&decode_entities(source), language.as_deref());
    Some((CardBlock::Code { language, lines }, end))
}

fn language_from_info(info: &str) -> Option<String> {
    let token = info.split_whitespace().next()?;
    let token = token
        .trim_start_matches('.')
        .trim_matches(',')
        .to_ascii_lowercase();
    (!token.is_empty()).then_some(token)
}

fn language_from_tag(tag: &str) -> Option<String> {
    let lowered = tag.to_ascii_lowercase();
    if let Some(language) = attribute_value(&lowered, "data-language") {
        return Some(language);
    }
    let classes = attribute_value(&lowered, "class")?;
    classes
        .split_whitespace()
        .map(|token| token.trim_start_matches("language-"))
        .find(|token| !token.is_empty() && !matches!(*token, "code" | "hljs"))
        .map(str::to_owned)
}

fn attribute_value(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=");
    let at = tag.find(&key)?;
    let tail = tag.get(at.saturating_add(key.len())..)?;
    let quote = tail.chars().next();
    if quote == Some('"') || quote == Some('\'') {
        let mark = quote.unwrap_or('"');
        let inner = tail.get(1..)?;
        return inner
            .find(mark)
            .and_then(|end| inner.get(..end))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
    }
    tail.split_whitespace()
        .next()
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
}

#[must_use]
pub fn highlight_code(
    code: &str,
    language: Option<&str>,
) -> Vec<Vec<CodeSpan>> {
    code.split('\n')
        .map(|line| highlight_line(line, language))
        .collect()
}

#[must_use]
pub fn highlight_line(line: &str, language: Option<&str>) -> Vec<CodeSpan> {
    let syntax = Syntax::from_language(language);
    let chars = line.char_indices().collect::<Vec<_>>();
    let mut spans = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        let (_, current) = chars[index];
        if let Some(closing) = syntax.comment_at(&chars, index) {
            let tail = chars
                .get(index..)
                .unwrap_or(&[])
                .iter()
                .map(|(_, c)| *c)
                .collect::<String>();
            let (end, closed) = match closing {
                Comment::Line => (tail.len(), false),
                Comment::Block(mark) => {
                    tail.find(mark).map_or((tail.len(), false), |at| {
                        (at.saturating_add(mark.len()), true)
                    })
                }
            };
            push_span(
                &mut spans,
                TokenKind::Comment,
                tail.get(..end).unwrap_or(""),
            );
            index = if closed {
                index.saturating_add(
                    tail.get(..end).map_or(0, |part| part.chars().count()),
                )
            } else {
                chars.len()
            };
            continue;
        }
        if current == '"'
            || current == '\''
            || (current == '`' && syntax.has_backticks())
        {
            let end = string_end(&chars, index, current, syntax);
            let text = slice(&chars, line, index, end);
            push_span(&mut spans, TokenKind::String, text);
            index = end;
            continue;
        }
        if current.is_ascii_digit() {
            let mut end = index.saturating_add(1);
            while let Some((_, character)) = chars.get(end) {
                if character.is_ascii_alphanumeric()
                    || matches!(character, '_' | '.')
                {
                    end = end.saturating_add(1);
                } else {
                    break;
                }
            }
            push_span(
                &mut spans,
                TokenKind::Number,
                slice(&chars, line, index, end),
            );
            index = end;
            continue;
        }
        if current.is_alphabetic() || current == '_' {
            let mut end = index.saturating_add(1);
            while let Some((_, character)) = chars.get(end) {
                if character.is_alphanumeric() || *character == '_' {
                    end = end.saturating_add(1);
                } else {
                    break;
                }
            }
            let word = slice(&chars, line, index, end);
            let kind = if syntax.is_keyword(word) {
                TokenKind::Keyword
            } else if chars.get(end).is_some_and(|(_, c)| *c == '(') {
                TokenKind::Function
            } else if syntax.is_type(word) {
                TokenKind::Type
            } else {
                TokenKind::Plain
            };
            push_span(&mut spans, kind, word);
            index = end;
            continue;
        }
        if is_operator(current) {
            let mut end = index.saturating_add(1);
            while let Some((_, character)) = chars.get(end) {
                if is_operator(*character) {
                    end = end.saturating_add(1);
                } else {
                    break;
                }
            }
            push_span(
                &mut spans,
                TokenKind::Operator,
                slice(&chars, line, index, end),
            );
            index = end;
            continue;
        }
        let kind = if current.is_whitespace() {
            TokenKind::Plain
        } else {
            TokenKind::Punctuation
        };
        push_span(
            &mut spans,
            kind,
            slice(&chars, line, index, index.saturating_add(1)),
        );
        index = index.saturating_add(1);
    }
    spans
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Comment {
    Line,
    Block(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Syntax {
    Rust,
    Python,
    Javascript,
    Shell,
    Json,
    Sql,
    CStyle,
    Css,
    Html,
    Generic,
}

impl Syntax {
    fn from_language(language: Option<&str>) -> Self {
        match language.unwrap_or("").to_ascii_lowercase().as_str() {
            "rs" | "rust" => Self::Rust,
            "py" | "python" | "python3" => Self::Python,
            "js" | "jsx" | "javascript" | "ts" | "tsx" | "typescript" => {
                Self::Javascript
            }
            "sh" | "bash" | "zsh" | "shell" | "console" => Self::Shell,
            "json" | "jsonc" => Self::Json,
            "sql" => Self::Sql,
            "c" | "cc" | "cpp" | "c++" | "h" | "hpp" | "java" | "go" | "cs"
            | "csharp" => Self::CStyle,
            "css" | "scss" => Self::Css,
            "html" | "xml" => Self::Html,
            _ => Self::Generic,
        }
    }

    const fn comment_prefixes(self) -> &'static [&'static str] {
        match self {
            Self::Python | Self::Shell => &["#"],
            Self::Sql => &["--", "/*"],
            Self::Json => &[],
            Self::Html => &["<!--"],
            Self::Css
            | Self::CStyle
            | Self::Rust
            | Self::Javascript
            | Self::Generic => &["//", "/*"],
        }
    }

    fn comment_at(
        self,
        chars: &[(usize, char)],
        index: usize,
    ) -> Option<Comment> {
        self.comment_prefixes().iter().find_map(|prefix| {
            let found = prefix.chars().enumerate().all(|(offset, expected)| {
                chars
                    .get(index.saturating_add(offset))
                    .is_some_and(|(_, c)| *c == expected)
            });
            let comment = match *prefix {
                "/*" => Comment::Block("*/"),
                "<!--" => Comment::Block("-->"),
                _ => Comment::Line,
            };
            found.then_some(comment)
        })
    }

    const fn has_backticks(self) -> bool {
        matches!(self, Self::Javascript | Self::Shell | Self::Generic)
    }

    fn is_keyword(self, word: &str) -> bool {
        let keywords = match self {
            Self::Rust => RUST_KEYWORDS,
            Self::Python => PYTHON_KEYWORDS,
            Self::Javascript => JAVASCRIPT_KEYWORDS,
            Self::Shell => SHELL_KEYWORDS,
            Self::Json => JSON_KEYWORDS,
            Self::Sql => SQL_KEYWORDS,
            Self::CStyle => CSTYLE_KEYWORDS,
            Self::Css | Self::Html | Self::Generic => COMMON_KEYWORDS,
        };
        keywords
            .iter()
            .any(|keyword| keyword.eq_ignore_ascii_case(word))
    }

    fn is_type(self, word: &str) -> bool {
        let types = match self {
            Self::Rust => RUST_TYPES,
            Self::Python => PYTHON_TYPES,
            Self::Javascript => JAVASCRIPT_TYPES,
            Self::CStyle => CSTYLE_TYPES,
            Self::Sql => SQL_TYPES,
            Self::Json
            | Self::Shell
            | Self::Css
            | Self::Html
            | Self::Generic => COMMON_TYPES,
        };
        types
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(word))
            || word.chars().next().is_some_and(char::is_uppercase)
    }
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn",
    "else", "enum", "false", "fn", "for", "if", "impl", "in", "let", "loop",
    "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
    "where", "while",
];
const PYTHON_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "False", "finally", "for", "from",
    "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not",
    "or", "pass", "raise", "return", "True", "try", "while", "with", "yield",
];
const JAVASCRIPT_KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "static",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "while",
];
const SHELL_KEYWORDS: &[&str] = &[
    "case", "cd", "do", "done", "echo", "elif", "else", "esac", "export", "fi",
    "for", "function", "if", "in", "local", "return", "set", "source", "then",
    "while",
];
const JSON_KEYWORDS: &[&str] = &["false", "null", "true"];
const SQL_KEYWORDS: &[&str] = &[
    "and", "as", "by", "create", "delete", "drop", "false", "from", "group",
    "insert", "into", "join", "limit", "not", "null", "on", "or", "order",
    "select", "set", "table", "true", "update", "values", "where",
];
const CSTYLE_KEYWORDS: &[&str] = &[
    "bool",
    "break",
    "case",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "defer",
    "delete",
    "double",
    "else",
    "false",
    "final",
    "float",
    "for",
    "func",
    "go",
    "if",
    "include",
    "int",
    "make",
    "map",
    "namespace",
    "new",
    "nil",
    "private",
    "public",
    "range",
    "return",
    "static",
    "struct",
    "switch",
    "template",
    "true",
    "void",
    "while",
];
const COMMON_KEYWORDS: &[&str] = &[
    "and", "break", "class", "const", "continue", "else", "false", "fn", "for",
    "from", "function", "if", "import", "in", "let", "new", "not", "null",
    "or", "return", "true", "while",
];
const RUST_TYPES: &[&str] = &[
    "bool", "char", "f32", "f64", "i16", "i32", "i64", "isize", "Option",
    "Result", "str", "String", "u16", "u32", "u64", "usize", "Vec",
];
const PYTHON_TYPES: &[&str] = &[
    "bool", "bytes", "dict", "float", "int", "list", "object", "set", "str",
    "tuple",
];
const JAVASCRIPT_TYPES: &[&str] = &[
    "Array", "Date", "Map", "number", "object", "Promise", "RegExp", "Set",
    "string", "boolean",
];
const CSTYLE_TYPES: &[&str] = &[
    "bool", "byte", "char", "double", "float", "int", "long", "short",
    "size_t", "string", "uchar", "uint", "void",
];
const SQL_TYPES: &[&str] = &["INTEGER", "REAL", "TEXT", "BLOB"];
const COMMON_TYPES: &[&str] = &["bool", "float", "int", "str", "string"];

fn slice<'a>(
    chars: &[(usize, char)],
    line: &'a str,
    start: usize,
    end: usize,
) -> &'a str {
    let from = chars.get(start).map_or(0, |(at, _)| *at);
    let to = chars.get(end).map_or(line.len(), |(at, _)| *at);
    line.get(from..to).unwrap_or("")
}

fn string_end(
    chars: &[(usize, char)],
    start: usize,
    quote: char,
    syntax: Syntax,
) -> usize {
    let mut end = start.saturating_add(1);
    let mut escaped = false;
    let mut triple = false;
    if syntax == Syntax::Python
        && chars.get(end).is_some_and(|(_, c)| *c == quote)
        && chars
            .get(end.saturating_add(1))
            .is_some_and(|(_, c)| *c == quote)
    {
        triple = true;
        end = end.saturating_add(2);
    }
    let limit = if triple { 3 } else { 1 };
    while let Some((_, character)) = chars.get(end) {
        if escaped {
            escaped = false;
        } else if *character == '\\' {
            escaped = true;
        } else if *character == quote {
            let mut close = end.saturating_add(1);
            let mut count: usize = 1;
            while count < limit
                && chars.get(close).is_some_and(|(_, c)| *c == quote)
            {
                close = close.saturating_add(1);
                count = count.saturating_add(1);
            }
            if count == limit {
                return close;
            }
        }
        end = end.saturating_add(1);
    }
    end
}

fn push_span(spans: &mut Vec<CodeSpan>, kind: TokenKind, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.kind == kind
    {
        last.text.push_str(text);
        return;
    }
    spans.push(CodeSpan {
        text: text.to_owned(),
        kind,
    });
}

fn is_operator(character: char) -> bool {
    matches!(
        character,
        '+' | '-'
            | '*'
            | '/'
            | '%'
            | '='
            | '<'
            | '>'
            | '!'
            | '&'
            | '|'
            | '^'
            | '~'
            | '?'
    )
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
    pub source: String,
    pub kind: MathKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MathKind {
    #[default]
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
            source: expression.to_owned(),
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
            source: String::new(),
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

/// Converts supported markup to colored text runs.
#[must_use]
pub(super) fn styled_text(input: &str) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut style = Style::default();
    let mut stack = Vec::new();
    let mut cursor = 0usize;
    while let Some(rest) = input.get(cursor..) {
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
        if rest.starts_with("<code")
            && let Some(tag_end) = rest.find('>')
        {
            let body_start = tag_end.saturating_add(1);
            if let Some(end) =
                rest.get(body_start..).and_then(|tail| tail.find("</code>"))
            {
                let body = rest
                    .get(body_start..body_start.saturating_add(end))
                    .unwrap_or("");
                push_text(&mut runs, body, style, true);
                cursor = cursor
                    .saturating_add(body_start)
                    .saturating_add(end)
                    .saturating_add("</code>".len());
                continue;
            }
        }
        if rest.starts_with('`')
            && let Some(end) = rest.get(1..).and_then(|tail| tail.find('`'))
        {
            push_text(
                &mut runs,
                rest.get(1..end.saturating_add(1)).unwrap_or(""),
                style,
                true,
            );
            cursor = cursor.saturating_add(end).saturating_add(2);
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
            let tag_end = end.saturating_add(1);
            let tag = rest.get(..tag_end).unwrap_or("");
            if tag.starts_with("<br")
                || tag.starts_with("<hr")
                || tag.starts_with("</div")
                || tag.starts_with("</p")
            {
                push_text(&mut runs, "\n", style, false);
            }
            let lower = tag.to_ascii_lowercase();
            if lower.starts_with("</span") || lower.starts_with("</font") {
                if let Some(previous) = stack.pop() {
                    style = previous;
                }
            } else if lower.starts_with("<span") || lower.starts_with("<font") {
                stack.push(style);
                style = style_from_tag(tag, &lower, style);
            }
            cursor = cursor.saturating_add(tag_end);
            continue;
        }
        let Some(character) = rest.chars().next() else {
            break;
        };
        let mut encoded = [0u8; 4];
        push_text(&mut runs, character.encode_utf8(&mut encoded), style, false);
        cursor = cursor.saturating_add(character.len_utf8());
    }
    for run in &mut runs {
        run.text = decode_entities(&run.text);
    }
    runs
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Style {
    foreground: Option<Rgba>,
    background: Option<Rgba>,
}

fn push_text(runs: &mut Vec<TextRun>, text: &str, style: Style, code: bool) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = runs.last_mut()
        && last.foreground == style.foreground
        && last.background == style.background
        && last.code == code
    {
        last.text.push_str(text);
    } else {
        runs.push(TextRun {
            text: text.to_owned(),
            foreground: style.foreground,
            background: style.background,
            math: false,
            code,
            ..Default::default()
        });
    }
}

/// Reads the two CSS properties the card color picker writes.
fn style_from_tag(tag: &str, lower: &str, parent: Style) -> Style {
    let mut style = parent;
    if let Some(declarations) = attribute(tag, lower, "style") {
        for declaration in declarations.split(';') {
            let Some((property, value)) = declaration.split_once(':') else {
                continue;
            };
            match property.trim() {
                "color" => {
                    style.foreground = css_color(value, parent.foreground);
                }
                "background-color" | "background" => {
                    style.background = css_color(value, parent.background);
                }
                _ => {}
            }
        }
    }
    if let Some(value) = attribute(tag, lower, "color") {
        style.foreground = css_color(value, parent.foreground);
    }
    style
}

fn attribute<'a>(tag: &'a str, lower: &str, name: &str) -> Option<&'a str> {
    let marker = format!("{name}=");
    let at = lower.find(&marker)?;
    let tail = tag.get(at.saturating_add(marker.len())..)?;
    let quote = tail.chars().next()?;
    if quote == '"' || quote == '\'' {
        let rest = tail.get(1..)?;
        let end = rest.find(quote)?;
        rest.get(..end)
    } else {
        tail.split(|c: char| c.is_whitespace() || c == '>').next()
    }
}

fn css_color(value: &str, inherited: Option<Rgba>) -> Option<Rgba> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "inherit" | "initial" | "unset" => inherited,
        "transparent" => None,
        _ => parse_hex_color(&value),
    }
}

/// Converts the supported markup subset to plain text with line breaks.
#[must_use]
pub fn strip_tags(input: &str) -> String {
    let text = styled_text(input)
        .iter()
        .map(|run| run.text.as_str())
        .collect::<String>();
    normalize_newlines(&text)
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

/// A cloze match together with its source range in the rendered markup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClozeSpan {
    pub start: usize,
    pub end: usize,
    /// Source range whose text matches `segment.text`, when it does.
    pub label_start: usize,
    pub label_end: usize,
    pub segment: ClozeSegment,
}

/// Renders a cloze field for a one-based card number and side, keeping ranges.
#[must_use]
pub(super) fn cloze_spans(
    text: &str,
    ordinal: u16,
    side: Side,
) -> Vec<ClozeSpan> {
    let number = ordinal.saturating_add(1);
    let mut spans = Vec::new();
    let mut cursor = 0usize;
    while let Some(rest) = text.get(cursor..) {
        let Some(start) = rest.find("{{c") else {
            push_plain(&mut spans, text, cursor, text.len());
            break;
        };
        let start = cursor.saturating_add(start);
        push_plain(&mut spans, text, cursor, start);
        let Some(rest) = text.get(start.saturating_add(2)..) else {
            break;
        };
        let Some(separator) = rest.find("::") else {
            push_plain(&mut spans, text, start, text.len());
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
        let end = body_start.saturating_add(body).saturating_add(2);
        spans.push(ClozeSpan {
            start,
            end,
            label_start: match kind {
                ClozeKind::ActiveQuestion => start,
                ClozeKind::Plain
                | ClozeKind::ActiveAnswer
                | ClozeKind::Inactive => body_start,
            },
            label_end: match kind {
                ClozeKind::ActiveQuestion => start,
                ClozeKind::Plain
                | ClozeKind::ActiveAnswer
                | ClozeKind::Inactive => {
                    body_start.saturating_add(answer.len())
                }
            },
            segment: ClozeSegment { text: label, kind },
        });
        cursor = end;
    }
    spans
}

/// Renders a cloze field for a one-based card number and side.
#[must_use]
pub fn cloze_segments(
    text: &str,
    ordinal: u16,
    side: Side,
) -> Vec<ClozeSegment> {
    cloze_spans(text, ordinal, side)
        .into_iter()
        .map(|span| span.segment)
        .collect()
}

fn push_plain(
    spans: &mut Vec<ClozeSpan>,
    text: &str,
    start: usize,
    end: usize,
) {
    let slice = text.get(start..end).unwrap_or("");
    if !slice.is_empty() {
        spans.push(ClozeSpan {
            start,
            end,
            label_start: start,
            label_end: end,
            segment: ClozeSegment {
                text: slice.to_owned(),
                kind: ClozeKind::Plain,
            },
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

    #[test]
    fn math_source_is_kept_for_typesetting() {
        let segments = math_segments("\\(\\mathbb{x} \\subseteq\\)");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].source, "\\mathbb{x} \\subseteq");
        assert_eq!(segments[0].kind, MathKind::Inline);
        assert_eq!(math_segments("plain text")[0].source, "");
    }
}
