//! Save thumbnails (suggestion S-024): a small picture of the town saved next to each generation, so the
//! Saved Worlds list can show what a world looks like without loading it.
//!
//! The picture uses the same colours as the in-game view (`pg_ui_model::palette`). PNG is written with
//! stored (uncompressed) deflate blocks: the images are tiny, and this keeps the encoder a few dozen lines
//! with no dependency.

use pg_core::map::Tile;
use pg_core::world::WorldState;
use pg_ui_model::palette::{pawn_color, tile_color};

pub const MAX_WIDTH: u32 = 96;
pub const MAX_HEIGHT: u32 = 72;

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xEDB8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for x in bytes {
        a = (a + u32::from(*x)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// Encodes 8-bit RGB pixels (`w * h * 3` bytes, row-major) as a PNG.
pub fn encode_png(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w as usize * 3 + 1) * h as usize);
    for row in rgb.chunks(w as usize * 3) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    if blocks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(b) = blocks.next() {
        let last = blocks.peek().is_none();
        z.push(u8::from(last));
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A picture of the world's first map (with its pawns), at most [`MAX_WIDTH`] by [`MAX_HEIGHT`] pixels,
/// or `None` when the world has no map.
pub fn thumbnail_png(world: &WorldState) -> Option<Vec<u8>> {
    let (_, map) = world.maps.iter().next()?;
    let (mw, mh) = (map.width().max(1) as u32, map.height().max(1) as u32);
    // The largest whole picture that fits, keeping the map's proportions.
    let scale_num = (MAX_WIDTH * mh).min(MAX_HEIGHT * mw);
    let (w, h) = if mw <= MAX_WIDTH && mh <= MAX_HEIGHT {
        (mw, mh)
    } else {
        (
            (mw * scale_num / (mw * mh)).clamp(1, MAX_WIDTH),
            (mh * scale_num / (mw * mh)).clamp(1, MAX_HEIGHT),
        )
    };
    let mut rgb = vec![0u8; w as usize * h as usize * 3];
    for y in 0..h {
        for x in 0..w {
            let t = Tile::new((x * mw / w) as i32, (y * mh / h) as i32);
            let c = tile_color(map, t);
            let i = (y as usize * w as usize + x as usize) * 3;
            if let Some(px) = rgb.get_mut(i..i + 3) {
                px.copy_from_slice(&c);
            }
        }
    }
    for (id, pawn) in world.pawns.iter() {
        if pawn.position.map != map.id {
            continue;
        }
        let (px, py) = (
            u32::try_from(pawn.position.tile.x).unwrap_or(0) * w / mw,
            u32::try_from(pawn.position.tile.y).unwrap_or(0) * h / mh,
        );
        let i = (py.min(h - 1) as usize * w as usize + px.min(w - 1) as usize) * 3;
        if let Some(p) = rgb.get_mut(i..i + 3) {
            p.copy_from_slice(&pawn_color(id));
        }
    }
    Some(encode_png(w, h, &rgb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_core::map::MapKind;

    /// Reads the PNG back: checks every chunk's CRC and the zlib stream, returning (w, h, raw scanlines).
    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let mut at = 8;
        let (mut w, mut h) = (0, 0);
        let mut idat = Vec::new();
        let mut ended = false;
        while at < png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let kind = &png[at + 4..at + 8];
            let body = &png[at + 4..at + 8 + len];
            let crc = u32::from_be_bytes(png[at + 8 + len..at + 12 + len].try_into().unwrap());
            assert_eq!(
                crc,
                crc32(body),
                "chunk {:?}",
                String::from_utf8_lossy(kind)
            );
            match kind {
                b"IHDR" => {
                    w = u32::from_be_bytes(png[at + 8..at + 12].try_into().unwrap());
                    h = u32::from_be_bytes(png[at + 12..at + 16].try_into().unwrap());
                    assert_eq!(&png[at + 16..at + 21], &[8, 2, 0, 0, 0]);
                }
                b"IDAT" => idat.extend_from_slice(&png[at + 8..at + 8 + len]),
                b"IEND" => ended = true,
                _ => panic!("unexpected chunk"),
            }
            at += 12 + len;
        }
        assert!(ended);
        assert_eq!(&idat[..2], &[0x78, 0x01]);
        let mut raw = Vec::new();
        let mut i = 2;
        loop {
            let last = idat[i] & 1 == 1;
            assert_eq!(idat[i] & 6, 0, "stored blocks only");
            let len = u16::from_le_bytes([idat[i + 1], idat[i + 2]]);
            let nlen = u16::from_le_bytes([idat[i + 3], idat[i + 4]]);
            assert_eq!(len, !nlen);
            raw.extend_from_slice(&idat[i + 5..i + 5 + len as usize]);
            i += 5 + len as usize;
            if last {
                break;
            }
        }
        assert_eq!(
            u32::from_be_bytes(idat[i..i + 4].try_into().unwrap()),
            adler32(&raw)
        );
        assert_eq!(i + 4, idat.len());
        (w, h, raw)
    }

    #[test]
    fn the_encoder_writes_a_valid_png_that_decodes_to_the_same_pixels() {
        for (w, h) in [(1u32, 1u32), (3, 2), (96, 72), (300, 70)] {
            let rgb: Vec<u8> = (0..w * h * 3).map(|i| (i * 7 % 251) as u8).collect();
            let (dw, dh, raw) = decode(&encode_png(w, h, &rgb));
            assert_eq!((dw, dh), (w, h));
            let rows: Vec<u8> = raw
                .chunks(w as usize * 3 + 1)
                .flat_map(|r| r[1..].to_vec())
                .collect();
            assert_eq!(rows, rgb, "{w}x{h}");
        }
    }

    #[test]
    fn checksums_match_published_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn a_world_becomes_a_picture_that_fits_the_limits() {
        let mut w = WorldState::new("Thumb", "t");
        assert!(thumbnail_png(&w).is_none(), "no map, no picture");
        let m = w.create_map(MapKind::Overworld, 200, 100).unwrap();
        w.spawn_pawn("A", m, Tile::new(10, 10)).unwrap();
        let (tw, th, _) = decode(&thumbnail_png(&w).unwrap());
        assert!(tw <= MAX_WIDTH && th <= MAX_HEIGHT && tw >= 1 && th >= 1);
        assert!(
            (i64::from(tw) * 100 - i64::from(th) * 200).abs() < 200,
            "proportions are kept: {tw}x{th}"
        );
        let mut small = WorldState::new("Small", "t");
        small.create_map(MapKind::Overworld, 10, 6).unwrap();
        assert_eq!(
            (
                decode(&thumbnail_png(&small).unwrap()).0,
                decode(&thumbnail_png(&small).unwrap()).1
            ),
            (10, 6)
        );
    }
}
