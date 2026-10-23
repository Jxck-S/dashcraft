//! Text measurement with the bundled fonts: wrapping, ellipsis, units and
//! determinism.

use pbi_render::scene::Color;
use pbi_render::text::{
    BundledFace, ELLIPSIS, FontBook, LINE_HEIGHT_EM, MeasureOptions, TextStyle,
};

fn style(family: &str, pt: f64) -> TextStyle {
    TextStyle::new(family, pt, Color::BLACK)
}

#[test]
fn single_line_metrics() {
    let book = FontBook::shared();
    let s = style("Segoe UI", 12.0);
    let layout = book.measure("Target Attainment %", &s, None);
    assert_eq!(layout.lines.len(), 1);
    let line = &layout.lines[0];
    assert_eq!(line.text, "Target Attainment %");
    assert!(line.width > 90.0 && line.width < 160.0, "{}", line.width);
    assert_eq!(layout.width, line.width);
    assert!((layout.height - 16.0 * LINE_HEIGHT_EM).abs() < 1e-9);
    assert!((line.baseline - 16.0 * 2210.0 / 2048.0).abs() < 1e-9);
    assert!(!layout.truncated);
    // Trailing spaces do not count toward the width.
    assert!((book.text_width("Target Attainment %   ", &s) - line.width).abs() < 1e-6);
}

#[test]
fn widths_scale_with_size_and_weight() {
    let book = FontBook::shared();
    let text = "Total Sales by Category";
    let w10 = book.text_width(text, &style("Segoe UI", 10.0));
    let w20 = book.text_width(text, &style("Segoe UI", 20.0));
    assert!((w20 / w10 - 2.0).abs() < 0.02, "{w10} {w20}");
    let light = book.text_width(text, &style("Segoe UI Light", 12.0));
    let regular = book.text_width(text, &style("Segoe UI", 12.0));
    let semibold = book.text_width(text, &style("Segoe UI Semibold", 12.0));
    let bold = book.text_width(text, &style("Segoe UI", 12.0).bold());
    assert!(light <= regular && regular < semibold && semibold <= bold);
    assert_eq!(style("wf_segoe-ui_bold", 9.0).face(), BundledFace::Bold);
    // Unknown families fall back to Selawik Regular.
    let din = book.text_width(text, &style("DIN", 12.0));
    let unknown = book.text_width(text, &style("No Such Font", 12.0));
    assert_eq!(din, regular);
    assert_eq!(unknown, regular);
}

#[test]
fn wraps_at_words_then_glyphs() {
    let book = FontBook::shared();
    let s = style("Segoe UI", 10.0);
    let text = "Select one or more filters from the left panel to update the KPI cards";
    let layout = book.measure(text, &s, Some(120.0));
    assert!(layout.lines.len() >= 3, "{:?}", layout.lines);
    for line in &layout.lines {
        assert!(line.width <= 120.01, "{line:?}");
        assert!(!line.text.ends_with(' '));
    }
    let words: Vec<&str> = layout
        .lines
        .iter()
        .flat_map(|l| l.text.split_whitespace())
        .collect();
    assert_eq!(words, text.split_whitespace().collect::<Vec<_>>());
    let tops: Vec<f64> = layout.lines.iter().map(|l| l.top).collect();
    assert!(
        tops.windows(2)
            .all(|w| (w[1] - w[0] - layout.line_height).abs() < 1e-9)
    );

    let long_word = "Supercalifragilisticexpialidocious";
    let glyphs = book.measure(long_word, &s, Some(40.0));
    assert!(glyphs.lines.len() > 1);
    assert_eq!(
        glyphs
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<String>(),
        long_word
    );

    let hard = book.measure("first\nsecond", &s, None);
    let texts: Vec<&str> = hard.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["first", "second"]);
}

#[test]
fn ellipsis_truncates_to_width() {
    let book = FontBook::shared();
    let s = style("Segoe UI Semibold", 12.0);
    let title = "Products Never Sold Count";
    let full = book.text_width(title, &s);
    // A slicer header this long is cut with an ellipsis below ~210px of
    // slicer width.
    assert!(full > 150.0 && full < 210.0, "{full}");
    assert_eq!(book.ellipsize(title, &s, full + 1.0), title);
    let cut = book.ellipsize(title, &s, 120.0);
    assert!(cut.ends_with(ELLIPSIS) && cut.len() < title.len(), "{cut}");
    assert!(book.text_width(&cut, &s) <= 120.01);
    assert!(title.starts_with(cut.trim_end_matches(ELLIPSIS)));
    assert_eq!(book.ellipsize(title, &s, 1.0), "");

    let opts = MeasureOptions {
        max_width: Some(100.0),
        wrap: true,
        max_lines: Some(2),
        ellipsis: true,
    };
    let text = "Review the store table for manager, region and channel details.";
    let layout = book.measure_with(text, &style("Segoe UI", 10.0), &opts);
    assert_eq!(layout.lines.len(), 2);
    assert!(layout.truncated);
    assert!(
        layout.lines[1].text.ends_with(ELLIPSIS),
        "{:?}",
        layout.lines
    );
    assert!(layout.lines.iter().all(|l| l.width <= 100.01));

    let one_line = MeasureOptions {
        max_width: Some(80.0),
        wrap: false,
        max_lines: None,
        ellipsis: true,
    };
    let layout = book.measure_with(text, &style("Segoe UI", 10.0), &one_line);
    assert_eq!(layout.lines.len(), 1);
    assert!(layout.lines[0].text.ends_with(ELLIPSIS));
}

#[test]
fn measurement_is_deterministic() {
    let text = "Products Never Sold — 6 (45.3%) αβγ 漢字 🙂";
    let s = style("'Segoe UI Semibold', wf_segoe-ui_semibold, helvetica", 11.0);
    let a = FontBook::new().measure(text, &s, Some(150.0));
    let b = FontBook::new().measure(text, &s, Some(150.0));
    let c = FontBook::shared().measure(text, &s, Some(150.0));
    assert_eq!(a, b);
    assert_eq!(a, c);
    assert!(!a.lines.is_empty());
}

#[test]
fn odd_inputs_do_not_panic() {
    let book = FontBook::shared();
    for pt in [f64::NAN, f64::INFINITY, -3.0, 0.0, 1e-9, 4000.0] {
        let _ = book.measure("abc def", &style("Segoe UI", pt), Some(10.0));
    }
    for w in [f64::NAN, -5.0, 0.0, f64::INFINITY] {
        let _ = book.measure_with(
            "abc def",
            &style("Segoe UI", 10.0),
            &MeasureOptions {
                max_width: Some(w),
                wrap: true,
                max_lines: Some(0),
                ellipsis: true,
            },
        );
    }
    let huge = "x".repeat(50_000);
    let layout = book.measure(&huge, &style("Segoe UI", 10.0), None);
    assert_eq!(layout.lines.len(), 1);
    assert_eq!(layout.lines[0].text.len(), 20_000);
}
