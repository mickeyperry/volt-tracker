//! 16-bit WAV writing, deliberately identical to `bufToWav` in beta2.html.
//!
//! The asymmetry in the last line is not a typo and must not be "fixed": the browser scales
//! negatives by 0x8000 and positives by 0x7FFF. Rounding it any other way puts a systematic
//! one-LSB offset between the two engines and the null test stops meaning anything.

use web_audio_api::AudioBuffer;

pub fn encode(buf: &AudioBuffer) -> Vec<u8> {
    encode_from(buf, 0)
}

/// The same, starting `skip` frames in.
///
/// A plugin that reports latency answers that much late, so an offline render of it is shifted by
/// exactly that many frames; trimming the head here is what puts its part back on the timeline the
/// caller scheduled against. `skip` past the end yields an empty (but still valid) file rather
/// than a panic.
pub fn encode_from(buf: &AudioBuffer, skip: usize) -> Vec<u8> {
    let nc = buf.number_of_channels();
    let len = buf.length().saturating_sub(skip);
    let sr = buf.sample_rate() as u32;
    let mut v = Vec::with_capacity(44 + len * nc * 2);

    let bytes = 44 + len * nc * 2;
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&((bytes - 8) as u32).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&(nc as u16).to_le_bytes());
    v.extend_from_slice(&sr.to_le_bytes());
    v.extend_from_slice(&(sr * nc as u32 * 2).to_le_bytes());
    v.extend_from_slice(&((nc * 2) as u16).to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&((len * nc * 2) as u32).to_le_bytes());

    let chans: Vec<&[f32]> = (0..nc).map(|c| buf.get_channel_data(c)).collect();
    for i in skip..skip + len {
        for ch in &chans {
            let s = ch[i].clamp(-1.0, 1.0);
            let q = if s < 0.0 { s * 32768.0 } else { s * 32767.0 };
            v.extend_from_slice(&(q as i16).to_le_bytes());
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};

    /// A plugin that reports latency answers late by exactly that much, and `encode_from` is what
    /// puts its part back on the caller's timeline. If the trim were off by a frame the plugin
    /// would sit a sample early or late against everything else in the render — audible as a
    /// comb filter the moment it is doubled by anything.
    #[test]
    fn encode_from_drops_exactly_the_frames_asked_for() {
        let ctx = OfflineAudioContext::new(1, 8, 44100.0);
        let mut buf = ctx.create_buffer(1, 8, 44100.0);
        buf.copy_to_channel(&[0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7], 0);

        let all = encode(&buf);
        let cut = encode_from(&buf, 3);
        // 44-byte header, then one i16 a frame
        assert_eq!(all.len(), 44 + 8 * 2);
        assert_eq!(cut.len(), 44 + 5 * 2, "five frames should survive a skip of three");

        let first = i16::from_le_bytes([cut[44], cut[45]]);
        let want = (0.3f32 * 32767.0) as i16;
        assert!((first - want).abs() <= 1, "first sample is frame 3, got {first} want {want}");

        // the declared sizes have to follow the trim, or the file lies about its own length
        let data_len = u32::from_le_bytes([cut[40], cut[41], cut[42], cut[43]]);
        assert_eq!(data_len as usize, 5 * 2);
        let riff = u32::from_le_bytes([cut[4], cut[5], cut[6], cut[7]]);
        assert_eq!(riff as usize, cut.len() - 8);
    }

    /// Skipping past the end is a valid, empty file — never a panic.
    #[test]
    fn a_skip_past_the_end_is_empty_not_a_crash() {
        let ctx = OfflineAudioContext::new(1, 4, 44100.0);
        let buf = ctx.create_buffer(1, 4, 44100.0);
        let out = encode_from(&buf, 99);
        assert_eq!(out.len(), 44);
    }
}
