//! Authoring helpers for built-in image-occlusion notes.
//!
//! Masks live in image-relative coordinates, so the same numbers are written to
//! Anki's `Occlusion` field and drawn by the add-card editor.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

/// Smallest normalized side a drawn mask may have.
pub const MIN_MASK_SIZE: f32 = 0.01;

/// Pattern offset so repeated masks do not stack on the same spot.
const DEFAULT_STEP: f32 = 0.04;

/// Shape kinds the editor can draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskShape {
    Rect,
    Ellipse,
}

impl MaskShape {
    /// Shape name used in Anki's `image-occlusion:` marker.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rect => "rect",
            Self::Ellipse => "ellipse",
        }
    }

    /// Label for the shape list.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rect => "Rectangle",
            Self::Ellipse => "Ellipse",
        }
    }
}

/// Mask geometry, relative to the image: all values are `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormalizedRect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl NormalizedRect {
    /// Clamps the corners into the image and rejects accidental clicks.
    #[must_use]
    pub fn from_points(start: (f32, f32), end: (f32, f32)) -> Option<Self> {
        let left = start.0.min(end.0).clamp(0.0, 1.0);
        let right = start.0.max(end.0).clamp(0.0, 1.0);
        let top = start.1.min(end.1).clamp(0.0, 1.0);
        let bottom = start.1.max(end.1).clamp(0.0, 1.0);
        let rect = Self {
            left,
            top,
            width: right - left,
            height: bottom - top,
        };
        (rect.width >= MIN_MASK_SIZE && rect.height >= MIN_MASK_SIZE)
            .then_some(rect)
    }
}

/// One drawn occlusion shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OcclusionMask {
    pub shape: MaskShape,
    pub rect: NormalizedRect,
}

impl OcclusionMask {
    /// Anki's marker for this shape on card `ordinal` (one-based).
    #[must_use]
    pub fn marker(&self, ordinal: u32) -> String {
        format!(
            "{{{{c{ordinal}::image-occlusion:{}:left={}:top={}:width={}:height={}}}}}",
            self.shape.as_str(),
            ratio(self.rect.left),
            ratio(self.rect.top),
            ratio(self.rect.width),
            ratio(self.rect.height),
        )
    }

    /// Mask center, for placing the cloze label.
    #[must_use]
    pub fn center(&self) -> (f32, f32) {
        (
            self.rect.left + self.rect.width / 2.0,
            self.rect.top + self.rect.height / 2.0,
        )
    }
}

/// A staggered mask for the "Add rectangle" button.
#[must_use]
pub fn default_mask(count: usize) -> OcclusionMask {
    let index = count % 8;
    let step = f32::from(u8::try_from(index).unwrap_or(0)) * DEFAULT_STEP;
    let rect = NormalizedRect::from_points(
        (0.2 + step, 0.2 + step),
        (0.5 + step, 0.4 + step),
    )
    .unwrap_or(NormalizedRect {
        left: 0.2,
        top: 0.2,
        width: 0.3,
        height: 0.2,
    });
    OcclusionMask {
        shape: MaskShape::Rect,
        rect,
    }
}

/// Anki's `Occlusion` field: one marker per mask, joined by `<br>`.
#[must_use]
pub fn occlusion_field(masks: &[OcclusionMask]) -> String {
    let mut markers = Vec::with_capacity(masks.len());
    for (index, mask) in masks.iter().enumerate() {
        let ordinal =
            u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1);
        markers.push(mask.marker(ordinal));
    }
    markers.join("<br>")
}

/// Anki's `Image` field value.
#[must_use]
pub fn image_field(name: &str) -> String {
    let escaped = name.replace('&', "&amp;").replace('"', "&quot;");
    format!("<img src=\"{escaped}\">")
}

/// Field values in the stock Image Occlusion model order.
#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "the five stock fields belong together"
)]
pub fn image_occlusion_fields(
    header: &str,
    back_extra: &str,
    comments: &str,
    image: &str,
    masks: &[OcclusionMask],
) -> Vec<String> {
    vec![
        occlusion_field(masks),
        image_field(image),
        header.to_owned(),
        back_extra.to_owned(),
        comments.to_owned(),
    ]
}

/// Extensions accepted for uploaded occlusion images.
pub const IMAGE_EXTENSIONS: [&str; 8] =
    ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "avif"];

/// True for file names the editor treats as images.
#[must_use]
pub fn is_image_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, extension)| {
        IMAGE_EXTENSIONS
            .iter()
            .any(|known| known.eq_ignore_ascii_case(extension))
    })
}

