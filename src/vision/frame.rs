//! Turns raw camera buffers into RGB images, plus small image helpers.
//!
//! Cameras report their pixel format, but some report it wrong (a few Mac
//! cameras label UYVY frames as YUYV). So the decoder looks at the buffer
//! itself: JPEG data starts with FF D8, and packed YUV, NV12, RGB and gray
//! frames all have different sizes.

use std::io::Cursor;

#[derive(Clone)]
pub struct Rgb {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl Rgb {
    pub fn gray(&self) -> Vec<u8> {
        let mut g = Vec::with_capacity(self.w * self.h);
        for px in self.data.chunks_exact(3) {
            g.push(((px[0] as u32 * 77 + px[1] as u32 * 150 + px[2] as u32 * 29) >> 8) as u8);
        }
        g
    }

    pub fn mirror(&mut self) {
        let w = self.w;
        if w < 2 {
            return;
        }
        for row in self.data.chunks_exact_mut(w * 3) {
            for x in 0..w / 2 {
                let a = x * 3;
                let b = (w - 1 - x) * 3;
                for c in 0..3 {
                    row.swap(a + c, b + c);
                }
            }
        }
    }

    /// A smaller copy, keeping every `step` pixel. Used for map tiles.
    pub fn shrink(&self, step: usize) -> Rgb {
        let step = step.max(1);
        let ow = (self.w / step).max(1);
        let oh = (self.h / step).max(1);
        let mut data = Vec::with_capacity(ow * oh * 3);
        for y in 0..oh {
            for x in 0..ow {
                let i = ((y * step).min(self.h - 1) * self.w + (x * step).min(self.w - 1)) * 3;
                data.extend_from_slice(&self.data[i..i + 3]);
            }
        }
        Rgb { w: ow, h: oh, data }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    Mjpeg,
    Yuyv,
    Nv12,
    Rgb,
    Bgr,
    Gray,
    Unknown,
}

/// Decodes a camera buffer into RGB no wider than `max_w`.
pub fn decode(buf: &[u8], w: usize, h: usize, hint: Hint, max_w: usize) -> Option<Rgb> {
    if buf.len() >= 4 && buf[0] == 0xFF && buf[1] == 0xD8 {
        return decode_jpeg(buf, max_w);
    }
    if w == 0 || h == 0 {
        return None;
    }
    let n = w * h;
    let step = ((w + max_w.max(1) - 1) / max_w.max(1)).max(1);
    if buf.len() == n * 3 {
        return Some(sample_rgb(buf, w, h, step, hint == Hint::Bgr));
    }
    if hint != Hint::Nv12 && buf.len() >= n * 2 && buf.len() % h == 0 && buf.len() / h >= w * 2 {
        let stride = buf.len() / h;
        return Some(sample_422(buf, w, h, stride, step));
    }
    if buf.len() >= n + n / 2 {
        return Some(sample_nv12(buf, w, h, step));
    }
    if buf.len() >= n {
        return Some(sample_gray(buf, w, h, step));
    }
    None
}

fn decode_jpeg(buf: &[u8], max_w: usize) -> Option<Rgb> {
    let mut dec = jpeg_decoder::Decoder::new(Cursor::new(buf));
    dec.read_info().ok()?;
    let info = dec.info()?;
    let (ow, oh) = (info.width as usize, info.height as usize);
    let (mut w, mut h) = (ow, oh);
    if max_w > 0 && w > max_w {
        let tw = max_w.min(u16::MAX as usize) as u16;
        let th = ((max_w * oh) / ow.max(1)).clamp(1, u16::MAX as usize) as u16;
        if let Ok((sw, sh)) = dec.scale(tw, th) {
            w = sw as usize;
            h = sh as usize;
        }
    }
    let px = dec.decode().ok()?;
    let fmt = dec.info()?.pixel_format;
    let (w, h) = if px.len() == w * h * 3 || px.len() == w * h {
        (w, h)
    } else {
        let i2 = dec.info()?;
        (i2.width as usize, i2.height as usize)
    };
    match fmt {
        jpeg_decoder::PixelFormat::RGB24 if px.len() >= w * h * 3 => Some(Rgb {
            w,
            h,
            data: px[..w * h * 3].to_vec(),
        }),
        jpeg_decoder::PixelFormat::L8 if px.len() >= w * h => {
            let mut data = Vec::with_capacity(w * h * 3);
            for &v in &px[..w * h] {
                data.extend_from_slice(&[v, v, v]);
            }
            Some(Rgb { w, h, data })
        }
        _ => None,
    }
}

fn clamp8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

fn yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let c = y as i32 - 16;
    let d = u as i32 - 128;
    let e = v as i32 - 128;
    [
        clamp8((298 * c + 409 * e + 128) >> 8),
        clamp8((298 * c - 100 * d - 208 * e + 128) >> 8),
        clamp8((298 * c + 516 * d + 128) >> 8),
    ]
}

fn sample_rgb(buf: &[u8], w: usize, h: usize, step: usize, bgr: bool) -> Rgb {
    let ow = (w / step).max(1);
    let oh = (h / step).max(1);
    let mut data = Vec::with_capacity(ow * oh * 3);
    for oy in 0..oh {
        for ox in 0..ow {
            let i = ((oy * step) * w + ox * step) * 3;
            if bgr {
                data.extend_from_slice(&[buf[i + 2], buf[i + 1], buf[i]]);
            } else {
                data.extend_from_slice(&buf[i..i + 3]);
            }
        }
    }
    Rgb { w: ow, h: oh, data }
}

/// Packed 4:2:2. Works out whether luma sits on the even bytes (YUYV) or the
/// odd bytes (UYVY) by checking which ones vary more.
fn sample_422(buf: &[u8], w: usize, h: usize, stride: usize, step: usize) -> Rgb {
    let mut stats = [(0f64, 0f64, 0f64); 2];
    let mut i = 0;
    let limit = (stride * h).min(buf.len());
    while i + 1 < limit {
        for k in 0..2 {
            let v = buf[i + k] as f64;
            stats[k].0 += v;
            stats[k].1 += v * v;
            stats[k].2 += 1.0;
        }
        i += 194;
    }
    let var = |s: (f64, f64, f64)| {
        if s.2 < 1.0 {
            0.0
        } else {
            s.1 / s.2 - (s.0 / s.2) * (s.0 / s.2)
        }
    };
    let luma_even = var(stats[0]) >= var(stats[1]);
    let ow = (w / step).max(1);
    let oh = (h / step).max(1);
    let mut data = vec![0u8; ow * oh * 3];
    for oy in 0..oh {
        let row = &buf[(oy * step) * stride..];
        for ox in 0..ow {
            let x = ox * step;
            let pair = (x / 2) * 4;
            if pair + 3 >= row.len() {
                continue;
            }
            let (y, u, v) = if luma_even {
                (row[pair + (x & 1) * 2], row[pair + 1], row[pair + 3])
            } else {
                (row[pair + 1 + (x & 1) * 2], row[pair], row[pair + 2])
            };
            let px = yuv_to_rgb(y, u, v);
            let o = (oy * ow + ox) * 3;
            data[o..o + 3].copy_from_slice(&px);
        }
    }
    Rgb { w: ow, h: oh, data }
}

fn sample_nv12(buf: &[u8], w: usize, h: usize, step: usize) -> Rgb {
    let ow = (w / step).max(1);
    let oh = (h / step).max(1);
    let uv = &buf[w * h..];
    let mut data = vec![0u8; ow * oh * 3];
    for oy in 0..oh {
        let y0 = oy * step;
        for ox in 0..ow {
            let x0 = ox * step;
            let luma = buf[y0 * w + x0];
            let ui = (y0 / 2) * w + (x0 / 2) * 2;
            let (u, v) = if ui + 1 < uv.len() { (uv[ui], uv[ui + 1]) } else { (128, 128) };
            let px = yuv_to_rgb(luma, u, v);
            let o = (oy * ow + ox) * 3;
            data[o..o + 3].copy_from_slice(&px);
        }
    }
    Rgb { w: ow, h: oh, data }
}

fn sample_gray(buf: &[u8], w: usize, h: usize, step: usize) -> Rgb {
    let ow = (w / step).max(1);
    let oh = (h / step).max(1);
    let mut data = Vec::with_capacity(ow * oh * 3);
    for oy in 0..oh {
        for ox in 0..ow {
            let v = buf[(oy * step) * w + ox * step];
            data.extend_from_slice(&[v, v, v]);
        }
    }
    Rgb { w: ow, h: oh, data }
}

/// Bilinear resize of a gray image. This is the exact sampling the face
/// detector was validated with.
pub fn resize_bilinear(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw * dh];
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < sw * sh {
        return out;
    }
    let sx = sw as f32 / dw as f32;
    let sy = sh as f32 / dh as f32;
    let maxx = sw as i64 - 1;
    let maxy = sh as i64 - 1;
    for y in 0..dh {
        let fy = (y as f32 + 0.5) * sy - 0.5;
        let y0f = fy.floor();
        let ty = fy - y0f;
        let y0 = (y0f as i64).clamp(0, maxy) as usize;
        let y1 = (y0f as i64 + 1).clamp(0, maxy) as usize;
        for x in 0..dw {
            let fx = (x as f32 + 0.5) * sx - 0.5;
            let x0f = fx.floor();
            let tx = fx - x0f;
            let x0 = (x0f as i64).clamp(0, maxx) as usize;
            let x1 = (x0f as i64 + 1).clamp(0, maxx) as usize;
            let a = src[y0 * sw + x0] as f32 * (1.0 - tx) + src[y0 * sw + x1] as f32 * tx;
            let b = src[y1 * sw + x0] as f32 * (1.0 - tx) + src[y1 * sw + x1] as f32 * tx;
            out[y * dw + x] = (a * (1.0 - ty) + b * ty + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Area average downscale, used for the small motion image.
pub fn downscale_box(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw * dh];
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < sw * sh {
        return out;
    }
    for y in 0..dh {
        let y0 = y * sh / dh;
        let y1 = ((y + 1) * sh / dh).max(y0 + 1).min(sh);
        for x in 0..dw {
            let x0 = x * sw / dw;
            let x1 = ((x + 1) * sw / dw).max(x0 + 1).min(sw);
            let mut s = 0u32;
            let mut n = 0u32;
            for yy in y0..y1 {
                let row = yy * sw;
                for xx in x0..x1 {
                    s += src[row + xx] as u32;
                    n += 1;
                }
            }
            out[y * dw + x] = (s / n.max(1)) as u8;
        }
    }
    out
}

/// Samples the rectangle (in pixels) out of a gray image into `ow` by `oh`.
pub fn crop_resize(src: &[u8], sw: usize, sh: usize, x: f32, y: f32, w: f32, h: f32, ow: usize, oh: usize) -> Vec<u8> {
    let mut out = vec![0u8; ow * oh];
    if sw == 0 || sh == 0 || ow == 0 || oh == 0 || src.len() < sw * sh {
        return out;
    }
    let maxx = sw as f32 - 1.0;
    let maxy = sh as f32 - 1.0;
    for oy in 0..oh {
        let fy = (y + (oy as f32 + 0.5) * h / oh as f32 - 0.5).clamp(0.0, maxy);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let ty = fy - y0 as f32;
        for ox in 0..ow {
            let fx = (x + (ox as f32 + 0.5) * w / ow as f32 - 0.5).clamp(0.0, maxx);
            let x0 = fx.floor() as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let tx = fx - x0 as f32;
            let a = src[y0 * sw + x0] as f32 * (1.0 - tx) + src[y0 * sw + x1] as f32 * tx;
            let b = src[y1 * sw + x0] as f32 * (1.0 - tx) + src[y1 * sw + x1] as f32 * tx;
            out[oy * ow + ox] = (a * (1.0 - ty) + b * ty + 0.5) as u8;
        }
    }
    out
}

pub fn encode_jpeg(img: &Rgb, quality: u8) -> Option<Vec<u8>> {
    if img.w == 0 || img.h == 0 || img.w > u16::MAX as usize || img.h > u16::MAX as usize {
        return None;
    }
    let mut out = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut out, quality);
    encoder
        .encode(&img.data, img.w as u16, img.h as u16, jpeg_encoder::ColorType::Rgb)
        .ok()?;
    Some(out)
}

pub fn decode_jpeg_file(bytes: &[u8]) -> Option<Rgb> {
    decode_jpeg(bytes, 0)
}
