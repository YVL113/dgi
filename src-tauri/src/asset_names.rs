use quick_xml::{Reader, escape::unescape, events::Event};
use std::collections::BTreeMap;

const MAX_NAME_LENGTH: usize = 120;
const MAX_METADATA_BYTES: usize = 128 * 1024;
const MAX_COMMENTS: usize = 256;
const STRING_PROPERTY: u8 = 0x01;
const INT32_PROPERTY: u8 = 0x03;
const INT64_PROPERTY: u8 = 0x1b;

#[derive(Default)]
pub(crate) struct Identity {
    pub name: Option<String>,
    pub source_id: Option<String>,
}

fn meaningful_name(name: &str, class: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty()
        || name == class
        || name.chars().take(MAX_NAME_LENGTH + 1).count() > MAX_NAME_LENGTH
        || name.chars().any(char::is_control)
    {
        return None;
    }
    Some(name.to_owned())
}

fn source_id(id: i64) -> Option<String> {
    (id > 0).then(|| id.to_string())
}

#[derive(Default)]
pub(crate) struct BinaryNames {
    classes: BTreeMap<u32, ClassNames>,
    root: Option<i32>,
}

struct ClassNames {
    class: String,
    referents: Vec<i32>,
    names: Option<Vec<u8>>,
    ids: Option<(u8, Vec<u8>)>,
}

impl BinaryNames {
    pub fn read_class(&mut self, mut bytes: &[u8]) -> Option<()> {
        let key = pop_u32(&mut bytes)?;
        let class = std::str::from_utf8(pop_length_prefixed(&mut bytes)?).ok()?;
        bytes = bytes.get(1..)?;
        let count = pop_u32(&mut bytes)? as usize;
        let referents = read_referents(take(&mut bytes, count.checked_mul(4)?)?, count)?;
        self.classes.insert(
            key,
            ClassNames {
                class: class.to_owned(),
                referents,
                names: None,
                ids: None,
            },
        );
        Some(())
    }

    pub fn read_property(&mut self, mut bytes: &[u8]) -> Option<()> {
        let class = self.classes.get_mut(&pop_u32(&mut bytes)?)?;
        let name = pop_length_prefixed(&mut bytes)?;
        let (&kind, bytes) = bytes.split_first()?;
        match (name, kind) {
            (b"Name", STRING_PROPERTY) => class.names = Some(bytes.to_vec()),
            (b"SourceAssetId", INT64_PROPERTY | INT32_PROPERTY) => {
                class.ids = Some((kind, bytes.to_vec()));
            }
            _ => {}
        }
        Some(())
    }

    pub fn read_parents(&mut self, mut bytes: &[u8], instance_count: usize) -> Option<()> {
        self.root = None;
        let (&version, rest) = bytes.split_first()?;
        bytes = rest;
        let count = pop_u32(&mut bytes)? as usize;
        if version != 0 || count != instance_count {
            return None;
        }
        let children = read_referents(take(&mut bytes, count.checked_mul(4)?)?, count)?;
        let parents = read_referents(bytes, count)?;
        let mut roots = children
            .into_iter()
            .zip(parents)
            .filter_map(|(child, parent)| (parent == -1).then_some(child));
        let root = roots.next()?;
        if roots.next().is_none() {
            self.root = Some(root);
        }
        Some(())
    }

    pub fn identity(&self) -> Identity {
        let Some(root) = self.root else {
            return Identity::default();
        };
        let mut instances = self.classes.values().filter_map(|class| {
            let index = class
                .referents
                .iter()
                .position(|referent| *referent == root)?;
            Some((class, index))
        });
        let Some((class, index)) = instances.next() else {
            return Identity::default();
        };
        if instances.next().is_some() {
            return Identity::default();
        }
        let name = class.names.as_deref().and_then(|mut bytes| {
            for _ in 0..index {
                pop_length_prefixed(&mut bytes)?;
            }
            let name = std::str::from_utf8(pop_length_prefixed(&mut bytes)?).ok()?;
            meaningful_name(name, &class.class)
        });
        let source_id = class.ids.as_ref().and_then(|(kind, bytes)| {
            let count = class.referents.len();
            let id = if *kind == INT64_PROPERTY {
                let wire = u64::from_be_bytes(interleaved(bytes, count, index)?);
                ((wire >> 1) as i64) ^ -((wire & 1) as i64)
            } else {
                let wire = u32::from_be_bytes(interleaved(bytes, count, index)?);
                i64::from(unzigzag_i32(wire))
            };
            source_id(id)
        });
        Identity { name, source_id }
    }
}

