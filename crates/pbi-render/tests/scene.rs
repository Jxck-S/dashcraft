//! Scene graph of a rendered page: hit regions and JSON round-trip.

mod layout_fixtures;

use layout_fixtures::{STORE_DETAIL, STORE_PERFORMANCE, stores_report};
use pbi_render::layout::{FrameOptions, LayoutOptions, PageLayout, render_page_frames};
use pbi_render::scene::{HitKind, Node, Rect, Scene};

fn close(a: &Rect, b: &Rect) -> bool {
    [
        (a.x, b.x),
        (a.y, b.y),
        (a.width, b.width),
        (a.height, b.height),
    ]
    .iter()
    .all(|(p, q)| (p - q).abs() < 1e-6)
}

#[test]
fn page_frames_expose_one_hit_region_per_visual() {
    let Some(fx) = stores_report() else { return };
    let page = fx.page(STORE_PERFORMANCE);
    let viewport = Some((2560.0, 1440.0));
    let scene = render_page_frames(
        &fx.report,
        page,
        &fx.theme,
        &FrameOptions {
            viewport,
            ..FrameOptions::default()
        },
    );
    let layout = PageLayout::compute_with(
        page,
        &fx.theme,
        &LayoutOptions::for_report(&fx.report, viewport),
    );
    let hits = scene.hit_regions();
    assert_eq!(hits.len(), 14);
    let mut shadowed = 0;
    for (hit, frame) in hits.iter().zip(&layout.visuals) {
        assert_eq!(hit.id.visual, frame.name);
        assert_eq!(hit.id.kind, HitKind::Visual);
        let mut want = layout.to_canvas(&frame.outer);
        // An outer drop shadow is part of the visual's group, so its hit
        // region also covers the shadow: the frame moved by the offset and
        // grown by the spread plus half the blur.
        if let Some(s) = frame.drop_shadow.as_ref().filter(|s| !s.inner) {
            let shadow = frame
                .outer
                .translate(s.offset.0, s.offset.1)
                .inset_uniform(-(s.spread + s.blur / 2.0));
            want = want.union(&layout.to_canvas(&shadow));
            shadowed += 1;
        }
        assert!(
            close(&hit.rect, &want),
            "{}: {:?} vs {want:?}",
            frame.name,
            hit.rect
        );
    }
    assert_eq!(shadowed, 1, "the KPI cards have a drop shadow");
    // The header shape (no chrome, at the bottom of the z-order) is
    // hit-testable too; the map placeholder is on top.
    let bottom = hits.first().expect("hits");
    assert_eq!(bottom.id.visual, "9b2792b8239b52dc5cbc");
    assert!(close(&bottom.rect, &Rect::new(0.0, 0.0, 2560.0, 128.0)));
    let top = hits.last().expect("hits");
    assert_eq!(top.id.visual, "a7199dc503d24da7b45e");
    assert!(close(&top.rect, &Rect::new(1944.0, 960.0, 568.0, 448.0)));

    // Path data serializes as SVG path text with 3 decimals, so the shadow
    // rings (offsets at 45°) round-trip to the same JSON, not the same
    // floats.
    let json = serde_json::to_string(&scene).expect("serialize");
    let back: Scene = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(serde_json::to_string(&back).expect("serialize"), json);
    assert_eq!(back.hit_regions().len(), hits.len());
    assert!(matches!(back.root.children.first(), Some(Node::Rect(_))));

    // Without drop shadows every coordinate round-trips exactly.
    let detail = fx.page(STORE_DETAIL);
    let scene = render_page_frames(
        &fx.report,
        detail,
        &fx.theme,
        &FrameOptions {
            viewport,
            ..FrameOptions::default()
        },
    );
    assert_eq!(scene.hit_regions().len(), detail.visuals.len());
    let json = serde_json::to_string(&scene).expect("serialize");
    let back: Scene = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, scene);
}
