use crate::bank::Bank;

const MAX_VOICES: usize = 16;
const MAX_GRAINS: usize = 3;
const MAX_SAMPLES: usize = 32;

/// What the plugin asks the engine to do when a note starts.
pub struct NoteOn {
    pub note: u8,
    pub velocity: f32,
    pub channel: u8,
    pub voice_id: Option<i32>,
    /// Force a recording. `None` picks at random from the enabled set.
    pub sample: Option<usize>,
    /// Force a start point in `0..1`. `None` picks at random.
    pub start_norm: Option<f32>,
}

pub struct Triggered {
    pub sample: usize,
    pub start_norm: f32,
}

#[derive(Clone, Copy)]
struct Grain {
    on: bool,
    age: usize,
    origin: f64,
}

impl Grain {
    const fn off() -> Self {
        Self {
            on: false,
            age: 0,
            origin: 0.0,
        }
    }
}

#[derive(Clone, Copy)]
struct Voice {
    active: bool,
    releasing: bool,
    note: u8,
    channel: u8,
    voice_id: Option<i32>,
    sample: usize,
    playhead: f64,
    grains: [Grain; MAX_GRAINS],
    env: f32,
    env_target: f32,
    env_inc: f32,
    env_left: u32,
    tuning: f32,
    serial: u32,
    until_next: usize,
}

impl Voice {
    const fn inactive() -> Self {
        Self {
            active: false,
            releasing: false,
            note: 0,
            channel: 0,
            voice_id: None,
            sample: 0,
            playhead: 0.0,
            grains: [Grain::off(); MAX_GRAINS],
            env: 0.0,
            env_target: 0.0,
            env_inc: 0.0,
            env_left: 0,
            tuning: 0.0,
            serial: 0,
            until_next: 0,
        }
    }
}

struct Rng(u32);

impl Rng {
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        if x == 0 {
            x = 0xA3C5_9AC3;
        }
        self.0 = x;
        x
    }

    fn gen_range(&mut self, n: usize) -> usize {
        if n <= 1 {
            0
        } else {
            (self.next_u32() as usize) % n
        }
    }

    fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 16_777_216.0
    }
}

pub struct Engine {
    bank: Bank,
    voices: [Voice; MAX_VOICES],
    rng: Rng,
    sample_rate: f32,
    grain_len: usize,
    hop: usize,
    hann: Vec<f32>,
    next_serial: u32,
}

impl Engine {
    pub fn new(sample_rate: f32, bank: Bank, seed: u32) -> Self {
        let mut engine = Self {
            bank,
            voices: [Voice::inactive(); MAX_VOICES],
            rng: Rng(seed | 1),
            sample_rate: 1.0,
            grain_len: 2,
            hop: 1,
            hann: Vec::new(),
            next_serial: 1,
        };
        engine.set_sample_rate(sample_rate);
        engine
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sample_rate = if sample_rate.is_finite() && sample_rate > 1.0 {
            sample_rate
        } else {
            44_100.0
        };
        if (self.sample_rate - sample_rate).abs() < 0.1 && !self.hann.is_empty() {
            return;
        }
        self.sample_rate = sample_rate;
        self.grain_len = grain_len_for(sample_rate);
        self.hop = (self.grain_len / 2).max(1);
        self.hann = (0..self.grain_len)
            .map(|i| {
                let phase = i as f32 / self.grain_len as f32;
                0.5 * (1.0 - (std::f32::consts::TAU * phase).cos())
            })
            .collect();
        self.silence();
    }

    pub fn set_bank(&mut self, bank: Bank) {
        self.bank = bank;
        self.silence();
    }

    pub fn bank_len(&self) -> usize {
        self.bank.samples.len()
    }

    pub fn sample_name(&self, index: usize) -> &str {
        self.bank
            .samples
            .get(index)
            .map(|sample| sample.name.as_str())
            .unwrap_or("")
    }

