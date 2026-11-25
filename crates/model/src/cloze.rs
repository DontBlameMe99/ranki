//! Cloze marker detection shared by card generation and image occlusion.

use std::collections::BTreeSet;

/// Maximum card ordinal Anki assigns to a cloze deletion.
pub const MAX_CLOZE_ORDINAL: u16 = 499;

/// A cloze marker and the numbers listed before its `::` separator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClozeMarker<'a> {
    /// One-based cloze numbers, kept in the order written by the author.
    pub numbers: Vec<u32>,
    /// Text between `::` and the closing braces.
    pub body: &'a str,
    /// Byte offset of the opening `{{` in the field text.
    pub start: usize,
    /// Byte offset just past the closing `}}`.
    pub end: usize,
}

/// Finds `{{cN::...}}` markers in field HTML.
///
/// Nested markers are returned separately. `MathJax` delimiters need no
/// special handling because the scan starts at `{{c` and stops at `::`.
#[must_use]
pub fn cloze_markers(text: &str) -> Vec<ClozeMarker<'_>> {
    let bytes = text.as_bytes();
    let mut markers = Vec::new();
    let mut position = 0usize;
    while position.saturating_add(2) < bytes.len() {
        if !is_marker_start(bytes, position) {
            position = position.saturating_add(1);
            continue;
        }
        let Some((numbers, after_numbers)) =
            parse_numbers(bytes, position.saturating_add(3))
        else {
            position = position.saturating_add(1);
            continue;
        };
        let Some(body_start) = separator_at(bytes, after_numbers) else {
            position = position.saturating_add(1);
            continue;
        };
        let body_offset = body_start.saturating_add(2);
        let Some(body_end) = closing_braces(bytes, body_offset) else {
            position = position.saturating_add(1);
            continue;
        };
        let body = text.get(body_offset..body_end).unwrap_or("");
        markers.push(ClozeMarker {
            numbers,
            body,
            start: position,
            end: body_end.saturating_add(2),
        });
        position = body_offset;
    }
    markers
}

/// Distinct card ordinals for all positive cloze numbers in the fields.
///
/// Anki ignores `c0` and caps ordinals at 499.
#[must_use]
pub fn cloze_ordinals(fields: &[String]) -> BTreeSet<u16> {
    let mut ordinals = BTreeSet::new();
    for field in fields {
        for marker in cloze_markers(field) {
            add_ordinals(&mut ordinals, &marker.numbers);
        }
    }
    ordinals
}

fn add_ordinals(ordinals: &mut BTreeSet<u16>, numbers: &[u32]) {
    for number in numbers {
        if let Some(ordinal) = cloze_ordinal(*number) {
            let _ = ordinals.insert(ordinal);
        }
    }
}

/// Card ordinal for a one-based cloze number. `c0` yields no card.
#[must_use]
pub fn cloze_ordinal(number: u32) -> Option<u16> {
    if number == 0 {
        return None;
    }
    let capped = number.saturating_sub(1).min(u32::from(MAX_CLOZE_ORDINAL));
    u16::try_from(capped).ok()
}

fn is_marker_start(bytes: &[u8], position: usize) -> bool {
    bytes.get(position) == Some(&b'{')
        && bytes.get(position.saturating_add(1)) == Some(&b'{')
        && bytes.get(position.saturating_add(2)) == Some(&b'c')
}

fn parse_numbers(
    bytes: &[u8],
    mut position: usize,
) -> Option<(Vec<u32>, usize)> {
    let mut numbers = Vec::new();
    loop {
        position = skip_spaces(bytes, position);
        let digits_start = position;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position = position.saturating_add(1);
        }
        if digits_start == position {
            return None;
        }
        numbers.push(parse_number(bytes.get(digits_start..position)?)?);
        position = skip_spaces(bytes, position);
        if bytes.get(position) == Some(&b',') {
            position = position.saturating_add(1);
        } else {
            break;
        }
    }
    Some((numbers, position))
}

fn parse_number(digits: &[u8]) -> Option<u32> {
    let mut value = 0u32;
    for digit in digits {
        let value_digit = digit.checked_sub(b'0')?;
        value = value.checked_mul(10)?.checked_add(u32::from(value_digit))?;
    }
    Some(value)
}

fn skip_spaces(bytes: &[u8], mut position: usize) -> usize {
    while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
        position = position.saturating_add(1);
    }
    position
}

fn separator_at(bytes: &[u8], position: usize) -> Option<usize> {
    if bytes.get(position) == Some(&b':')
        && bytes.get(position.saturating_add(1)) == Some(&b':')
    {
        Some(position)
    } else {
        None
    }
}

#[expect(
    clippy::excessive_nesting,
    reason = "brace scanning is clearer in one pass"
)]
fn closing_braces(bytes: &[u8], position: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = position;
    while index.saturating_add(1) < bytes.len() {
        if let Some(end) = mathjax_end(bytes, index) {
            index = end;
            continue;
        }
        let is_open = bytes.get(index) == Some(&b'{')
            && bytes.get(index.saturating_add(1)) == Some(&b'{');
        let is_close = bytes.get(index) == Some(&b'}')
            && bytes.get(index.saturating_add(1)) == Some(&b'}');
        if is_open {
            depth = depth.saturating_add(1);
            index = index.saturating_add(2);
        } else if is_close {
            if depth == 0 {
                return Some(index);
            }
            depth = depth.saturating_sub(1);
            index = index.saturating_add(2);
        } else {
            index = index.saturating_add(1);
        }
    }
    None
}

#[expect(
    clippy::excessive_nesting,
    reason = "delimiter scanning is clearer in one pass"
)]
fn mathjax_end(bytes: &[u8], index: usize) -> Option<usize> {
    for (opener, closer) in [
        (&b"\\["[..], &b"\\]"[..]),
        (&b"\\("[..], &b"\\)"[..]),
        (&b"$$"[..], &b"$$"[..]),
    ] {
        if bytes.get(index..index.saturating_add(opener.len())) == Some(opener)
        {
            let mut cursor = index.saturating_add(opener.len());
            while cursor.saturating_add(closer.len()) <= bytes.len() {
                if bytes.get(cursor..cursor.saturating_add(closer.len()))
                    == Some(closer)
                {
                    return Some(cursor.saturating_add(closer.len()));
                }
                cursor = cursor.saturating_add(1);
            }
            return Some(bytes.len());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::cloze_markers;
    use super::cloze_ordinals;

    fn fields(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn extracts_sorted_distinct_cloze_ordinals() {
        let values = fields(&[
            "{{c3::three}} {{c1::one::hint}} {{c0::never}}",
            "{{c1,2::shared}} {{c500::capped}}",
        ]);
        assert_eq!(
            cloze_ordinals(&values).into_iter().collect::<Vec<_>>(),
            vec![0, 1, 2, 499]
        );
    }

    #[test]
    fn nested_cloze_returns_each_marker() {
        let values = fields(&["{{c1::outer {{c2::inner}} end}}"]);
        let markers = cloze_markers(&values[0]);
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].numbers, [1]);
        assert_eq!(markers[1].numbers, [2]);
        assert_eq!(
            &values[0][markers[0].start..markers[0].end],
            "{{c1::outer {{c2::inner}} end}}"
        );
        assert_eq!(
            &values[0][markers[1].start..markers[1].end],
            "{{c2::inner}}"
        );
    }

    #[test]
    fn mathjax_closing_braces_do_not_end_a_marker() {
        let values = fields(&["{{c1::value with \\(a }} b\\) math}}"]);
        let markers = cloze_markers(&values[0]);
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].body, "value with \\(a }} b\\) math");
    }
}
