//! Developer aid: raw captures straight from the frame buffer (`--capture <file.png>`).
//!
//! The window loop asks the GPU painter for a copy of the finished frame (what is on screen, not a
//! screenshot of the desktop), and this module writes it as a PNG. The encoder is a few lines of stored
//! (uncompressed) deflate blocks so the app needs no image library; files are large but exact.

use std::path::{Path, PathBuf};

/// A capture to take: where to write it and after how many frames (so the screen has settled).
pub struct Capture {
    pub path: PathBuf,
    pub after_frames: u32,
}

/// Writes `rgba` (4 bytes a pixel, row by row from the top) as a PNG file.
pub fn write_png(path: &Path, width: usize, height: usize, rgba: &[u8]) -> Result<(), String> {
    let bytes = encode_png(width, height, rgba)?;
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(dir);
        }
    }
    std::fs::write(path, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))
}

pub fn encode_png(width: usize, height: usize, rgba: &[u8]) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || rgba.len() != width * height * 4 {
        return Err("the capture has no pixels or the wrong size".to_owned());
    }
    // Each row is a filter byte (0, none) and then the pixels.
    let mut raw = Vec::with_capacity(height * (width * 4 + 1));
    for row in rgba.chunks(width * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    // zlib: header, stored blocks of up to 65535 bytes, Adler-32.
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(b) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        let n = b.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bits, RGBA, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &x in data {
        crc ^= u32::from(x);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_match_their_published_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn a_png_has_the_signature_the_header_and_an_end_chunk() {
        let px = [
            255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 9, 9, 9, 255,
        ];
        let png = encode_png(2, 2, &px).unwrap_or_default();
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        assert!(encode_png(2, 2, &px[..8]).is_err());
        assert!(encode_png(0, 2, &[]).is_err());
    }

    #[test]
    fn large_images_are_split_into_blocks() {
        let px = vec![7u8; 300 * 300 * 4];
        let png = encode_png(300, 300, &px).unwrap_or_default();
        assert!(png.len() > 300 * 300 * 4);
    }
}
