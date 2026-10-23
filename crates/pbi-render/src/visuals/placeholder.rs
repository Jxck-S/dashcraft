//! Placeholder for unsupported visual types: a light box with the visual
//! type and its bound fields (`Role: field, field`), centred and
//! ellipsized; lines that do not fit are dropped. (owner: visuals-basics)

use pbi_model::Visual;

use crate::scene::{Color, Group, Paint, RectNode, Stroke, TextAnchor};

use super::VisualCtx;
use super::common::{ellipsize, text_node};

/// Fill of the placeholder box.
pub const PLACEHOLDER_FILL: Color = Color::rgb(0xF3, 0xF2, 0xF1);
/// Dashed outline of the placeholder box.
pub const PLACEHOLDER_OUTLINE: Color = Color::rgb(0xC8, 0xC6, 0xC4);

/// Bound fields per role, in first-seen order: `[(role, [display names])]`.
pub fn bound_fields(visual: &Visual) -> Vec<(String, Vec<String>)> {
    visual
        .roles()
        .into_iter()
        .map(|role| {
            let mut labels: Vec<String> = Vec::new();
            for p in visual.projections_for(role) {
                let l = p.label().to_owned();
                if !l.is_empty() && !labels.contains(&l) {
                    labels.push(l);
                }
            }
            (role.to_owned(), labels)
        })
        .filter(|(_, labels)| !labels.is_empty())
        .collect()
}

/// Placeholder content (see the module docs).
pub fn render(ctx: &VisualCtx<'_>) -> Option<Group> {
    let c = ctx.frame.content;
    if c.width <= 1.0 || c.height <= 1.0 {
        return None;
    }
    let mut g = Group::new().with_class("pbi-placeholder");
    g.push(
        RectNode::new(c.inset_uniform(0.5))
            .fill(Paint::solid(PLACEHOLDER_FILL))
            .stroke(Stroke::new(PLACEHOLDER_OUTLINE, 1.0).with_dash(vec![4.0, 3.0])),
    );
    let title_style = ctx.theme.text_class("semiboldLabel").with_color(
        ctx.theme
            .named_color("foregroundNeutralSecondary")
            .unwrap_or(Color::rgb(0x60, 0x5E, 0x5C)),
    );
    let field_style = ctx.theme.text_class("smallLightLabel");
    let visual_type = if ctx.visual.visual_type.is_empty() {
        "visual".to_owned()
    } else {
        ctx.visual.visual_type.clone()
    };
    let mut lines = vec![(visual_type, &title_style)];
    for (role, labels) in bound_fields(ctx.visual) {
        lines.push((format!("{role}: {}", labels.join(", ")), &field_style));
    }
    let width = (c.width - 8.0).max(0.0);
    let mut kept = Vec::new();
    let mut height = 0.0;
    for (text, style) in lines {
        let lh = style.line_height();
        if height + lh > c.height - 2.0 {
            break;
        }
        height += lh;
        kept.push((ellipsize(ctx.fonts, &text, style, width), style));
    }
    let (cx, cy) = c.center();
    let mut top = cy - height / 2.0;
    for (text, style) in kept {
        g.push(text_node(
            cx,
            top + style.ascent(),
            text,
            style,
            TextAnchor::Middle,
            None,
        ));
        top += style.line_height();
    }
    Some(g)
}
