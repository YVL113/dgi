use draco_core::{DataType, DecoderBuffer, GeometryAttributeType, Mesh, MeshDecoder};

const MAX_VERTICES: usize = 300_000;
const MAX_FACES: usize = 500_000;
const FLOAT3_BYTES: usize = 12;
const FACE_BYTES: usize = 12;
const FULL_VERTEX_BYTES: usize = 40;

struct Geometry {
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
}

pub fn preview(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let header_end = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or("The mesh has no version header.")?;
    let (version, rest) = bytes.split_at(header_end);
    let body = &rest[1..];
    let mut geometry = match version.trim_ascii_end() {
        b"version 1.00" => text_mesh(body, 0.5),
        b"version 1.01" => text_mesh(body, 1.0),
        b"version 2.00" | b"version 3.00" | b"version 3.01" | b"version 4.00" | b"version 4.01"
        | b"version 5.00" => fixed_mesh(body),
        b"version 6.00" | b"version 7.00" => chunked_mesh(body),
        _ => return Err("This Roblox mesh version is unsupported.".into()),
    }
    .ok_or("The Roblox mesh is incomplete, invalid, or too large to preview.")?;

    let vertices = geometry.positions.len() / 3;
    let faces = geometry.indices.len() / 3;
    if !valid_counts(vertices, faces)
        || geometry
            .positions
            .iter()
            .any(|value| !(value.abs() <= 1.0e9))
        || geometry
            .indices
            .iter()
            .any(|index| *index as usize >= vertices)
    {
        return Err("The Roblox mesh contains invalid geometry.".into());
    }
    if geometry.normals.len() != geometry.positions.len()
        || geometry.normals.chunks_exact(3).any(|normal| {
            let magnitude = normal.iter().map(|value| value * value).sum::<f32>();
            !magnitude.is_finite() || magnitude < 1.0e-10
        })
    {
        geometry.normals.clear();
    }

    // The WebView uploads these little-endian buffers directly to Three.js.
    let mut result = Vec::with_capacity(
        12 + 4 * (geometry.positions.len() + geometry.normals.len() + geometry.indices.len()),
    );
    result.extend((vertices as u32).to_le_bytes());
    result.extend((faces as u32).to_le_bytes());
    result.extend(u32::from(!geometry.normals.is_empty()).to_le_bytes());
    for value in geometry.positions.into_iter().chain(geometry.normals) {
        result.extend(value.to_le_bytes());
    }
    for index in geometry.indices {
        result.extend(index.to_le_bytes());
    }
    Ok(result)
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<usize> {
    Some(u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?) as usize)
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<usize> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as usize)
}

fn valid_counts(vertices: usize, faces: usize) -> bool {
    (3..=MAX_VERTICES).contains(&vertices) && (1..=MAX_FACES).contains(&faces)
}

fn text_mesh(body: &[u8], scale: f32) -> Option<Geometry> {
    let (count, data) = std::str::from_utf8(body).ok()?.split_once('\n')?;
    let faces: usize = count.trim().parse().ok()?;
    let vertices = faces.checked_mul(3)?;
    if !valid_counts(vertices, faces) {
        return None;
    }
    let mut values = data
        .split(['[', ']', ','])
        .map(str::trim)
        .filter(|part| !part.is_empty());
    let mut positions = Vec::with_capacity(vertices * 3);
    let mut normals = Vec::with_capacity(vertices * 3);
    for _ in 0..vertices {
        for _ in 0..3 {
            positions.push(values.next()?.parse::<f32>().ok()? * scale);
        }
        for _ in 0..3 {
            normals.push(values.next()?.parse().ok()?);
        }
        for _ in 0..3 {
            values.next()?.parse::<f32>().ok()?;
        } // UVs
    }
    values.next().is_none().then(|| Geometry {
        positions,
        normals,
        indices: (0..vertices as u32).collect(),
    })
}

fn fixed_mesh(bytes: &[u8]) -> Option<Geometry> {
    let header = u16_at(bytes, 0)?;
    let (stride, vertices, faces, lods, envelopes) = match header {
        12 if *bytes.get(3)? as usize == FACE_BYTES => (
            *bytes.get(2)? as usize,
            u32_at(bytes, 4)?,
            u32_at(bytes, 8)?,
            0,
            0,
        ),
        16 if *bytes.get(3)? as usize == FACE_BYTES && u16_at(bytes, 4)? == 4 => (
            *bytes.get(2)? as usize,
            u32_at(bytes, 8)?,
            u32_at(bytes, 12)?,
            u16_at(bytes, 6)?,
            0,
        ),
        24 | 32 => (
            FULL_VERTEX_BYTES,
            u32_at(bytes, 4)?,
            u32_at(bytes, 8)?,
            u16_at(bytes, 12)?,
            if u16_at(bytes, 14)? == 0 { 0 } else { 8 },
        ),
        _ => return None,
    };
    if !valid_counts(vertices, faces) || !matches!(stride, 36 | FULL_VERTEX_BYTES) {
        return None;
    }
    let face_start = header + vertices * (stride + envelopes);
    let lod_start = face_start + faces * FACE_BYTES;
    let mut geometry = raw_geometry(
        bytes.get(header..header + vertices * stride)?,
        stride,
        bytes.get(face_start..lod_start)?,
    );
    truncate_lods(&mut geometry, bytes.get(lod_start..lod_start + lods * 4)?);
    Some(geometry)
}

