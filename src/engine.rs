use crate::bank::Bank;

const MAX_VOICES: usize = 16;
const MAX_GRAINS: usize = 3;
const SMEAR_GRAINS: usize = 6;
const MAX_SAMPLES: usize = 32;

/// What the plugin asks the engine to do when a note starts.
pub struct NoteOn {
    pub note: u8,
    pub velocity: f32,
    pub channel: u8,
    pub voice_id: Option<i32>,
    /// Force a recording. `None` picks at random, weighted by each recording's probability.
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
    /// This slice was scattered instead of phase-locked to the playhead.
    smear: bool,
    /// Multiplier on the note's read step. `1` follows the note, `-1` flips that grain.
    step_mul: f64,
    gain: f32,
    len: usize,
    /// Stereo placement for one scattered slice, `-1` left to `1` right.
    pan: f32,
}

impl Grain {
    const fn off() -> Self {
        Self {
            on: false,
            age: 0,
            origin: 0.0,
            smear: false,
            step_mul: 1.0,
            gain: 1.0,
            len: 0,
            pan: 0.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PanSide {
    Center,
    Left,
    Right,
}

/// Chances that apply to every note, decided once when the note starts.
#[derive(Clone, Copy)]
pub struct GlobalRoll {
    /// Chance the note plays backward and fades in.
    pub swell: f32,
    /// Chance the note is bitcrushed.
    pub crush: f32,
    /// Chance the note is randomly low-passed or high-passed.
    pub pass: f32,
    /// Chance the note stutters on a random clock division.
    pub stutter: f32,
    /// Chance the note's slices scatter around the playhead.
    pub smear: f32,
}

impl GlobalRoll {
    pub const fn off() -> Self {
        Self {
            swell: 0.0,
            crush: 0.0,
            pass: 0.0,
            stutter: 0.0,
            smear: 0.0,
        }
    }
}

/// Per-recording chances, decided once when a note starts.
#[derive(Clone, Copy)]
pub struct SliceRolls {
    /// Chance, `0..1`, that this note plays the recording backward.
    pub reverse: f32,
    /// `-1` always hard-left, `0` the recording's own stereo, `1` always hard-right.
    /// Values between are the chance of a hard pan to that side.
    pub pan: f32,
    /// Chance, `0..1`, that this note plays an octave higher.
    pub octave: f32,
}

impl SliceRolls {
    pub const fn neutral() -> Self {
        Self {
            reverse: 0.0,
            pan: 0.0,
            octave: 0.0,
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
    reverse: bool,
    pan: PanSide,
    octave_up: bool,
    crush: bool,
    crush_steps: f32,
    crush_hold: u32,
    crush_len: u32,
    held_l: f32,
    held_r: f32,
    pass: PassMode,
    pass_coeff: f32,
    pass_l: f32,
    pass_r: f32,
    stutter: StutterDiv,
    stutter_pos: u32,
    smear: bool,
    /// Scatter radius in recording frames.
    smear_radius: f64,
    /// Scattered slice length in output samples.
    smear_len: usize,
    smear_flip: f32,
    smear_shimmer: f32,
    smear_detune: f32,
    smear_width: f32,
    smear_gap_min: usize,
    smear_gap_max: usize,
    smear_voices: u8,
    smear_wait: usize,
    smear_grains: [Grain; SMEAR_GRAINS],
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
            reverse: false,
            pan: PanSide::Center,
            octave_up: false,
            crush: false,
            crush_steps: 32.0,
            crush_hold: 0,
            crush_len: 1,
            held_l: 0.0,
            held_r: 0.0,
            pass: PassMode::Off,
            pass_coeff: 0.0,
            pass_l: 0.0,
            pass_r: 0.0,
            stutter: StutterDiv::Off,
            stutter_pos: 0,
            smear: false,
            smear_radius: 0.0,
            smear_len: 0,
            smear_flip: 0.0,
            smear_shimmer: 0.0,
            smear_detune: 0.0,
            smear_width: 0.0,
            smear_gap_min: 1,
            smear_gap_max: 1,
            smear_voices: 0,
            smear_wait: 0,
            smear_grains: [Grain::off(); SMEAR_GRAINS],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PassMode {
    Off,
    Low,
    High,
}

/// A clock division, measured in quarter-note beats.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StutterDiv {
    Off,
    /// 1/8.
    Eighth,
    /// 1/16.
    Sixteenth,
    /// Sixteenth-note triplet.
    SixteenthTriplet,
    /// 1/32.
    ThirtySecond,
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
    global: GlobalRoll,
    clock_bpm: f32,
    /// Remaining crush degrees, dealt in a shuffled order.
    crush_bag: [CrushDegree; 3],
    crush_left: u8,
    /// Remaining smear degrees, dealt in a shuffled order.
    smear_bag: [SmearDegree; 3],
    smear_left: u8,
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
            global: GlobalRoll::off(),
            clock_bpm: 120.0,
            crush_bag: [CrushDegree::Little, CrushDegree::Moderate, CrushDegree::Lot],
            crush_left: 0,
            smear_bag: [SmearDegree::Little, SmearDegree::Moderate, SmearDegree::Lot],
            smear_left: 0,
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

    pub fn set_global(&mut self, roll: GlobalRoll) {
        self.global = roll;
    }

    /// Musical clock in BPM. Stutter divisions are measured against it.
    pub fn set_clock(&mut self, bpm: f32) {
        self.clock_bpm = if bpm.is_finite() {
            bpm.clamp(20.0, 400.0)
        } else {
            120.0
        };
    }

    pub fn note_on(
        &mut self,
        request: NoteOn,
        weights: &[f32],
        rolls: &[SliceRolls],
        attack_samples: f32,
    ) -> Option<Triggered> {
        let velocity = request.velocity.clamp(0.0, 1.0);
        if velocity <= 0.0 {
            return None;
        }
        let sample_index = self.choose_sample(request.sample, weights)?;
        let pcm = &self.bank.samples[sample_index];
        let frames = pcm.frames.max(1) as f64;
        let source_rate = pcm.sample_rate as f64;
        let start = match request.start_norm {
            Some(norm) => norm.clamp(0.0, 0.999_999) as f64 * frames,
            None => self.rng.unit() as f64 * frames,
        };
        let roll = rolls.get(sample_index).copied().unwrap_or(SliceRolls::neutral());
        let mut reverse = self.rng.unit() < roll.reverse.clamp(0.0, 1.0);
        let pan = rolled_pan(&mut self.rng, roll.pan);
        let octave_up = self.rng.unit() < roll.octave.clamp(0.0, 1.0);
        let swell = fired(&mut self.rng, self.global.swell);
        if swell {
            reverse = true;
        }
        let crush = fired(&mut self.rng, self.global.crush);
        let (crush_steps, crush_len) = if crush {
            crush_setup(self.take_crush(), self.sample_rate)
        } else {
            (32.0, 1)
        };
        let (pass, pass_coeff) = self.roll_pass();
        let stutter = self.roll_stutter();
        let smear = fired(&mut self.rng, self.global.smear);
        let smear_tone = if smear {
            Some(smear_setup(self.take_smear()))
        } else {
            None
        };
        let attack_samples = if swell {
            attack_samples.max(self.sample_rate * SWELL_SECONDS)
        } else {
            attack_samples
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
        voice.reverse = reverse;
        voice.pan = pan;
        voice.octave_up = octave_up;
        voice.crush = crush;
        voice.crush_steps = crush_steps;
        voice.crush_len = crush_len;
        voice.pass = pass;
        voice.pass_coeff = pass_coeff;
        voice.stutter = stutter;
        if let Some(tone) = smear_tone {
            voice.smear = true;
            voice.smear_radius = tone.radius_secs * source_rate;
            voice.smear_len = (self.sample_rate as f64 * tone.grain_secs)
                .round()
                .max(2.0) as usize;
            voice.smear_flip = tone.flip;
            voice.smear_shimmer = tone.shimmer;
            voice.smear_detune = tone.detune;
            voice.smear_width = tone.width;
            voice.smear_voices = tone.voices;
            let gap_min = (self.sample_rate as f64 * tone.gap_min).round().max(1.0);
            let gap_max = (self.sample_rate as f64 * tone.gap_max).round().max(gap_min);
            voice.smear_gap_min = gap_min as usize;
            voice.smear_gap_max = gap_max as usize;
        }
        set_env(&mut voice, velocity, attack_samples);
        self.voices[slot] = voice;

        Some(Triggered {
            sample: sample_index,
            start_norm: (start / frames) as f32,
        })
    }

    /// Release one voice by id, without falling back to another note on that key.
    pub fn release_id(&mut self, voice_id: i32, release_samples: f32) {
        let slot = self.voices.iter().position(|voice| {
            voice.active && !voice.releasing && voice.voice_id == Some(voice_id)
        });
        if let Some(index) = slot {
            self.voices[index].releasing = true;
            set_env(&mut self.voices[index], 0.0, release_samples);
        }
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
    /// With keytracking on, the played note transposes around `root`. With it off, every note
    /// sounds at `root`, measured from C4, which is how the recordings were played.
    pub fn render(&mut self, root: i32, tempo_ratio: f32, keytrack: bool) -> (f32, f32) {
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
            let (voice_l, voice_r) = self.render_voice(index, root, tempo_ratio, keytrack);
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

    /// Normalized playhead of the newest sounding voice in each slot, in parts per million.
    /// Silent slots are `u32::MAX`. `serials` is scratch and can be discarded.
    pub fn write_playheads(&self, heads: &mut [u32], serials: &mut [u32]) {
        let count = heads.len().min(serials.len());
        for index in 0..count {
            heads[index] = u32::MAX;
            serials[index] = 0;
        }
        for voice in &self.voices {
            if !voice.active || voice.sample >= count {
                continue;
            }
            let slot = voice.sample;
            if heads[slot] != u32::MAX && voice.serial < serials[slot] {
                continue;
            }
            let Some(pcm) = self.bank.samples.get(slot) else {
                continue;
            };
            if pcm.frames == 0 {
                continue;
            }
            let norm = (voice.playhead / pcm.frames as f64).rem_euclid(1.0);
            heads[slot] = ((norm * 1_000_000.0).round() as u32).min(999_999);
            serials[slot] = voice.serial;
        }
    }

    pub fn playhead(&self, slot: usize) -> Option<f64> {
        self.voices
            .get(slot)
            .filter(|voice| voice.active)
            .map(|voice| voice.playhead)
    }

    fn choose_sample(&mut self, forced: Option<usize>, weights: &[f32]) -> Option<usize> {
        let weight_of = |index: usize| weights.get(index).copied().unwrap_or(1.0).max(0.0);
        if let Some(index) = forced {
            if index < self.bank.samples.len()
                && weight_of(index) > 0.0
                && self.bank.samples[index].frames > 8
            {
                return Some(index);
            }
            return None;
        }
        let mut choices = [0usize; MAX_SAMPLES];
        let mut choice_weight = [0.0f32; MAX_SAMPLES];
        let mut count = 0;
        let mut total = 0.0f32;
        for (index, sample) in self.bank.samples.iter().enumerate() {
            if index >= MAX_SAMPLES {
                break;
            }
            let weight = weight_of(index);
            if weight > 0.0 && sample.frames > 8 {
                choices[count] = index;
                choice_weight[count] = weight;
                total += weight;
                count += 1;
            }
        }
        if count == 0 || total <= 0.0 {
            return None;
        }
        let mut pick = self.rng.unit() * total;
        for index in 0..count {
            pick -= choice_weight[index];
            if pick <= 0.0 {
                return Some(choices[index]);
            }
        }
        Some(choices[count - 1])
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

    fn render_voice(
        &mut self,
        index: usize,
        root: i32,
        tempo_ratio: f32,
        keytrack: bool,
    ) -> (f32, f32) {
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
        let reverse = self.voices[index].reverse;
        let pan = self.voices[index].pan;
        let octave_up = self.voices[index].octave_up;
        let mut grains = self.voices[index].grains;
        let mut playhead = self.voices[index].playhead;
        let mut until_next = self.voices[index].until_next;
        let direction = if reverse { -1.0 } else { 1.0 };
        let mut pitch = pitch_ratio(note, root, tuning, keytrack);
        if octave_up {
            pitch *= 2.0;
        }
        let read_inc = pitch as f64 * sr_ratio * direction;

        if self.voices[index].smear {
            return self.render_smear(
                index,
                sample_index,
                frames as f64,
                read_inc,
                playhead,
                tempo_ratio,
                sr_ratio,
                direction,
                pan,
            );
        }

        for grain in &mut grains {
            if grain.on && grain.age >= grain_len {
                grain.on = false;
            }
        }
        if until_next == 0 {
            // The playhead walks at the tempo, while each slice is read at the pitch.
            // Those only land on the same waveform when the two rates match. Anywhere
            // else, the next slice has to be slid onto the one already playing or the
            // overlap combs and the recording sounds warped.
            let origin = if (pitch - tempo_ratio).abs() <= 0.000_8 {
                playhead
            } else {
                let previous = grains
                    .iter()
                    .filter(|grain| grain.on)
                    .min_by_key(|grain| grain.age);
                match previous {
                    Some(previous) => {
                        self.align_origin(sample_index, playhead, read_inc, previous)
                    }
                    None => playhead,
                }
            };
            if let Some(slot) = grains.iter_mut().find(|grain| !grain.on) {
                slot.on = true;
                slot.age = 0;
                slot.origin = origin;
            }
            let advance = hop as f64 * tempo_ratio as f64 * sr_ratio * direction;
            playhead = (playhead + advance).rem_euclid(frames as f64);
            until_next = hop;
        }
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

        self.voices[index].grains = grains;
        self.voices[index].playhead = playhead;
        self.voices[index].until_next = until_next;
        let (left, right) = place(left, right, pan);
        let (left, right) = self.color_voice(index, left, right);
        let env = self.voices[index].env;
        let gate = self.stutter_gate(index);
        (left * env * gate, right * env * gate)
    }

    /// A slow cloud of blooming slices. They do not share a clock, so the note
    /// does not pump. Each slice drifts in pitch and sits a little off center.
    fn render_smear(
        &mut self,
        index: usize,
        sample_index: usize,
        frames: f64,
        read_inc: f64,
        mut playhead: f64,
        tempo_ratio: f32,
        sr_ratio: f64,
        direction: f64,
        pan: PanSide,
    ) -> (f32, f32) {
        playhead = (playhead + tempo_ratio as f64 * sr_ratio * direction).rem_euclid(frames);
        let radius = self.voices[index].smear_radius;
        let center = self.voices[index].smear_len.max(2);
        let flip = self.voices[index].smear_flip;
        let shimmer = self.voices[index].smear_shimmer;
        let detune = self.voices[index].smear_detune;
        let width = self.voices[index].smear_width;
        let gap_min = self.voices[index].smear_gap_min.max(1);
        let gap_max = self.voices[index].smear_gap_max.max(gap_min);
        let max_voices = self.voices[index].smear_voices.max(1) as usize;
        let mut grains = self.voices[index].smear_grains;
        let mut wait = self.voices[index].smear_wait;

        for grain in &mut grains {
            if grain.on && grain.age >= grain.len {
                grain.on = false;
            }
        }
        if wait == 0 {
            let sounding = grains.iter().filter(|grain| grain.on).count();
            if sounding < max_voices {
                if let Some(slot) = grains.iter_mut().find(|grain| !grain.on) {
                    let spread = self.rng.unit() as f64 * 2.0 - 1.0;
                    let jitter = spread * spread.abs() * radius;
                    let cents = (self.rng.unit() * 2.0 - 1.0) * detune;
                    let mut step_mul = 2.0_f64.powf(cents as f64 / 1_200.0);
                    let mut gain = 0.38 + self.rng.unit() * 0.16;
                    if self.rng.unit() < flip {
                        step_mul = -step_mul;
                    }
                    if self.rng.unit() < shimmer {
                        let harmonic = if self.rng.unit() < 0.5 { 2.0 } else { 1.498_307 };
                        step_mul *= harmonic;
                        gain *= 0.34;
                    }
                    let span = (center as f64 * (0.72 + self.rng.unit() as f64 * 0.7))
                        .round()
                        .max(2.0) as usize;
                    slot.on = true;
                    slot.age = 0;
                    slot.origin = (playhead + jitter).rem_euclid(frames);
                    slot.smear = true;
                    slot.step_mul = step_mul;
                    slot.gain = gain;
                    slot.len = span;
                    slot.pan = (self.rng.unit() * 2.0 - 1.0) * width;
                }
            }
            let pick = self.rng.unit();
            wait = gap_min + ((gap_max - gap_min) as f32 * pick).round() as usize;
        } else {
            wait -= 1;
        }

        let mut left = 0.0f32;
        let mut right = 0.0f32;
        for grain in &mut grains {
            if !grain.on {
                continue;
            }
            let limit = grain.len.max(1);
            let age = grain.age.min(limit.saturating_sub(1));
            let window = bloom_window(age, limit);
            let position = grain.origin + grain.age as f64 * read_inc * grain.step_mul;
            let (sample_l, sample_r) = self.bank.samples[sample_index].read(position);
            let level = window * grain.gain;
            let steer = grain.pan.clamp(-1.0, 1.0);
            left += sample_l * level * (1.0 - steer.max(0.0));
            right += sample_r * level * (1.0 + steer.min(0.0));
            grain.age += 1;
        }

        self.voices[index].smear_grains = grains;
        self.voices[index].smear_wait = wait;
        self.voices[index].playhead = playhead;
        let (left, right) = place(left, right, pan);
        let (left, right) = self.color_voice(index, left, right);
        let env = self.voices[index].env;
        let gate = self.stutter_gate(index);
        (left * env * gate, right * env * gate)
    }

    fn roll_pass(&mut self) -> (PassMode, f32) {
        if !fired(&mut self.rng, self.global.pass) {
            return (PassMode::Off, 0.0);
        }
        let low = self.rng.unit() < 0.5;
        let degree = self.rng.unit();
        let (mode, hz) = if low {
            (PassMode::Low, 280.0 * (1_200.0_f32 / 280.0).powf(degree))
        } else {
            (PassMode::High, 800.0 * (2_400.0_f32 / 800.0).powf(degree))
        };
        (mode, cutoff_coeff(self.sample_rate, hz))
    }

    fn roll_stutter(&mut self) -> StutterDiv {
        if !fired(&mut self.rng, self.global.stutter) {
            return StutterDiv::Off;
        }
        match (self.rng.unit() * 4.0).floor() as u32 {
            0 => StutterDiv::Eighth,
            1 => StutterDiv::Sixteenth,
            2 => StutterDiv::SixteenthTriplet,
            _ => StutterDiv::ThirtySecond,
        }
    }

    fn color_voice(&mut self, index: usize, mut left: f32, mut right: f32) -> (f32, f32) {
        let voice = &mut self.voices[index];
        match voice.pass {
            PassMode::Off => {}
            PassMode::Low => {
                left = one_pole(&mut voice.pass_l, left, voice.pass_coeff);
                right = one_pole(&mut voice.pass_r, right, voice.pass_coeff);
            }
            PassMode::High => {
                let low_l = one_pole(&mut voice.pass_l, left, voice.pass_coeff);
                let low_r = one_pole(&mut voice.pass_r, right, voice.pass_coeff);
                left -= low_l;
                right -= low_r;
            }
        }
        if voice.crush {
            if voice.crush_hold == 0 {
                let steps = voice.crush_steps;
                voice.held_l = quantize(left, steps);
                voice.held_r = quantize(right, steps);
                voice.crush_hold = voice.crush_len;
            }
            voice.crush_hold -= 1;
            left = voice.held_l;
            right = voice.held_r;
        }
        (left, right)
    }

    fn stutter_gate(&mut self, index: usize) -> f32 {
        let voice = &mut self.voices[index];
        let beats = match voice.stutter {
            StutterDiv::Off => return 1.0,
            StutterDiv::Eighth => 0.5,
            StutterDiv::Sixteenth => 0.25,
            StutterDiv::SixteenthTriplet => 1.0 / 6.0,
            StutterDiv::ThirtySecond => 0.125,
        };
        let period = ((beats * 60.0 / self.clock_bpm) * self.sample_rate)
            .round()
            .clamp(2.0, self.sample_rate * 4.0) as u32;
        let pos = voice.stutter_pos % period;
        voice.stutter_pos = voice.stutter_pos.wrapping_add(1);
        let half = (period / 2).max(1);
        if pos >= half {
            return 0.0;
        }
        let fade = (period / 12).clamp(1, 96).min(half);
        let edge = pos.min(half - pos) as f32 / fade as f32;
        edge.clamp(0.0, 1.0)
    }

    /// Slide a new slice, in output samples, until its overlap matches the slice already
    /// sounding. The tempo playhead stays where it is; only the read position moves, and
    /// only when the match is clearly better than leaving it alone.
    fn align_origin(
        &self,
        sample_index: usize,
        ideal: f64,
        read_inc: f64,
        previous: &Grain,
    ) -> f64 {
        if !read_inc.is_finite() || read_inc.abs() < 1.0e-8 {
            return ideal;
        }
        let sample_rate = self.sample_rate as f64;
        let max_delta = (sample_rate * 0.025).round() as i32;
        let remaining = self.grain_len.saturating_sub(previous.age.saturating_add(1));
        let compare = ((sample_rate * 0.012).round() as usize).clamp(64, 512).min(remaining);
        if max_delta < 8 || compare < 64 {
            return ideal;
        }

        let pcm = &self.bank.samples[sample_index];
        let mut heard = [0.0f32; 512];
        for (k, slot) in heard.iter_mut().enumerate().take(compare) {
            let position = previous.origin + (previous.age as f64 + k as f64) * read_inc;
            let (left, right) = pcm.read(position);
            *slot = left + right;
        }

        let score = |delta: i32| -> f32 {
            let mut dot = 0.0f32;
            let mut energy_heard = 0.0f32;
            let mut energy_next = 0.0f32;
            let mut k = 0usize;
            while k < compare {
                let position = ideal + (delta as f64 + k as f64) * read_inc;
                let (left, right) = pcm.read(position);
                let sample = left + right;
                dot += heard[k] * sample;
                energy_heard += heard[k] * heard[k];
                energy_next += sample * sample;
                k += 2;
            }
            if energy_heard <= 1.0e-12 || energy_next <= 1.0e-12 {
                0.0
            } else {
                dot / (energy_heard.sqrt() * energy_next.sqrt())
            }
        };

        let at_ideal = score(0);
        let mut best_delta = 0i32;
        let mut best_score = at_ideal;
        let mut delta = -max_delta;
        while delta <= max_delta {
            let candidate = score(delta);
            if candidate > best_score + 1.0e-4
                || ((candidate - best_score).abs() <= 1.0e-4 && delta.abs() < best_delta.abs())
            {
                best_score = candidate;
                best_delta = delta;
            }
            delta += 8;
        }
        let coarse = best_delta;
        for delta in (coarse - 8).max(-max_delta)..=(coarse + 8).min(max_delta) {
            let candidate = score(delta);
            if candidate > best_score + 1.0e-4
                || ((candidate - best_score).abs() <= 1.0e-4 && delta.abs() < best_delta.abs())
            {
                best_score = candidate;
                best_delta = delta;
            }
        }
        if best_score < at_ideal + 0.12 {
            return ideal;
        }

        let left = score(best_delta - 1);
        let right = score(best_delta + 1);
        let denom = left - 2.0 * best_score + right;
        let fraction = if denom.abs() < 1.0e-6 {
            0.0
        } else {
            (0.5 * (left - right) / denom).clamp(-0.5, 0.5)
        };
        ideal + (best_delta as f64 + fraction as f64) * read_inc
    }
}

fn rolled_pan(rng: &mut Rng, pan: f32) -> PanSide {
    let pan = pan.clamp(-1.0, 1.0);
    if pan < 0.0 && rng.unit() < -pan {
        PanSide::Left
    } else if pan > 0.0 && rng.unit() < pan {
        PanSide::Right
    } else {
        PanSide::Center
    }
}

fn place(left: f32, right: f32, pan: PanSide) -> (f32, f32) {
    match pan {
        PanSide::Center => (left, right),
        PanSide::Left => (0.5 * (left + right), 0.0),
        PanSide::Right => (0.0, 0.5 * (left + right)),
    }
}

/// How long a swell takes to rise from silence, in seconds.
const SWELL_SECONDS: f32 = 1.15;

fn fired(rng: &mut Rng, chance: f32) -> bool {
    let chance = if chance.is_finite() {
        chance.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if chance <= 0.0 {
        false
    } else if chance >= 1.0 {
        true
    } else {
        rng.unit() < chance
    }
}

fn cutoff_coeff(sample_rate: f32, hz: f32) -> f32 {
    let hz = hz.clamp(20.0, sample_rate * 0.45);
    1.0 - (-std::f32::consts::TAU * hz / sample_rate).exp()
}

fn one_pole(state: &mut f32, input: f32, coeff: f32) -> f32 {
    *state += coeff * (input - *state);
    *state
}

/// How hard a crushed note is. The middle degree is the original crush.
#[derive(Clone, Copy)]
enum CrushDegree {
    Little,
    Moderate,
    Lot,
}

impl Engine {
    /// Deal the next crush degree. The bag is shuffled, then each degree is used once.
    fn take_crush(&mut self) -> CrushDegree {
        if self.crush_left == 0 {
            self.crush_bag = [CrushDegree::Little, CrushDegree::Moderate, CrushDegree::Lot];
            for index in (1..3).rev() {
                let swap = (self.rng.unit() * (index as f32 + 1.0)).floor() as usize;
                self.crush_bag.swap(index, swap);
            }
            self.crush_left = 3;
        }
        self.crush_left -= 1;
        self.crush_bag[self.crush_left as usize]
    }

    /// Deal the next smear degree. The bag is shuffled, then each degree is used once.
    fn take_smear(&mut self) -> SmearDegree {
        if self.smear_left == 0 {
            self.smear_bag = [SmearDegree::Little, SmearDegree::Moderate, SmearDegree::Lot];
            for index in (1..3).rev() {
                let swap = (self.rng.unit() * (index as f32 + 1.0)).floor() as usize;
                self.smear_bag.swap(index, swap);
            }
            self.smear_left = 3;
        }
        self.smear_left -= 1;
        self.smear_bag[self.smear_left as usize]
    }
}

/// Bit depth and sample-hold for one degree. Moderate matches the original crush.
fn crush_setup(degree: CrushDegree, sample_rate: f32) -> (f32, u32) {
    let (steps, divisor, min_hold, max_hold) = match degree {
        CrushDegree::Little => (96.0, 11_000.0, 2.0, 24.0),
        CrushDegree::Moderate => (32.0, 5_500.0, 2.0, 48.0),
        CrushDegree::Lot => (12.0, 2_750.0, 4.0, 96.0),
    };
    let hold = (sample_rate / divisor).round().clamp(min_hold, max_hold) as u32;
    (steps, hold.max(1))
}

fn quantize(sample: f32, steps: f32) -> f32 {
    (sample.clamp(-1.0, 1.0) * steps).round() / steps
}

/// How far a smeared note scatters its slices. Little is a blur, lot is a cloud.
#[derive(Clone, Copy)]
enum SmearDegree {
    Little,
    Moderate,
    Lot,
}

struct SmearTone {
    radius_secs: f64,
    grain_secs: f64,
    gap_min: f64,
    gap_max: f64,
    voices: u8,
    detune: f32,
    flip: f32,
    shimmer: f32,
    width: f32,
}

fn smear_setup(degree: SmearDegree) -> SmearTone {
    match degree {
        SmearDegree::Little => SmearTone {
            radius_secs: 0.05,
            grain_secs: 0.62,
            gap_min: 0.32,
            gap_max: 0.58,
            voices: 3,
            detune: 7.0,
            flip: 0.06,
            shimmer: 0.08,
            width: 0.22,
        },
        SmearDegree::Moderate => SmearTone {
            radius_secs: 0.22,
            grain_secs: 0.78,
            gap_min: 0.18,
            gap_max: 0.46,
            voices: 4,
            detune: 14.0,
            flip: 0.24,
            shimmer: 0.22,
            width: 0.42,
        },
        SmearDegree::Lot => SmearTone {
            radius_secs: 0.55,
            grain_secs: 0.95,
            gap_min: 0.12,
            gap_max: 0.38,
            voices: 6,
            detune: 20.0,
            flip: 0.38,
            shimmer: 0.40,
            width: 0.62,
        },
    }
}

/// A long fade-in and a slower fade-out, so a slice blooms instead of pulsing.
fn bloom_window(age: usize, len: usize) -> f32 {
    if len <= 1 {
        return 1.0;
    }
    let t = (age as f32 / len as f32).clamp(0.0, 1.0);
    if t < 0.55 {
        let u = t / 0.55;
        0.5 - 0.5 * (u * std::f32::consts::PI).cos()
    } else {
        let u = ((t - 0.55) / 0.45).clamp(0.0, 1.0);
        0.5 + 0.5 * (u * std::f32::consts::PI).cos()
    }
}

/// C4 is the untransposed pitch of the recordings.
const SAMPLE_MIDI: f32 = 60.0;

fn pitch_ratio(note: u8, root: i32, tuning: f32, keytrack: bool) -> f32 {
    let semis = if keytrack {
        note as f32 - root as f32 + tuning
    } else {
        root as f32 - SAMPLE_MIDI + tuning
    };
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

    fn envelope_variation(samples: &[f32], block: usize) -> f32 {
        let mut sum = 0.0f32;
        let mut sum_sq = 0.0f32;
        let mut count = 0.0f32;
        for chunk in samples.chunks(block) {
            if chunk.len() < block {
                break;
            }
            let rms = (chunk.iter().map(|sample| sample * sample).sum::<f32>() / block as f32).sqrt();
            sum += rms;
            sum_sq += rms * rms;
            count += 1.0;
        }
        if count < 2.0 {
            return 0.0;
        }
        let mean = sum / count;
        if mean <= 1.0e-6 {
            return 0.0;
        }
        let variance = (sum_sq / count - mean * mean).max(0.0);
        variance.sqrt() / mean
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
            out.push(engine.render(root, tempo, true).0);
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
                &[1.0, 1.0],
                &[],
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
                &[1.0, 1.0],
                &[],
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
                &[1.0, 1.0],
                &[],
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
                &[1.0, 1.0],
                &[],
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
    fn keytracking_off_uses_the_set_key_for_every_note() {
        let measure = |note: u8, root: i32| -> f32 {
            let mut engine = Engine::new(44_100.0, tone_bank(), 1);
            engine
                .note_on(
                    NoteOn {
                        note,
                        velocity: 1.0,
                        channel: 0,
                        voice_id: None,
                        sample: Some(0),
                        start_norm: Some(0.0),
                    },
                    &[1.0, 1.0],
                &[],
                    1.0,
                )
                .unwrap();
            let mut out = Vec::with_capacity(44_100);
            for _ in 0..44_100 {
                out.push(engine.render(root, 1.0, false).0);
            }
            let grain = engine.grain_len;
            frequency(&out[grain..grain + 12_000], 44_100.0)
        };

        let low = measure(48, 60);
        let high = measure(72, 60);
        assert!(
            (low - 220.0).abs() < 8.0,
            "C3 should play the recording as it is, got {low}"
        );
        assert!(
            (high - 220.0).abs() < 8.0,
            "C5 should play the recording as it is, got {high}"
        );

        let whole_step = 220.0 * 2.0f32.powf(2.0 / 12.0);
        let to_d = measure(48, 62);
        assert!(
            (to_d - whole_step).abs() < 8.0,
            "D4 should transpose every note the same amount, got {to_d}"
        );

        let octave_down = measure(72, 48);
        let ratio = octave_down / high;
        assert!(
            (ratio - 0.5).abs() < 0.08,
            "C3 as the set key should drop every note an octave, got {octave_down} Hz (ratio {ratio})"
        );
    }

    #[test]
    fn original_pitch_transposes_without_pulsing() {
        for root in [48, 53, 62, 67, 72] {
            let mut engine = Engine::new(44_100.0, tone_bank(), 1);
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
                    &[1.0, 1.0],
                    &[],
                    1.0,
                )
                .unwrap();
            let mut out = Vec::with_capacity(44_100);
            for _ in 0..44_100 {
                out.push(engine.render(root, 1.0, false).0);
            }
            let grain = engine.grain_len;
            let region = &out[grain..grain + 24_000];
            let semis = root as f32 - 60.0;
            let target = 220.0 * 2.0f32.powf(semis / 12.0);
            let freq = frequency(region, 44_100.0);
            let wobble = envelope_variation(region, 700);
            assert!(
                (freq - target).abs() < 8.0,
                "root {root} should sit near {target} Hz, got {freq}"
            );
            assert!(
                wobble < 0.08,
                "root {root} should stay steady, envelope variation {wobble}"
            );
        }
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
                &[1.0, 1.0],
                &[],
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
                &[0.0, 1.0],
                &[],
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
                    &[1.0, 1.0],
                &[],
                    1.0,
                )
                .unwrap();
            seen_samples[triggered.sample] = true;
            starts.push(triggered.start_norm);
            engine.note_off(0, 60, None, 1.0);
            let _ = engine.render(60, 1.0, true);
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
                &[1.0, 1.0],
                &[],
                1.0,
            )
            .unwrap();
        for _ in 0..(44_100 / 2) {
            let (left, right) = engine.render(60, 1.2, true);
            assert!(left.is_finite() && right.is_finite());
            assert!(left.abs() < 2.0 && right.abs() < 2.0);
        }
    }

    #[test]
    fn reverse_walks_the_playhead_backward() {
        let mut engine = Engine::new(44_100.0, tone_bank(), 1);
        engine
            .note_on(
                NoteOn {
                    note: 60,
                    velocity: 1.0,
                    channel: 0,
                    voice_id: None,
                    sample: Some(0),
                    start_norm: Some(0.5),
                },
                &[1.0, 1.0],
                &[
                    SliceRolls {
                        reverse: 1.0,
                        pan: 0.0,
                        octave: 0.0,
                    },
                    SliceRolls::neutral(),
                ],
                1.0,
            )
            .unwrap();
        let start = engine.playhead(0).unwrap();
        let _ = render_for(&mut engine, 8_000, 60, 1.0);
        let later = engine.playhead(0).unwrap();
        assert!(
            later < start,
            "reverse playhead {later} should sit behind {start}"
        );
    }

    #[test]
    fn octave_at_full_raises_the_pitch() {
        let mut engine = Engine::new(44_100.0, tone_bank(), 1);
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
                &[1.0, 1.0],
                &[
                    SliceRolls {
                        reverse: 0.0,
                        pan: 0.0,
                        octave: 1.0,
                    },
                    SliceRolls::neutral(),
                ],
                1.0,
            )
            .unwrap();
        let rendered = render_for(&mut engine, 44_100, 60, 1.0);
        let grain = engine.grain_len;
        let freq = frequency(&rendered[grain..grain + 12_000], 44_100.0);
        assert!(
            (freq - 440.0).abs() < 12.0,
            "an octave up from 220 Hz should be near 440, got {freq}"
        );
    }

    #[test]
    fn full_left_pan_is_silent_on_the_right() {
        let mut engine = Engine::new(44_100.0, tone_bank(), 1);
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
                &[1.0, 1.0],
                &[
                    SliceRolls {
                        reverse: 0.0,
                        pan: -1.0,
                        octave: 0.0,
                    },
                    SliceRolls::neutral(),
                ],
                1.0,
            )
            .unwrap();
        let mut left_energy = 0.0f32;
        let mut right_energy = 0.0f32;
        for _ in 0..4_000 {
            let (left, right) = engine.render(60, 1.0, true);
            left_energy += left * left;
            right_energy += right * right;
        }
        assert!(left_energy > 1.0, "left energy {left_energy}");
        assert!(
            right_energy < 1.0e-4,
            "hard left should leave the right silent, energy {right_energy}"
        );
    }

    fn hold_first(engine: &mut Engine, start: f32) {
        engine
            .note_on(
                NoteOn {
                    note: 60,
                    velocity: 1.0,
                    channel: 0,
                    voice_id: None,
                    sample: Some(0),
                    start_norm: Some(start),
                },
                &[1.0, 1.0],
                &[],
                1.0,
            )
            .unwrap();
    }

    #[test]
    fn swell_plays_backward_and_fades_in() {
        let mut engine = Engine::new(44_100.0, tone_bank(), 1);
        engine.set_global(GlobalRoll {
            swell: 1.0,
            ..GlobalRoll::off()
        });
        hold_first(&mut engine, 0.5);
        let start = engine.playhead(0).unwrap();
        let mut early = 0.0f32;
        let mut later = 0.0f32;
        for frame in 0..22_000 {
            let (left, _) = engine.render(60, 1.0, true);
            if (4_000..6_000).contains(&frame) {
                early += left.abs();
            }
            if (18_000..20_000).contains(&frame) {
                later += left.abs();
            }
        }
        let head = engine.playhead(0).unwrap();
        assert!(head < start, "a swell walks backward, {head} behind {start}");
        assert!(
            later > early * 2.0,
            "a swell gets louder, early {early} later {later}"
        );
    }

    #[test]
    fn crush_holds_samples() {
        let mut engine = Engine::new(44_100.0, tone_bank(), 1);
        engine.set_global(GlobalRoll {
            crush: 1.0,
            ..GlobalRoll::off()
        });
        hold_first(&mut engine, 0.0);
        let mut held = 0usize;
        let mut previous = 0.0f32;
        for frame in 0..12_000 {
            let (left, _) = engine.render(60, 1.0, true);
            if frame > 4_000 && left == previous {
                held += 1;
            }
            previous = left;
        }
        assert!(
            held > 4_000,
            "bitcrush should repeat samples, held {held}"
        );
    }

    #[test]
    fn crush_round_robins_three_degrees() {
        assert_eq!(crush_setup(CrushDegree::Moderate, 44_100.0), (32.0, 8));
        let (little_steps, little_hold) = crush_setup(CrushDegree::Little, 44_100.0);
        let (lot_steps, lot_hold) = crush_setup(CrushDegree::Lot, 44_100.0);
        assert!(little_steps > 32.0 && little_hold < 8);
        assert!(lot_steps < 32.0 && lot_hold > 8);

        let mut engine = Engine::new(44_100.0, tone_bank(), 1);
        engine.set_global(GlobalRoll {
            crush: 1.0,
            ..GlobalRoll::off()
        });
        let mut dealt = [0.0f32; 3];
        for index in 0..3 {
            engine.note_on(
                NoteOn {
                    note: 60 + index as u8,
                    velocity: 1.0,
                    channel: 0,
                    voice_id: Some(index as i32),
                    sample: Some(0),
                    start_norm: Some(0.0),
                },
                &[1.0],
                &[SliceRolls::neutral()],
                1.0,
            );
            dealt[index] = engine
                .voices
                .iter()
                .find(|voice| voice.voice_id == Some(index as i32))
                .unwrap()
                .crush_steps;
        }
        dealt.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(dealt, [lot_steps, 32.0, little_steps]);
    }

    #[test]
    fn smear_deals_three_degrees_and_leaves_the_recording() {
        let little = (44_100.0_f64 * 0.62).round() as usize;
        let moderate = (44_100.0_f64 * 0.78).round() as usize;
        let lot = (44_100.0_f64 * 0.95).round() as usize;
        let mut engine = Engine::new(44_100.0, tone_bank(), 1);
        engine.set_global(GlobalRoll {
            smear: 1.0,
            ..GlobalRoll::off()
        });
        let mut dealt = [0usize; 3];
        for index in 0..3 {
            engine.note_on(
                NoteOn {
                    note: 60 + index as u8,
                    velocity: 1.0,
                    channel: 0,
                    voice_id: Some(index as i32),
                    sample: Some(0),
                    start_norm: Some(0.0),
                },
                &[1.0],
                &[SliceRolls::neutral()],
                1.0,
            );
            dealt[index] = engine
                .voices
                .iter()
                .find(|voice| voice.voice_id == Some(index as i32))
                .unwrap()
                .smear_len;
        }
        dealt.sort_unstable();
        assert_eq!(dealt, [little, moderate, lot]);

        let mut clean = Engine::new(44_100.0, tone_bank(), 3);
        let mut smeared = Engine::new(44_100.0, tone_bank(), 3);
        smeared.set_global(GlobalRoll {
            smear: 1.0,
            ..GlobalRoll::off()
        });
        let request = NoteOn {
            note: 60,
            velocity: 1.0,
            channel: 0,
            voice_id: None,
            sample: Some(0),
            start_norm: Some(0.2),
        };
        clean
            .note_on(request, &[1.0], &[SliceRolls::neutral()], 1.0)
            .unwrap();
        smeared
            .note_on(
                NoteOn {
                    note: 60,
                    velocity: 1.0,
                    channel: 0,
                    voice_id: None,
                    sample: Some(0),
                    start_norm: Some(0.2),
                },
                &[1.0],
                &[SliceRolls::neutral()],
                1.0,
            )
            .unwrap();
        let plain = render_for(&mut clean, 16_000, 60, 1.0);
        let cloud = render_for(&mut smeared, 16_000, 60, 1.0);
        assert!(
            correlation(&plain, &cloud) < 0.92,
            "smear should pull the note off the straight recording"
        );
    }

    #[test]
    fn stutter_chops_the_note() {
        let mut engine = Engine::new(44_100.0, tone_bank(), 1);
        engine.set_clock(120.0);
        engine.set_global(GlobalRoll {
            stutter: 1.0,
            ..GlobalRoll::off()
        });
        hold_first(&mut engine, 0.0);
        let mut quiet = 0usize;
        let mut heard = 0usize;
        for frame in 0..22_050 {
            let (left, _) = engine.render(60, 1.0, true);
            if frame < 4_000 {
                continue;
            }
            if left.abs() < 1.0e-4 {
                quiet += 1;
            } else {
                heard += 1;
            }
        }
        assert!(heard > 1_000, "the stutter should still sound, heard {heard}");
        assert!(
            quiet > heard / 2,
            "the stutter should cut the note, quiet {quiet} heard {heard}"
        );
    }

    #[test]
    fn low_pass_dulling_and_high_pass_thinning() {
        let mut low_state = 0.0f32;
        let mut high_state = 0.0f32;
        let coeff = cutoff_coeff(44_100.0, 400.0);
        let mut dry = 0.0f32;
        let mut low_energy = 0.0f32;
        let mut high_energy = 0.0f32;
        for i in 0..4_000 {
            let sample = (std::f32::consts::TAU * 3_000.0 * i as f32 / 44_100.0).sin();
            let low = one_pole(&mut low_state, sample, coeff);
            let low_only = one_pole(&mut high_state, sample, coeff);
            dry += sample * sample;
            low_energy += low * low;
            high_energy += (sample - low_only) * (sample - low_only);
        }
        assert!(
            low_energy < dry * 0.25,
            "a low pass should dull 3 kHz, low {low_energy} dry {dry}"
        );
        assert!(
            high_energy > dry * 0.5,
            "a high pass should keep 3 kHz, high {high_energy} dry {dry}"
        );
    }
}
