//! A tiny PNG writer (stored, uncompressed deflate) for screenshot mode. The
//! screenshots workflow recompresses the files afterwards.

use std::path::Path;

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for n in 0..256u32 {
        let mut c = n;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        table[n as usize] = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut c = Vec::with_capacity(4 + data.len());
    c.extend_from_slice(kind);
    c.extend_from_slice(data);
    out.extend_from_slice(&c);
    out.extend_from_slice(&crc32(&c).to_be_bytes());
}

pub fn encode(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let stride = w as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * h as usize);
    for y in 0..h as usize {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * stride..(y + 1) * stride]);
    }
    let mut z = vec![0x78, 0x01];
    if raw.is_empty() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(b) = blocks.next() {
        let last = blocks.peek().is_none();
        z.push(last as u8);
        let len = b.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

pub fn write(path: &Path, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    if rgba.len() < w as usize * h as usize * 4 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the image is smaller than its size says"));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, encode(w, h, rgba))
}
