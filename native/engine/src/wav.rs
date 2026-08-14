//! 16-bit WAV writing, deliberately identical to `bufToWav` in beta2.html.
//!
//! The asymmetry in the last line is not a typo and must not be "fixed": the browser scales
//! negatives by 0x8000 and positives by 0x7FFF. Rounding it any other way puts a systematic
//! one-LSB offset between the two engines and the null test stops meaning anything.

use web_audio_api::AudioBuffer;

pub fn encode(buf: &AudioBuffer) -> Vec<u8> {
    let nc = buf.number_of_channels();
    let len = buf.length();
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
    for i in 0..len {
        for ch in &chans {
            let s = ch[i].clamp(-1.0, 1.0);
            let q = if s < 0.0 { s * 32768.0 } else { s * 32767.0 };
            v.extend_from_slice(&(q as i16).to_le_bytes());
        }
    }
    v
}
