use std::io::Cursor;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::names::SAMPLE_NAMES;

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// One decoded recording. `data` is mono, or stereo interleaved left/right.
pub struct Pcm {
    pub name: String,
    pub sample_rate: f32,
    pub frames: usize,
    pub channels: u8,
    pub data: Vec<f32>,
}

impl Pcm {
    pub fn mono(name: impl Into<String>, sample_rate: f32, data: Vec<f32>) -> Self {
        let frames = data.len();
        Self {
            name: name.into(),
            sample_rate,
            frames,
            channels: 1,
            data,
        }
    }

    pub fn seconds(&self) -> f32 {
        if self.sample_rate <= 0.0 {
            0.0
        } else {
            self.frames as f32 / self.sample_rate
        }
    }

    /// Scale the peak to `target` so the eighteen recordings sit in a similar range.
    pub fn normalize(&mut self, target: f32) {
        let peak = self
            .data
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        if peak > 1.0e-8 {
            let gain = target / peak;
            for sample in &mut self.data {
                *sample *= gain;
            }
        }
    }

    pub fn read(&self, position: f64) -> (f32, f32) {
        if self.frames < 2 {
            return (0.0, 0.0);
        }
        let wrapped = position.rem_euclid(self.frames as f64);
        let index = wrapped.floor() as isize;
        let frac = (wrapped - index as f64) as f32;
        let y0 = self.frame(index - 1);
        let y1 = self.frame(index);
        let y2 = self.frame(index + 1);
        let y3 = self.frame(index + 2);
        (
            hermite(y0.0, y1.0, y2.0, y3.0, frac),
            hermite(y0.1, y1.1, y2.1, y3.1, frac),
        )
    }

    fn frame(&self, index: isize) -> (f32, f32) {
        let n = self.frames as isize;
        let mut i = index % n;
        if i < 0 {
            i += n;
        }
        let i = i as usize;
        if self.channels == 1 {
            let sample = self.data[i];
            (sample, sample)
        } else {
            (self.data[i * 2], self.data[i * 2 + 1])
        }
    }
}

fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c0 = y1;
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + c0
}

pub struct Bank {
    pub samples: Vec<Pcm>,
    pub errors: Vec<String>,
}

impl Bank {
    pub fn empty() -> Self {
        Self {
            samples: Vec::new(),
            errors: Vec::new(),
        }
    }
}

pub struct SampleInfo {
    pub name: String,
    pub seconds: f32,
}

#[derive(Clone, Default)]
pub struct BankSnapshot {
    pub infos: Vec<SampleInfo>,
    pub errors: Vec<String>,
}

impl Clone for SampleInfo {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            seconds: self.seconds,
        }
    }
}

impl Bank {
    pub fn snapshot(&self) -> BankSnapshot {
        BankSnapshot {
            infos: self
                .samples
                .iter()
                .map(|sample| SampleInfo {
                    name: sample.name.clone(),
                    seconds: sample.seconds(),
                })
                .collect(),
            errors: self.errors.clone(),
        }
    }
}

pub fn load_embedded() -> Bank {
    let mut samples = Vec::new();
    let mut errors = Vec::new();
    for asset in ASSETS {
        match decode(asset.name, asset.bytes) {
            Ok(mut pcm) => {
                pcm.normalize(0.89);
                samples.push(pcm);
            }
            Err(error) => errors.push(format!("{}: {error}", asset.name)),
        }
    }
    samples.sort_by_key(|sample| name_rank(&sample.name));
    Bank { samples, errors }
}

fn name_rank(name: &str) -> (usize, String) {
    match SAMPLE_NAMES.iter().position(|known| *known == name) {
        Some(index) => (index, String::new()),
        None => (SAMPLE_NAMES.len(), name.to_string()),
    }
}

pub fn decode(name: &str, bytes: &[u8]) -> Result<Pcm, String> {
    if bytes.is_empty() {
        return Err("file is empty".to_string());
    }
    let cursor = Cursor::new(bytes.to_vec());
    let source = MediaSourceStream::new(Box::new(cursor), Default::default());
    let hint = Hint::new();
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| format!("could not read audio ({error})"))?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| "no audio track".to_string())?;
    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| "missing sample rate".to_string())?;
    if sample_rate < 8_000 {
        return Err(format!("sample rate {sample_rate} is too low"));
    }

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| format!("could not decode ({error})"))?;

    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut channels = track
        .codec_params
        .channels
        .map(|channels| channels.count())
        .unwrap_or(0);

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymError::DecodeError(_)) => continue,
            Err(SymError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(_) => break,
        };
        let spec = *decoded.spec();
        if channels == 0 {
            channels = spec.channels.count();
        }
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        let packet_channels = spec.channels.count().max(1);
        for frame in buffer.samples().chunks(packet_channels) {
            let l = frame.first().copied().unwrap_or(0.0);
            let r = if packet_channels > 1 {
                frame.get(1).copied().unwrap_or(l)
            } else {
                l
            };
            left.push(l);
            right.push(r);
        }
    }

    if left.is_empty() {
        return Err("no audio frames".to_string());
    }

    let mono = channels <= 1;
    let frames = left.len();
    let data = if mono {
        left
    } else {
        let mut interleaved = Vec::with_capacity(frames * 2);
        for index in 0..frames {
            interleaved.push(left[index]);
            interleaved.push(right[index]);
        }
        interleaved
    };

    Ok(Pcm {
        name: name.to_string(),
        sample_rate: sample_rate as f32,
        frames,
        channels: if mono { 1 } else { 2 },
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_bytes(samples: &[f32], sample_rate: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let data_len = (samples.len() * 2) as u32;
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            let value = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn decodes_a_wav_roundtrip() {
        let sample_rate = 44_100;
        let mut source = Vec::new();
        for i in 0..(sample_rate / 4) {
            let t = i as f32 / sample_rate as f32;
            source.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.4);
        }
        let pcm = decode("tone", &wav_bytes(&source, sample_rate)).expect("decode");
        assert_eq!(pcm.channels, 1);
        assert!((pcm.sample_rate - sample_rate as f32).abs() < 0.1);
        assert!(pcm.frames > source.len() / 2);
        let mut dot = 0.0f32;
        let mut energy = 0.0f32;
        let n = source.len().min(pcm.frames);
        for i in 0..n {
            dot += source[i] * pcm.data[i];
            energy += pcm.data[i] * pcm.data[i];
        }
        assert!(dot > 0.0, "decoded audio should keep the sine's polarity");
        assert!(energy > 1.0, "decoded audio should not be silence");
    }

    #[test]
    fn embedded_bank_is_well_formed() {
        let bank = load_embedded();
        assert!(
            bank.errors.is_empty(),
            "embedded samples failed to decode: {:?}",
            bank.errors
        );
        assert_eq!(bank.samples.len(), SAMPLE_NAMES.len());
        for (sample, expected) in bank.samples.iter().zip(SAMPLE_NAMES) {
            assert_eq!(sample.name, *expected);
        }
        for sample in &bank.samples {
            assert!(sample.frames > 8, "{}", sample.name);
            assert!(sample.sample_rate >= 8_000.0, "{}", sample.name);
            let peak = sample
                .data
                .iter()
                .fold(0.0f32, |peak, value| peak.max(value.abs()));
            assert!(peak <= 0.92, "{} peak {peak}", sample.name);
            assert!(peak > 0.2, "{} was silent after normalize", sample.name);
        }
    }
}