    pub fn hop_len(&self) -> usize {
        self.hop
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn any_active(&self) -> bool {
        self.voices.iter().any(|voice| voice.active)
    }

    pub fn silence(&mut self) {
        self.voices = [Voice::inactive(); MAX_VOICES];
    }

    pub fn note_on(
        &mut self,
        request: NoteOn,
        enabled: &[bool],
        attack_samples: f32,
    ) -> Option<Triggered> {
        let velocity = request.velocity.clamp(0.0, 1.0);
        if velocity <= 0.0 {
            return None;
        }
        let sample_index = self.choose_sample(request.sample, enabled)?;
        let pcm = &self.bank.samples[sample_index];
        let frames = pcm.frames.max(1) as f64;
        let start = match request.start_norm {
            Some(norm) => norm.clamp(0.0, 0.999_999) as f64 * frames,
            None => self.rng.unit() as f64 * frames,
        };

        let slot = self.allocate_voice();
        let serial = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1);
        let mut voice = Voice::inactive();
        voice.active = true;
        voice.note = request.note;
        voice.channel = request.channel;
        voice.voice_id = request.voice_id;
        voice.sample = sample_index;
        voice.playhead = start;
        voice.serial = serial;
        voice.until_next = 0;
        set_env(&mut voice, velocity, attack_samples);
        self.voices[slot] = voice;

        Some(Triggered {
            sample: sample_index,
            start_norm: (start / frames) as f32,
        })
    }

    pub fn note_off(&mut self, channel: u8, note: u8, voice_id: Option<i32>, release_samples: f32) {
        let slot = if let Some(id) = voice_id {
            self.voices
                .iter()
                .position(|voice| voice.active && !voice.releasing && voice.voice_id == Some(id))
        } else {
            None
        };
        let slot = slot.or_else(|| {
            let mut chosen = None;
            let mut best_serial = 0u32;
            for (index, voice) in self.voices.iter().enumerate() {
                if voice.active
                    && !voice.releasing
                    && voice.channel == channel
                    && voice.note == note
                    && (chosen.is_none() || voice.serial >= best_serial)
                {
                    chosen = Some(index);
                    best_serial = voice.serial;
                }
            }
            chosen
        });
        if let Some(index) = slot {
            self.voices[index].releasing = true;
            set_env(&mut self.voices[index], 0.0, release_samples);
        }
    }

