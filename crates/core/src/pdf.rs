//! Print-at-home PDF: pattern pieces at 1:1, tiled over A4 or Letter pages.
//!
//! The document starts with information pages (a 50 mm print-check square,
//! the materials list, which pages make up each piece, and the assembly
//! steps), then each piece split over a grid of pages. Neighbouring pages
//! overlap by `overlap_mm`; corner marks show where to trim, and each page
//! is labelled with its piece and grid position (row letter, column
//! number), so the sheets can be taped together.
//!
//! The PDF is written directly (PDF 1.4, uncompressed vector content, the
//! built-in Helvetica font) and is plain ASCII. Drawings are in millimetres
//! with `y` up, like the flat pattern; one `cm` operator scales them to
//! points (72 / 25.4 per mm).

use crate::flatten::FlatPanel;
use crate::pattern::{PatternPiece, TrimStrip};
use glam::DVec2;
use std::fmt::Write;

/// Paper size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Paper {
    #[default]
    A4,
    Letter,
}

impl Paper {
    /// Width and height (mm), portrait.
    pub fn size_mm(self) -> (f64, f64) {
        match self {
            Paper::A4 => (210.0, 297.0),
            Paper::Letter => (215.9, 279.4),
        }
    }
}

/// Page set-up.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct PrintOptions {
    pub paper: Paper,
    /// Unprinted border (mm); most printers need about 5–10 mm.
    pub margin_mm: f64,
    /// Overlap between neighbouring pages (mm).
    pub overlap_mm: f64,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            paper: Paper::A4,
            margin_mm: 10.0,
            overlap_mm: 10.0,
        }
    }
}

/// A polyline in a drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub points: Vec<DVec2>,
    pub closed: bool,
    pub width_mm: f64,
    pub dashed: bool,
}

/// A label in a drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub at: DVec2,
    pub size_mm: f64,
    pub text: String,
}

/// Vector content of one piece, in mm, `y` up.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Drawing {
    pub title: String,
    pub strokes: Vec<Stroke>,
    pub labels: Vec<Label>,
}