/// Image files in `dir`, sorted by name. A missing directory lists nothing.
#[must_use]
pub fn media_images(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && is_image_name(&entry.file_name().to_string_lossy())
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Copies `source` into the collection media directory and returns its file
/// name. An existing name gets a numeric suffix so uploads never overwrite media.
///
/// # Errors
///
/// Returns an error for a missing, unsupported, or unsafe file name, or when
/// the copy into `media_dir` fails.
pub fn import_image(source: &Path, media_dir: &Path) -> io::Result<String> {
    let invalid =
        |message: &str| io::Error::new(ErrorKind::InvalidInput, message);
    let name = source
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| invalid("invalid image file name"))?;
    if !is_image_name(name) {
        return Err(invalid("choose an image file"));
    }
    if name.contains(['\\', '"', '<', '>']) {
        return Err(invalid("unsafe image file name"));
    }
    let (stem, extension) = name
        .rsplit_once('.')
        .ok_or_else(|| invalid("invalid image file name"))?;

    fs::create_dir_all(media_dir)?;
    let mut stored = name.to_owned();
    let mut suffix = 2;
    while media_dir.join(&stored).exists() {
        stored = format!("{stem}-{suffix}.{extension}");
        suffix += 1;
    }
    fs::copy(source, media_dir.join(&stored))?;
    Ok(stored)
}

/// Converts a window position into image-relative coordinates.
#[must_use]
pub fn relative_position(
    position: (f32, f32),
    origin: (f32, f32),
    size: (f32, f32),
) -> (f32, f32) {
    let ratio = |value: f32, start: f32, length: f32| {
        if length <= 0.0 {
            0.0
        } else {
            (value - start) / length
        }
    };
    (
        ratio(position.0, origin.0, size.0).clamp(0.0, 1.0),
        ratio(position.1, origin.1, size.1).clamp(0.0, 1.0),
    )
}

