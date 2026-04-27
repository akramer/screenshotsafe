use image::{Rgba, RgbaImage};
use imageproc::drawing;
use std::path::Path;

use crate::models::{Annotation, CropRect};

/// Render a screenshot with annotations burned in.
///
/// 1. Load the original image
/// 2. Apply crop rectangle (if any)
/// 3. Draw all annotations onto the cropped image
/// 4. Save to the output path
pub fn render_screenshot(
    original_path: &str,
    output_path: &str,
    annotations: &[Annotation],
    crop_rect: &Option<CropRect>,
) -> crate::Result<()> {
    let img = image::open(original_path)?;

    // Apply crop
    let img = if let Some(crop) = crop_rect {
        img.crop_imm(crop.x, crop.y, crop.w, crop.h)
    } else {
        img
    };

    let mut canvas = img.to_rgba8();

    // Draw each annotation
    for annotation in annotations {
        draw_annotation(&mut canvas, annotation, crop_rect);
    }

    // Ensure output directory exists
    if let Some(parent) = Path::new(output_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    canvas.save(output_path)?;
    Ok(())
}

/// Draw a single annotation onto the canvas.
/// Coordinates are in original image space; they get offset by the crop origin.
fn draw_annotation(canvas: &mut RgbaImage, annotation: &Annotation, crop: &Option<CropRect>) {
    let (ox, oy) = crop
        .as_ref()
        .map(|c| (c.x as f64, c.y as f64))
        .unwrap_or((0.0, 0.0));

    match annotation {
        Annotation::Redact { x, y, w, h } => {
            let rx = (*x - ox).max(0.0) as i32;
            let ry = (*y - oy).max(0.0) as i32;
            let rw = *w as i32;
            let rh = *h as i32;
            fill_rect(canvas, rx, ry, rw, rh, Rgba([0, 0, 0, 255]));
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
            let rx = (*x - ox).max(0.0) as i32;
            let ry = (*y - oy).max(0.0) as i32;
            let rw = *w as i32;
            let rh = *h as i32;
            let rgba = parse_color(color);

            if *filled {
                fill_rect(canvas, rx, ry, rw, rh, rgba);
            } else {
                draw_rect_outline(canvas, rx, ry, rw, rh, *stroke_width as i32, rgba);
            }
        }
        Annotation::Arrow {
            x1,
            y1,
            x2,
            y2,
            color,
            stroke_width,
        } => {
            let ax1 = (*x1 - ox) as f32;
            let ay1 = (*y1 - oy) as f32;
            let ax2 = (*x2 - ox) as f32;
            let ay2 = (*y2 - oy) as f32;
            let rgba = parse_color(color);
            let sw = *stroke_width as i32;

            // Draw line body
            draw_thick_line(canvas, ax1, ay1, ax2, ay2, sw, rgba);

            // Draw arrowhead
            draw_arrowhead(canvas, ax1, ay1, ax2, ay2, sw, rgba);
        }
        Annotation::Line {
            x1,
            y1,
            x2,
            y2,
            color,
            stroke_width,
        } => {
            let lx1 = (*x1 - ox) as f32;
            let ly1 = (*y1 - oy) as f32;
            let lx2 = (*x2 - ox) as f32;
            let ly2 = (*y2 - oy) as f32;
            let rgba = parse_color(color);
            draw_thick_line(canvas, lx1, ly1, lx2, ly2, *stroke_width as i32, rgba);
        }
        Annotation::Text {
            x,
            y,
            text: _,
            font_size: _,
            color,
        } => {
            // For v1, draw a simple colored marker rectangle where text would be.
            // Full text rendering with fonts will be added in a future iteration
            // when we integrate ab_glyph or embed a font file.
            let tx = (*x - ox).max(0.0) as i32;
            let ty = (*y - oy).max(0.0) as i32;
            let rgba = parse_color(color);
            // Draw a small indicator rectangle for text position
            fill_rect(canvas, tx, ty, 8, 8, rgba);
        }
    }
}

/// Parse a hex color string like "#ff0000" into an Rgba pixel.
fn parse_color(hex: &str) -> Rgba<u8> {
    let hex = hex.trim_start_matches('#');
    if hex.len() >= 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
        Rgba([r, g, b, 255])
    } else {
        Rgba([255, 0, 0, 255]) // default to red on parse failure
    }
}

/// Fill a rectangle on the canvas.
fn fill_rect(canvas: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, color: Rgba<u8>) {
    let (cw, ch) = canvas.dimensions();
    for py in y.max(0)..(y + h).min(ch as i32) {
        for px in x.max(0)..(x + w).min(cw as i32) {
            canvas.put_pixel(px as u32, py as u32, color);
        }
    }
}

/// Draw a rectangle outline with a given stroke width.
fn draw_rect_outline(
    canvas: &mut RgbaImage,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    stroke: i32,
    color: Rgba<u8>,
) {
    // Top edge
    fill_rect(canvas, x, y, w, stroke, color);
    // Bottom edge
    fill_rect(canvas, x, y + h - stroke, w, stroke, color);
    // Left edge
    fill_rect(canvas, x, y, stroke, h, color);
    // Right edge
    fill_rect(canvas, x + w - stroke, y, stroke, h, color);
}

/// Draw a thick line using multiple offset antialiased lines.
fn draw_thick_line(
    canvas: &mut RgbaImage,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    thickness: i32,
    color: Rgba<u8>,
) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.001 {
        return;
    }
    let nx = -dy / len;
    let ny = dx / len;

    let half = thickness as f32 / 2.0;
    for i in 0..thickness {
        let offset = -half + i as f32 + 0.5;
        let ox = nx * offset;
        let oy = ny * offset;
        drawing::draw_line_segment_mut(
            canvas,
            (x1 + ox, y1 + oy),
            (x2 + ox, y2 + oy),
            color,
        );
    }
}

/// Draw an arrowhead at the end of a line.
fn draw_arrowhead(
    canvas: &mut RgbaImage,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    stroke: i32,
    color: Rgba<u8>,
) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.001 {
        return;
    }

    let arrow_len = (stroke as f32 * 5.0).max(15.0);
    let arrow_angle = 0.5_f32; // ~28 degrees

    let ux = dx / len;
    let uy = dy / len;

    let cos_a = arrow_angle.cos();
    let sin_a = arrow_angle.sin();

    let p1x = x2 - arrow_len * (ux * cos_a - uy * sin_a);
    let p1y = y2 - arrow_len * (uy * cos_a + ux * sin_a);
    let p2x = x2 - arrow_len * (ux * cos_a + uy * sin_a);
    let p2y = y2 - arrow_len * (uy * cos_a - ux * sin_a);

    draw_thick_line(canvas, x2, y2, p1x, p1y, stroke, color);
    draw_thick_line(canvas, x2, y2, p2x, p2y, stroke, color);
}
