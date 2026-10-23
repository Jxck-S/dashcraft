//! Dev tool: render report pages to PNG/SVG with their data.
//!
//! ```text
//! cargo run -p pbi-render --example render_page -- <file.pbip|.pbix> \
//!     [--blob-map <map.json>] [--report <substring>] \
//!     [--page <index|name>] [--out target/tmp/render] [--scale 2] [--svg]
//!
//! cargo run -p pbi-render --example render_page -- \
//!     reference/examples/acme-outfitters/AcmeOutfitters.pbip \
//!     --blob-map reference/examples/acme-outfitters/blob-map.json --report Stores
//! ```
//! Writes `<out>/<report>__<page>.png` (and `.svg` with `--svg`) for every
//! matching page. Output stays under `target/`: renders of your own reports
//! contain their data and must never be committed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use pbi_core::ReportSource;
use pbi_render::png::{PngOptions, render_png};
use pbi_render::query::{InteractionState, QueryEnv};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::theme::ThemeResolver;
use pbi_render::visuals::{PageRenderOptions, render_report_page};
use pbi_session::{FixedClock, RefreshOptions, Session, SessionOptions};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut path: Option<PathBuf> = None;
    let mut blob_map: Option<PathBuf> = None;
    let mut report_filter: Option<String> = None;
    let mut page_filter: Option<String> = None;
    let mut out = PathBuf::from("target/tmp/render");
    let mut scale = 2.0_f32;
    let mut svg = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--blob-map" => blob_map = args.next().map(PathBuf::from),
            "--report" => report_filter = args.next(),
            "--page" => page_filter = args.next(),
            "--out" => out = args.next().map(PathBuf::from).expect("--out needs a value"),
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(2.0),
            "--svg" => svg = true,
            _ => path = Some(PathBuf::from(a)),
        }
    }
    let path = path.expect("usage: render_page <file> [options]");
    std::fs::create_dir_all(&out).unwrap();

    let clock = {
        use chrono::TimeZone;
        FixedClock::utc(
            chrono::Utc
                .with_ymd_and_hms(2026, 10, 21, 14, 5, 0)
                .unwrap(),
        )
    };
    let session = Session::open(
        &path,
        SessionOptions {
            blob_map,
            clock: Some(Arc::new(clock)),
            ..SessionOptions::default()
        },
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let refresh = rt.block_on(session.refresh(RefreshOptions::all())).unwrap();
    if !refresh.is_ok() {
        eprintln!("warning: refresh had errors: {refresh:#?}");
    }

    let env = QueryEnv::from_session(&session);
    let doc = session.document();
    for (i, report) in doc.reports.iter().enumerate() {
        if let Some(f) = &report_filter
            && !report.name.to_lowercase().contains(&f.to_lowercase())
        {
            continue;
        }
        let theme = match session.opened().report_sources.get(i) {
            Some(ReportSource::Pbir { report: pbir, .. }) => ThemeResolver::from_pbir(pbir),
            _ => ThemeResolver::default(),
        };
        for (pi, page) in report.pages.iter().enumerate() {
            let page_label = page.display_name.clone();
            if let Some(f) = &page_filter
                && *f != pi.to_string()
                && !page_label.contains(f.as_str())
                && page.name != *f
            {
                continue;
            }
            let (state, state_warnings) = InteractionState::from_page(page);
            for w in &state_warnings {
                eprintln!("  slicer state: {w}");
            }
            let rendered = render_report_page(
                &env,
                report,
                page,
                &theme,
                &state,
                &PageRenderOptions::default(),
            );
            for (name, r) in &rendered.data {
                if let Err(e) = r {
                    eprintln!("  visual {name}: {e}");
                }
            }
            let stem = format!("{}__{}", sanitize(&report.name), sanitize(&page_label));
            let png = render_png(
                &rendered.scene,
                &PngOptions {
                    scale,
                    ..PngOptions::default()
                },
            )
            .unwrap();
            let file = out.join(format!("{stem}.png"));
            std::fs::write(&file, png).unwrap();
            println!("{}", file.display());
            if svg {
                let s = to_svg(&rendered.scene, &SvgOptions::default());
                std::fs::write(out.join(format!("{stem}.svg")), s).unwrap();
            }
        }
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
