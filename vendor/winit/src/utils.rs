// A poly-fill for `lazy_cell`
// Replace with std::sync::LazyLock when https://github.com/rust-lang/rust/issues/109736 is stabilized.

// This isn't used on every platform, which can come up as dead code warnings.
#![allow(dead_code)]

use std::ops::Deref;
use std::sync::OnceLock;

pub(crate) struct Lazy<T> {
    cell: OnceLock<T>,
    init: fn() -> T,
}

impl<T> Lazy<T> {
    pub const fn new(f: fn() -> T) -> Self {
        Self { cell: OnceLock::new(), init: f }
    }
}

impl<T> Deref for Lazy<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &'_ T {
        self.cell.get_or_init(self.init)
    }
}

// Android reports IME spans in UTF-16 code units, while winit uses UTF-8 byte
// offsets. Out-of-range offsets are clamped, and an offset inside a surrogate
// pair maps to the end of that character.
pub(crate) fn utf16_offset_to_byte_offset(text: &str, utf16_offset: usize) -> usize {
    let mut utf16_seen = 0;
    for (byte_offset, character) in text.char_indices() {
        if utf16_seen >= utf16_offset {
            return byte_offset;
        }
        utf16_seen += character.len_utf16();
        if utf16_seen > utf16_offset {
            return byte_offset + character.len_utf8();
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::utf16_offset_to_byte_offset;

    #[test]
    fn utf16_offsets_map_to_utf8_byte_offsets() {
        let text = "aé🦀b";
        assert_eq!(utf16_offset_to_byte_offset(text, 0), 0);
        assert_eq!(utf16_offset_to_byte_offset(text, 1), 1);
        assert_eq!(utf16_offset_to_byte_offset(text, 2), 3);
        assert_eq!(utf16_offset_to_byte_offset(text, 4), 7);
        assert_eq!(utf16_offset_to_byte_offset(text, 5), 8);
    }

    #[test]
    fn utf16_offsets_clamp_outside_character_boundaries() {
        let text = "a🦀b";
        assert_eq!(utf16_offset_to_byte_offset(text, 2), 5);
        assert_eq!(utf16_offset_to_byte_offset(text, 10), 6);
        assert_eq!(utf16_offset_to_byte_offset("", 10), 0);
    }
}
