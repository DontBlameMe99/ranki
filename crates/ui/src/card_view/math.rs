//! Typeset `MathJax` expressions with a native TeX renderer.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    sync::{Arc, OnceLock},
};

use gpui::{
    AnyElement, IntoElement, ParentElement, RenderImage, Rgba, Styled, div,
    img, px,
};
use image::Frame;
use latex_rust::{
    Color, Dim, MathFont, MathStyle, PngBackground, PngOptions,
    layout_with_em_size_pt, parse, render_png,
};

use super::MathKind;

const CACHE_LIMIT: usize = 96;
const RASTER_DPI: i64 = 288;
const PT_PER_PX: f64 = 72.0 / 96.0;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MathKey {
    source: String,
    color: u32,
    size_px: u32,
    display: bool,
}

#[derive(Clone)]
struct CachedMath {
    image: Arc<RenderImage>,
    width: f32,
    height: f32,
    baseline: f32,
}

#[derive(Default)]
struct MathCache {
    items: HashMap<MathKey, CachedMath>,
    order: VecDeque<MathKey>,
}

thread_local! {
    static CACHE: RefCell<MathCache> = RefCell::new(MathCache::default());
}

fn font() -> Option<&'static MathFont> {
    static FONT: OnceLock<Option<MathFont>> = OnceLock::new();
    FONT.get_or_init(|| MathFont::stix_two_math().ok()).as_ref()
}

fn dim_value(dim: &Dim) -> f32 {
    f32::from_bits(dim.to_ieee32_bits())
}

fn rounded_size(size_px: f32) -> u32 {
    if !size_px.is_finite() || size_px < 1.0 {
        return 0;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "size is finite and positive"
    )]
    let rounded = size_px.round() as u32;
    rounded
}

fn color_components(color: Rgba) -> (u8, u8, u8) {
    let bits = u32::from(color);
    let component = |shift: u32| {
        u8::try_from((bits >> shift) & 0xff_u32).unwrap_or(u8::MAX)
    };
    (component(24), component(16), component(8))
}

fn render(
    source: &str,
    kind: MathKind,
    color: Rgba,
    size_px: f32,
) -> Option<CachedMath> {
    let size = rounded_size(size_px);
    if size == 0 || source.trim().is_empty() {
        return None;
    }
    let key = MathKey {
        source: source.to_owned(),
        color: u32::from(color),
        size_px: size,
        display: kind == MathKind::Block,
    };
    if let Some(cached) = CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let hit = cache.items.get(&key).cloned();
        if hit.is_some() {
            cache.order.retain(|item| item != &key);
            cache.order.push_back(key.clone());
        }
        hit
    }) {
        return Some(cached);
    }

    let font = font()?;
    let ast = parse(source).ok()?;
    let em_size = Dim::parse(&format!("{:.3}", f64::from(size) * PT_PER_PX));
    let style = if kind == MathKind::Block {
        MathStyle::Display
    } else {
        MathStyle::Text
    };
    let boxed = layout_with_em_size_pt(&ast, font, style, &em_size).ok()?;
    let (red, green, blue) = color_components(color);
    let options = PngOptions {
        font_size_pt: em_size,
        dpi: Dim::from_i64(RASTER_DPI),
        color: Color::rgb(red, green, blue),
        background: PngBackground::Transparent,
        display: kind == MathKind::Block,
    };
    let png = render_png(&boxed, font, &options).ok()?;
    let buffer =
        image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .ok()?
            .into_rgba8();
    let mut buffer = buffer;
    for pixel in buffer.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let image = Arc::new(RenderImage::new(vec![Frame::new(buffer)]));
    let size_f = f32::from(u16::try_from(size).unwrap_or(u16::MAX));
    let cached = CachedMath {
        image,
        width: dim_value(&boxed.width) * size_f,
        height: (dim_value(&boxed.height) + dim_value(&boxed.depth)) * size_f,
        baseline: dim_value(&boxed.height) * size_f,
    };

    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.items.insert(key.clone(), cached.clone()).is_none() {
            cache.order.push_back(key);
        }
        while cache.order.len() > CACHE_LIMIT
            && let Some(oldest) = cache.order.pop_front()
        {
            let _ = cache.items.remove(&oldest);
        }
    });
    Some(cached)
}

/// Renders a math run as an inline image, falling back to its source on error.
#[must_use]
pub fn math_element(
    source: &str,
    kind: MathKind,
    color: Rgba,
    size_px: f32,
) -> AnyElement {
    let Some(math) = render(source, kind, color, size_px) else {
        return div().child(source.to_owned()).into_any_element();
    };
    let image = img(math.image).w(px(math.width)).h(px(math.height));
    div()
        .relative()
        .top(px(size_px * 0.3 + math.height * 0.5 - math.baseline))
        .child(image)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{MathKind, render};
    use latex_rust::Dim;

    fn color() -> gpui::Rgba {
        gpui::Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }
    }

    #[test]
    fn renders_anki_math_commands() {
        for source in [
            r"\mathbb{x}",
            r"\subseteq",
            r"\frac{a+b}{c}",
            r"\sqrt{x_1}",
            r"\sum_{i=1}^n i",
            r"\begin{pmatrix}a&b\\c&d\end{pmatrix}",
            r"\text{bold}",
        ] {
            let rendered = render(source, MathKind::Inline, color(), 32.0);
            assert!(rendered.is_some(), "{source} must render");
            let rendered = rendered.expect("rendered math");
            assert!(rendered.width > 0.0, "{source} must have width");
            assert!(rendered.height > 0.0, "{source} must have height");
        }
    }

    #[test]
    fn unsupported_math_falls_back() {
        let rendered =
            render(r"\unsupportedcommand{x}", MathKind::Inline, color(), 32.0);
        assert!(rendered.is_none());
    }

    #[test]
    fn dimension_values_are_readable() {
        assert_eq!(super::dim_value(&Dim::one()), 1.0);
    }
}
