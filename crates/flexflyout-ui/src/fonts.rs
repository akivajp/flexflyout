//! Installing the host's fonts so that mixed scripts share one baseline.
//!
//! egui lays out a row with the metrics of the family's first font and, for
//! a glyph from a fallback font, centers that font's line height in the row
//! instead of aligning baselines. A Japanese font with a large line gap
//! (Yu Gothic) then sits several pixels above Latin text from Segoe UI. Each
//! fallback font gets a vertical tweak that moves its baseline back down.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily, FontTweak};

/// Vertical metrics of a font, as fractions of the font size (em).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VerticalMetrics {
    /// Distance from the baseline up to the top of the line (positive).
    pub ascent: f32,
    /// Distance from the baseline down to the bottom (negative).
    pub descent: f32,
    /// Extra gap between lines.
    pub leading: f32,
}

impl VerticalMetrics {
    /// Reads the metrics of font number `index` in a `.ttf`/`.ttc` file.
    pub fn read(bytes: &[u8], index: u32) -> Option<Self> {
        use skrifa::MetadataProvider as _;
        let font = skrifa::FontRef::from_index(bytes, index).ok()?;
        // The same call epaint makes: unscaled (font units), default axes.
        let m = font.metrics(
            skrifa::instance::Size::unscaled(),
            skrifa::instance::LocationRef::default(),
        );
        let em = f32::from(m.units_per_em);
        (em > 0.0).then(|| Self {
            ascent: m.ascent / em,
            descent: m.descent / em,
            leading: m.leading / em,
        })
    }

    /// Line height as egui computes it.
    fn height(&self) -> f32 {
        self.ascent - self.descent + self.leading
    }
}

/// How far (in em, downwards) a glyph of the `fallback` font must move to
/// stand on the baseline of the `primary` font in egui's layout.
///
/// egui puts a fallback glyph's baseline at
/// `ascent_f + (height_p - height_f) / 2`; the primary's is at `ascent_p`.
pub fn baseline_offset(primary: VerticalMetrics, fallback: VerticalMetrics) -> f32 {
    primary.ascent - fallback.ascent - 0.5 * (primary.height() - fallback.height())
}

/// Puts `fonts` (name and file contents, primary first) in front of egui's
/// own fonts in every family, each fallback tweaked onto the primary's
/// baseline. Returns the offsets applied (em), for the log.
pub fn install(ctx: &egui::Context, fonts: Vec<(String, Vec<u8>)>) -> Vec<(String, f32)> {
    let mut definitions = FontDefinitions::default();
    let mut primary = None;
    let mut applied = Vec::new();
    for (i, (name, bytes)) in fonts.into_iter().enumerate() {
        let metrics = VerticalMetrics::read(&bytes, 0);
        let offset = match (primary, metrics) {
            (None, Some(m)) => {
                primary = Some(m);
                0.0
            }
            (Some(p), Some(m)) => baseline_offset(p, m),
            // Unreadable metrics: egui will probably reject the font too.
            (_, None) => 0.0,
        };
        let tweak = FontTweak {
            y_offset_factor: offset,
            ..FontTweak::default()
        };
        definitions.font_data.insert(
            name.clone(),
            Arc::new(FontData::from_owned(bytes).tweak(tweak)),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            definitions
                .families
                .entry(family)
                .or_default()
                .insert(i, name.clone());
        }
        applied.push((name, offset));
    }
    ctx.set_fonts(definitions);
    applied
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_metrics_need_no_offset() {
        let m = VerticalMetrics {
            ascent: 0.9,
            descent: -0.2,
            leading: 0.0,
        };
        assert_eq!(baseline_offset(m, m), 0.0);
    }

    #[test]
    fn a_taller_fallback_is_moved_down() {
        // A Latin primary and a fallback with a larger ascent and line gap,
        // which egui would center and so raise above the baseline.
        let primary = VerticalMetrics {
            ascent: 1.0,
            descent: -0.25,
            leading: 0.0,
        };
        let fallback = VerticalMetrics {
            ascent: 1.1,
            descent: -0.3,
            leading: 0.2,
        };
        let offset = baseline_offset(primary, fallback);
        // egui's baseline for the fallback: 1.1 + (1.25 - 1.6) / 2 = 0.925;
        // the primary's is 1.0, so the fallback must go 0.075 em down.
        assert!((offset - 0.075).abs() < 1e-6, "{offset}");
    }
}