/// Formats a normalized coordinate the way Anki writes it in markers.
#[must_use]
pub fn ratio(value: f32) -> String {
    let rounded = (value.clamp(0.0, 1.0) * 10_000.0).round() / 10_000.0;
    let text = format!("{rounded:.4}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        String::from("0")
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MaskShape, NormalizedRect, OcclusionMask, default_mask, image_field,
        image_occlusion_fields, import_image, is_image_name, occlusion_field,
        ratio, relative_position,
    };
    use ranki_model::parse_image_occlusion_field;
    use std::fs;
    use std::path::PathBuf;

    #[expect(
        clippy::too_many_arguments,
        reason = "the four coordinates read like the marker grammar"
    )]
    fn rect(left: f32, top: f32, width: f32, height: f32) -> NormalizedRect {
        NormalizedRect {
            left,
            top,
            width,
            height,
        }
    }

    #[test]
    fn rect_marker_round_trips_through_the_parser() {
        let mask = OcclusionMask {
            shape: MaskShape::Rect,
            rect: rect(0.25, 0.125, 0.5, 0.25),
        };
        let marker = mask.marker(1);
        assert_eq!(
            marker,
            "{{c1::image-occlusion:rect:left=0.25:top=0.125:width=0.5:height=0.25}}"
        );
        let field = parse_image_occlusion_field(&marker);
        assert_eq!(field.shapes.len(), 1);
        assert_eq!(
            field.shapes[0].shape,
            ranki_model::image_occlusion::ShapeKind::Rect
        );
        assert_eq!(field.shapes[0].ordinals, [1]);
        for (key, value) in [
            ("left", "0.25"),
            ("top", "0.125"),
            ("width", "0.5"),
            ("height", "0.25"),
        ] {
            assert_eq!(
                field.shapes[0].properties.get(key).map(String::as_str),
                Some(value)
            );
        }
    }

    #[test]
    fn ellipse_markers_get_one_ordinal_each_in_order() {
        let masks = [
            OcclusionMask {
                shape: MaskShape::Ellipse,
                rect: rect(0.1, 0.2, 0.3, 0.4),
            },
            OcclusionMask {
                shape: MaskShape::Rect,
                rect: rect(0.5, 0.5, 0.2, 0.1),
            },
        ];
        let field = occlusion_field(&masks);
        let parsed = parse_image_occlusion_field(&field);
        assert_eq!(parsed.shapes.len(), 2);
        assert_eq!(parsed.shapes[0].ordinals, [1]);
        assert_eq!(parsed.shapes[1].ordinals, [2]);
        assert_eq!(
            parsed.shapes[0].shape,
            ranki_model::image_occlusion::ShapeKind::Ellipse
        );
        assert!(field.contains("<br>{{c2::image-occlusion:rect:"));
    }

    #[test]
    fn drag_points_normalize_whatever_direction_they_came_from() {
        let dragged =
            NormalizedRect::from_points((0.7, 0.6), (0.2, 0.1)).expect("drag");
        assert_eq!(dragged, rect(0.2, 0.1, 0.5, 0.5));
        let clamped = NormalizedRect::from_points((-0.2, 0.5), (1.4, 0.9))
            .expect("clamped drag");
        assert!((clamped.left - 0.0).abs() < f32::EPSILON);
        assert!((clamped.top - 0.5).abs() < f32::EPSILON);
        assert!((clamped.width - 1.0).abs() < f32::EPSILON);
        assert!((clamped.height - 0.4).abs() < 1e-5, "{clamped:?}");
    }

    #[test]
    fn clicks_and_slivers_do_not_create_masks() {
        assert_eq!(NormalizedRect::from_points((0.5, 0.5), (0.5, 0.5)), None);
        assert_eq!(NormalizedRect::from_points((0.5, 0.5), (0.505, 0.9)), None);
    }

    #[test]
    fn field_values_follow_the_stock_model_order() {
        let masks = [OcclusionMask {
            shape: MaskShape::Rect,
            rect: rect(0.1, 0.1, 0.2, 0.2),
        }];
        let fields = image_occlusion_fields("H", "B", "C", "pic.png", &masks);
        assert_eq!(fields.len(), 5);
        assert!(fields[0].starts_with("{{c1::image-occlusion:rect:"));
        assert_eq!(fields[1], "<img src=\"pic.png\">");
        assert_eq!(&fields[2..], ["H", "B", "C"]);
        assert_eq!(image_field("a.png"), "<img src=\"a.png\">");
        assert_eq!(image_field("a\"b.png"), "<img src=\"a&quot;b.png\">");
        assert_eq!(image_field("a&b.png"), "<img src=\"a&amp;b.png\">");
    }

    #[test]
    fn default_masks_stagger_so_repeats_stay_visible() {
        let first = default_mask(0);
        let second = default_mask(1);
        assert_ne!(first.rect.left, second.rect.left);
        assert!(second.rect.left > first.rect.left);
        assert!(default_mask(0).rect.width >= super::MIN_MASK_SIZE);
    }

    #[test]
    fn pointer_positions_become_image_ratios() {
        assert_eq!(
            relative_position((150.0, 60.0), (100.0, 10.0), (200.0, 100.0)),
            (0.25, 0.5)
        );
        assert_eq!(
            relative_position((10.0, 10.0), (100.0, 10.0), (0.0, 0.0)),
            (0.0, 0.0)
        );
        assert_eq!(
            relative_position((400.0, -40.0), (100.0, 10.0), (200.0, 100.0)),
            (1.0, 0.0)
        );
    }

    #[test]
    fn ratios_are_trimmed_to_four_places() {
        assert_eq!(ratio(0.123_456), "0.1235");
        assert_eq!(ratio(0.5), "0.5");
        assert_eq!(ratio(0.0), "0");
        assert_eq!(ratio(1.0), "1");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(".tmp")
            .join(format!("occlusion-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch directory");
        dir
    }

    #[test]
    fn image_names_only_match_known_extensions() {
        assert!(is_image_name("diagram.PNG"));
        assert!(!is_image_name("audio.mp3"));
        assert!(!is_image_name("no-extension"));
    }

    #[test]
    fn uploaded_images_keep_existing_media_and_reject_other_files() {
        let root = scratch("upload");
        let source_dir = root.join("source");
        let media_dir = root.join("media");
        fs::create_dir_all(&source_dir).expect("source directory");
        fs::create_dir_all(&media_dir).expect("media directory");
        let source = source_dir.join("diagram.png");
        fs::write(&source, b"new").expect("source image");
        fs::write(media_dir.join("diagram.png"), b"old").expect("old image");

        let stored = import_image(&source, &media_dir).expect("copy image");
        assert_eq!(stored, "diagram-2.png");
        assert_eq!(fs::read(media_dir.join(&stored)).unwrap(), b"new");
        assert_eq!(fs::read(media_dir.join("diagram.png")).unwrap(), b"old");

        let text = source_dir.join("notes.txt");
        fs::write(&text, b"not an image").expect("text file");
        assert!(import_image(&text, &media_dir).is_err());
        let unsafe_name = source_dir.join("bad\"name.png");
        fs::write(&unsafe_name, b"image").expect("unsafe name");
        assert!(import_image(&unsafe_name, &media_dir).is_err());

        fs::remove_dir_all(&root).expect("cleanup");
    }
}