fn raw_geometry(vertices: &[u8], stride: usize, faces: &[u8]) -> Geometry {
    fn floats(bytes: &[u8]) -> impl Iterator<Item = f32> + '_ {
        bytes
            .chunks_exact(4)
            .map(|value| f32::from_le_bytes(value.try_into().unwrap()))
    }
    let positions = vertices
        .chunks_exact(stride)
        .flat_map(|vertex| floats(&vertex[..FLOAT3_BYTES]))
        .collect();
    let normals = vertices
        .chunks_exact(stride)
        .flat_map(|vertex| floats(&vertex[FLOAT3_BYTES..2 * FLOAT3_BYTES]))
        .collect();
    let indices = faces
        .chunks_exact(4)
        .map(|index| u32::from_le_bytes(index.try_into().unwrap()))
        .collect();
    Geometry {
        positions,
        normals,
        indices,
    }
}

fn chunked_mesh(mut body: &[u8]) -> Option<Geometry> {
    let mut geometry = None;
    let mut lods = None;
    while !body.is_empty() {
        let kind = body.get(..8)?;
        let revision = u32_at(body, 8)?;
        let size = u32_at(body, 12)?;
        let (chunk, next) = body.get(16..)?.split_at_checked(size)?;
        match (kind, revision) {
            (b"COREMESH", 1) if geometry.is_none() => {
                let count = u32_at(chunk, 0)?;
                if !(3..=MAX_VERTICES).contains(&count) {
                    return None;
                }
                let face_count_offset = 4 + count * FULL_VERTEX_BYTES;
                let faces = u32_at(chunk, face_count_offset)?;
                if !valid_counts(count, faces) {
                    return None;
                }
                let face_start = face_count_offset + 4;
                geometry = Some(raw_geometry(
                    chunk.get(4..face_count_offset)?,
                    FULL_VERTEX_BYTES,
                    chunk.get(face_start..face_start + faces * FACE_BYTES)?,
                ));
            }
            (b"COREMESH", 2) if geometry.is_none() => {
                let stream_size = u32_at(chunk, 0)?;
                let stream = chunk.get(4..)?;
                if stream.len() != stream_size {
                    return None;
                }
                geometry = Some(draco_mesh(stream)?);
            }
            (b"LODS\0\0\0\0", 1) => {
                if let Some(count) = u32_at(chunk, 3)
                    && let Some(offsets) = chunk.get(7..)
                    && offsets.len() / 4 == count
                {
                    lods = Some(offsets);
                }
            }
            _ => {}
        }
        body = next;
    }
    let mut geometry = geometry?;
    if let Some(offsets) = lods {
        truncate_lods(&mut geometry, offsets);
    }
    Some(geometry)
}

fn draco_mesh(stream: &[u8]) -> Option<Geometry> {
    if !stream.starts_with(b"DRACO") || stream.get(7) != Some(&1) {
        return None;
    }
    let mut mesh = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(stream), &mut mesh)
        .ok()?;
    let vertices = mesh.num_points();
    if !valid_counts(vertices, mesh.num_faces()) {
        return None;
    }
    let positions = mesh.named_attribute(GeometryAttributeType::Position)?;
    if positions.num_components() != 3 || positions.data_type() != DataType::Float32 {
        return None;
    }
    let positions = positions.read_f32s(vertices, 3);
    // Roblox's Draco streams store normals as the generic attribute with unique ID 1.
    let normals = mesh
        .named_attribute(GeometryAttributeType::Normal)
        .or_else(|| mesh.attribute_by_unique_id(1))
        .filter(|attribute| {
            attribute.num_components() == 3 && attribute.data_type() == DataType::Float32
        })
        .map_or_else(Vec::new, |attribute| attribute.read_f32s(vertices, 3));
    let indices = mesh
        .faces()
        .iter()
        .flat_map(|face| face.map(|point| point.0))
        .collect();
    Some(Geometry {
        positions,
        normals,
        indices,
    })
}

fn truncate_lods(geometry: &mut Geometry, offsets: &[u8]) {
    // LODs share one index buffer. Render the first level when the offsets form a partition.
    let count = geometry.indices.len() / 3;
    let mut previous = 0;
    if offsets.len() % 4 != 0 || u32_at(offsets, 0) != Some(0) {
        return;
    }
    for offset in (4..offsets.len()).step_by(4) {
        let next = u32_at(offsets, offset).unwrap();
        if next < previous || next > count {
            return;
        }
        previous = next;
    }
    if previous == count
        && let Some(end) = u32_at(offsets, 4)
        && end > 0
    {
        geometry.indices.truncate(end * 3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_legacy_text_and_rejects_incomplete_or_oversized_geometry() {
        let triangle =
            b"version 1.00\r\n1\r\n[0,0,0][0,0,1][0,0,0][2,0,0][0,0,1][1,0,0][0,2,0][0,0,1][0,1,0]";
        let result = preview(triangle).unwrap();
        assert_eq!(u32_at(&result, 0), Some(3));
        assert_eq!(u32_at(&result, 4), Some(1));
        assert_eq!(u32_at(&result, 8), Some(1));
        assert_eq!(u32_at(&result, 24), Some(1.0f32.to_bits() as usize));
        assert!(preview(b"version 1.00\n999999999\n").is_err());
        assert!(preview(b"version 7.00\nCOREMESH\x02\0\0\0\xff\xff\xff\xff").is_err());
    }
}
