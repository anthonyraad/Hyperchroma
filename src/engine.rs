use crate::bank::Bank;

const MAX_VOICES: usize = 16;
const MAX_GRAINS: usize = 3;
const SMEAR_GRAINS: usize = 6;
const MAX_SAMPLES: usize = 32;
const ESCAPE_COMBS: usize = 4;
const ESCAPE_REGIONS: usize = 8;

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

/// Chances that apply to every note, decided once when the note starts.
#[derive(Clone, Copy)]
pub struct GlobalRoll {
    /// Chance the note plays backward and fades in.
    pub swell: f32,
    /// Chance the note is bitcrushed.
    pub crush: f32,
    /// Chance the note is randomly low-passed or high-passed.
    pub pass: f32,
    /// Chance the note repeats its last division as a roll.
    pub stutter: f32,
    /// Chance the note's slices scatter around the playhead.
    pub smear: f32,
    /// Chance the note is sent through the spectral gate.
    pub splatter: f32,
    /// Chance the note is set into a short, distant reverb.
    pub escape: f32,
    /// Chance the note eases toward the other knobs, or from them back to dry.
    pub drift: f32,
    /// Chance the chosen color plays its B-side. `0` is always A, `1` is always B.
    pub b_side: f32,
    /// When on, colors with Spread at Center pick a random stereo position per note.
    pub stereoentropic: bool,
    /// Reverse chance used only when a color's Reverse knob is at zero.
    pub reverse_floor: f32,
}

