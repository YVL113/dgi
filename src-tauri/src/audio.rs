const ID3_HEADER_BYTES: usize = 10;
const OGG_PAGE_HEADER_BYTES: usize = 27;
const MPEG1_LAYER3_BITRATES: [usize; 15] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];
const MPEG2_LAYER3_BITRATES: [usize; 15] =
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
const MPEG_SAMPLE_RATES: [usize; 3] = [44_100, 48_000, 32_000];

pub fn mime(bytes: &[u8]) -> Option<&'static str> {
    if is_ogg_audio(bytes) {
        return Some("audio/ogg");
    }
    if bytes.len() >= 44 && bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        return Some("audio/wav");
    }
    // MP3 and some FLAC files can begin with an ID3v2 tag.
    let frames = bytes.get(id3_header_size(bytes).unwrap_or_default()..)?;
    if frames.len() >= 42
        && frames.starts_with(b"fLaC")
        && frames[4] & 0x7f == 0
        && frames[5..8] == [0, 0, 34]
    {
        return Some("audio/flac");
    }
    let frame_size = mp3_frame_size(frames)?;
    let remainder = frames.get(frame_size..)?;
    if remainder.len() < 4 || remainder.starts_with(b"TAG") || mp3_frame_size(remainder).is_some() {
        return Some("audio/mpeg");
    }
    None
}

/// Offset past a valid ID3v2 tag, even when only its header was sampled.
pub fn id3_header_size(bytes: &[u8]) -> Option<usize> {
    let [b'I', b'D', b'3', version @ 2..=4, ..] = bytes.get(..ID3_HEADER_BYTES)? else {
        return None;
    };
    let size = bytes[6..10].iter().try_fold(0, |total, byte| {
        (*byte < 128).then_some((total << 7) | usize::from(*byte))
    })?;
    let footer = if *version == 4 && bytes[5] & 0x10 != 0 {
        ID3_HEADER_BYTES
    } else {
        0
    };
    Some(ID3_HEADER_BYTES + size + footer)
}

fn is_ogg_audio(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"OggS\0") {
        return false;
    }
    let Some(&segments) = bytes.get(OGG_PAGE_HEADER_BYTES - 1) else {
        return false;
    };
    let Some(packets) = bytes.get(OGG_PAGE_HEADER_BYTES + usize::from(segments)..) else {
        return false;
    };
    segments > 0
        && ((packets.len() >= 19 && packets.starts_with(b"OpusHead"))
            || (packets.len() >= 30 && packets.starts_with(b"\x01vorbis"))
            || packets.starts_with(b"\x7fFLAC\x01"))
}

fn mp3_frame_size(bytes: &[u8]) -> Option<usize> {
    let [0xff, flags, data, emphasis] = *bytes.first_chunk::<4>()? else {
        return None;
    };
    if flags & 0xe6 != 0xe2 || emphasis & 3 == 2 {
        return None;
    }
    let version = (flags >> 3) & 3;
    let (divisor, multiplier, bitrates) = match version {
        3 => (1, 144_000, MPEG1_LAYER3_BITRATES),
        2 => (2, 72_000, MPEG2_LAYER3_BITRATES),
        0 => (4, 72_000, MPEG2_LAYER3_BITRATES),
        _ => return None,
    };
    let bitrate = *bitrates.get(usize::from(data >> 4))?;
    if bitrate == 0 {
        return None;
    }
    let rate = MPEG_SAMPLE_RATES.get(usize::from((data >> 2) & 3))? / divisor;
    let padding = usize::from((data >> 1) & 1);
    Some(multiplier * bitrate / rate + padding)
}
