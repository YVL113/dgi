use image::{DynamicImage, codecs::dds::DdsDecoder};

pub const KTX2_MIME: &str = "image/ktx2";
const KTX2_MAGIC: &[u8] = b"\xABKTX 20\xBB\r\n\x1A\n";
const HEADER_SIZE: usize = 80;
const LEVEL_ENTRY_SIZE: usize = 24;
const MAX_DIMENSION: u32 = 8192;
const MAX_PREVIEW_PIXELS: u32 = 16_777_216;

struct Texture {
    width: u32,
    height: u32,
    levels: u32,
    compression: u32,
    fourcc: &'static [u8; 4],
    block_bytes: u32,
}

impl Texture {
    fn read(bytes: &[u8]) -> Option<Self> {
        if !bytes.starts_with(KTX2_MAGIC) || bytes.len() < HEADER_SIZE {
            return None;
        }
        let (fourcc, block_bytes) = match word(bytes, 12)? {
            131..=134 => (b"DXT1", 8),  // Vulkan BC1 UNORM/SRGB.
            135..=136 => (b"DXT3", 16), // BC2.
            137..=138 => (b"DXT5", 16), // BC3.
            _ => return None,
        };
        let width = word(bytes, 20)?;
        let height = word(bytes, 24)?;
        let levels = word(bytes, 40)?;
        let compression = word(bytes, 44)?;
        if !(1..=MAX_DIMENSION).contains(&width)
            || !(1..=MAX_DIMENSION).contains(&height)
            || !(1..=14).contains(&levels)
            || !matches!(compression, 0 | 2)
            || word(bytes, 28)? != 0
            || word(bytes, 32)? > 1
            || word(bytes, 36)? != 1
        {
            return None;
        }
        Some(Self {
            width,
            height,
            levels,
            compression,
            fourcc,
            block_bytes,
        })
    }
}

pub fn is_supported_ktx2(bytes: &[u8]) -> bool {
    Texture::read(bytes).is_some()
}

pub fn decode_ktx2(bytes: &[u8], thumbnail: bool) -> Result<DynamicImage, String> {
    let texture = Texture::read(bytes).ok_or("This texture uses an unsupported KTX2 format.")?;
    decode_level(bytes, &texture, thumbnail)
        .ok_or_else(|| "The cached KTX2 texture is invalid or too large to preview.".into())
}

fn decode_level(bytes: &[u8], texture: &Texture, thumbnail: bool) -> Option<DynamicImage> {
    let mut level = 0;
    let mut width = texture.width;
    let mut height = texture.height;
    while thumbnail && level + 1 < texture.levels && width.max(height) > 512 {
        level += 1;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    let padded_width = width.div_ceil(4) * 4;
    let padded_height = height.div_ceil(4) * 4;
    if padded_width.checked_mul(padded_height)? > MAX_PREVIEW_PIXELS {
        return None;
    }
    let expected = (padded_width / 4 * (padded_height / 4) * texture.block_bytes) as usize;
    let entry = HEADER_SIZE + level as usize * LEVEL_ENTRY_SIZE;
    let min_offset = HEADER_SIZE + texture.levels as usize * LEVEL_ENTRY_SIZE;
    let start = level_word(bytes, entry)?;
    let len = level_word(bytes, entry + 8)?;
    let expanded = level_word(bytes, entry + 16)?;
    if start < min_offset || expanded != expected {
        return None;
    }
    let data = bytes.get(start..start.checked_add(len)?)?;
    let decompressed;
    let blocks = if texture.compression == 2 {
        decompressed = zstd::bulk::decompress(data, expected).ok()?;
        &decompressed[..]
    } else {
        data
    };
    if blocks.len() != expected {
        return None;
    }

    // Reuse the image crate's BC decoder by passing it a one-level DDS stream.
    // DDS decodes whole 4x4 blocks, so crop back to the KTX2 image dimensions.
    let mut dds = [0; 128];
    dds[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [
        (4, 124_u32),
        (8, 0x0008_1007),
        (12, padded_height),
        (16, padded_width),
        (20, expected as u32),
        (76, 32),
        (80, 4),
        (108, 0x1000),
    ] {
        dds[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    dds[84..88].copy_from_slice(texture.fourcc);
    let decoder = DdsDecoder::new(std::io::Read::chain(dds.as_slice(), blocks)).ok()?;
    let image = DynamicImage::from_decoder(decoder).ok()?;
    Some(image.crop_imm(0, 0, width, height))
}

fn word(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn level_word(bytes: &[u8], at: usize) -> Option<usize> {
    let value = u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?);
    usize::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_zstd_bc1_and_rejects_truncated_mip() {
        let red_block = [0x00, 0xf8, 0, 0, 0, 0, 0, 0];
        let compressed = zstd::bulk::compress(&red_block, 1).unwrap();
        let mut bytes = vec![0; HEADER_SIZE + LEVEL_ENTRY_SIZE];
        bytes[..12].copy_from_slice(KTX2_MAGIC);
        for (offset, value) in [(12, 131_u32), (20, 3), (24, 2), (36, 1), (40, 1), (44, 2)] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (offset, value) in [(80, 104_u64), (88, compressed.len() as u64), (96, 8)] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&compressed);
        let image = decode_ktx2(&bytes, false).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (3, 2));
        assert_eq!(image.get_pixel(2, 1).0, [255, 0, 0, 255]);
        bytes.pop();
        assert!(decode_ktx2(&bytes, false).is_err());
    }
}