impl Drawing {
    fn bounds(&self) -> (DVec2, DVec2) {
        let mut lo = DVec2::splat(f64::INFINITY);
        let mut hi = DVec2::splat(f64::NEG_INFINITY);
        for p in self.strokes.iter().flat_map(|s| &s.points) {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        for l in &self.labels {
            lo = lo.min(l.at);
            hi = hi.max(l.at);
        }
        (lo, hi)
    }
}

/// A pattern piece: cut line, glue line where it has an allowance, notches
/// with their numbers.
pub fn piece_drawing(title: &str, fp: &FlatPanel, piece: &PatternPiece) -> Drawing {
    let mut d = Drawing {
        title: title.to_string(),
        ..Default::default()
    };
    d.strokes.push(Stroke {
        points: piece.cut_outline.clone(),
        closed: true,
        width_mm: 0.3,
        dashed: false,
    });
    for (k, e) in fp.panel.boundary.iter().enumerate() {
        if piece.edge_allowance[k] > 0.0 {
            d.strokes.push(Stroke {
                points: vec![fp.uv[e.from as usize], fp.uv[e.to as usize]],
                closed: false,
                width_mm: 0.2,
                dashed: true,
            });
        }
    }
    for n in &piece.notches {
        let out = piece.edge_allowance[n.boundary_edge].max(2.0);
        d.strokes.push(Stroke {
            points: vec![n.position - 4.0 * n.normal, n.position + out * n.normal],
            closed: false,
            width_mm: 0.3,
            dashed: false,
        });
        d.labels.push(Label {
            at: n.position - 7.0 * n.normal - DVec2::new(1.0, 1.0),
            size_mm: 2.5,
            text: n.number.to_string(),
        });
    }
    // The piece's name (the title up to any " — ") in its middle, so a
    // loose page can be told apart.
    let c = fp.uv.iter().sum::<DVec2>() / fp.uv.len() as f64;
    let short = title.split(" — ").next().unwrap_or(title);
    d.labels.push(Label {
        at: c - DVec2::new(1.5 * short.len() as f64, 0.0),
        size_mm: 6.0,
        text: short.to_string(),
    });
    d
}

/// A trim strip with marks at each quarter of its length.
pub fn trim_drawing(title: &str, t: &TrimStrip) -> Drawing {
    let mut d = Drawing {
        title: title.to_string(),
        ..Default::default()
    };
    d.strokes.push(Stroke {
        points: t.outline(),
        closed: true,
        width_mm: 0.3,
        dashed: false,
    });
    for k in 1..4 {
        let x = t.length_mm * k as f64 / 4.0;
        d.strokes.push(Stroke {
            points: vec![DVec2::new(x, 0.0), DVec2::new(x, t.width_mm)],
            closed: false,
            width_mm: 0.2,
            dashed: true,
        });
    }
    d.labels.push(Label {
        at: DVec2::new(2.0, t.width_mm + 3.0),
        size_mm: 4.0,
        text: title.to_string(),
    });
    d
}

const PT_PER_MM: f64 = 72.0 / 25.4;

/// PDF text strings: ASCII only, with `(`, `)` and `\` escaped.
fn pdf_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '–' | '—' => out.push('-'),
            '×' => out.push('x'),
            '↔' => out.push_str("<->"),
            '→' => out.push_str("->"),
            '²' => out.push('2'),
            '…' => out.push_str("..."),
            '‘' | '’' => out.push('\''),
            '“' | '”' => out.push('"'),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

/// Wrap text, taking Helvetica as about 0.45 em per character.
fn wrap(text: &str, size_mm: f64, width_mm: f64) -> Vec<String> {
    let max = ((width_mm / (0.45 * size_mm)) as usize).max(10);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > max {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Page content in mm.
#[derive(Default)]
struct Page {
    ops: String,
}

impl Page {
    fn text(&mut self, x: f64, y: f64, size: f64, s: &str) {
        let _ = writeln!(
            self.ops,
            "BT /F1 {size:.2} Tf {x:.3} {y:.3} Td ({}) Tj ET",
            pdf_text(s)
        );
    }
    fn stroke(&mut self, s: &Stroke) {
        if s.points.len() < 2 {
            return;
        }
        let _ = write!(
            self.ops,
            "{:.3} w {} 0 d ",
            s.width_mm,
            if s.dashed { "[2 1]" } else { "[]" }
        );
        for (i, p) in s.points.iter().enumerate() {
            let _ = write!(
                self.ops,
                "{:.3} {:.3} {} ",
                p.x,
                p.y,
                if i == 0 { "m" } else { "l" }
            );
        }
        let _ = writeln!(self.ops, "{}", if s.closed { "h S" } else { "S" });
    }
    fn line(&mut self, a: DVec2, b: DVec2, w: f64) {
        let _ = writeln!(
            self.ops,
            "{w:.3} w [] 0 d {:.3} {:.3} m {:.3} {:.3} l S",
            a.x, a.y, b.x, b.y
        );
    }
}

/// Build the PDF. `info` paragraphs go on the first pages after the
/// print-check square.
pub fn pattern_pdf(
    title: &str,
    info: &[String],
    drawings: &[Drawing],
    opts: &PrintOptions,
) -> String {
    let (pw, ph) = opts.paper.size_mm();
    let m = opts.margin_mm;
    let (aw, ah) = (pw - 2.0 * m, ph - 2.0 * m);
    let mut pages: Vec<Page> = Vec::new();

    // Tiling of each drawing: (cols, rows, origin) with an overlap.
    let step = DVec2::new(aw - opts.overlap_mm, ah - opts.overlap_mm);
    let tiles: Vec<(usize, usize, DVec2, DVec2)> = drawings
        .iter()
        .map(|d| {
            let (lo, hi) = d.bounds();
            let size = hi - lo;
            let cols = ((size.x - opts.overlap_mm) / step.x).ceil().max(1.0) as usize;
            let rows = ((size.y - opts.overlap_mm) / step.y).ceil().max(1.0) as usize;
            (cols, rows, lo, hi)
        })
        .collect();

    // Information pages: the title, the print-check square (top right of
    // the first page) and the paragraphs, wrapped beside the square and
    // then at full width.
    let mut paragraphs: Vec<(f64, String)> = vec![(7.0, title.to_string())];
    paragraphs.extend(info.iter().map(|p| (3.5, p.clone())));
    paragraphs.push((
        3.5,
        "Pages of each piece (rows A, B, ... from the top; columns 1, 2, ... from the left):"
            .into(),
    ));
    for (d, (c, r, _, _)) in drawings.iter().zip(&tiles) {
        paragraphs.push((3.5, format!("{}: {r} row(s) x {c} column(s)", d.title)));
    }
    let mut page = Page::default();
    let sq = DVec2::new(pw - m - 50.0, ph - m - 50.0);
    page.stroke(&Stroke {
        points: vec![
            sq,
            sq + DVec2::new(50.0, 0.0),
            sq + DVec2::new(50.0, 50.0),
            sq + DVec2::new(0.0, 50.0),
        ],
        closed: true,
        width_mm: 0.3,
        dashed: false,
    });
    page.text(sq.x + 3.0, sq.y + 27.0, 3.2, "50 mm square:");
    page.text(sq.x + 3.0, sq.y + 22.0, 3.2, "measure it");
    page.text(sq.x + 3.0, sq.y + 17.0, 3.2, "before cutting");
    let mut beside_square = true;
    let mut y = ph - m - 8.0;
    for (size, text) in &paragraphs {
        let line_h = size * 1.35;
        let mut rest: Vec<&str> = text.split_whitespace().collect();
        while !rest.is_empty() {
            if y - line_h < m {
                pages.push(std::mem::take(&mut page));
                y = ph - m - 8.0;
                beside_square = false;
            }
            let width = if beside_square && y > sq.y - line_h {
                aw - 58.0
            } else {
                aw
            };
            // One line's worth of words.
            let line = wrap(&rest.join(" "), *size, width).remove(0);
            let used = line.split_whitespace().count();
            rest.drain(..used);
            page.text(m, y, *size, &line);
            y -= line_h;
        }
        y -= 2.0;
    }
    pages.push(page);

    // Piece pages.
    for (d, &(cols, rows, lo, hi)) in drawings.iter().zip(&tiles) {
        for r in 0..rows {
            for c in 0..cols {
                // Window in drawing coordinates; rows run from the top.
                let x0 = lo.x + c as f64 * step.x;
                let y1 = hi.y - r as f64 * step.y;
                let y0 = y1 - ah;
                let win_lo = DVec2::new(x0, y0);
                let win_hi = DVec2::new(x0 + aw, y1);
                let touches = d.strokes.iter().any(|s| {
                    s.points.windows(2).any(|w| {
                        let (a, b) = (w[0].min(w[1]), w[0].max(w[1]));
                        a.x <= win_hi.x && b.x >= win_lo.x && a.y <= win_hi.y && b.y >= win_lo.y
                    })
                });
                if !touches {
                    continue;
                }
                let mut p = Page::default();
                let tag = format!("{}{}", (b'A' + (r % 26) as u8) as char, c + 1);
                p.text(
                    m + 6.0,
                    ph - m + 3.0,
                    3.5,
                    &format!(
                        "{}  -  page {tag} (row {} of {rows}, column {} of {cols})",
                        d.title,
                        r + 1,
                        c + 1
                    ),
                );
                p.text(pw - m - 12.0, m - 6.0, 3.5, &tag);
                // Corner marks at the printable area.
                for (cx, cy) in [(m, m), (pw - m, m), (m, ph - m), (pw - m, ph - m)] {
                    p.line(DVec2::new(cx - 4.0, cy), DVec2::new(cx + 4.0, cy), 0.15);
                    p.line(DVec2::new(cx, cy - 4.0), DVec2::new(cx, cy + 4.0), 0.15);
                }
                // Clip to the printable area and shift the window into it.
                let _ = writeln!(p.ops, "q {m:.3} {m:.3} {aw:.3} {ah:.3} re W n");
                let _ = writeln!(p.ops, "1 0 0 1 {:.3} {:.3} cm", m - x0, m - y0);
                for s in &d.strokes {
                    p.stroke(s);
                }
                for l in &d.labels {
                    p.text(l.at.x, l.at.y, l.size_mm, &l.text);
                }
                let _ = writeln!(p.ops, "Q");
                pages.push(p);
            }
        }
    }
    write_pdf(&pages, pw, ph)
}

/// Serialise pages (content in mm) as a PDF document.
fn write_pdf(pages: &[Page], pw: f64, ph: f64) -> String {
    let mut out = String::new();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut String, body: String| {
        offsets.push(out.len());
        let n = offsets.len();
        let _ = write!(out, "{n} 0 obj\n{body}\nendobj\n");
    };
    out.push_str("%PDF-1.4\n");
    // 1: catalog, 2: pages, 3: font, then (page, content) pairs.
    let n = pages.len();
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    obj(&mut out, "<< /Type /Catalog /Pages 2 0 R >>".into());
    obj(
        &mut out,
        format!("<< /Type /Pages /Count {n} /Kids [{}] >>", kids.join(" ")),
    );
    obj(
        &mut out,
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
    );
    let (w, h) = (pw * PT_PER_MM, ph * PT_PER_MM);
    for (i, p) in pages.iter().enumerate() {
        let content = format!(
            "{PT_PER_MM:.6} 0 0 {PT_PER_MM:.6} 0 0 cm 0 g 0 G\n{}",
            p.ops
        );
        obj(
            &mut out,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
                5 + 2 * i
            ),
        );
        obj(
            &mut out,
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len() + 1
            ),
        );
    }
    let xref = out.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for o in &offsets {
        let _ = writeln!(tail, "{o:010} 00000 n ");
    }
    let _ = write!(
        tail,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        offsets.len() + 1
    );
    out.push_str(&tail);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::TrimStrip;

    fn strip(len: f64) -> TrimStrip {
        TrimStrip {
            edge: "neck".into(),
            edge_length_mm: len / 0.9,
            length_mm: len,
            width_mm: 12.0,
            panels: vec![0],
        }
    }

    #[test]
    fn writes_a_well_formed_pdf_with_tiles() {
        let d = trim_drawing("neck trim", &strip(500.0));
        let info = vec![
            "Cut each piece (carefully).".to_string(),
            "Glue – press × hold".to_string(),
        ];
        let pdf = pattern_pdf("Test", &info, &[d], &PrintOptions::default());
        let text = pdf;
        assert!(text.is_ascii());
        assert!(text.starts_with("%PDF-1.4") && text.ends_with("%%EOF\n"));
        // 1 info page + a 500 mm strip over 180 mm wide tiles (170 mm step): 3 columns.
        assert!(text.contains("/Count 4"), "{}", &text[..300]);
        assert!(text.contains("(Cut each piece \\(carefully\\).) Tj"));
        assert!(text.contains("(Glue - press x hold) Tj"));
        assert!(text.contains("page A3 \\(row 1 of 1, column 3 of 3\\)"));
        // The xref offsets point at the objects.
        let xref_at: usize = text
            .rsplit("startxref\n")
            .next()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(text[xref_at..].starts_with("xref"));
        for (i, line) in text[xref_at..].lines().skip(3).take(5).enumerate() {
            let off: usize = line[..10].parse().unwrap();
            assert!(
                text[off..].starts_with(&format!("{} 0 obj", i + 1)),
                "object {}",
                i + 1
            );
        }
        // Letter paper changes the media box.
        let letter = pattern_pdf(
            "Test",
            &[],
            &[trim_drawing("t", &strip(100.0))],
            &PrintOptions {
                paper: Paper::Letter,
                ..Default::default()
            },
        );
        assert!(letter.contains("/MediaBox [0 0 612.00 792.00]"));
    }

    #[test]
    fn wraps_text() {
        let lines = wrap(
            "one two three four five six seven eight nine ten",
            4.0,
            40.0,
        );
        assert!(lines.len() > 1 && lines.iter().all(|l| l.len() <= 20));
    }
}
