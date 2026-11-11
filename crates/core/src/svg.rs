//! SVG export at 1:1 scale.
//!
//! The document is sized in millimetres (`width="…mm"`) with a `viewBox` in
//! the same units, so printing at 100 % reproduces the pattern at true
//! size; a 50 mm scale bar is included to check the print. The piece is
//! drawn as seen from outside the body (`y` flipped for SVG's downward
//! axis), i.e. the face that touches the body points away from the viewer.
//!
//! ## Heatmap
//! Each triangle can be filled by a stretch value `t`:
//! - [`HeatmapMetric::MaxStretch`]: `t = σ_max`;
//! - [`HeatmapMetric::AreaStretch`]: `t = σ_min σ_max`.
//!
//! Colour position `x = ln t / ln(range^k)` clamped to `[−1, 1]`, with
//! `k = 1` for max stretch and `k = 2` for area stretch (an area stretch of
//! `range²` corresponds to `range` in each direction). `x = −1` is blue
//! (slack), `0` white (unstretched), `+1` orange (stretched); colours are
//! interpolated linearly in sRGB.

use crate::flatten::FlatPanel;
use crate::strain::PrincipalStretch;
use glam::DVec2;
use std::fmt::Write;

/// Which stretch measure colours the heatmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HeatmapMetric {
    /// Largest principal stretch `σ_max`.
    MaxStretch,
    /// Area stretch `σ_min σ_max`.
    AreaStretch,
}

impl HeatmapMetric {
    fn value(self, s: &PrincipalStretch) -> f64 {
        match self {
            HeatmapMetric::MaxStretch => s.max,
            HeatmapMetric::AreaStretch => s.area(),
        }
    }
    fn exponent(self) -> f64 {
        match self {
            HeatmapMetric::MaxStretch => 1.0,
            HeatmapMetric::AreaStretch => 2.0,
        }
    }
    fn label(self) -> &'static str {
        match self {
            HeatmapMetric::MaxStretch => "max stretch",
            HeatmapMetric::AreaStretch => "area stretch",
        }
    }
}

/// SVG rendering options.
#[derive(Debug, Clone, PartialEq)]
pub struct SvgOptions {
    /// Heatmap fill, or `None` for outline only.
    pub heatmap: Option<HeatmapMetric>,
    /// Per-direction stretch at which the colour saturates (e.g. 1.5).
    pub range: f64,
    /// Blank margin around the piece (mm).
    pub margin: f64,
    /// Title printed above the piece.
    pub label: Option<String>,
}

impl Default for SvgOptions {
    fn default() -> Self {
        Self { heatmap: None, range: 1.5, margin: 10.0, label: None }
    }
}

const SLACK: [f64; 3] = [33.0, 102.0, 172.0]; // #2166ac
const NEUTRAL: [f64; 3] = [255.0, 255.0, 255.0];
const TAUT: [f64; 3] = [230.0, 97.0, 1.0]; // #e66101

