//! The demo camera: a drawn room with a doorway on the left, where someone
//! walks in, looks around and leaves again every 22 seconds. It goes through
//! the same motion, tracking and room map code as a real camera, so it's a
//! fair way to try RoomGuard without a webcam.

use super::frame::Rgb;
use super::pose::Person;

const W: usize = 640;
const H: usize = 480;

fn rect(img: &mut [u8], x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 3]) {
    let (x0, x1) = (x0.clamp(0, W as i32) as usize, x1.clamp(0, W as i32) as usize);
    let (y0, y1) = (y0.clamp(0, H as i32) as usize, y1.clamp(0, H as i32) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * W + x) * 3;
            img[i..i + 3].copy_from_slice(&c);
        }
    }
}

fn disc(img: &mut [u8], cx: f32, cy: f32, r: f32, c: [u8; 3]) {
    let (x0, x1) = ((cx - r).floor() as i32, (cx + r).ceil() as i32);
    let (y0, y1) = ((cy - r).floor() as i32, (cy + r).ceil() as i32);
    for y in y0.max(0)..y1.min(H as i32) {
        for x in x0.max(0)..x1.min(W as i32) {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            if dx * dx + dy * dy <= r * r {
                let i = (y as usize * W + x as usize) * 3;
                img[i..i + 3].copy_from_slice(&c);
            }
        }
    }
}

/// Where the visitor's feet are at time `t`, if they're in the room.
pub fn walker(t: f32) -> Option<(f32, f32)> {
    let s = t.rem_euclid(22.0);
    let (door, floor) = (125.0, 432.0);
    if s < 3.0 {
        None
    } else if s < 9.0 {
        let k = (s - 3.0) / 6.0;
        Some((door + k * 355.0, floor))
    } else if s < 12.0 {
        Some((480.0, floor))
    } else if s < 18.0 {
        let k = (s - 12.0) / 6.0;
        Some((480.0 - k * 355.0, floor))
    } else if s < 19.0 {
        Some((door, floor))
    } else {
        None
    }
}

/// The visitor as the body tracker would see them: a person box with a full
/// 17-joint walking skeleton matching the drawn figure.
pub fn people(t: f32) -> Vec<Person> {
    let Some((x, f)) = walker(t) else {
        return Vec::new();
    };
    let swing = (t * 7.0).sin() * 9.0;
    let (wf, hf) = (W as f32, H as f32);
    let j = |dx: f32, dy: f32| ((x + dx) / wf, (f + dy) / hf, 0.9f32);
    let kp = [
        j(0.0, -200.0),
        j(-6.0, -207.0),
        j(6.0, -207.0),
        j(-18.0, -204.0),
        j(18.0, -204.0),
        j(-24.0, -176.0),
        j(24.0, -176.0),
        j(-27.0, -142.0),
        j(27.0, -142.0),
        j(-27.0, -112.0),
        j(27.0, -112.0),
        j(-11.0, -90.0),
        j(11.0, -90.0),
        j(-9.0 + swing * 0.4, -46.0),
        j(9.0 - swing * 0.4, -46.0),
        j(-9.0 + swing * 0.75, -4.0),
        j(9.0 - swing * 0.75, -4.0),
    ];
    vec![Person {
        x0: (x - 31.0) / wf,
        y0: (f - 226.0) / hf,
        x1: (x + 31.0) / wf,
        y1: f / hf,
        score: 0.9,
        kp,
    }]
}

pub fn scene(t: f32) -> Rgb {
    let mut img = vec![0u8; W * H * 3];
    for y in 0..H {
        let k = y as f32 / H as f32;
        let c = [(204.0 - 26.0 * k) as u8, (197.0 - 26.0 * k) as u8, (184.0 - 26.0 * k) as u8];
        for x in 0..W {
            let i = (y * W + x) * 3;
            img[i..i + 3].copy_from_slice(&c);
        }
    }
    rect(&mut img, 0, 404, 640, 480, [122, 104, 86]);
    rect(&mut img, 0, 396, 640, 404, [232, 229, 221]);
    // Doorway, open onto a dim hall.
    rect(&mut img, 78, 146, 172, 404, [214, 209, 199]);
    rect(&mut img, 88, 156, 162, 404, [62, 58, 54]);
    // Window and desk.
    rect(&mut img, 468, 106, 604, 252, [222, 219, 212]);
    rect(&mut img, 476, 114, 596, 244, [168, 197, 222]);
    rect(&mut img, 300, 326, 446, 340, [112, 82, 60]);
    rect(&mut img, 306, 340, 314, 398, [92, 68, 50]);
    rect(&mut img, 432, 340, 440, 398, [92, 68, 50]);
    if let Some((x, feet)) = walker(t) {
        let swing = (t * 7.0).sin() * 9.0;
        let xi = x as i32;
        let fi = feet as i32;
        rect(&mut img, xi - 15 + swing as i32 / 2, fi - 92, xi - 3 + swing as i32, fi, [48, 52, 70]);
        rect(&mut img, xi + 3 - swing as i32, fi - 92, xi + 15 - swing as i32 / 2, fi, [48, 52, 70]);
        rect(&mut img, xi - 22, fi - 182, xi + 22, fi - 88, [72, 98, 140]);
        rect(&mut img, xi - 30, fi - 172, xi - 22, fi - 110, [72, 98, 140]);
        rect(&mut img, xi + 22, fi - 172, xi + 30, fi - 110, [72, 98, 140]);
        disc(&mut img, x, feet - 204.0, 20.0, [206, 170, 138]);
        disc(&mut img, x, feet - 214.0, 17.0, [52, 40, 34]);
    }
    // A little sensor noise, like a real webcam.
    let frame = (t * 15.0) as u32;
    for (i, v) in img.iter_mut().enumerate() {
        let mut h = (i as u32).wrapping_mul(2_654_435_761) ^ frame.wrapping_mul(40_503);
        h ^= h >> 15;
        h = h.wrapping_mul(2_246_822_519);
        h ^= h >> 13;
        let n = (h % 7) as i32 - 3;
        *v = (*v as i32 + n).clamp(0, 255) as u8;
    }
    Rgb { w: W, h: H, data: img }
}