    pub fn choke(&mut self, channel: u8, note: u8, voice_id: Option<i32>) {
        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }
            let matches = match voice_id {
                Some(id) => voice.voice_id == Some(id),
                None => voice.channel == channel && voice.note == note,
            };
            if matches {
                *voice = Voice::inactive();
            }
        }
    }

    pub fn set_tuning(&mut self, channel: u8, note: u8, voice_id: Option<i32>, semitones: f32) {
        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }
            let matches = match voice_id {
                Some(id) => voice.voice_id == Some(id),
                None => voice.channel == channel && voice.note == note,
            };
            if matches {
                voice.tuning = semitones.clamp(-48.0, 48.0);
            }
        }
    }

    /// Mix every sounding voice for one output frame.
    /// `tempo_ratio` is host BPM divided by the recording BPM. Pitch is independent of it.
    pub fn render(&mut self, root: i32, tempo_ratio: f32) -> (f32, f32) {
        let tempo_ratio = if tempo_ratio.is_finite() {
            tempo_ratio.clamp(0.05, 8.0)
        } else {
            1.0
        };
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        for index in 0..MAX_VOICES {
            if !self.voices[index].active {
                continue;
            }
            let (voice_l, voice_r) = self.render_voice(index, root, tempo_ratio);
            left += voice_l;
            right += voice_r;
        }
        if !left.is_finite() {
            left = 0.0;
        }
        if !right.is_finite() {
            right = 0.0;
        }
        (left, right)
    }

    pub fn voice_count(&self) -> usize {
        self.voices.iter().filter(|voice| voice.active).count()
    }

    pub fn active_mask(&self) -> u32 {
        let mut mask = 0u32;
        for voice in &self.voices {
            if voice.active && voice.sample < 32 {
                mask |= 1 << voice.sample;
            }
        }
        mask
    }

    pub fn playhead(&self, slot: usize) -> Option<f64> {
        self.voices
            .get(slot)
            .filter(|voice| voice.active)
            .map(|voice| voice.playhead)
    }

    fn choose_sample(&mut self, forced: Option<usize>, enabled: &[bool]) -> Option<usize> {
        if let Some(index) = forced {
            if index < self.bank.samples.len()
                && enabled.get(index).copied().unwrap_or(false)
                && self.bank.samples[index].frames > 8
            {
                return Some(index);
            }
            return None;
        }
        let mut choices = [0usize; MAX_SAMPLES];
        let mut count = 0;
        for (index, sample) in self.bank.samples.iter().enumerate() {
            if index >= MAX_SAMPLES {
                break;
            }
            if enabled.get(index).copied().unwrap_or(false) && sample.frames > 8 {
                choices[count] = index;
                count += 1;
            }
        }
        if count == 0 {
            None
        } else {
            Some(choices[self.rng.gen_range(count)])
        }
    }

    fn allocate_voice(&mut self) -> usize {
        if let Some(index) = self.voices.iter().position(|voice| !voice.active) {
            return index;
        }
        let mut best = 0usize;
        let mut best_key = (
            !self.voices[0].releasing,
            self.voices[0].env,
            self.voices[0].serial,
        );
        for (index, voice) in self.voices.iter().enumerate().skip(1) {
            let key = (!voice.releasing, voice.env, voice.serial);
            if key < best_key {
                best_key = key;
                best = index;
            }
        }
        best
    }

    fn render_voice(&mut self, index: usize, root: i32, tempo_ratio: f32) -> (f32, f32) {
        advance_env(&mut self.voices[index]);
        if !self.voices[index].active {
            return (0.0, 0.0);
        }

        let sample_index = self.voices[index].sample;
        let (frames, sample_rate) = match self.bank.samples.get(sample_index) {
            Some(pcm) if pcm.frames >= 2 => (pcm.frames, pcm.sample_rate),
            _ => {
                self.voices[index] = Voice::inactive();
                return (0.0, 0.0);
            }
        };

        let sr_ratio = sample_rate as f64 / self.sample_rate as f64;
        let hop = self.hop;
        let grain_len = self.grain_len;
        let note = self.voices[index].note;
        let tuning = self.voices[index].tuning;
        let mut grains = self.voices[index].grains;
        let mut playhead = self.voices[index].playhead;
        let mut until_next = self.voices[index].until_next;

        for grain in &mut grains {
            if grain.on && grain.age >= grain_len {
                grain.on = false;
            }
        }
        if until_next == 0 {
            if let Some(slot) = grains.iter_mut().find(|grain| !grain.on) {
                slot.on = true;
                slot.age = 0;
                slot.origin = playhead;
            }
            let advance = hop as f64 * tempo_ratio as f64 * sr_ratio;
            playhead = (playhead + advance).rem_euclid(frames as f64);
            until_next = hop;
        }

        let pitch = pitch_ratio(note, root, tuning);
        let read_inc = pitch as f64 * sr_ratio;
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        for grain in &mut grains {
            if !grain.on {
                continue;
            }
            let age = grain.age.min(grain_len.saturating_sub(1));
            let window = self.hann[age];
            let position = grain.origin + grain.age as f64 * read_inc;
            let (sample_l, sample_r) = self.bank.samples[sample_index].read(position);
            left += sample_l * window;
            right += sample_r * window;
            grain.age += 1;
        }
        if until_next > 0 {
            until_next -= 1;
        }

        let env = self.voices[index].env;
        self.voices[index].grains = grains;
        self.voices[index].playhead = playhead;
        self.voices[index].until_next = until_next;
        (left * env, right * env)
    }
}

fn pitch_ratio(note: u8, root: i32, tuning: f32) -> f32 {
    let semis = note as f32 - root as f32 + tuning;
    2.0f32.powf(semis / 12.0)
}

fn set_env(voice: &mut Voice, target: f32, samples: f32) {
    voice.env_target = target;
    if samples <= 1.0 {
        voice.env = target;
        voice.env_inc = 0.0;
        voice.env_left = 0;
        return;
    }
    let steps = samples.round().max(1.0);
    voice.env_inc = (target - voice.env) / steps;
    voice.env_left = steps as u32;
}

fn advance_env(voice: &mut Voice) {
    if voice.env_left > 0 {
        voice.env += voice.env_inc;
        voice.env_left -= 1;
        if voice.env_left == 0 {
            voice.env = voice.env_target;
        }
    }
    if voice.releasing && voice.env <= 0.000_8 {
        voice.env = 0.0;
        voice.active = false;
    }
}