/// Diverging colour for a stretch value (see module docs).
pub fn stretch_color(t: f64, range: f64, exponent: f64) -> String {
    let x = if t.is_finite() && t > 0.0 { (t.ln() / (exponent * range.ln())).clamp(-1.0, 1.0) } else { 1.0 };
    let (end, a) = if x < 0.0 { (SLACK, -x) } else { (TAUT, x) };
    let c: [u8; 3] = std::array::from_fn(|i| (NEUTRAL[i] + a * (end[i] - NEUTRAL[i])).round() as u8);
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Render one flattened panel as a standalone SVG document.
pub fn panel_svg(fp: &FlatPanel, opts: &SvgOptions) -> String {
    // Flip y so the piece reads correctly in SVG's y-down space.
    let pts: Vec<DVec2> = fp.uv.iter().map(|p| DVec2::new(p.x, -p.y)).collect();
    let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
    for p in &pts {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let m = opts.margin;
    let header = 6.0 + if opts.label.is_some() { 6.0 } else { 0.0 };
    let footer = 12.0;
    let x0 = lo.x - m;
    let y0 = lo.y - m - header;
    let w = (hi.x - lo.x) + 2.0 * m;
    let w = w.max(70.0);
    let h = (hi.y - lo.y) + 2.0 * m + header + footer;

    let mut s = String::new();
    let _ = writeln!(s, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.3}mm" height="{h:.3}mm" viewBox="{x0:.3} {y0:.3} {w:.3} {h:.3}">"#
    );
    let _ = writeln!(s, r#"<g font-family="sans-serif" font-size="3.5" fill="black">"#);
    let mut ty = lo.y - m - header + 4.5;
    if let Some(label) = &opts.label {
        let _ = writeln!(s, r#"<text x="{:.3}" y="{ty:.3}" font-size="5">{}</text>"#, lo.x - m + 2.0, escape(label));
        ty += 6.0;
    }
    let r = &fp.strain;
    let _ = writeln!(
        s,
        r#"<text x="{:.3}" y="{ty:.3}">stretch {:.3}–{:.3}, mean area ×{:.3}{}</text>"#,
        lo.x - m + 2.0,
        r.min_stretch,
        r.max_stretch,
        r.mean_area_stretch,
        if r.flipped.is_empty() { String::new() } else { format!(", {} FLIPPED", r.flipped.len()) }
    );
    let _ = writeln!(s, "</g>");

    if let Some(metric) = opts.heatmap {
        let _ = writeln!(s, r#"<g id="heatmap" stroke-width="0.05" stroke-linejoin="round">"#);
        for (t, tri) in fp.panel.triangles.iter().enumerate() {
            let c = stretch_color(metric.value(&r.triangles[t]), opts.range, metric.exponent());
            let [a, b, d] = tri.map(|i| pts[i as usize]);
            let _ = writeln!(
                s,
                r#"<path d="M{:.3} {:.3}L{:.3} {:.3}L{:.3} {:.3}Z" fill="{c}" stroke="{c}"/>"#,
                a.x, a.y, b.x, b.y, d.x, d.y
            );
        }
        let _ = writeln!(s, "</g>");
    }

    // Outline along the boundary loop.
    let mut d = String::new();
    for (k, e) in fp.panel.boundary.iter().enumerate() {
        let p = pts[e.from as usize];
        let _ = write!(d, "{}{:.3} {:.3}", if k == 0 { "M" } else { "L" }, p.x, p.y);
    }
    d.push('Z');
    let _ = writeln!(s, r#"<path id="outline" d="{d}" fill="none" stroke="black" stroke-width="0.3" stroke-linejoin="round"/>"#);

    // Footer: 50 mm scale bar and legend.
    let fy = hi.y + m + 4.0;
    let fx = lo.x - m + 2.0;
    let _ = writeln!(
        s,
        r#"<g id="scale" stroke="black" stroke-width="0.3"><path d="M{fx:.3} {:.3}V{fy:.3}H{:.3}V{:.3}" fill="none"/></g>"#,
        fy - 1.5,
        fx + 50.0,
        fy - 1.5
    );
    let _ = writeln!(s, r#"<text x="{:.3}" y="{:.3}" font-family="sans-serif" font-size="3">50 mm</text>"#, fx, fy + 4.0);
    if let Some(metric) = opts.heatmap {
        let lx = fx + 56.0;
        let k = metric.exponent();
        let stops = [1.0 / opts.range.powf(k), 1.0, opts.range.powf(k)];
        for (i, v) in stops.iter().enumerate() {
            let x = lx + i as f64 * 14.0;
            let _ = writeln!(
                s,
                r#"<rect x="{x:.3}" y="{:.3}" width="4" height="3" fill="{}" stroke="black" stroke-width="0.1"/><text x="{:.3}" y="{fy:.3}" font-family="sans-serif" font-size="3">{v:.2}</text>"#,
                fy - 2.5,
                stretch_color(*v, opts.range, k),
                x + 5.0
            );
        }
        let _ = writeln!(
            s,
            r#"<text x="{lx:.3}" y="{:.3}" font-family="sans-serif" font-size="3">{}</text>"#,
            fy + 4.0,
            metric.label()
        );
    }
    let _ = writeln!(s, "</svg>");
    s
}

fn escape(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{flatten_garment, testmesh, FlattenOptions, SeamSet};

    #[test]
    fn colours() {
        assert_eq!(stretch_color(1.0, 1.5, 1.0), "#ffffff");
        assert_eq!(stretch_color(1.5, 1.5, 1.0), "#e66101");
        assert_eq!(stretch_color(1.0 / 1.5, 1.5, 1.0), "#2166ac");
        assert_eq!(stretch_color(2.25, 1.5, 2.0), "#e66101");
        assert_eq!(stretch_color(10.0, 1.5, 1.0), "#e66101");
    }

    #[test]
    fn svg_is_mm_sized_and_contains_outline() {
        let m = testmesh::grid(4, 2, 25.0);
        let fp = &flatten_garment(&m, &SeamSet::default(), &FlattenOptions::default()).unwrap()[0];
        let svg = panel_svg(fp, &SvgOptions { heatmap: Some(HeatmapMetric::AreaStretch), label: Some("A&B".into()), ..Default::default() });
        // 100 mm × 0.85 wide piece + 2 × 10 mm margin.
        assert!(svg.contains(r#"width="105.000mm""#), "{}", &svg[..300]);
        assert!(svg.contains(r#"id="outline""#));
        assert_eq!(svg.matches("stroke=\"#").count(), m.triangles().len());
        assert!(svg.contains("A&amp;B"));
    }
}
