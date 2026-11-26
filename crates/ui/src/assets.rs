//! Embedded UI icons and the media asset source used by GPUI images.

use std::{borrow::Cow, fs, path::PathBuf};

use gpui::{AssetSource, Result, SharedString};

// Crane's source filter drops non-Rust assets, so the icon ships as a
// constant. `assets/rai.svg` stays the editable copy.
const RAI: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="800" height="800" viewBox="0 0 211.6667 211.66669">
  <defs>
    <filter id="glow" style="color-interpolation-filters:sRGB;" x="-0.056453245" y="-0.13570336" width="1.1329358" height="1.3195536">
      <feOffset dx="3" dy="3"/>
      <feGaussianBlur stdDeviation="3" result="blur"/>
      <feFlood flood-color="rgb(0,0,0)" flood-opacity="1" result="flood"/>
      <feComposite in="flood" in2="SourceGraphic" operator="in" result="composite"/>
      <feBlend in="blur" in2="composite" mode="normal" result="fbSourceGraphic"/>
      <feColorMatrix in="fbSourceGraphic" values="0 0 0 -1 0 0 0 0 -1 0 0 0 0 -1 0 0 0 0 1 0"/>
      <feColorMatrix in="fbSourceGraphic" values="1.3 0 0 0.0345 -0.15 0 1.3 0 0.0345 -0.15 0 0 1.3 0.0345 -0.15 0 0 0 1 0"/>
    </filter>
  </defs>
  <text
     xml:space="preserve"
     style="font-weight:bold;font-size:79.375px;font-family:Comfortaa;fill:#000000;fill-rule:evenodd;stroke:#181825;stroke-width:2.51118;stroke-opacity:1;stroke-linejoin:round;paint-order:markers fill stroke;filter:url(#glow)"
     x="4.7136173"
     y="147.98276"
     transform="translate(23.41563,0.95911) matrix(1.0246445,0,0,0.96727946,17.908656,-8.9300719)"><tspan style="fill:#cba6f7">r</tspan><tspan style="fill:#94e2d5">a</tspan><tspan style="fill:#f38ba8">i</tspan></text>
</svg>
"#;

fn icon(body: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\">{body}</svg>"
    )
}