fn take<'a>(input: &mut &'a [u8], length: usize) -> Option<&'a [u8]> {
    let (value, rest) = input.split_at_checked(length)?;
    *input = rest;
    Some(value)
}

fn pop_length_prefixed<'a>(input: &mut &'a [u8]) -> Option<&'a [u8]> {
    let length = pop_u32(input)? as usize;
    take(input, length)
}

fn pop_u32(input: &mut &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(take(input, 4)?.try_into().ok()?))
}

fn interleaved<const N: usize>(bytes: &[u8], count: usize, index: usize) -> Option<[u8; N]> {
    if bytes.len() != count.checked_mul(N)? || index >= count {
        return None;
    }
    Some(std::array::from_fn(|column| bytes[column * count + index]))
}

fn unzigzag_i32(value: u32) -> i32 {
    ((value >> 1) as i32) ^ -((value & 1) as i32)
}

fn read_referents(bytes: &[u8], count: usize) -> Option<Vec<i32>> {
    let mut last = 0_i32;
    (0..count)
        .map(|index| {
            let next = unzigzag_i32(u32::from_be_bytes(interleaved(bytes, count, index)?));
            last = last.checked_add(next)?;
            Some(last)
        })
        .collect()
}

pub(crate) fn xml_identity(bytes: &[u8]) -> Option<Identity> {
    let mut reader = Reader::from_reader(bytes);
    let mut depth = 0_usize;
    let mut class = None;
    let mut properties = false;
    let mut identity = Identity::default();
    loop {
        match reader.read_event().ok()? {
            Event::Start(tag) => {
                match (depth, tag.local_name().as_ref()) {
                    (0, "roblox") => {}
                    (0, _) => return None,
                    (1, "Item") => {
                        if class.is_some() {
                            return None;
                        }
                        class = Some(tag.try_get_attribute("class").ok()??.value.into_owned());
                    }
                    (2, "Properties") if class.is_some() => properties = true,
                    (3, "string" | "int64" | "int") if properties => {
                        let label = tag.try_get_attribute("name").ok()?;
                        let name = label.as_ref().map(|attr| attr.value.as_ref());
                        let tag_name = tag.local_name();
                        if matches!(
                            (tag_name.as_ref(), name),
                            ("string", Some("Name")) | ("int64" | "int", Some("SourceAssetId"))
                        ) {
                            let raw = reader.read_text(tag.name()).ok()?;
                            if raw.len() <= MAX_NAME_LENGTH * 6 && !raw.contains('<') {
                                let text = unescape(&raw).ok()?;
                                if name == Some("Name") {
                                    identity.name = meaningful_name(&text, class.as_deref()?);
                                } else {
                                    identity.source_id =
                                        text.trim().parse().ok().and_then(source_id);
                                }
                            }
                            continue;
                        }
                    }
                    _ => {}
                }
                depth += 1;
            }
            Event::End(tag) => {
                depth = depth.checked_sub(1)?;
                if depth == 2 && tag.local_name().as_ref() == "Properties" {
                    properties = false;
                }
            }
            Event::Empty(tag) if depth == 1 && tag.local_name().as_ref() == "Item" => return None,
            Event::DocType(_) => return None,
            Event::Eof => return (depth == 0 && class.is_some()).then_some(identity),
            _ => {}
        }
    }
}

pub(crate) fn media_name(bytes: &[u8], mime: &str) -> Option<String> {
    match mime {
        "audio/ogg" => ogg_title(bytes),
        "image/png" => png_title(bytes),
        _ => None,
    }
}

fn png_title(bytes: &[u8]) -> Option<String> {
    let mut bytes = bytes.get(8..bytes.len().min(MAX_METADATA_BYTES))?;
    while let Some(header) = take(&mut bytes, 8) {
        let length = u32::from_be_bytes(header[..4].try_into().ok()?) as usize;
        let value = take(&mut bytes, length)?;
        take(&mut bytes, 4)?; // CRC
        match &header[4..] {
            b"tEXt" => {
                if let Some(title) = value.strip_prefix(b"Title\0")
                    && title.len() <= MAX_NAME_LENGTH
                {
                    let title: String = title.iter().copied().map(char::from).collect();
                    if let Some(name) = meaningful_name(&title, "") {
                        return Some(name);
                    }
                }
            }
            b"iTXt" => {
                if let Some(value) = value.strip_prefix(b"Title\0\0\0") {
                    let mut fields = value.splitn(3, |byte| *byte == 0);
                    fields.next()?; // language
                    fields.next()?; // translated keyword
                    let title = std::str::from_utf8(fields.next()?).ok()?;
                    if let Some(name) = meaningful_name(title, "") {
                        return Some(name);
                    }
                }
            }
            b"IEND" => break,
            _ => {}
        }
    }
    None
}

