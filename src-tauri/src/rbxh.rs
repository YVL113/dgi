//! Roblox HTTP cache records. Offsets after the URL are relative to its length.
const MAGIC: &[u8] = b"RBXH\x02\0\0\0";
const FIXED_SIZE: usize = 37;
const CHECKSUM_SEED: u32 = 0x00c4_e564;

pub struct Record<'a> {
    pub body: &'a [u8],
    metadata_offset: usize,
    body_offset: usize,
}

pub fn parse(bytes: &[u8], file_len: u64) -> Option<Record<'_>> {
    if !bytes.starts_with(MAGIC) {
        return None;
    }
    let url_len = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?) as usize;
    let metadata_offset = 13_usize.checked_add(url_len)?;
    let fields = bytes.get(metadata_offset..metadata_offset.checked_add(24)?)?;
    if u32::from_le_bytes(fields[..4].try_into().ok()?) != 200 {
        return None;
    }
    let headers_len = u32::from_le_bytes(fields[4..8].try_into().ok()?) as usize;
    let body_len = u32::from_le_bytes(fields[12..16].try_into().ok()?) as usize;
    let body_offset = FIXED_SIZE.checked_add(url_len)?.checked_add(headers_len)?;
    if body_offset.checked_add(body_len)? as u64 != file_len {
        return None;
    }
    Some(Record {
        body: bytes.get(body_offset..)?,
        metadata_offset,
        body_offset,
    })
}

pub fn replace_body(original: &[u8], body: &[u8]) -> Result<Vec<u8>, String> {
    let record =
        parse(original, original.len() as u64).ok_or("The cached HTTP response is invalid.")?;
    let body_len =
        u32::try_from(body.len()).map_err(|_| "Replacement exceeds the cache format limit.")?;
    let mut output = original[..record.body_offset].to_vec();
    let offset = record.metadata_offset;
    output[offset + 12..offset + 16].copy_from_slice(&body_len.to_le_bytes());
    output[offset + 16..offset + 20].copy_from_slice(&xxh32(body).to_le_bytes());
    output.extend_from_slice(body);
    Ok(output)
}

pub fn xxh32(bytes: &[u8]) -> u32 {
    const P1: u32 = 0x9e37_79b1;
    const P2: u32 = 0x85eb_ca77;
    const P3: u32 = 0xc2b2_ae3d;
    const P4: u32 = 0x27d4_eb2f;
    const P5: u32 = 0x1656_67b1;
    if bytes.is_empty() {
        return 0;
    }
    let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let mut pos = 0;
    let mut hash = if bytes.len() >= 16 {
        let mut lanes = [
            CHECKSUM_SEED.wrapping_add(P1).wrapping_add(P2),
            CHECKSUM_SEED.wrapping_add(P2),
            CHECKSUM_SEED,
            CHECKSUM_SEED.wrapping_sub(P1),
        ];
        while pos + 16 <= bytes.len() {
            for (index, lane) in lanes.iter_mut().enumerate() {
                *lane = lane
                    .wrapping_add(word(pos + index * 4).wrapping_mul(P2))
                    .rotate_left(13)
                    .wrapping_mul(P1);
            }
            pos += 16;
        }
        lanes[0]
            .rotate_left(1)
            .wrapping_add(lanes[1].rotate_left(7))
            .wrapping_add(lanes[2].rotate_left(12))
            .wrapping_add(lanes[3].rotate_left(18))
    } else {
        CHECKSUM_SEED.wrapping_add(P5)
    };
    hash = hash.wrapping_add(bytes.len() as u32);
    while pos + 4 <= bytes.len() {
        hash = hash
            .wrapping_add(word(pos).wrapping_mul(P3))
            .rotate_left(17)
            .wrapping_mul(P4);
        pos += 4;
    }
    while pos < bytes.len() {
        hash = hash
            .wrapping_add((bytes[pos] as u32).wrapping_mul(P5))
            .rotate_left(11)
            .wrapping_mul(P1);
        pos += 1;
    }
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(P2);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(P3);
    hash ^ (hash >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn record(body: &[u8], url: &[u8], headers: &[u8]) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&(url.len() as u32).to_le_bytes());
        bytes.extend_from_slice(url);
        bytes.push(0);
        for value in [
            200,
            headers.len() as u32,
            if headers.is_empty() {
                0
            } else {
                xxh32(headers)
            },
            body.len() as u32,
            xxh32(body),
            0,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(headers);
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn preserves_url_and_headers_and_updates_body_length_and_checksum() {
        let original = record(
            b"old payload",
            b"https://example.invalid/asset",
            b"roblox-assettypeid: 24\r\n",
        );
        let replacement = b"a different animation payload";
        let updated = replace_body(&original, replacement).unwrap();
        let parsed = parse(&updated, updated.len() as u64).unwrap();
        assert_eq!(parsed.body, replacement);
        let metadata = parsed.metadata_offset;
        assert_eq!(&updated[..metadata + 12], &original[..metadata + 12]);
        assert_eq!(
            &updated[metadata + 20..parsed.body_offset],
            &original[metadata + 20..parsed.body_offset]
        );
        assert_eq!(
            u32::from_le_bytes(updated[metadata + 16..metadata + 20].try_into().unwrap()),
            xxh32(replacement)
        );
        assert!(parse(&updated[..updated.len() - 1], (updated.len() - 1) as u64).is_none());
    }

    #[test]
    fn checksum_matches_independent_xxhash_vectors() {
        // Produced with Python xxhash.xxh32_intdigest(data, seed=0x00c4e564).
        assert_eq!(xxh32(b"abc"), 0xa9ad_ea63);
        assert_eq!(xxh32(b"0123456789abcdef0123456789abcdef"), 0x10e8_87ae);
    }
}