const ICONS: &[(&str, &str)] = &[
    ("icon-plus", "<path d=\"M12 5v14M5 12h14\"/>"),
    (
        "icon-upload",
        "<path d=\"M12 16V4m-5 5 5-5 5 5\"/><path d=\"M4 16v3h16v-3\"/>",
    ),
    (
        "icon-search",
        "<circle cx=\"11\" cy=\"11\" r=\"7\"/><path d=\"m20 20-4-4\"/>",
    ),
    ("icon-chevron-down", "<path d=\"m6 9 6 6 6-6\"/>"),
    ("icon-chevron-right", "<path d=\"m9 6 6 6-6 6\"/>"),
    (
        "icon-grid",
        "<rect x=\"3\" y=\"3\" width=\"7\" height=\"7\" rx=\"1\"/><rect x=\"14\" y=\"3\" width=\"7\" height=\"7\" rx=\"1\"/><rect x=\"3\" y=\"14\" width=\"7\" height=\"7\" rx=\"1\"/><rect x=\"14\" y=\"14\" width=\"7\" height=\"7\" rx=\"1\"/>",
    ),
    (
        "icon-book",
        "<path d=\"M4 19.5A2.5 2.5 0 0 1 6.5 17H20\"/><path d=\"M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z\"/>",
    ),
    (
        "icon-chart",
        "<path d=\"M3 3v18h18\"/><path d=\"M7 15v3M12 9v9M17 5v13\"/>",
    ),
    (
        "icon-settings",
        "<circle cx=\"12\" cy=\"12\" r=\"3\"/><path d=\"M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6V21h-4v-.1a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9 1.7 1.7 0 0 0-1.6-1H3v-4h.1a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1a1.7 1.7 0 0 0 1.9.3 1.7 1.7 0 0 0 1-1.6V3h4v.1a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1H21v4h-.1a1.7 1.7 0 0 0-1.5 1z\"/>",
    ),
    ("icon-play", "<path d=\"m7 4 13 8-13 8V4z\"/>"),
    (
        "icon-volume",
        "<path d=\"M11 5 6 9H2v6h4l5 4V5z\"/><path d=\"M15.5 8.5a5 5 0 0 1 0 7\"/>",
    ),
    (
        "icon-more",
        "<circle cx=\"5\" cy=\"12\" r=\"1\"/><circle cx=\"12\" cy=\"12\" r=\"1\"/><circle cx=\"19\" cy=\"12\" r=\"1\"/>",
    ),
    ("icon-check", "<path d=\"m5 12 4 4L19 6\"/>"),
    ("icon-x", "<path d=\"M18 6 6 18M6 6l12 12\"/>"),
    ("icon-arrow-left", "<path d=\"M19 12H5m6-7-7 7 7 7\"/>"),
    (
        "icon-trash",
        "<path d=\"M3 6h18M8 6V4h8v2m-9 0 1 14h8l1-14\"/>",
    ),
    (
        "icon-pencil",
        "<path d=\"M12 20h9\"/><path d=\"M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z\"/>",
    ),
    (
        "icon-undo",
        "<path d=\"M3 7v6h6\"/><path d=\"M3 13a9 9 0 1 0 3-7\"/>",
    ),
    ("icon-flag", "<path d=\"M5 21V4h13l-2 4 2 4H5\"/>"),
    (
        "icon-tag",
        "<path d=\"M20.6 13.4 12 22l-9-9V3h10l7.6 7.6a2 2 0 0 1 0 2.8z\"/><circle cx=\"7.5\" cy=\"7.5\" r=\"1\"/>",
    ),
    (
        "icon-bell",
        "<path d=\"M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9\"/><path d=\"M10 21h4\"/>",
    ),
    (
        "icon-clock",
        "<circle cx=\"12\" cy=\"12\" r=\"9\"/><path d=\"M12 7v5l3 2\"/>",
    ),
    (
        "icon-layers",
        "<path d=\"m12 2 9 5-9 5-9-5 9-5z\"/><path d=\"m3 12 9 5 9-5M3 17l9 5 9-5\"/>",
    ),
    ("icon-code", "<path d=\"m16 18 6-6-6-6M8 6l-6 6 6 6\"/>"),
    ("icon-math", "<path d=\"M18 7V4H6l6 8-6 8h12v-3\"/>"),
    (
        "icon-image",
        "<rect x=\"3\" y=\"3\" width=\"18\" height=\"18\" rx=\"2\"/><circle cx=\"9\" cy=\"9\" r=\"2\"/><path d=\"m21 15-3.1-3.1a2 2 0 0 0-2.8 0L6 21\"/>",
    ),
];

/// Serves embedded icons and local Anki media without touching the collection.
pub struct Assets {
    pub media_dir: PathBuf,
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == "rai.svg" {
            return Ok(Some(Cow::Borrowed(RAI.as_bytes())));
        }
        if let Some((_, body)) = ICONS.iter().find(|(name, _)| *name == path) {
            let svg = icon(body);
            return Ok(Some(Cow::Owned(svg.into_bytes())));
        }
        let file = self.media_dir.join(path);
        Ok(fs::read(file).ok().map(Cow::Owned))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let Ok(entries) = fs::read_dir(self.media_dir.join(path)) else {
            return Ok(Vec::new());
        };
        Ok(entries
            .filter_map(|entry| {
                Some(SharedString::from(
                    entry.ok()?.file_name().to_string_lossy().into_owned(),
                ))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use gpui::AssetSource;

    use super::{Assets, RAI};

    #[test]
    fn embedded_icons_and_logo_are_available() {
        let assets = Assets {
            media_dir: std::path::PathBuf::from("."),
        };
        assert!(assets.load("rai.svg").unwrap().is_some());
        assert!(assets.load("icon-plus").unwrap().is_some());
        for name in ["icon-code", "icon-math", "icon-image"] {
            assert!(
                assets.load(name).unwrap().is_some(),
                "{name} must be embedded"
            );
        }
        assert!(assets.load("missing-file").unwrap().is_none());
    }

    #[test]
    fn embedded_logo_matches_the_asset_copy() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/rai.svg");
        if let Ok(svg) = std::fs::read_to_string(path) {
            assert!(RAI.contains(svg.trim()));
        }
    }
}
