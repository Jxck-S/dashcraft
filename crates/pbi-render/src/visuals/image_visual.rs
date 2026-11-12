//! Image visuals (`image`): a report image (`general.imageUrl`, or
//! `image.sourceFile`; see [`crate::images`]) drawn in the content box with
//! `imageScaling.imageScalingType` (`Normal`, `Fit`, `Fill`; default `Fit`).
//! Web URLs (`image.sourceUrl`, never fetched), data-bound images and
//! images that are missing or refused draw the placeholder.
//! (owner: visuals-basics)

use crate::images::{ImageScaling, place};
use crate::scene::{Clip, Group};

use super::{VisualCtx, placeholder};

/// Renders the image (see the module docs).
pub fn render(ctx: &VisualCtx<'_>) -> Option<Group> {
    let area = ctx.frame.content;
    if area.is_empty() {
        return None;
    }
    let props = ctx.theme.visual(ctx.visual);
    let o = props.objects();
    let Some((image, scaling)) = o
        .image("general", "imageUrl")
        .or_else(|| o.image("image", "sourceFile"))
    else {
        return placeholder::render(ctx);
    };
    let scaling = o
        .text("imageScaling", "imageScalingType")
        .and_then(|s| ImageScaling::parse(&s))
        .or(scaling)
        .unwrap_or(ImageScaling::Fit);
    let mut g = Group::new()
        .with_class("pbi-image")
        .with_clip(Clip::rect(area));
    for node in place(&image, area, scaling) {
        g.push(node);
    }
    Some(g)
}