fn grain_len_for(sample_rate: f32) -> usize {
    let mut length = (sample_rate * 0.080).round() as usize;
    if length < 64 {
        length = 64;
    }
    if length % 2 == 1 {
        length += 1;
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bank::Pcm;

    fn sine(frames: usize, sample_rate: f32, freq: f32, amp: f32) -> Vec<f32> {
        (0..frames)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / sample_rate).sin() * amp)
            .collect()
    }

    fn tone_bank() -> Bank {
        let sample_rate = 44_100.0;
        let frames = (sample_rate as usize) * 8;
        Bank {
            samples: vec![
                Pcm::mono("sine", sample_rate, sine(frames, sample_rate, 220.0, 0.5)),
                Pcm::mono("other", sample_rate, sine(frames, sample_rate, 330.0, 0.5)),
            ],
            errors: Vec::new(),
        }
    }

    fn frequency(samples: &[f32], sample_rate: f32) -> f32 {
        let mut crossings = 0.0f32;
        for pair in samples.windows(2) {
            if (pair[0] <= 0.0 && pair[1] > 0.0) || (pair[0] >= 0.0 && pair[1] < 0.0) {
                crossings += 1.0;
            }
        }
        crossings * sample_rate / (2.0 * samples.len() as f32)
    }

    fn correlation(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len());
        let mut dot = 0.0f32;
        let mut aa = 0.0f32;
        let mut bb = 0.0f32;
        for i in 0..n {
            dot += a[i] * b[i];
            aa += a[i] * a[i];
            bb += b[i] * b[i];
        }
        if aa <= 1.0e-12 || bb <= 1.0e-12 {
            0.0
        } else {
            dot / (aa.sqrt() * bb.sqrt())
        }
    }

    fn render_for(engine: &mut Engine, frames: usize, root: i32, tempo: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames);
        for _ in 0..frames {
            out.push(engine.render(root, tempo).0);
        }
        out
    }

    #[test]
    fn original_pitch_and_tempo_reconstruct_the_recording() {
        let bank = tone_bank();
        let source = bank.samples[0].data.clone();
        let mut engine = Engine::new(44_100.0, bank, 1);
        engine
            .note_on(
                NoteOn {
                    note: 60,
                    velocity: 1.0,
                    channel: 0,
                    voice_id: None,
                    sample: Some(0),
                    start_norm: Some(0.0),
                },
                &[true, true],
                1.0,
            )
            .unwrap();
        let rendered = render_for(&mut engine, 44_100, 60, 1.0);
        let grain = engine.grain_len;
        let region = &rendered[grain..grain + 8_000];
        let reference = &source[grain..grain + 8_000];
        let score = correlation(region, reference);
        assert!(
            score > 0.98,
            "expected the slice to match the recording at the root, correlation {score}"
        );
        let freq = frequency(region, 44_100.0);
        assert!((freq - 220.0).abs() < 6.0, "frequency {freq}");
    }

    #[test]
    fn pitch_and_tempo_are_independent() {
        let mut pitched = Engine::new(44_100.0, tone_bank(), 1);
        pitched
            .note_on(
                NoteOn {
                    note: 72,
                    velocity: 1.0,
                    channel: 0,
                    voice_id: None,
                    sample: Some(0),
                    start_norm: Some(0.0),
                },
                &[true, true],
                1.0,
            )
            .unwrap();
        let high = render_for(&mut pitched, 44_100, 60, 1.0);
        let grain = pitched.grain_len;
        let high_freq = frequency(&high[grain..grain + 12_000], 44_100.0);
        assert!(
            (high_freq - 440.0).abs() < 12.0,
            "octave should be near 440 Hz, got {high_freq}"
        );

        let mut stretched = Engine::new(44_100.0, tone_bank(), 1);
        stretched
            .note_on(
                NoteOn {
                    note: 60,
                    velocity: 1.0,
                    channel: 0,
                    voice_id: None,
                    sample: Some(0),
                    start_norm: Some(0.0),
                },
                &[true, true],
                1.0,
            )
            .unwrap();
        let frames = 44_100;
        let _ = render_for(&mut stretched, frames, 60, 2.0);
        let playhead = stretched.playhead(0).unwrap();
        let expected = frames as f64 * 2.0;
        let tolerance = stretched.hop_len() as f64 * 2.0 + 2.0;
        assert!(
            (playhead - expected).abs() < tolerance,
            "playhead {playhead} expected near {expected}"
        );

        let mut timed = Engine::new(44_100.0, tone_bank(), 1);
        timed
            .note_on(
                NoteOn {
                    note: 60,
                    velocity: 1.0,
                    channel: 0,
                    voice_id: None,
                    sample: Some(0),
                    start_norm: Some(0.0),
                },
                &[true, true],
                1.0,
            )
            .unwrap();
        let doubled = render_for(&mut timed, 44_100, 60, 2.0);
        let grain = timed.grain_len;
        let freq = frequency(&doubled[grain..grain + 12_000], 44_100.0);
        assert!(
            (freq - 220.0).abs() < 8.0,
            "double tempo should keep the pitch near 220 Hz, got {freq}"
        );
    }

    #[test]
    fn host_sample_rate_does_not_change_pitch() {
        let mut engine = Engine::new(48_000.0, tone_bank(), 3);
        engine
            .note_on(
                NoteOn {
                    note: 60,
                    velocity: 1.0,
                    channel: 0,
                    voice_id: None,
                    sample: Some(0),
                    start_norm: Some(0.0),
                },
                &[true, true],
                1.0,
            )
            .unwrap();
        let rendered = render_for(&mut engine, 48_000, 60, 1.0);
        let grain = engine.grain_len;
        let freq = frequency(&rendered[grain..grain + 16_000], 48_000.0);
        assert!(
            (freq - 220.0).abs() < 8.0,
            "48 kHz playback should stay at 220 Hz, got {freq}"
        );
        let playhead = engine.playhead(0).unwrap();
        let expected = 44_100.0;
        let tolerance = engine.hop_len() as f64 * (44_100.0 / 48_000.0) + 4.0;
        assert!(
            (playhead - expected).abs() < tolerance,
            "one second of output should walk one second of the recording, playhead {playhead}"
        );
    }

    #[test]
    fn release_reaches_silence_and_disabled_samples_are_skipped() {
        let mut engine = Engine::new(44_100.0, tone_bank(), 9);
        engine
            .note_on(
                NoteOn {
                    note: 60,
                    velocity: 1.0,
                    channel: 0,
                    voice_id: None,
                    sample: None,
                    start_norm: None,
                },
                &[false, true],
                1.0,
            )
            .unwrap();
        assert_eq!(engine.voices[0].sample, 1);
        let _ = render_for(&mut engine, 2_000, 60, 1.0);
        engine.note_off(0, 60, None, 44_100.0 * 0.05);
        let tail = render_for(&mut engine, 8_000, 60, 1.0);
        let late = &tail[6_000..];
        let rms = (late.iter().map(|s| s * s).sum::<f32>() / late.len() as f32).sqrt();
        assert!(rms < 0.001, "release tail rms {rms}");
        assert!(!engine.any_active());
    }

    #[test]
    fn random_notes_vary_the_slice() {
        let mut engine = Engine::new(44_100.0, tone_bank(), 42);
        let mut seen_samples = [false; 2];
        let mut starts = Vec::new();
        for i in 0..40 {
            let triggered = engine
                .note_on(
                    NoteOn {
                        note: 60,
                        velocity: 0.8,
                        channel: 0,
                        voice_id: None,
                        sample: None,
                        start_norm: None,
                    },
                    &[true, true],
                    1.0,
                )
                .unwrap();
            seen_samples[triggered.sample] = true;
            starts.push(triggered.start_norm);
            engine.note_off(0, 60, None, 1.0);
            let _ = engine.render(60, 1.0);
            engine.silence();
            let _ = i;
        }
        assert!(
            seen_samples[0] && seen_samples[1],
            "both recordings should be drawn"
        );
        let mean = starts.iter().sum::<f32>() / starts.len() as f32;
        let var = starts.iter().map(|s| (s - mean).powi(2)).sum::<f32>() / starts.len() as f32;
        assert!(
            var > 0.01,
            "start points should spread across the recording, var {var}"
        );
    }

    #[test]
    fn held_note_wraps_without_blowing_up() {
        let mut engine = Engine::new(44_100.0, tone_bank(), 5);
        engine
            .note_on(
                NoteOn {
                    note: 64,
                    velocity: 0.7,
                    channel: 0,
                    voice_id: None,
                    sample: Some(0),
                    start_norm: Some(0.99),
                },
                &[true, true],
                1.0,
            )
            .unwrap();
        for _ in 0..(44_100 / 2) {
            let (left, right) = engine.render(60, 1.2);
            assert!(left.is_finite() && right.is_finite());
            assert!(left.abs() < 2.0 && right.abs() < 2.0);
        }
    }
}
