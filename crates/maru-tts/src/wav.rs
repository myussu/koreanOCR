//! A WAV file's length, without a decoder: enough to report how long speech lasts and to
//! recognise a reply that is not audio at all.

/// What the header of a PCM WAV file says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub seconds: f32,
}

/// Parse a RIFF/WAVE file's `fmt ` and `data` chunks. `None` if it is not one.
///
/// A streaming server may write a data length of 0 or `u32::MAX` because it does not know the
/// length yet; the real length is then whatever follows.
pub fn info(bytes: &[u8]) -> Option<WavInfo> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let (mut pos, mut format) = (12usize, None);
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = pos + 8;
        match id {
            b"fmt " if body + 16 <= bytes.len() => {
                let channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().ok()?);
                let rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().ok()?);
                let bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().ok()?);
                format = Some((channels, rate, bits));
            }
            b"data" => {
                let (channels, sample_rate, bits) = format?;
                let available = bytes.len() - body;
                let len = if size == 0 || size > available { available } else { size };
                let frame = usize::from(channels) * usize::from(bits / 8);
                if frame == 0 || sample_rate == 0 {
                    return None;
                }
                return Some(WavInfo { sample_rate, channels, bits, seconds: (len / frame) as f32 / sample_rate as f32 });
            }
            _ => {}
        }
        pos = body + size + (size & 1);
    }
    None
}

/// A 16-bit mono WAV of `samples` (tests and the evaluation's silence between sentences).
pub fn encode_pcm16(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Join 16-bit mono WAVs of the same rate into one, with `gap_secs` of silence between them.
pub fn concat_pcm16(parts: &[Vec<u8>], gap_secs: f32) -> Option<Vec<u8>> {
    let mut rate = None;
    let mut samples: Vec<i16> = Vec::new();
    for part in parts {
        let i = info(part)?;
        if i.channels != 1 || i.bits != 16 || rate.is_some_and(|r| r != i.sample_rate) {
            return None;
        }
        rate = Some(i.sample_rate);
        if !samples.is_empty() {
            samples.extend(std::iter::repeat_n(0i16, (gap_secs * i.sample_rate as f32) as usize));
        }
        let data = part.windows(4).position(|w| w == b"data").map(|p| p + 8)?;
        samples.extend(part[data..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])));
    }
    Some(encode_pcm16(&samples, rate?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_length_of_what_it_wrote() {
        let wav = encode_pcm16(&vec![0; 24_000], 24_000);
        let i = info(&wav).unwrap();
        assert_eq!((i.sample_rate, i.channels, i.bits), (24_000, 1, 16));
        assert!((i.seconds - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_streaming_header_with_an_unknown_length_counts_what_follows() {
        let mut wav = encode_pcm16(&vec![1; 12_000], 24_000);
        let at = wav.windows(4).position(|w| w == b"data").unwrap() + 4;
        for len in [0u32, u32::MAX] {
            wav[at..at + 4].copy_from_slice(&len.to_le_bytes());
            assert!((info(&wav).unwrap().seconds - 0.5).abs() < 1e-4, "length field {len}");
        }
    }

    #[test]
    fn anything_else_is_not_a_wav() {
        assert!(info(b"").is_none());
        assert!(info(b"{\"error\":\"nope\"}").is_none());
        assert!(info(b"ID3\x03\x00\x00\x00\x00\x00\x00").is_none(), "an MP3 is not a WAV");
        assert!(info(&encode_pcm16(&[], 24_000)).unwrap().seconds == 0.0);
    }

    #[test]
    fn joins_with_silence_between() {
        let a = encode_pcm16(&vec![5; 2_400], 24_000);
        let b = encode_pcm16(&vec![7; 2_400], 24_000);
        let both = concat_pcm16(&[a, b], 0.5).unwrap();
        assert!((info(&both).unwrap().seconds - (0.1 + 0.5 + 0.1)).abs() < 1e-3);
        assert!(concat_pcm16(&[encode_pcm16(&[0], 24_000), encode_pcm16(&[0], 16_000)], 0.1).is_none(), "different rates cannot be joined");
    }
}
