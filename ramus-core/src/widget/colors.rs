use serde::{Deserialize, Serialize};

use crate::models::{AlbumColorInfo, UltraBlurColors, VibrantPalette};

/// Lightness floor for an accent drawn over a dark background.
const MIN_ACCENT_LIGHTNESS: f64 = 0.55;

/// Brand accent, used when an album has no palette to take one from.
pub const DEFAULT_ACCENT: &str = "f16280";

/// Brand gradient corners (Pinky Sunset), used when an album has neither
/// server colours nor a palette.
pub const DEFAULT_CORNERS: [&str; 4] = ["853e43", "875147", "854256", "853646"];

/// Where a widget's gradient corners came from. Anything but `Art` is a
/// stand-in the widget replaces by computing corners from the cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CornerSource {
    /// Computed by the client from the same cover the widget shows.
    Art,
    /// The server's UltraBlur colours.
    Server,
    /// Derived from the cached palette swatches.
    Palette,
    /// The brand gradient.
    Default,
}

/// Colours a widget paints with: four gradient corners and the accent, as
/// six-digit hex without a leading `#`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetColors {
    pub top_left: String,
    pub top_right: String,
    pub bottom_right: String,
    pub bottom_left: String,
    pub accent: String,
    pub source: CornerSource,
}

fn strip_hash(hex: &str) -> String {
    hex.strip_prefix('#').unwrap_or(hex).to_string()
}