impl GlobalRoll {
    pub const fn off() -> Self {
        Self {
            swell: 0.0,
            crush: 0.0,
            pass: 0.0,
            stutter: 0.0,
            smear: 0.0,
            splatter: 0.0,
            escape: 0.0,
            drift: 0.0,
            b_side: 0.0,
            stereoentropic: false,
            reverse_floor: 0.0,
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
    /// How this recording meets the global effects. `0.5` leaves them as set.
    /// `0` is immune. `1` catches at least one effect that is turned up.
    pub charm: f32,
}

impl SliceRolls {
    pub const fn neutral() -> Self {
        Self {
            reverse: 0.0,
            pan: 0.0,
            octave: 0.0,
            charm: 0.5,
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
    /// When true, this voice reads the color's B-side recording.
    side_b: bool,
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
    /// Stereo placement for the note, `-1` hard left to `1` hard right. `0` keeps the recording.
    pan: f32,
    /// Where this note's stereo started, and where Drift walks it.
    pan_from: f32,
    pan_to: f32,
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
    /// Captured roll length in output samples. `0` when the note is not repeating.
    stutter_len: u32,
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
    drift: bool,
    drift_from_center: bool,
    drift_pos: u32,
    drift_len: u32,
    /// Eased position of the current drift, `0` dry and `1` at the destinations.
    drift_now: f32,
    dest_swell: f32,
    dest_crush: f32,
    dest_pass: f32,
    dest_stutter: f32,
    dest_smear: f32,
    dest_splatter: f32,
    dest_escape: f32,
    crush_target_steps: f32,
    crush_target_len: u32,
    pass_hz: f32,
    escape: bool,
    escape_wet: f32,
    escape_dry: f32,
    escape_fb: [f32; ESCAPE_COMBS],
    escape_damp: [f32; ESCAPE_COMBS],
    escape_damp_coeff: f32,
    escape_comb_len: [usize; ESCAPE_COMBS],
    escape_comb_pos: [usize; ESCAPE_COMBS],
    escape_pre_len: usize,
    escape_pre_pos: usize,
    escape_ap_len: [usize; 2],
    escape_ap_pos: [usize; 2],
    escape_ap_fb: f32,
    escape_side_len: usize,
    escape_side_pos: usize,
    escape_lp: f32,
    escape_lp_coeff: f32,
    /// Output frames to keep the voice alive after the envelope, so the wash can finish.
    escape_tail: u32,
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
            side_b: false,
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
            pan: 0.0,
            pan_from: 0.0,
            pan_to: 0.0,
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
            stutter_len: 0,
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
            drift: false,
            drift_from_center: true,
            drift_pos: 0,
            drift_len: 0,
            drift_now: 1.0,
            dest_swell: 0.0,
            dest_crush: 0.0,
            dest_pass: 0.0,
            dest_stutter: 0.0,
            dest_smear: 0.0,
            dest_splatter: 0.0,
            dest_escape: 0.0,
            crush_target_steps: 32.0,
            crush_target_len: 1,
            pass_hz: 0.0,
            escape: false,
            escape_wet: 0.0,
            escape_dry: 1.0,
            escape_fb: [0.0; ESCAPE_COMBS],
            escape_damp: [0.0; ESCAPE_COMBS],
            escape_damp_coeff: 0.0,
            escape_comb_len: [1; ESCAPE_COMBS],
            escape_comb_pos: [0; ESCAPE_COMBS],
            escape_pre_len: 1,
            escape_pre_pos: 0,
            escape_ap_len: [1; 2],
            escape_ap_pos: [0; 2],
            escape_ap_fb: 0.0,
            escape_side_len: 1,
            escape_side_pos: 0,
            escape_lp: 0.0,
            escape_lp_coeff: 1.0,
            escape_tail: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PassMode {
    Off,
    Low,
    High,
}

#[derive(Clone, Copy)]
enum Weather {
    Swell,
    Crush,
    Pass,
    Stutter,
    Smear,
    Splatter,
    Escape,
    Drift,
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
    escape_bag: [EscapeDegree; 3],
    escape_left: u8,
    /// One captured roll per voice, laid out as `voice * stutter_cap`.
    stutter_l: Vec<f32>,
    stutter_r: Vec<f32>,
    stutter_cap: usize,
    /// Delay lines for Escape, `voice * ESCAPE_REGIONS * escape_region`.
    escape_buf: Vec<f32>,
    escape_region: usize,
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
            escape_bag: [EscapeDegree::Little, EscapeDegree::Moderate, EscapeDegree::Lot],
            escape_left: 0,
            stutter_l: Vec::new(),
            stutter_r: Vec::new(),
            stutter_cap: 0,
            escape_buf: Vec::new(),
            escape_region: 1,
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
        self.stutter_cap = stutter_cap_for(sample_rate);
        let cells = self.stutter_cap.saturating_mul(MAX_VOICES);
        self.stutter_l = vec![0.0; cells];
        self.stutter_r = vec![0.0; cells];
        self.escape_region = escape_region_for(sample_rate);
        let escape_cells = self
            .escape_region
            .saturating_mul(ESCAPE_REGIONS)
            .saturating_mul(MAX_VOICES);
        self.escape_buf = vec![0.0; escape_cells];
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
        let side_b = self.bank.has_b(sample_index)
            && self.rng.unit() < self.global.b_side.clamp(0.0, 1.0);
        let pcm = self.bank.pcm(sample_index, side_b)?;
        let frames = pcm.frames.max(1) as f64;
        let source_rate = pcm.sample_rate as f64;
        let start = match request.start_norm {
            Some(norm) => norm.clamp(0.0, 0.999_999) as f64 * frames,
            None => self.rng.unit() as f64 * frames,
        };
        let roll = rolls.get(sample_index).copied().unwrap_or(SliceRolls::neutral());
        let reverse_chance = if roll.reverse.abs() < 0.005 {
            self.global.reverse_floor
        } else {
            roll.reverse
        };
        let mut reverse = self.rng.unit() < reverse_chance.clamp(0.0, 1.0);
        let pan = if self.global.stereoentropic && roll.pan.abs() < 0.005 {
            self.rng.unit() * 2.0 - 1.0
        } else {
            rolled_pan(&mut self.rng, roll.pan)
        };
        let octave_up = self.rng.unit() < roll.octave.clamp(0.0, 1.0);
        let charm = if roll.charm.is_finite() {
            roll.charm.clamp(0.0, 1.0)
        } else {
            0.5
        };
        let damp = if charm < 0.5 { charm / 0.5 } else { 1.0 };
        let chance = |value: f32| value * damp;
        let mut drift = fired(&mut self.rng, chance(self.global.drift));
        let mut drift_from_center = true;
        let mut drift_len = 0u32;
        if drift {
            drift_from_center = self.rng.unit() < 0.5;
            let seconds = 2.15 + self.rng.unit() * 1.35;
            drift_len = (self.sample_rate * seconds).round().max(8.0) as u32;
        }
        let knob = |value: f32| value.clamp(0.0, 1.0);
        let mut swell = if drift {
            false
        } else {
            fired(&mut self.rng, chance(self.global.swell))
        };
        let mut dest_swell = if drift {
            knob(self.global.swell)
        } else if swell {
            1.0
        } else {
            0.0
        };
        if drift && dest_swell > 0.2 {
            reverse = true;
        } else if swell {
            reverse = true;
        }
        let mut crush_on = if drift {
            knob(self.global.crush) > 0.001
        } else {
            fired(&mut self.rng, chance(self.global.crush))
        };
        let (mut crush_steps, mut crush_len) = if crush_on {
            crush_setup(self.take_crush(), self.sample_rate)
        } else {
            (32.0, 1)
        };
        let mut dest_crush = if drift {
            knob(self.global.crush)
        } else if crush_on {
            1.0
        } else {
            0.0
        };
        let (mut pass, mut pass_hz, mut pass_coeff) = if drift {
            if knob(self.global.pass) > 0.001 {
                let (mode, hz) = self.pick_pass();
                let opened = if drift_from_center {
                    0.0
                } else {
                    knob(self.global.pass)
                };
                let hz_now = drifted_hz(mode, hz, opened);
                (mode, hz, cutoff_coeff(self.sample_rate, hz_now))
            } else {
                (PassMode::Off, 0.0, 0.0)
            }
        } else {
            let (mode, coeff) = self.roll_pass(chance(self.global.pass));
            (mode, 0.0, coeff)
        };
        let mut dest_pass = if drift {
            knob(self.global.pass)
        } else if pass != PassMode::Off {
            1.0
        } else {
            0.0
        };
        let mut stutter = if drift {
            if knob(self.global.stutter) > 0.001 {
                self.pick_stutter()
            } else {
                StutterDiv::Off
            }
        } else {
            self.roll_stutter(chance(self.global.stutter))
        };
        let mut dest_stutter = if drift {
            knob(self.global.stutter)
        } else if stutter != StutterDiv::Off {
            1.0
        } else {
            0.0
        };
        let smear = if drift {
            knob(self.global.smear) > 0.001
        } else {
            fired(&mut self.rng, chance(self.global.smear))
        };
        let mut smear_tone = if smear {
            Some(smear_setup(self.take_smear()))
        } else {
            None
        };
        let mut dest_smear = if drift {
            knob(self.global.smear)
        } else if smear {
            1.0
        } else {
            0.0
        };
        let splatter = if drift {
            knob(self.global.splatter) > 0.001
        } else {
            fired(&mut self.rng, chance(self.global.splatter))
        };
        let mut dest_splatter = if drift {
            knob(self.global.splatter)
        } else if splatter {
            1.0
        } else {
            0.0
        };
        let mut escape = if drift {
            knob(self.global.escape) > 0.001
        } else {
            fired(&mut self.rng, chance(self.global.escape))
        };
        let mut escape_tone = if escape {
            Some(escape_setup(self.take_escape()))
        } else {
            None
        };
        let mut dest_escape = if drift {
            knob(self.global.escape)
        } else if escape {
            1.0
        } else {
            0.0
        };
        let caught = swell
            || crush_on
            || pass != PassMode::Off
            || stutter != StutterDiv::Off
            || smear
            || splatter
            || escape
            || (drift
                && (self.global.swell > 0.001
                    || self.global.crush > 0.001
                    || self.global.pass > 0.001
                    || self.global.stutter > 0.001
                    || self.global.smear > 0.001
                    || self.global.splatter > 0.001
                    || self.global.escape > 0.001));
        if charm > 0.5 && !caught {
            if let Some(kind) = self.charm_catch(charm) {
                match kind {
                    Weather::Swell => {
                        swell = true;
                        reverse = true;
                        dest_swell = 1.0;
                    }
                    Weather::Crush => {
                        crush_on = true;
                        let (steps, hold) = crush_setup(self.take_crush(), self.sample_rate);
                        crush_steps = steps;
                        crush_len = hold;
                        dest_crush = 1.0;
                    }
                    Weather::Pass => {
                        let (mode, hz) = self.pick_pass();
                        pass = mode;
                        pass_hz = hz;
                        pass_coeff = cutoff_coeff(self.sample_rate, hz);
                        dest_pass = 1.0;
                    }
                    Weather::Stutter => {
                        stutter = self.pick_stutter();
                        dest_stutter = 1.0;
                    }
                    Weather::Smear => {
                        smear_tone = Some(smear_setup(self.take_smear()));
                        dest_smear = 1.0;
                    }
                    Weather::Splatter => {
                        dest_splatter = 1.0;
                    }
                    Weather::Escape => {
                        escape = true;
                        escape_tone = Some(escape_setup(self.take_escape()));
                        dest_escape = 1.0;
                    }
                    Weather::Drift => {
                        drift = true;
                        drift_from_center = self.rng.unit() < 0.5;
                        let seconds = 2.15 + self.rng.unit() * 1.35;
                        drift_len = (self.sample_rate * seconds).round().max(8.0) as u32;
                        dest_swell = knob(self.global.swell);
                        dest_crush = knob(self.global.crush);
                        dest_pass = knob(self.global.pass);
                        dest_stutter = knob(self.global.stutter);
                        dest_smear = knob(self.global.smear);
                        dest_splatter = knob(self.global.splatter);
                        dest_escape = knob(self.global.escape);
                        if dest_swell > 0.2 {
                            reverse = true;
                        }
                        if dest_crush > 0.001 {
                            crush_on = true;
                            let (steps, hold) = crush_setup(self.take_crush(), self.sample_rate);
                            crush_steps = steps;
                            crush_len = hold;
                        }
                        if dest_pass > 0.001 {
                            let (mode, hz) = self.pick_pass();
                            let opened = if drift_from_center { 0.0 } else { dest_pass };
                            pass = mode;
                            pass_hz = hz;
                            pass_coeff = cutoff_coeff(self.sample_rate, drifted_hz(mode, hz, opened));
                        }
                        if dest_stutter > 0.001 {
                            stutter = self.pick_stutter();
                        }
                        if dest_smear > 0.001 {
                            smear_tone = Some(smear_setup(self.take_smear()));
                        }
                        if dest_escape > 0.001 {
                            escape = true;
                            escape_tone = Some(escape_setup(self.take_escape()));
                        }
                    }
                }
            }
        }
        let mut attack_samples = attack_samples;
        if drift && drift_from_center && dest_swell > 0.2 {
            attack_samples = attack_samples.max(self.sample_rate * SWELL_SECONDS * dest_swell);
        } else if swell {
            attack_samples = attack_samples.max(self.sample_rate * SWELL_SECONDS);
        }
        // Stereo only walks when the note already has a seat: Spread off center, or
        // Stereoentropic (which gives a centered Spread a random start).
        let pan_drifts = drift && (roll.pan.abs() >= 0.005 || self.global.stereoentropic);
        let pan_to = if pan_drifts {
            if self.rng.unit() < 0.5 {
                -1.0
            } else {
                1.0
            }
        } else {
            pan
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
        voice.side_b = side_b;
        voice.playhead = start;
        voice.serial = serial;
        voice.until_next = 0;
        voice.reverse = reverse;
        voice.pan = pan;
        voice.pan_from = pan;
        voice.pan_to = pan_to;
        voice.octave_up = octave_up;
        voice.crush = crush_on;
        voice.crush_steps = crush_steps;
        voice.crush_len = crush_len;
        voice.crush_target_steps = crush_steps;
        voice.crush_target_len = crush_len;
        voice.pass = pass;
        voice.pass_hz = pass_hz;
        voice.pass_coeff = pass_coeff;
        voice.stutter = stutter;
        voice.stutter_len = self.stutter_period(stutter);
        voice.stutter_pos = 0;
        voice.drift = drift;
        voice.drift_from_center = drift_from_center;
        voice.drift_pos = 0;
        voice.drift_len = drift_len;
        voice.drift_now = if drift && drift_from_center { 0.0 } else { 1.0 };
        voice.dest_swell = dest_swell;
        voice.dest_crush = dest_crush;
        voice.dest_pass = dest_pass;
        voice.dest_stutter = dest_stutter;
        voice.dest_smear = dest_smear;
        voice.dest_splatter = dest_splatter;
        voice.dest_escape = dest_escape;
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
        if let Some(tone) = escape_tone {
            self.arm_escape(&mut voice, &tone);
        }
        set_env(&mut voice, velocity, attack_samples);
        self.voices[slot] = voice;
        if escape {
            self.clear_escape(slot);
        }

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
        let (dry_l, dry_r, wet_l, wet_r) = self.render_buses(root, tempo_ratio, keytrack);
        (dry_l + wet_l, dry_r + wet_r)
    }

    /// Dry notes, then notes that caught Splatter. The spectral gate runs once on the wet bus.
    pub fn render_buses(
        &mut self,
        root: i32,
        tempo_ratio: f32,
        keytrack: bool,
    ) -> (f32, f32, f32, f32) {
        let tempo_ratio = if tempo_ratio.is_finite() {
            tempo_ratio.clamp(0.05, 8.0)
        } else {
            1.0
        };
        let mut dry_l = 0.0f32;
        let mut dry_r = 0.0f32;
        let mut wet_l = 0.0f32;
        let mut wet_r = 0.0f32;
        for index in 0..MAX_VOICES {
            if !self.voices[index].active {
                continue;
            }
            let (voice_l, voice_r) = self.render_voice(index, root, tempo_ratio, keytrack);
            let send = effect_amount(&self.voices[index], self.voices[index].dest_splatter);
            let keep = 1.0 - send;
            dry_l += voice_l * keep;
            dry_r += voice_r * keep;
            wet_l += voice_l * send;
            wet_r += voice_r * send;
        }
        (finite_sample(dry_l), finite_sample(dry_r), finite_sample(wet_l), finite_sample(wet_r))
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

    /// Colors whose newest sounding voice is on the B-side.
    pub fn b_side_mask(&self) -> u32 {
        let mut mask = 0u32;
        let mut serials = [0u32; 32];
        let mut seen = [false; 32];
        for voice in &self.voices {
            if !voice.active || voice.sample >= 32 {
                continue;
            }
            let slot = voice.sample;
            if seen[slot] && voice.serial < serials[slot] {
                continue;
            }
            seen[slot] = true;
            serials[slot] = voice.serial;
            if voice.side_b {
                mask |= 1 << slot;
            } else {
                mask &= !(1 << slot);
            }
        }
        mask
    }

    /// Normalized playhead of the newest sounding voice in each slot, in parts per million.
    /// Silent slots are `u32::MAX`. `weather` is the strongest global-effect amount for that
    /// voice, also in parts per million. `serials` is scratch and can be discarded.
    pub fn write_playheads(&self, heads: &mut [u32], weather: &mut [u32], serials: &mut [u32]) {
        let count = heads.len().min(weather.len()).min(serials.len());
        for index in 0..count {
            heads[index] = u32::MAX;
            weather[index] = 0;
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
            let Some(pcm) = self.bank.pcm(slot, voice.side_b) else {
                continue;
            };
            if pcm.frames == 0 {
                continue;
            }
            let norm = (voice.playhead / pcm.frames as f64).rem_euclid(1.0);
            heads[slot] = ((norm * 1_000_000.0).round() as u32).min(999_999);
            weather[slot] = (voice_weather_intensity(voice) * 1_000_000.0).round() as u32;
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
        let side_b = self.voices[index].side_b;
        let (frames, sample_rate) = match self.bank.pcm(sample_index, side_b) {
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
        // A leap keeps the playhead with the doubled read, so the slices overlap
        // on the waveform. Leaving the playhead at the tempo puts each new slice
        // about 40 ms away from the one already sounding, and that overlap combs.
        let schedule = if octave_up { pitch } else { tempo_ratio };

        if self.voices[index].smear && !self.voices[index].drift {
            return self.render_smear(
                index,
                sample_index,
                frames as f64,
                read_inc,
                playhead,
                schedule,
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
            let origin = if (pitch - schedule).abs() <= 0.000_8 {
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
            let advance = hop as f64 * schedule as f64 * sr_ratio * direction;
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
            let (sample_l, sample_r) = self
                .bank
                .pcm(sample_index, side_b)
                .map(|pcm| pcm.read(position))
                .unwrap_or((0.0, 0.0));
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
        self.apply_drift(index);
        let smear_amount = effect_amount(&self.voices[index], self.voices[index].dest_smear);
        if smear_amount > 0.001 {
            let (cloud_l, cloud_r) =
                self.smear_sum(index, sample_index, frames as f64, read_inc, playhead);
            left = left * (1.0 - smear_amount) + cloud_l * smear_amount;
            right = right * (1.0 - smear_amount) + cloud_r * smear_amount;
        }
        self.finish_voice(index, left, right, pan)
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
        schedule: f32,
        sr_ratio: f64,
        direction: f64,
        pan: f32,
    ) -> (f32, f32) {
        playhead = (playhead + schedule as f64 * sr_ratio * direction).rem_euclid(frames);
        let (left, right) = self.smear_sum(index, sample_index, frames, read_inc, playhead);
        self.voices[index].playhead = playhead;
        self.apply_drift(index);
        self.finish_voice(index, left, right, pan)
    }

    /// Scattered slices around `playhead`. Does not move the playhead.
    fn smear_sum(
        &mut self,
        index: usize,
        sample_index: usize,
        frames: f64,
        read_inc: f64,
        playhead: f64,
    ) -> (f32, f32) {
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
            let side_b = self.voices[index].side_b;
            let (sample_l, sample_r) = self
                .bank
                .pcm(sample_index, side_b)
                .map(|pcm| pcm.read(position))
                .unwrap_or((0.0, 0.0));
            let level = window * grain.gain;
            let steer = grain.pan.clamp(-1.0, 1.0);
            left += sample_l * level * (1.0 - steer.max(0.0));
            right += sample_r * level * (1.0 + steer.min(0.0));
            grain.age += 1;
        }

        self.voices[index].smear_grains = grains;
        self.voices[index].smear_wait = wait;
        (left, right)
    }

    fn finish_voice(&mut self, index: usize, left: f32, right: f32, pan: f32) -> (f32, f32) {
        let (left, right) = place(left, right, pan);
        let (left, right) = self.color_voice(index, left, right);
        let stutter_amount = effect_amount(&self.voices[index], self.voices[index].dest_stutter);
        let (left, right) = self.stutter_repeat(index, left, right, stutter_amount);
        let voice = &self.voices[index];
        let bloom = if voice.drift && voice.drift_from_center {
            1.0 - voice.dest_swell.clamp(0.0, 1.0) * 0.72 * (1.0 - voice.drift_now)
        } else {
            1.0
        };
        let env = voice.env;
        let (left, right) = (left * env * bloom, right * env * bloom);
        self.escape_mix(index, left, right)
    }

    fn roll_pass(&mut self, chance: f32) -> (PassMode, f32) {
        if !fired(&mut self.rng, chance) {
            return (PassMode::Off, 0.0);
        }
        let (mode, hz) = self.pick_pass();
        (mode, cutoff_coeff(self.sample_rate, hz))
    }

    fn pick_pass(&mut self) -> (PassMode, f32) {
        let low = self.rng.unit() < 0.5;
        let degree = self.rng.unit();
        if low {
            (PassMode::Low, 280.0 * (1_200.0_f32 / 280.0).powf(degree))
        } else {
            (
                PassMode::High,
                800.0 * (2_400.0_f32 / 800.0).powf(degree),
            )
        }
    }

    fn roll_stutter(&mut self, chance: f32) -> StutterDiv {
        if !fired(&mut self.rng, chance) {
            return StutterDiv::Off;
        }
        self.pick_stutter()
    }

    /// Above noon, a dry roll can still catch one enabled global effect.
    /// The catch is weighted by how far each knob is turned up.
    fn charm_catch(&mut self, charm: f32) -> Option<Weather> {
        let lean = ((charm - 0.5) / 0.5).clamp(0.0, 1.0);
        if lean <= 0.0 {
            return None;
        }
        if lean < 1.0 && !fired(&mut self.rng, lean) {
            return None;
        }
        let weights = [
            (Weather::Swell, self.global.swell),
            (Weather::Crush, self.global.crush),
            (Weather::Pass, self.global.pass),
            (Weather::Stutter, self.global.stutter),
            (Weather::Smear, self.global.smear),
            (Weather::Splatter, self.global.splatter),
            (Weather::Escape, self.global.escape),
            (Weather::Drift, self.global.drift),
        ];
        let total: f32 = weights
            .iter()
            .map(|(_, weight)| weight.max(0.0))
            .filter(|weight| *weight > 0.001)
            .sum();
        if total <= 0.0 {
            return None;
        }
        let mut pick = self.rng.unit() * total;
        let mut chosen = None;
        for (kind, weight) in weights {
            let weight = weight.max(0.0);
            if weight <= 0.001 {
                continue;
            }
            chosen = Some(kind);
            pick -= weight;
            if pick <= 0.0 {
                return Some(kind);
            }
        }
        chosen
    }

    fn pick_stutter(&mut self) -> StutterDiv {
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

    fn stutter_period(&self, div: StutterDiv) -> u32 {
        let beats = match div {
            StutterDiv::Off => return 0,
            StutterDiv::Eighth => 0.5,
            StutterDiv::Sixteenth => 0.25,
            StutterDiv::SixteenthTriplet => 1.0 / 6.0,
            StutterDiv::ThirtySecond => 0.125,
        };
        let cap = self.stutter_cap.max(2) as f32;
        ((beats * 60.0 / self.clock_bpm) * self.sample_rate)
            .round()
            .clamp(2.0, cap) as u32
    }

    /// Capture one division, then blend the live note into that repeating slice.
    /// `amount` is `1` for a full roll and `0` for the untouched note.
    fn stutter_repeat(&mut self, index: usize, left: f32, right: f32, amount: f32) -> (f32, f32) {
        let period = self.voices[index].stutter_len as usize;
        if period < 2 || self.stutter_cap < period {
            return (left, right);
        }
        let base = index * self.stutter_cap;
        let pos = self.voices[index].stutter_pos as usize;
        if pos < period {
            self.stutter_l[base + pos] = left;
            self.stutter_r[base + pos] = right;
            self.voices[index].stutter_pos = (pos + 1) as u32;
            return (left, right);
        }
        let cursor = (pos - period) % period;
        let fade = repeat_fade(self.sample_rate, period);
        let loop_l = loop_sample(&self.stutter_l[base..base + period], cursor, fade);
        let loop_r = loop_sample(&self.stutter_r[base..base + period], cursor, fade);
        self.voices[index].stutter_pos = self.voices[index].stutter_pos.wrapping_add(1);
        let amount = amount.clamp(0.0, 1.0);
        if amount >= 0.999 {
            (loop_l, loop_r)
        } else if amount <= 0.001 {
            (left, right)
        } else {
            (
                left * (1.0 - amount) + loop_l * amount,
                right * (1.0 - amount) + loop_r * amount,
            )
        }
    }

    fn apply_drift(&mut self, index: usize) {
        let sample_rate = self.sample_rate;
        let voice = &mut self.voices[index];
        if !voice.drift || voice.drift_len == 0 {
            voice.drift_now = 1.0;
            return;
        }
        let t = (voice.drift_pos as f32 / voice.drift_len as f32).min(1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        if voice.drift_pos < voice.drift_len {
            voice.drift_pos += 1;
        }
        voice.drift_now = if voice.drift_from_center {
            eased
        } else {
            1.0 - eased
        };
        if (voice.pan_from - voice.pan_to).abs() > 1.0e-4 {
            voice.pan = voice.pan_from + (voice.pan_to - voice.pan_from) * eased;
        }
        if voice.dest_crush > 0.0 {
            let amount = (voice.dest_crush * voice.drift_now).clamp(0.0, 1.0);
            voice.crush = amount > 0.035;
            voice.crush_steps =
                voice.crush_target_steps + (96.0 - voice.crush_target_steps) * (1.0 - amount);
            let hold = 1.0 + (voice.crush_target_len.saturating_sub(1) as f32) * amount;
            voice.crush_len = hold.round().clamp(1.0, 96.0) as u32;
        }
        if voice.pass != PassMode::Off && voice.dest_pass > 0.0 {
            let depth = (voice.dest_pass * voice.drift_now).clamp(0.0, 1.0);
            let hz = drifted_hz(voice.pass, voice.pass_hz, depth);
            voice.pass_coeff = cutoff_coeff(sample_rate, hz);
        }
    }

    fn arm_escape(&mut self, voice: &mut Voice, tone: &EscapeTone) {
        let sample_rate = self.sample_rate;
        let region = self.escape_region.max(2);
        let samples = |seconds: f32| -> usize {
            (sample_rate * seconds).round().clamp(1.0, (region - 1) as f32) as usize
        };
        let comb_seconds = [0.0113_f32, 0.0137, 0.0169, 0.0197];
        for (index, seconds) in comb_seconds.iter().enumerate() {
            let len = samples(*seconds);
            voice.escape_comb_len[index] = len;
            voice.escape_comb_pos[index] = 0;
            voice.escape_fb[index] = comb_feedback(len, sample_rate, tone.rt60);
        }
        voice.escape = true;
        voice.escape_wet = tone.wet;
        voice.escape_dry = tone.dry;
        voice.escape_damp = [0.0; ESCAPE_COMBS];
        voice.escape_damp_coeff = (tone.tone_hz / 8_000.0).clamp(0.08, 0.8);
        voice.escape_pre_len = samples(tone.predelay);
        voice.escape_pre_pos = 0;
        voice.escape_ap_len = [samples(0.0053), samples(0.0017)];
        voice.escape_ap_pos = [0, 0];
        voice.escape_ap_fb = 0.62;
        voice.escape_side_len = samples(0.0008);
        voice.escape_side_pos = 0;
        voice.escape_lp = 0.0;
        voice.escape_lp_coeff = cutoff_coeff(sample_rate, tone.tone_hz);
        voice.escape_tail = (tone.rt60 * sample_rate * 1.8).round().clamp(1.0, sample_rate * 0.45) as u32;
    }

    fn clear_escape(&mut self, index: usize) {
        let region = self.escape_region.max(1);
        let start = index.saturating_mul(ESCAPE_REGIONS).saturating_mul(region);
        let end = start.saturating_add(ESCAPE_REGIONS.saturating_mul(region));
        if end <= self.escape_buf.len() {
            self.escape_buf[start..end].fill(0.0);
        }
    }

    fn escape_mix(&mut self, index: usize, left: f32, right: f32) -> (f32, f32) {
        if !self.voices[index].escape {
            return (left, right);
        }
        let amount = effect_amount(&self.voices[index], self.voices[index].dest_escape);
        let region = self.escape_region.max(1);
        let start = index * ESCAPE_REGIONS * region;
        let end = start + ESCAPE_REGIONS * region;
        if end > self.escape_buf.len() {
            return (left, right);
        }

        let voice = &self.voices[index];
        let mut comb_pos = voice.escape_comb_pos;
        let comb_len = voice.escape_comb_len;
        let feedback = voice.escape_fb;
        let mut damp = voice.escape_damp;
        let damp_coeff = voice.escape_damp_coeff;
        let mut pre_pos = voice.escape_pre_pos;
        let pre_len = voice.escape_pre_len;
        let mut ap_pos = voice.escape_ap_pos;
        let ap_len = voice.escape_ap_len;
        let ap_fb = voice.escape_ap_fb;
        let mut side_pos = voice.escape_side_pos;
        let side_len = voice.escape_side_len;
        let mut lp = voice.escape_lp;
        let lp_coeff = voice.escape_lp_coeff;
        let wet_gain = voice.escape_wet * amount;
        let dry_gain = 1.0 + (voice.escape_dry - 1.0) * amount;

        let input = (left + right) * 0.5 * amount;
        let buf = &mut self.escape_buf[start..end];
        let pre = delay_tick(&mut buf[0..region], &mut pre_pos, pre_len, input);
        let mut tank = 0.0f32;
        for comb in 0..ESCAPE_COMBS {
            let offset = (comb + 1) * region;
            tank += comb_tick(
                &mut buf[offset..offset + region],
                &mut comb_pos[comb],
                comb_len[comb],
                &mut damp[comb],
                damp_coeff,
                feedback[comb],
                pre,
            );
        }
        tank = tank * 0.25 * 0.82 + pre * 0.18;
        for ap in 0..2 {
            let offset = (ESCAPE_COMBS + 1 + ap) * region;
            tank = allpass_tick(
                &mut buf[offset..offset + region],
                &mut ap_pos[ap],
                ap_len[ap],
                ap_fb,
                tank,
            );
        }
        let wet_l = one_pole(&mut lp, tank, lp_coeff);
        let side_offset = (ESCAPE_COMBS + 3) * region;
        let wet_r = delay_tick(
            &mut buf[side_offset..side_offset + region],
            &mut side_pos,
            side_len,
            wet_l,
        );

        let voice = &mut self.voices[index];
        voice.escape_comb_pos = comb_pos;
        voice.escape_damp = damp;
        voice.escape_pre_pos = pre_pos;
        voice.escape_ap_pos = ap_pos;
        voice.escape_side_pos = side_pos;
        voice.escape_lp = lp;
        (
            left * dry_gain + wet_l * wet_gain,
            right * dry_gain + wet_r * wet_gain,
        )
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

        let side_b = self
            .voices
            .iter()
            .find(|voice| voice.active && voice.sample == sample_index)
            .map(|voice| voice.side_b)
            .unwrap_or(false);
        let Some(pcm) = self.bank.pcm(sample_index, side_b) else {
            return ideal;
        };
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

fn rolled_pan(rng: &mut Rng, pan: f32) -> f32 {
    let pan = pan.clamp(-1.0, 1.0);
    if pan < 0.0 && rng.unit() < -pan {
        -1.0
    } else if pan > 0.0 && rng.unit() < pan {
        1.0
    } else {
        0.0
    }
}

fn finite_sample(sample: f32) -> f32 {
    if sample.is_finite() {
        sample
    } else {
        0.0
    }
}

fn place(left: f32, right: f32, pan: f32) -> (f32, f32) {
    let pan = if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    if pan.abs() < 1.0e-4 {
        return (left, right);
    }
    let mid = 0.5 * (left + right);
    // Constant-power: -1 all left, 1 all right. Matches the old hard L/R at the extremes.
    let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    (mid * angle.cos(), mid * angle.sin())
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

/// How far away an escaped note sits. Every degree stays a short wash.
#[derive(Clone, Copy, PartialEq)]
enum EscapeDegree {
    Little,
    Moderate,
    Lot,
}

struct EscapeTone {
    wet: f32,
    dry: f32,
    rt60: f32,
    tone_hz: f32,
    predelay: f32,
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

    fn take_escape(&mut self) -> EscapeDegree {
        if self.escape_left == 0 {
            self.escape_bag = [EscapeDegree::Little, EscapeDegree::Moderate, EscapeDegree::Lot];
            for index in (1..3).rev() {
                let swap = (self.rng.unit() * (index as f32 + 1.0)).floor() as usize;
                self.escape_bag.swap(index, swap);
            }
            self.escape_left = 3;
        }
        self.escape_left -= 1;
        self.escape_bag[self.escape_left as usize]
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

fn escape_setup(degree: EscapeDegree) -> EscapeTone {
    match degree {
        EscapeDegree::Little => EscapeTone {
            wet: 0.75,
            dry: 0.58,
            rt60: 0.09,
            tone_hz: 2_600.0,
            predelay: 0.008,
        },
        EscapeDegree::Moderate => EscapeTone {
            wet: 0.86,
            dry: 0.40,
            rt60: 0.15,
            tone_hz: 1_800.0,
            predelay: 0.013,
        },
        EscapeDegree::Lot => EscapeTone {
            wet: 1.0,
            dry: 0.24,
            rt60: 0.23,
            tone_hz: 1_150.0,
            predelay: 0.018,
        },
    }
}

fn escape_region_for(sample_rate: f32) -> usize {
    (sample_rate * 0.022).ceil() as usize + 2
}

fn comb_feedback(delay_samples: usize, sample_rate: f32, rt60: f32) -> f32 {
    let delay_t = delay_samples as f32 / sample_rate.max(1.0);
    10.0_f32
        .powf(-3.0 * delay_t / rt60.max(0.02))
        .clamp(0.05, 0.7)
}

fn effect_amount(voice: &Voice, dest: f32) -> f32 {
    if voice.drift {
        (dest.clamp(0.0, 1.0) * voice.drift_now).clamp(0.0, 1.0)
    } else if dest >= 0.5 {
        1.0
    } else {
        0.0
    }
}

/// Strongest global-effect presence on this voice. Drift scales the destinations, so
/// weather rises when drifting into an effect and falls when drifting back to dry.
fn voice_weather_intensity(voice: &Voice) -> f32 {
    [
        voice.dest_swell,
        voice.dest_crush,
        voice.dest_pass,
        voice.dest_stutter,
        voice.dest_smear,
        voice.dest_splatter,
        voice.dest_escape,
    ]
    .into_iter()
    .map(|dest| effect_amount(voice, dest))
    .fold(0.0f32, f32::max)
}

fn drifted_hz(mode: PassMode, target: f32, depth: f32) -> f32 {
    let depth = depth.clamp(0.0, 1.0);
    match mode {
        PassMode::Low => log_lerp(7_200.0, target, depth),
        PassMode::High => log_lerp(48.0, target, depth),
        PassMode::Off => target,
    }
}

fn log_lerp(from: f32, to: f32, t: f32) -> f32 {
    if t <= 0.0 {
        from
    } else if t >= 1.0 {
        to
    } else {
        (from.max(1.0).ln() + (to.max(1.0).ln() - from.max(1.0).ln()) * t).exp()
    }
}

fn delay_tick(buf: &mut [f32], pos: &mut usize, len: usize, input: f32) -> f32 {
    let len = len.max(1).min(buf.len());
    let cursor = *pos % len;
    let out = buf[cursor];
    buf[cursor] = input;
    *pos = (cursor + 1) % len;
    out
}

fn comb_tick(
    buf: &mut [f32],
    pos: &mut usize,
    len: usize,
    damp: &mut f32,
    damp_coeff: f32,
    feedback: f32,
    input: f32,
) -> f32 {
    let len = len.max(1).min(buf.len());
    let cursor = *pos % len;
    let delayed = buf[cursor];
    *damp += damp_coeff * (delayed - *damp);
    buf[cursor] = input + *damp * feedback;
    *pos = (cursor + 1) % len;
    delayed
}

fn allpass_tick(buf: &mut [f32], pos: &mut usize, len: usize, feedback: f32, input: f32) -> f32 {
    let len = len.max(1).min(buf.len());
    let cursor = *pos % len;
    let delayed = buf[cursor];
    let out = delayed - feedback * input;
    buf[cursor] = input + feedback * out;
    *pos = (cursor + 1) % len;
    out
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
        if voice.escape_tail > 0 {
            voice.escape_tail -= 1;
        } else {
            voice.active = false;
        }
    }
}

fn stutter_cap_for(sample_rate: f32) -> usize {
    // An eighth note at the slowest clock, 20 BPM.
    (sample_rate * 1.5).round() as usize + 4
}

fn repeat_fade(sample_rate: f32, period: usize) -> usize {
    let fade = (sample_rate * 0.004).round() as usize;
    fade.clamp(1, (period / 4).max(1)).min(period.saturating_sub(1))
}

fn loop_sample(buf: &[f32], cursor: usize, fade: usize) -> f32 {
    let here = buf[cursor];
    if fade == 0 || cursor >= fade {
        return here;
    }
    let tail = buf[buf.len() - fade + cursor];
    let t = (cursor as f32 / fade as f32).clamp(0.0, 1.0);
    tail * (1.0 - t).sqrt() + here * t.sqrt()
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
            b_sides: vec![None, None],
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
                        charm: 0.5,
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
                        charm: 0.5,
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
        let head = engine.playhead(0).unwrap();
        assert!(
            head > 12_000.0,
            "a leap should walk the recording with the octave, playhead {head}"
        );
        let steady = &rendered[grain..grain + 12_000];
        let mut loud = 0.0f32;
        let mut quiet = f32::MAX;
        for chunk in steady.chunks(800) {
            let energy = chunk.iter().map(|sample| sample * sample).sum::<f32>() / chunk.len() as f32;
            loud = loud.max(energy);
            quiet = quiet.min(energy);
        }
        assert!(
            quiet > loud * 0.45,
            "a leap should stay even instead of chopping, quiet {quiet} loud {loud}"
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
                        charm: 0.5,
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
    fn splatter_catches_the_note_and_leaves_the_recording_intact() {
        let mut missed = Engine::new(44_100.0, tone_bank(), 4);
        missed.set_global(GlobalRoll {
            splatter: 0.0,
            ..GlobalRoll::off()
        });
        let mut caught = Engine::new(44_100.0, tone_bank(), 4);
        caught.set_global(GlobalRoll {
            splatter: 1.0,
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
        missed
            .note_on(request, &[1.0], &[SliceRolls::neutral()], 1.0)
            .unwrap();
        caught
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
        assert!(missed.voices[0].dest_splatter < 0.001);
        assert!(caught.voices[0].dest_splatter > 0.99);

        let mut dry_energy = 0.0f32;
        let mut wet_energy = 0.0f32;
        let mut difference = 0.0f32;
        for _ in 0..4_000 {
            let (plain_l, _) = missed.render(60, 1.0, true);
            let (dry_l, _, wet_l, _) = caught.render_buses(60, 1.0, true);
            dry_energy += dry_l.abs();
            wet_energy += wet_l.abs();
            difference += (plain_l - (dry_l + wet_l)).abs();
        }
        assert!(
            dry_energy < 1.0e-3,
            "a caught note should leave the dry bus, energy {dry_energy}"
        );
        assert!(wet_energy > 1.0, "a caught note should feed the gate, energy {wet_energy}");
        assert!(
            difference < 1.0e-3,
            "catching splatter should not change the note itself, difference {difference}"
        );
    }

    #[test]
    fn stutter_loops_the_captured_division() {
        let mut engine = Engine::new(44_100.0, tone_bank(), 1);
        engine.set_clock(120.0);
        engine.set_global(GlobalRoll {
            stutter: 1.0,
            ..GlobalRoll::off()
        });
        hold_first(&mut engine, 0.0);
        let period = engine.voices[0].stutter_len as usize;
        let divisions = [0.5_f32, 0.25, 1.0 / 6.0, 0.125];
        let expected: Vec<usize> = divisions
            .iter()
            .map(|beats| (beats * 60.0 / 120.0 * 44_100.0).round() as usize)
            .collect();
        assert!(
            expected.contains(&period),
            "period {period} should be an eighth, sixteenth, triplet, or thirty-second"
        );

        let mut out = Vec::with_capacity(period * 4);
        for _ in 0..(period * 4) {
            out.push(engine.render(60, 1.0, true).0);
        }
        let fade = (44_100.0_f32 * 0.004).round() as usize;
        let body = fade..period - fade;
        for (captured, rolled) in out[body.clone()].iter().zip(&out[period + body.start..]) {
            assert!(
                (captured - rolled).abs() < 1.0e-6,
                "the roll should repeat the captured slice"
            );
        }
        let first_roll = &out[period..period * 2];
        let second_roll = &out[period * 2..period * 3];
        assert!(
            correlation(first_roll, second_roll) > 0.999,
            "each retrigger should be the same slice"
        );
        let heard = out[period..period * 3]
            .iter()
            .filter(|sample| sample.abs() >= 1.0e-4)
            .count();
        assert!(
            heard > period,
            "the roll should keep sounding through the division, heard {heard}"
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

    #[test]
    fn escape_deals_three_short_washes() {
        let mut engine = Engine::new(44_100.0, tone_bank(), 1);
        engine.set_global(GlobalRoll {
            escape: 1.0,
            ..GlobalRoll::off()
        });
        let mut wet = [0.0f32; 3];
        for index in 0..3 {
            engine
                .note_on(
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
                )
                .unwrap();
            wet[index] = engine
                .voices
                .iter()
                .find(|voice| voice.voice_id == Some(index as i32))
                .unwrap()
                .escape_wet;
            assert!(engine.voices[index].escape_tail > 0);
            assert!(engine.voices[index].escape_tail <= (44_100.0 * 0.45) as u32);
        }
        wet.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!((wet[0] - 0.75).abs() < 1.0e-4);
        assert!((wet[1] - 0.86).abs() < 1.0e-4);
        assert!((wet[2] - 1.0).abs() < 1.0e-4);

        let mut dry = Engine::new(44_100.0, tone_bank(), 4);
        let mut washed = Engine::new(44_100.0, tone_bank(), 4);
        washed.set_global(GlobalRoll {
            escape: 1.0,
            ..GlobalRoll::off()
        });
        hold_first(&mut dry, 0.1);
        hold_first(&mut washed, 0.1);
        let mut difference = 0.0f32;
        let mut plain = 0.0f32;
        let mut width = 0.0f32;
        for _ in 0..8_000 {
            let (dry_l, _) = dry.render(60, 1.0, true);
            let (wet_l, wet_r) = washed.render(60, 1.0, true);
            difference += (dry_l - wet_l).abs();
            plain += dry_l.abs();
            width += (wet_l - wet_r).abs();
        }
        assert!(
            difference > plain * 0.12,
            "escape should set the note back, difference {difference} plain {plain}"
        );
        assert!(width > 1.0, "the wash should sit apart from the dry stereo, width {width}");

        washed.note_off(0, 60, None, 1.0);
        let mut tail = 0.0f32;
        for _ in 0..2_000 {
            let (left, _) = washed.render(60, 1.0, true);
            tail += left.abs();
        }
        assert!(tail > 0.05, "the short tail should still speak, energy {tail}");
        let mut late = 0.0f32;
        for _ in 0..(44_100 / 2) {
            let (left, _) = washed.render(60, 1.0, true);
            late = left.abs();
        }
        assert!(late < 1.0e-3, "the wash should be gone within half a second, left {late}");
        assert!(!washed.any_active());
    }

    #[test]
    fn drift_moves_between_dry_and_the_set_knobs() {
        let mut settled = Engine::new(44_100.0, tone_bank(), 5);
        let mut empty = Engine::new(44_100.0, tone_bank(), 5);
        empty.set_global(GlobalRoll {
            drift: 1.0,
            ..GlobalRoll::off()
        });
        hold_first(&mut settled, 0.2);
        hold_first(&mut empty, 0.2);
        assert!(empty.voices[0].drift);
        assert!(empty.voices[0].dest_pass < 0.001);
        let plain = render_for(&mut settled, 8_000, 60, 1.0);
        let untouched = render_for(&mut empty, 8_000, 60, 1.0);
        assert!(
            correlation(&plain, &untouched) > 0.999,
            "drift with every other knob down should leave the note dry"
        );

        let mut moving = Engine::new(44_100.0, tone_bank(), 2);
        moving.set_global(GlobalRoll {
            drift: 1.0,
            pass: 1.0,
            ..GlobalRoll::off()
        });
        hold_first(&mut moving, 0.0);
        let voice = &moving.voices[0];
        assert!(voice.dest_pass > 0.99);
        assert!(voice.pass != PassMode::Off);
        let start = voice.pass_coeff;
        let travel = voice.drift_len as usize;
        for _ in 0..travel {
            moving.render(60, 1.0, true);
        }
        let end = moving.voices[0].pass_coeff;
        assert!(
            (start - end).abs() > 0.002,
            "the filter should travel between open and the pass destination, {start} -> {end}"
        );
        let toward_center = !moving.voices[0].drift_from_center;
        if toward_center {
            assert!(moving.voices[0].drift_now < 0.05);
        } else {
            assert!(moving.voices[0].drift_now > 0.95);
        }
    }

    fn note_with_charm(engine: &mut Engine, charm: f32) {
        let mut rolls = [SliceRolls::neutral(); 2];
        rolls[0].charm = charm;
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
                &rolls,
                1.0,
            )
            .unwrap();
    }

    #[test]
    fn charm_shields_at_zero_and_catches_one_effect_at_full() {
        let mut shielded = Engine::new(44_100.0, tone_bank(), 3);
        shielded.set_global(GlobalRoll {
            swell: 1.0,
            crush: 1.0,
            escape: 1.0,
            ..GlobalRoll::off()
        });
        note_with_charm(&mut shielded, 0.0);
        let voice = &shielded.voices[0];
        assert!(!voice.reverse, "charm at zero should skip swell");
        assert!(!voice.crush, "charm at zero should skip crush");
        assert!(!voice.escape, "charm at zero should skip escape");

        let mut balanced = Engine::new(44_100.0, tone_bank(), 3);
        balanced.set_global(GlobalRoll {
            escape: 1.0,
            ..GlobalRoll::off()
        });
        note_with_charm(&mut balanced, 0.5);
        assert!(balanced.voices[0].escape, "noon should leave a full knob alone");

        let mut charmed = Engine::new(44_100.0, tone_bank(), 8);
        charmed.set_global(GlobalRoll {
            escape: 0.15,
            ..GlobalRoll::off()
        });
        for index in 0..6 {
            charmed.silence();
            note_with_charm(&mut charmed, 1.0);
            assert!(
                charmed.voices[0].escape,
                "full charm should catch the only enabled effect, note {index}"
            );
        }

        let mut quiet = Engine::new(44_100.0, tone_bank(), 8);
        quiet.set_global(GlobalRoll::off());
        note_with_charm(&mut quiet, 1.0);
        let voice = &quiet.voices[0];
        assert!(!voice.escape && !voice.crush && !voice.smear && !voice.drift);
    }
}
