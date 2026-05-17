use image::{GenericImageView, RgbaImage};
use std::path::Path;

use crate::models::{Annotation, CropRect};

/// Render a screenshot with annotations burned in.
///
/// 1. Load the original image
/// 2. Apply crop rectangle (if any)
/// 3. Build an SVG overlay from the annotations
/// 4. Render the SVG with resvg (proper antialiasing, text, filled arrowheads)
/// 5. Composite the SVG layer onto the cropped image
/// 6. Save to the output path
pub fn render_screenshot(
    original_path: &str,
    output_path: &str,
    annotations: &[Annotation],
    crop_rect: &Option<CropRect>,
    image_dpi: f64,
) -> crate::Result<()> {
    let img = image::open(original_path)?;

    // Apply crop
    let img = if let Some(crop) = crop_rect {
        img.crop_imm(crop.x, crop.y, crop.w, crop.h)
    } else {
        img
    };

    let (width, height) = img.dimensions();
    let mut canvas = img.to_rgba8();

    // If there are annotations, render them as SVG and composite
    if !annotations.is_empty() {
        let svg_str = build_svg(width, height, annotations, crop_rect, image_dpi);
        let overlay = render_svg_to_pixmap(&svg_str, width, height)?;
        composite_overlay(&mut canvas, &overlay);
    }

    // Ensure output directory exists
    if let Some(parent) = Path::new(output_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    canvas.save(output_path)?;
    Ok(())
}

/// Build an SVG document string from a list of annotations.
/// Coordinates are in original image space; they get offset by the crop origin.
fn build_svg(
    width: u32,
    height: u32,
    annotations: &[Annotation],
    crop: &Option<CropRect>,
    image_dpi: f64,
) -> String {
    let visual_scale = (image_dpi / 100.0).clamp(0.1, 10.0);
    let (ox, oy) = crop
        .as_ref()
        .map(|c| (c.x as f64, c.y as f64))
        .unwrap_or((0.0, 0.0));

    let mut elements = String::new();

    let shadow_offset = 2.0 * visual_scale;
    let shadow_blur = 2.0 * visual_scale;
    elements.push_str(&format!(
        "<defs>\n\
            <filter id=\"shadow\" x=\"-20%\" y=\"-20%\" width=\"140%\" height=\"140%\">\n\
                <feDropShadow dx=\"{shadow_offset}\" dy=\"{shadow_offset}\" stdDeviation=\"{shadow_blur}\" flood-color=\"#000000\" flood-opacity=\"0.3\"/>\n\
            </filter>\n\
        </defs>"
    ));

    for annotation in annotations {
        match annotation {
            Annotation::Redact { x, y, w, h } => {
                let rx = *x - ox;
                let ry = *y - oy;
                let fill = "#000000";
                elements.push_str(&format!(
                    r#"<rect x="{rx}" y="{ry}" width="{w}" height="{h}" fill="{fill}" filter="url(#shadow)" />"#,
                ));
            }
            Annotation::Rect {
                x,
                y,
                w,
                h,
                color,
                filled,
                stroke_width,
            } => {
                let render_stroke_width = stroke_width * visual_scale;
                let rx = *x - ox + render_stroke_width / 2.0;
                let ry = *y - oy + render_stroke_width / 2.0;
                let escaped_color = svg_escape_attr(color);
                let fill_val = if *filled { &escaped_color } else { "none" };
                elements.push_str(&format!(
                    r#"<rect x="{rx}" y="{ry}" width="{w}" height="{h}" fill="{fill_val}" stroke="{escaped_color}" stroke-width="{render_stroke_width}" filter="url(#shadow)" />"#,
                ));
            }
            Annotation::Line {
                x1,
                y1,
                x2,
                y2,
                color,
                stroke_width,
            } => {
                let render_stroke_width = stroke_width * visual_scale;
                let lx1 = *x1 - ox;
                let ly1 = *y1 - oy;
                let lx2 = *x2 - ox;
                let ly2 = *y2 - oy;
                let escaped_color = svg_escape_attr(color);
                elements.push_str(&format!(
                    r#"<line x1="{lx1}" y1="{ly1}" x2="{lx2}" y2="{ly2}" stroke="{escaped_color}" stroke-width="{render_stroke_width}" filter="url(#shadow)" />"#,
                ));
            }
            Annotation::Arrow {
                x1,
                y1,
                x2,
                y2,
                color,
                stroke_width,
            } => {
                let render_stroke_width = stroke_width * visual_scale;
                let ax1 = *x1 - ox;
                let ay1 = *y1 - oy;
                let ax2 = *x2 - ox;
                let ay2 = *y2 - oy;
                let escaped_color = svg_escape_attr(color);

                let dx = ax2 - ax1;
                let dy = ay2 - ay1;
                let len = (dx * dx + dy * dy).sqrt();

                if len > 0.001 {
                    let head_len = (*stroke_width * 5.0).max(15.0) * visual_scale;
                    let angle = dy.atan2(dx);
                    let spread = std::f64::consts::PI / 6.0; // 30 degrees

                    let p0x = ax2;
                    let p0y = ay2;
                    let p1x = ax2 - head_len * (angle - spread).cos();
                    let p1y = ay2 - head_len * (angle - spread).sin();
                    let p2x = ax2 - (head_len * 0.6) * angle.cos();
                    let p2y = ay2 - (head_len * 0.6) * angle.sin();
                    let p3x = ax2 - head_len * (angle + spread).cos();
                    let p3y = ay2 - head_len * (angle + spread).sin();

                    let line_end_dist = (head_len * 0.6).min(len);
                    let lx2 = ax2 - line_end_dist * angle.cos();
                    let ly2 = ay2 - line_end_dist * angle.sin();

                    // Draw arrow grouped with shadow filter
                    elements.push_str(&format!(
                        r#"<g filter="url(#shadow)">
                            <line x1="{ax1}" y1="{ay1}" x2="{lx2}" y2="{ly2}" stroke="{escaped_color}" stroke-width="{render_stroke_width}" />
                            <polygon points="{p0x},{p0y} {p1x},{p1y} {p2x},{p2y} {p3x},{p3y}" fill="{escaped_color}" stroke="{escaped_color}" stroke-width="1" stroke-linejoin="miter" />
                        </g>"#
                    ));
                }
            }
            Annotation::Text {
                x,
                y,
                text,
                font_size,
                color,
            } => {
                let render_font_size = font_size * visual_scale;
                let tx = *x - ox;
                let ty = *y - oy;
                let escaped_color = svg_escape_attr(color);

                // SVG <text> y is the baseline; offset by ~0.90em to approximate top-left origin
                // matching Fabric.js IText positioning (incorporating its 1.16 default line height padding).
                let baseline_y = ty + render_font_size * 0.90;

                let mut tspans = String::new();
                for (i, line) in text.split('\n').enumerate() {
                    let escaped_line = svg_escape_text(line);
                    let dy = if i == 0 { 0.0 } else { render_font_size * 1.16 };
                    tspans.push_str(&format!(
                        r#"<tspan x="{tx}" dy="{dy}">{escaped_line}</tspan>"#
                    ));
                }

                elements.push_str(&format!(
                    r#"<text x="{tx}" y="{baseline_y}" font-family="Liberation Sans, Arial, sans-serif" font-size="{render_font_size}" fill="{escaped_color}" filter="url(#shadow)">{tspans}</text>"#,
                ));
            }
        }
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{elements}</svg>"#,
    )
}

/// Render an SVG string into a tiny-skia Pixmap.
fn render_svg_to_pixmap(
    svg_str: &str,
    width: u32,
    height: u32,
) -> crate::Result<resvg::tiny_skia::Pixmap> {
    use resvg::tiny_skia;
    use resvg::usvg;
    use std::sync::Arc;

    // Set up font database for text rendering
    let mut fontdb = resvg::usvg::fontdb::Database::new();
    fontdb.load_system_fonts();

    // Parse SVG — in usvg 0.44, fontdb goes into Options
    let mut opt = usvg::Options::default();
    opt.fontdb = Arc::new(fontdb);
    let tree = usvg::Tree::from_str(svg_str, &opt)
        .map_err(|e| crate::AppError::Internal(format!("SVG parse error: {}", e)))?;

    // Create pixmap
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| crate::AppError::Internal("Failed to create pixmap".into()))?;

    // Render
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );

    Ok(pixmap)
}