fn hex_to_rgb(hex: &str) -> Option<[u8; 3]> {
    let h = hex.strip_prefix('#').unwrap_or(hex);
    if h.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// The accent for a palette: the first swatch (vibrant, light vibrant,
/// muted, dark vibrant, light muted, dark muted) that is light enough, or
/// that can be scaled up to the lightness floor while keeping its hue and
/// still reads as a colour rather than near-black.
pub fn accent_from_palette(p: &VibrantPalette) -> [u8; 3] {
    let candidates = [
        &p.vibrant,
        &p.light_vibrant,
        &p.muted,
        &p.dark_vibrant,
        &p.light_muted,
        &p.dark_muted,
    ];
    for hex in candidates.into_iter().flatten() {
        let Some(rgb) = hex_to_rgb(hex) else { continue };
        let max = *rgb.iter().max().unwrap() as f64 / 255.0;
        let min = *rgb.iter().min().unwrap() as f64 / 255.0;
        let l = (max + min) / 2.0;
        if l >= MIN_ACCENT_LIGHTNESS {
            return rgb;
        }
        let factor = MIN_ACCENT_LIGHTNESS / l.max(0.01);
        let boosted = rgb.map(|c| (c as f64 * factor).round().min(255.0) as u8);
        if *boosted.iter().max().unwrap() > 50 {
            return boosted;
        }
    }
    [140, 140, 140]
}

/// Gradient corners derived from palette swatches, for albums the server
/// supplied no colours for.
pub fn blur_colors_from_palette(p: &VibrantPalette) -> UltraBlurColors {
    let fallback = p
        .dark_muted
        .as_deref()
        .or(p.muted.as_deref())
        .or(p.dark_vibrant.as_deref())
        .unwrap_or("333333");
    let pick = |a: &Option<String>, b: &Option<String>| {
        strip_hash(a.as_deref().or(b.as_deref()).unwrap_or(fallback))
    };
    UltraBlurColors {
        top_left: pick(&p.dark_muted, &p.muted),
        top_right: pick(&p.dark_vibrant, &p.dark_muted),
        bottom_left: pick(&p.muted, &p.dark_muted),
        bottom_right: pick(&p.dark_muted, &p.muted),
    }
}

/// Widget colours for an album: corners computed from `thumb` when the
/// client saved some for that cover, else the server's colours, else from
/// the palette, else the brand gradient; accent from the palette, else the
/// brand accent.
pub fn widget_colors(info: Option<&AlbumColorInfo>, thumb: Option<&str>) -> WidgetColors {
    let art = info
        .and_then(|i| i.art_colors.as_ref())
        .filter(|a| thumb == Some(a.thumb.as_str()))
        .map(|a| &a.colors);
    let colors = info.and_then(|i| i.colors.as_ref());
    let palette = info.and_then(|i| i.palette.as_ref());
    let corners_of = |c: &UltraBlurColors| {
        [
            strip_hash(&c.top_left),
            strip_hash(&c.top_right),
            strip_hash(&c.bottom_right),
            strip_hash(&c.bottom_left),
        ]
    };
    let (corners, source) = match (art, colors, palette) {
        (Some(c), _, _) => (corners_of(c), CornerSource::Art),
        (None, Some(c), _) => (corners_of(c), CornerSource::Server),
        (None, None, Some(p)) => {
            let c = blur_colors_from_palette(p);
            (
                [c.top_left, c.top_right, c.bottom_right, c.bottom_left],
                CornerSource::Palette,
            )
        }
        (None, None, None) => (DEFAULT_CORNERS.map(String::from), CornerSource::Default),
    };
    let accent = palette
        .map(|p| {
            let [r, g, b] = accent_from_palette(p);
            format!("{r:02x}{g:02x}{b:02x}")
        })
        .unwrap_or_else(|| DEFAULT_ACCENT.to_string());
    let [top_left, top_right, bottom_right, bottom_left] = corners;
    WidgetColors {
        top_left,
        top_right,
        bottom_right,
        bottom_left,
        accent,
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ArtBlurColors;

    fn palette() -> VibrantPalette {
        VibrantPalette {
            vibrant: None,
            dark_vibrant: None,
            light_vibrant: None,
            muted: None,
            dark_muted: None,
            light_muted: None,
        }
    }

    #[test]
    fn accent_takes_first_light_enough_swatch() {
        // #31b062 has lightness 0.441, so it is scaled by 0.55 / 0.441.
        let p = VibrantPalette {
            vibrant: Some("#31b062".into()),
            ..palette()
        };
        assert_eq!(accent_from_palette(&p), [61, 219, 122]);
        let bright = VibrantPalette {
            vibrant: Some("#98d49a".into()),
            ..palette()
        };
        assert_eq!(accent_from_palette(&bright), [0x98, 0xd4, 0x9a]);
    }

    #[test]
    fn accent_skips_near_black_after_boost() {
        let p = VibrantPalette {
            dark_muted: Some("#050505".into()),
            ..palette()
        };
        assert_eq!(accent_from_palette(&p), [140, 140, 140]);
    }

    #[test]
    fn blur_from_palette_uses_dark_muted_for_tl_and_br() {
        let p = VibrantPalette {
            dark_muted: Some("#31465a".into()),
            muted: Some("#6a9387".into()),
            dark_vibrant: Some("#186855".into()),
            ..palette()
        };
        let c = blur_colors_from_palette(&p);
        assert_eq!(c.top_left, "31465a");
        assert_eq!(c.bottom_right, "31465a");
        assert_eq!(c.top_right, "186855");
        assert_eq!(c.bottom_left, "6a9387");
        assert_eq!(blur_colors_from_palette(&palette()).top_left, "333333");
    }

    #[test]
    fn widget_colors_falls_back_to_pinky_sunset_and_brand_accent() {
        let c = widget_colors(None, None);
        assert_eq!(
            (
                c.top_left.as_str(),
                c.top_right.as_str(),
                c.bottom_right.as_str(),
                c.bottom_left.as_str()
            ),
            ("853e43", "875147", "854256", "853646")
        );
        assert_eq!(c.accent, "f16280");
        let empty = AlbumColorInfo {
            colors: None,
            palette: None,
            art_colors: None,
        };
        assert_eq!(widget_colors(Some(&empty), None), c);
        assert_eq!(c.source, CornerSource::Default);
    }

    #[test]
    fn widget_colors_prefers_server_corners_over_palette() {
        let info = AlbumColorInfo {
            colors: Some(UltraBlurColors {
                top_left: "1a1e45".into(),
                top_right: "2d572f".into(),
                bottom_right: "4f6731".into(),
                bottom_left: "11323e".into(),
            }),
            palette: Some(VibrantPalette {
                vibrant: Some("#31b062".into()),
                dark_muted: Some("#31465a".into()),
                ..palette()
            }),
            art_colors: None,
        };
        let c = widget_colors(Some(&info), None);
        assert_eq!(c.source, CornerSource::Server);
        assert_eq!(c.top_left, "1a1e45");
        assert_eq!(c.bottom_left, "11323e");
        assert_eq!(c.accent, "3ddb7a");
    }

    #[test]
    fn widget_colors_derives_corners_from_palette_without_server_colours() {
        let info = AlbumColorInfo {
            colors: None,
            palette: Some(VibrantPalette {
                dark_muted: Some("#31465a".into()),
                ..palette()
            }),
            art_colors: None,
        };
        let c = widget_colors(Some(&info), None);
        assert_eq!(c.top_left, "31465a");
        assert_eq!(c.source, CornerSource::Palette);
    }

    fn corners(top_left: &str) -> UltraBlurColors {
        UltraBlurColors {
            top_left: top_left.into(),
            top_right: "222222".into(),
            bottom_right: "333333".into(),
            bottom_left: "444444".into(),
        }
    }

    fn info_with_art(thumb: &str) -> AlbumColorInfo {
        AlbumColorInfo {
            colors: Some(corners("1a1e45")),
            palette: None,
            art_colors: Some(ArtBlurColors {
                thumb: thumb.into(),
                colors: corners("5e2a31"),
            }),
        }
    }

    #[test]
    fn widget_colors_prefers_corners_saved_from_the_same_cover() {
        let info = info_with_art("/library/metadata/7/thumb/1700000000");
        let c = widget_colors(Some(&info), Some("/library/metadata/7/thumb/1700000000"));
        assert_eq!(c.source, CornerSource::Art);
        assert_eq!(c.top_left, "5e2a31");
        assert_eq!(c.bottom_left, "444444");
    }

    #[test]
    fn widget_colors_ignores_corners_saved_from_another_cover() {
        let info = info_with_art("/library/metadata/7/thumb/1600000000");
        let c = widget_colors(Some(&info), Some("/library/metadata/7/thumb/1700000000"));
        assert_eq!(c.source, CornerSource::Server);
        assert_eq!(c.top_left, "1a1e45");
        let no_cover = widget_colors(Some(&info), None);
        assert_eq!(no_cover.source, CornerSource::Server);
    }
}
