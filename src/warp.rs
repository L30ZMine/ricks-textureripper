//! Perspective un-warp: given four free corners of a quad on a source image,
//! produce a flat rectangular texture by mapping each output pixel back through
//! a homography and bilinearly sampling the source.

use egui::Pos2;
use image::{Rgba, RgbaImage};

/// Maximum output dimension (keeps live recompute snappy).
const MAX_OUT: f32 = 2048.0;

/// Full-resolution output dimensions a quad would un-warp to (scale `1.0`). Used
/// so a low-res preview can be scaled back up to the same size, keeping the rip's
/// atlas footprint stable while a handle is dragged.
pub fn natural_size(corners: [Pos2; 4]) -> (u32, u32) {
    let dist = |a: Pos2, b: Pos2| ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
    let top = dist(corners[0], corners[1]);
    let bottom = dist(corners[3], corners[2]);
    let left = dist(corners[0], corners[3]);
    let right = dist(corners[1], corners[2]);
    let w = top.max(bottom).round().clamp(1.0, MAX_OUT) as u32;
    let h = left.max(right).round().clamp(1.0, MAX_OUT) as u32;
    (w, h)
}

/// Un-warps the quad `corners` (order: TL, TR, BR, BL, in image-local pixels)
/// from `src` into a flat RGBA image. Returns `None` if degenerate.
///
/// `scale` (0,1] reduces the output resolution — the heavy per-output-pixel
/// sampling — for cheap live previews while a handle is being dragged. Pass
/// `1.0` for a full-resolution result.
pub fn unwarp_quad(src: &RgbaImage, corners: [Pos2; 4], scale: f32) -> Option<RgbaImage> {
    let dist = |a: Pos2, b: Pos2| ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();

    let top = dist(corners[0], corners[1]);
    let bottom = dist(corners[3], corners[2]);
    let left = dist(corners[0], corners[3]);
    let right = dist(corners[1], corners[2]);

    let scale = scale.clamp(0.05, 1.0);
    let out_w = (top.max(bottom) * scale).round().clamp(1.0, MAX_OUT) as u32;
    let out_h = (left.max(right) * scale).round().clamp(1.0, MAX_OUT) as u32;
    if out_w == 0 || out_h == 0 {
        return None;
    }

    // Homography mapping output-rect corners -> source quad corners.
    let dst_out = [
        (0.0, 0.0),
        (out_w as f64, 0.0),
        (out_w as f64, out_h as f64),
        (0.0, out_h as f64),
    ];
    let src_pts = [
        (corners[0].x as f64, corners[0].y as f64),
        (corners[1].x as f64, corners[1].y as f64),
        (corners[2].x as f64, corners[2].y as f64),
        (corners[3].x as f64, corners[3].y as f64),
    ];
    let h = homography(dst_out, src_pts)?;

    let sw = src.width() as f64;
    let sh = src.height() as f64;
    let mut out = RgbaImage::new(out_w, out_h);

    // Per-output-row work (the hot loop): map each pixel back through the
    // homography and bilinearly sample the source. Pixels outside the source stay
    // transparent (the buffer is zero-initialised).
    let fill_row = |oy: u32, row: &mut [u8]| {
        let v = oy as f64 + 0.5;
        for ox in 0..out_w {
            let u = ox as f64 + 0.5;
            let den = h[6] * u + h[7] * v + h[8];
            if den.abs() < 1e-12 {
                continue;
            }
            let x = (h[0] * u + h[1] * v + h[2]) / den;
            let y = (h[3] * u + h[4] * v + h[5]) / den;
            if x < 0.0 || y < 0.0 || x > sw - 1.0 || y > sh - 1.0 {
                continue;
            }
            let off = ox as usize * 4;
            row[off..off + 4].copy_from_slice(&sample_bilinear(src, x, y).0);
        }
    };

    // The perspective warp is the single most expensive operation, so spread the
    // output rows across all CPU cores. Small outputs (e.g. a downscaled live
    // preview) stay single-threaded to avoid thread-spawn overhead.
    let stride = out_w as usize * 4;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    if threads <= 1 || out_h < 64 {
        for (oy, row) in out.chunks_mut(stride).enumerate() {
            fill_row(oy as u32, row);
        }
    } else {
        let rows_per = (out_h as usize).div_ceil(threads);
        std::thread::scope(|s| {
            for (band, chunk) in out.chunks_mut(rows_per * stride).enumerate() {
                let fill_row = &fill_row;
                let row0 = (band * rows_per) as u32;
                s.spawn(move || {
                    for (i, row) in chunk.chunks_mut(stride).enumerate() {
                        fill_row(row0 + i as u32, row);
                    }
                });
            }
        });
    }

    Some(out)
}

fn sample_bilinear(img: &RgbaImage, x: f64, y: f64) -> Rgba<u8> {
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(img.width() - 1);
    let y1 = (y0 + 1).min(img.height() - 1);
    let fx = (x - x0 as f64) as f32;
    let fy = (y - y0 as f64) as f32;

    let p00 = img.get_pixel(x0, y0).0;
    let p10 = img.get_pixel(x1, y0).0;
    let p01 = img.get_pixel(x0, y1).0;
    let p11 = img.get_pixel(x1, y1).0;

    let mut out = [0u8; 4];
    for c in 0..4 {
        let top = p00[c] as f32 * (1.0 - fx) + p10[c] as f32 * fx;
        let bot = p01[c] as f32 * (1.0 - fx) + p11[c] as f32 * fx;
        out[c] = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
    }
    Rgba(out)
}

/// Computes the 3x3 homography (row-major, h[8] == 1) mapping `from[i]` to
/// `to[i]` for four correspondences. Returns `None` if the system is singular.
fn homography(from: [(f64, f64); 4], to: [(f64, f64); 4]) -> Option<[f64; 9]> {
    let mut a = [[0.0f64; 8]; 8];
    let mut b = [0.0f64; 8];

    for i in 0..4 {
        let (u, v) = from[i];
        let (x, y) = to[i];
        a[2 * i] = [u, v, 1.0, 0.0, 0.0, 0.0, -u * x, -v * x];
        b[2 * i] = x;
        a[2 * i + 1] = [0.0, 0.0, 0.0, u, v, 1.0, -u * y, -v * y];
        b[2 * i + 1] = y;
    }

    let h = solve8(a, b)?;
    Some([h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], 1.0])
}

/// Solves an 8x8 linear system via Gauss-Jordan with partial pivoting.
fn solve8(mut a: [[f64; 8]; 8], mut b: [f64; 8]) -> Option<[f64; 8]> {
    const N: usize = 8;
    for col in 0..N {
        // Partial pivot.
        let mut piv = col;
        let mut best = a[col][col].abs();
        for r in (col + 1)..N {
            if a[r][col].abs() > best {
                best = a[r][col].abs();
                piv = r;
            }
        }
        if best < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);

        let d = a[col][col];
        for r in 0..N {
            if r == col {
                continue;
            }
            let f = a[r][col] / d;
            if f != 0.0 {
                for c in col..N {
                    a[r][c] -= f * a[col][c];
                }
                b[r] -= f * b[col];
            }
        }
    }

    let mut x = [0.0f64; 8];
    for i in 0..N {
        x[i] = b[i] / a[i][i];
    }
    Some(x)
}