/// Alpha-composite the SVG overlay onto the base RGBA image.
fn composite_overlay(canvas: &mut RgbaImage, overlay: &resvg::tiny_skia::Pixmap) {
    let (cw, ch) = canvas.dimensions();
    let ow = overlay.width();
    let oh = overlay.height();
    let overlay_data = overlay.data();

    for y in 0..ch.min(oh) {
        for x in 0..cw.min(ow) {
            let idx = (y * ow + x) as usize * 4;
            // tiny-skia uses premultiplied alpha
            let sa = overlay_data[idx + 3] as u32;
            if sa == 0 {
                continue;
            }

            let pixel = canvas.get_pixel(x, y);
            let [dr, dg, db, da] = pixel.0;

            if sa == 255 {
                // Fully opaque — unpremultiply and write directly
                canvas.put_pixel(
                    x,
                    y,
                    image::Rgba([
                        overlay_data[idx],
                        overlay_data[idx + 1],
                        overlay_data[idx + 2],
                        255,
                    ]),
                );
            } else {
                // Blend (source is premultiplied)
                let sr = overlay_data[idx] as u32;
                let sg = overlay_data[idx + 1] as u32;
                let sb = overlay_data[idx + 2] as u32;
                let inv_sa = 255 - sa;

                let r = sr + (dr as u32 * inv_sa / 255);
                let g = sg + (dg as u32 * inv_sa / 255);
                let b = sb + (db as u32 * inv_sa / 255);
                let a = sa + (da as u32 * inv_sa / 255);

                canvas.put_pixel(x, y, image::Rgba([r as u8, g as u8, b as u8, a as u8]));
            }
        }
    }
}

/// Escape special characters for SVG attribute values.
fn svg_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape special characters for SVG text content.
fn svg_escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
