//! Reads the print resolution a raster file declares.

/// Dots per inch from the PNG `pHYs` chunk (unit metre) or the JPEG JFIF APP0
/// density (unit 1 = dpi, 2 = dpcm). Everything else gives `None`, as do zero
/// densities and unit-less (aspect ratio only) values.
pub fn read_dpi(bytes: &[u8]) -> Option<f64> {
    let dpi = if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        png_dpi(bytes)?
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg_dpi(bytes)?
    } else {
        return None;
    };
    (dpi.is_finite() && dpi > 0.0).then_some(dpi)
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn png_dpi(bytes: &[u8]) -> Option<f64> {
    let mut pos = 8;
    while pos + 8 <= bytes.len() {
        let len = be32(&bytes[pos..]) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let data = bytes.get(pos + 8..pos.checked_add(8)?.checked_add(len)?)?;
        match kind {
            b"pHYs" if len == 9 => {
                // Pixels per unit along x; the unit is the metre when 1.
                return (data[8] == 1).then(|| f64::from(be32(data)) * 0.0254);
            }
            // The chunk must come before the pixel data.
            b"IDAT" | b"IEND" => return None,
            _ => {}
        }
        pos += 12 + len;
    }
    None
}

fn jpeg_dpi(bytes: &[u8]) -> Option<f64> {
    let mut pos = 2;
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xff {
            return None;
        }
        let marker = bytes[pos + 1];
        if marker == 0xff {
            pos += 1; // fill byte
            continue;
        }
        // No length: standalone markers.
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            pos += 2;
            continue;
        }
        // Start of scan or end of image: no APP0 can follow.
        if marker == 0xda || marker == 0xd9 {
            return None;
        }
        let len = usize::from(u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]));
        if len < 2 {
            return None;
        }
        let data = bytes.get(pos + 4..pos + 2 + len)?;
        if marker == 0xe0 && data.len() >= 12 && data.starts_with(b"JFIF\0") {
            // version (2), units (1), Xdensity (2), Ydensity (2)
            let x = f64::from(u16::from_be_bytes([data[8], data[9]]));
            return match data[7] {
                1 => Some(x),
                2 => Some(x * 2.54),
                _ => None,
            };
        }
        pos += 2 + len;
    }
    None
}