fn ogg_title(bytes: &[u8]) -> Option<String> {
    let mut bytes = &bytes[..bytes.len().min(MAX_METADATA_BYTES)];
    let mut packet = Vec::new();
    let mut packet_count = 0;
    let mut stream = None;
    while let Some(header) = take(&mut bytes, 27) {
        if !header.starts_with(b"OggS\0") {
            return None;
        }
        let flags = header[5];
        let serial = &header[14..18];
        if *stream.get_or_insert(serial) != serial || (flags & 1 != 0) == packet.is_empty() {
            return None;
        }
        let segments = take(&mut bytes, header[26] as usize)?;
        for &segment in segments {
            packet.extend_from_slice(take(&mut bytes, segment as usize)?);
            if segment == u8::MAX {
                continue;
            }
            packet_count += 1;
            match packet_count {
                1 if packet.starts_with(b"OpusHead") || packet.starts_with(b"\x01vorbis") => {
                    packet.clear()
                }
                2 => {
                    let mut comments = packet
                        .strip_prefix(b"OpusTags")
                        .or_else(|| packet.strip_prefix(b"\x03vorbis"))?;
                    pop_length_prefixed(&mut comments)?; // vendor
                    let count = pop_u32(&mut comments)? as usize;
                    if count > MAX_COMMENTS {
                        return None;
                    }
                    for _ in 0..count {
                        let entry = pop_length_prefixed(&mut comments)?;
                        if entry
                            .get(..6)
                            .is_some_and(|key| key.eq_ignore_ascii_case(b"TITLE="))
                        {
                            let title = std::str::from_utf8(&entry[6..]).ok()?;
                            if let Some(name) = meaningful_name(title, "") {
                                return Some(name);
                            }
                        }
                    }
                    return None;
                }
                _ => return None,
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_identity_uses_root_instance_instead_of_names_from_children() {
        let mut reader = BinaryNames::default();
        let mut class = vec![0; 4];
        class.extend_from_slice(&5_u32.to_le_bytes());
        class.extend_from_slice(b"Model");
        class.push(0);
        class.extend_from_slice(&2_u32.to_le_bytes());
        // Referents 0 and 1 store zig-zag-encoded deltas 0 and 2.
        class.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 2]);
        reader.read_class(&class).unwrap();
        let mut names = vec![0; 4];
        names.extend_from_slice(&4_u32.to_le_bytes());
        names.extend_from_slice(b"Name\x01");
        for name in [b"Child".as_slice(), b"Original name".as_slice()] {
            names.extend_from_slice(&(name.len() as u32).to_le_bytes());
            names.extend_from_slice(name);
        }
        reader.read_property(&names).unwrap();
        let mut parents = vec![0];
        parents.extend_from_slice(&2_u32.to_le_bytes());
        // Subjects [0, 1], parents [1, -1] so the second named instance is the root.
        parents.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 2]);
        parents.extend_from_slice(&[0, 0, 0, 0, 0, 0, 2, 3]);
        reader.read_parents(&parents, 2).unwrap();
        assert_eq!(reader.identity().name.as_deref(), Some("Original name"));
    }

    #[test]
    fn reads_vorbis_title_after_an_unrelated_short_comment() {
        let mut comment = b"\x03vorbis".to_vec();
        comment.extend_from_slice(&0_u32.to_le_bytes()); // vendor
        comment.extend_from_slice(&2_u32.to_le_bytes());
        for field in [b"X=A".as_slice(), b"Title=Original audio".as_slice()] {
            comment.extend_from_slice(&(field.len() as u32).to_le_bytes());
            comment.extend_from_slice(field);
        }
        let mut page = vec![0; 27];
        page[..5].copy_from_slice(b"OggS\0");
        page[26] = 2;
        page.extend_from_slice(&[7, comment.len() as u8]);
        page.extend_from_slice(b"\x01vorbis");
        page.extend_from_slice(&comment);
        assert_eq!(ogg_title(&page).as_deref(), Some("Original audio"));

        page.truncate(page.len() - 2);
        assert!(ogg_title(&page).is_none());
    }
}
