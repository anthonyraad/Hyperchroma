#![allow(semicolon_in_expressions_from_non_local_macros)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, EguiState};

use crate::bank::BankSnapshot;
use crate::engine::{Engine, GlobalRoll, NoteOn, SliceRolls};
use crate::names::{note_name, parse_note};
use crate::spectral_gate::SpectralGate;

pub mod bank;
mod editor;
pub mod engine;
pub mod names;
mod spectral_gate;

use editor::KbState;

const BUS_TRIM: f32 = 0.5;

const SAMPLE_SLOTS: usize = 32;

pub struct SharedUi {
    pub snapshot: Mutex<BankSnapshot>,
    pub peak_bits: AtomicU32,
    pub last_index: AtomicU32,
    pub last_ppm: AtomicU32,
    pub active_mask: AtomicU32,
    /// Colors whose newest voice is reading the B-side.
    pub b_side_mask: AtomicU32,
    pub voices: AtomicU32,
    pub host_bpm_x100: AtomicU32,
    /// Bank slot the editor asked to audition, or `u32::MAX` when idle.
    pub preview_slot: AtomicU32,
    /// Playhead in parts per million, or `u32::MAX` when that recording is silent.
    pub playhead_ppm: [AtomicU32; SAMPLE_SLOTS],
    /// How hard a global effect is hitting the newest voice in that slot, in parts per
    /// million. Drift multiplies this, so the waveform can settle or stir over the note.
    pub weather_ppm: [AtomicU32; SAMPLE_SLOTS],
}

impl Default for SharedUi {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(BankSnapshot::default()),
            peak_bits: AtomicU32::new(0),
            last_index: AtomicU32::new(u32::MAX),
            last_ppm: AtomicU32::new(0),
            active_mask: AtomicU32::new(0),
            b_side_mask: AtomicU32::new(0),
            voices: AtomicU32::new(0),
            host_bpm_x100: AtomicU32::new(0),
            preview_slot: AtomicU32::new(u32::MAX),
            playhead_ppm: std::array::from_fn(|_| AtomicU32::new(u32::MAX)),
            weather_ppm: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
}

pub struct Hyperchroma {
    params: Arc<HyperchromaParams>,
    engine: Engine,
    shared: Arc<SharedUi>,
    samples_loaded: bool,
    meter: f32,
    peak_decay: f32,
    /// Samples left in the editor's audition, and whether one is sounding.
    preview_left: u32,
    preview_on: bool,
    gate: SpectralGate,
    dry_l: Vec<f32>,
    dry_r: Vec<f32>,
    wet_l: Vec<f32>,
    wet_r: Vec<f32>,
}

#[derive(Params)]
pub struct SliceParams {
    #[id = "prob"]
    pub probability: FloatParam,

    #[id = "rev"]
    pub reverse: FloatParam,

    #[id = "pan"]
    pub pan: FloatParam,

    #[id = "oct"]
    pub octave: FloatParam,

    #[id = "charm"]
    pub charm: FloatParam,
}

impl SliceParams {
    fn new() -> Self {
        Self {
            probability: percent_param("Luck", 1.0),
            reverse: percent_param("Reverse", 0.0),
            pan: FloatParam::new(
                "Spread",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_step_size(0.01)
            .with_value_to_string(Arc::new(pan_text))
            .with_string_to_value(Arc::new(pan_parse)),
            octave: percent_param("Leap", 0.0),
            charm: percent_param("Charm", 0.5),
        }
    }
}

fn percent_param(name: &str, default: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Linear {
            min: 0.0,
            max: 1.0,
        },
    )
    .with_step_size(0.01)
    .with_value_to_string(Arc::new(percent_text))
    .with_string_to_value(Arc::new(percent_parse))
}

fn percent_text(value: f32) -> String {
    format!("{}%", (value.clamp(0.0, 1.0) * 100.0).round() as i32)
}

fn percent_parse(text: &str) -> Option<f32> {
    let trimmed = text.trim();
    let marked = trimmed.ends_with('%');
    let number: f32 = trimmed.trim_end_matches('%').trim().parse().ok()?;
    let value = if marked || number > 1.0 {
        number / 100.0
    } else {
        number
    };
    (0.0..=1.0).contains(&value).then_some(value)
}

fn pan_text(value: f32) -> String {
    let percent = (value.abs().clamp(0.0, 1.0) * 100.0).round() as i32;
    if percent == 0 {
        "Center".to_string()
    } else if value < 0.0 {
        format!("L {percent}%")
    } else {
        format!("R {percent}%")
    }
}

fn pan_parse(text: &str) -> Option<f32> {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("center")
        || trimmed.eq_ignore_ascii_case("c")
        || trimmed == "0"
    {
        return Some(0.0);
    }
    let lower = trimmed.to_ascii_lowercase();
    let (sign, rest) = if let Some(rest) = lower.strip_prefix("l") {
        (-1.0, rest)
    } else if let Some(rest) = lower.strip_prefix("r") {
        (1.0, rest)
    } else if let Some(rest) = lower.strip_prefix('-') {
        (-1.0, rest)
    } else {
        (1.0, lower.as_str())
    };
    let number: f32 = rest
        .trim()
        .trim_end_matches('%')
        .trim()
        .parse()
        .ok()?;
    let magnitude = if number > 1.0 { number / 100.0 } else { number };
    Some((sign * magnitude).clamp(-1.0, 1.0))
}

#[derive(Params)]
pub struct HyperchromaParams {
    #[persist = "editor-state-9"]
    editor_state: Arc<EguiState>,

    #[id = "gain"]
    pub gain: FloatParam,

    #[id = "attack"]
    pub attack: FloatParam,

    #[id = "release"]
    pub release: FloatParam,

    #[id = "root"]
    pub root: IntParam,

    #[id = "keytrack"]
    pub keytrack: BoolParam,

    #[id = "stereoentropic"]
    pub stereoentropic: BoolParam,

    #[id = "tempo"]
    pub tempo: FloatParam,

    #[id = "sample_bpm"]
    pub sample_bpm: FloatParam,

    #[id = "sync"]
    pub sync: BoolParam,

    #[id = "swell"]
    pub swell: FloatParam,

    #[id = "crush"]
    pub crush: FloatParam,

    #[id = "pass"]
    pub pass: FloatParam,

    #[id = "stutter"]
    pub stutter: FloatParam,

    #[id = "smear"]
    pub smear: FloatParam,

    #[id = "splatter"]
    pub splatter: FloatParam,

    #[id = "escape"]
    pub escape: FloatParam,

    #[id = "drift"]
    pub drift: FloatParam,

    /// Chance a color plays its B-side. `0` is always A, `1` is always B, in 5% steps.
    #[id = "bside"]
    pub b_side: FloatParam,

    /// Reverse chance for colors whose Reverse knob is still at zero, in 5% steps.
    #[id = "revfloor"]
    pub reverse_floor: FloatParam,

    /// Chance a note pumps like a kick sidechain, in 5% steps.
    #[id = "duck"]
    pub duck: FloatParam,

    /// Chance a note slowly drifts sharp and flat, in 5% steps.
    #[id = "detune"]
    pub detune: FloatParam,

    /// Rates a ducked note may choose. At least a quarter note stays available.
    #[id = "duck4"]
    pub duck_quarter: BoolParam,

    #[id = "duck2"]
    pub duck_half: BoolParam,

    #[id = "duck1"]
    pub duck_whole: BoolParam,

    #[nested(id_prefix = "bug", group = "Poppy")]
    pub bug: SliceParams,
    #[nested(id_prefix = "dark", group = "Rosehip")]
    pub dark: SliceParams,
    #[nested(id_prefix = "dragon", group = "Peony")]
    pub dragon: SliceParams,
    #[nested(id_prefix = "electric", group = "Apricot")]
    pub electric: SliceParams,
    #[nested(id_prefix = "fairy", group = "Persimmon")]
    pub fairy: SliceParams,
    #[nested(id_prefix = "fighting", group = "Marigold")]
    pub fighting: SliceParams,
    #[nested(id_prefix = "fire", group = "Buttercup")]
    pub fire: SliceParams,
    #[nested(id_prefix = "flying", group = "Lichen")]
    pub flying: SliceParams,
    #[nested(id_prefix = "ghost", group = "Fern")]
    pub ghost: SliceParams,
    #[nested(id_prefix = "grass", group = "Pine")]
    pub grass: SliceParams,
    #[nested(id_prefix = "ground", group = "Eucalyptus")]
    pub ground: SliceParams,
    #[nested(id_prefix = "ice", group = "Juniper")]
    pub ice: SliceParams,
    #[nested(id_prefix = "normal", group = "Cornflower")]
    pub normal: SliceParams,
    #[nested(id_prefix = "poison", group = "Iris")]
    pub poison: SliceParams,
    #[nested(id_prefix = "psychic", group = "Lavender")]
    pub psychic: SliceParams,
    #[nested(id_prefix = "rock", group = "Orchid")]
    pub rock: SliceParams,
    #[nested(id_prefix = "steel", group = "Aubergine")]
    pub steel: SliceParams,
    #[nested(id_prefix = "water", group = "Chestnut")]
    pub water: SliceParams,
    #[nested(id_prefix = "sage", group = "Birch")]
    pub sage: SliceParams,
    #[nested(id_prefix = "clay", group = "Indigo")]
    pub clay: SliceParams,
    #[nested(id_prefix = "mulberry", group = "Mulberry")]
    pub mulberry: SliceParams,
    #[nested(id_prefix = "olive", group = "Olive")]
    pub olive: SliceParams,
    #[nested(id_prefix = "aloe", group = "Aloe")]
    pub aloe: SliceParams,
    #[nested(id_prefix = "cinnamon", group = "Cinnamon")]
    pub cinnamon: SliceParams,
}

impl HyperchromaParams {
    pub fn slice(&self, name: &str) -> Option<&SliceParams> {
        Some(match name {
            "bug" => &self.bug,
            "dark" => &self.dark,
            "dragon" => &self.dragon,
            "electric" => &self.electric,
            "fairy" => &self.fairy,
            "fighting" => &self.fighting,
            "fire" => &self.fire,
            "flying" => &self.flying,
            "ghost" => &self.ghost,
            "grass" => &self.grass,
            "ground" => &self.ground,
            "ice" => &self.ice,
            "normal" => &self.normal,
            "poison" => &self.poison,
            "psychic" => &self.psychic,
            "rock" => &self.rock,
            "steel" => &self.steel,
            "water" => &self.water,
            "sage" => &self.sage,
            "clay" => &self.clay,
            "mulberry" => &self.mulberry,
            "olive" => &self.olive,
            "aloe" => &self.aloe,
            "cinnamon" => &self.cinnamon,
            _ => return None,
        })
    }
}

impl Default for Hyperchroma {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.subsec_nanos())
            .unwrap_or(1);
        Self {
            params: Arc::new(HyperchromaParams::default()),
            engine: Engine::new(44_100.0, bank::Bank::empty(), seed),
            shared: Arc::new(SharedUi::default()),
            samples_loaded: false,
            meter: 0.0,
            peak_decay: 0.999,
            preview_left: 0,
            preview_on: false,
            gate: SpectralGate::new(44_100.0),
            dry_l: Vec::new(),
            dry_r: Vec::new(),
            wet_l: Vec::new(),
            wet_r: Vec::new(),
        }
    }
}

impl Default for HyperchromaParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(
                editor::WINDOW_COLLAPSED.0,
                editor::WINDOW_COLLAPSED.1,
            ),
            gain: FloatParam::new(
                "Gain",
                -6.0,
                FloatRange::Linear {
                    min: -36.0,
                    max: 12.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_step_size(0.1)
            .with_unit(" dB"),
            attack: FloatParam::new(
                "Attack",
                45.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 4_000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_step_size(1.0)
            .with_unit(" ms"),
            release: FloatParam::new(
                "Release",
                800.0,
                FloatRange::Skewed {
                    min: 10.0,
                    max: 8_000.0,
                    factor: FloatRange::skew_factor(-1.2),
                },
            )
            .with_step_size(1.0)
            .with_unit(" ms"),
            root: IntParam::new("Transpose", 60, IntRange::Linear { min: 0, max: 127 })
                .with_value_to_string(Arc::new(|value| note_name(value)))
                .with_string_to_value(Arc::new(|text| parse_note(text))),
            keytrack: BoolParam::new("Keytracking", false),
            stereoentropic: BoolParam::new("Stereoentropic", false),
            tempo: FloatParam::new(
                "Tempo",
                120.0,
                FloatRange::Linear {
                    min: 40.0,
                    max: 240.0,
                },
            )
            .with_step_size(0.1)
            .with_unit(" BPM"),
            sample_bpm: FloatParam::new(
                "Sample BPM",
                120.0,
                FloatRange::Linear {
                    min: 40.0,
                    max: 240.0,
                },
            )
            .with_step_size(0.1)
            .with_unit(" BPM"),
            sync: BoolParam::new("Follow host tempo", true),
            swell: percent_param("Swell", 0.0),
            crush: percent_param("Crush", 0.0),
            pass: percent_param("Pass", 0.0),
            stutter: percent_param("Stutter", 0.0),
            smear: percent_param("Smear", 0.0),
            splatter: percent_param("Splatter", 0.0),
            escape: percent_param("Wash", 0.0),
            drift: percent_param("Drift", 0.0),
            b_side: FloatParam::new(
                "B-side",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 1.0,
                },
            )
            .with_step_size(0.05)
            .with_value_to_string(Arc::new(percent_text))
            .with_string_to_value(Arc::new(percent_parse)),
            reverse_floor: FloatParam::new(
                "Reverse floor",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 1.0,
                },
            )
            .with_step_size(0.05)
            .with_value_to_string(Arc::new(percent_text))
            .with_string_to_value(Arc::new(percent_parse)),
            duck: FloatParam::new(
                "Ducking",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 1.0,
                },
            )
            .with_step_size(0.05)
            .with_value_to_string(Arc::new(percent_text))
            .with_string_to_value(Arc::new(percent_parse)),
            detune: FloatParam::new(
                "Detune",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 1.0,
                },
            )
            .with_step_size(0.05)
            .with_value_to_string(Arc::new(percent_text))
            .with_string_to_value(Arc::new(percent_parse)),
            duck_quarter: BoolParam::new("Duck 1/4", true),
            duck_half: BoolParam::new("Duck 1/2", false),
            duck_whole: BoolParam::new("Duck 1/1", false),
            bug: SliceParams::new(),
            dark: SliceParams::new(),
            dragon: SliceParams::new(),
            electric: SliceParams::new(),
            fairy: SliceParams::new(),
            fighting: SliceParams::new(),
            fire: SliceParams::new(),
            flying: SliceParams::new(),
            ghost: SliceParams::new(),
            grass: SliceParams::new(),
            ground: SliceParams::new(),
            ice: SliceParams::new(),
            normal: SliceParams::new(),
            poison: SliceParams::new(),
            psychic: SliceParams::new(),
            rock: SliceParams::new(),
            steel: SliceParams::new(),
            water: SliceParams::new(),
            sage: SliceParams::new(),
            clay: SliceParams::new(),
            mulberry: SliceParams::new(),
            olive: SliceParams::new(),
            aloe: SliceParams::new(),
            cinnamon: SliceParams::new(),
        }
    }
}

impl Plugin for Hyperchroma {
    const NAME: &'static str = "Hyperchroma";
    const VENDOR: &'static str = "Raadscapes";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        let shared = self.shared.clone();
        let egui_state = params.editor_state.clone();
        create_egui_editor(
            egui_state.clone(),
            KbState::new(),
            |ctx, _| editor::theme(ctx),
            move |ctx, setter, kb| {
                editor::draw(ctx, setter, kb, &params, &shared, &egui_state);
            },
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        // Decoding logs at info. The scanner has nowhere to send that, so keep it quiet.
        nih_plug::log::set_max_level(nih_plug::log::LevelFilter::Warn);
        self.engine.set_sample_rate(buffer_config.sample_rate);
        self.gate.set_sample_rate(buffer_config.sample_rate);
        context.set_latency_samples(self.gate.latency_samples());
        let block = buffer_config.max_buffer_size.max(1) as usize;
        self.dry_l.resize(block, 0.0);
        self.dry_r.resize(block, 0.0);
        self.wet_l.resize(block, 0.0);
        self.wet_r.resize(block, 0.0);
        if !self.samples_loaded {
            let loaded = bank::load_embedded();
            if let Ok(mut snapshot) = self.shared.snapshot.lock() {
                *snapshot = loaded.snapshot();
            }
            self.engine.set_bank(loaded);
            self.samples_loaded = true;
        }
        self.peak_decay =
            0.25f64.powf((buffer_config.sample_rate as f64 * 150.0 / 1000.0).recip()) as f32;
        true
    }

    fn reset(&mut self) {
        self.engine.silence();
        self.gate.reset();
        self.preview_on = false;
        self.preview_left = 0;
        self.shared.preview_slot.store(u32::MAX, Ordering::Relaxed);
        self.meter = 0.0;
        self.shared.voices.store(0, Ordering::Relaxed);
        self.shared.active_mask.store(0, Ordering::Relaxed);
        self.shared.b_side_mask.store(0, Ordering::Relaxed);
        self.shared.peak_bits.store(0, Ordering::Relaxed);
        for slot in &self.shared.playhead_ppm {
            slot.store(u32::MAX, Ordering::Relaxed);
        }
        for slot in &self.shared.weather_ppm {
            slot.store(0, Ordering::Relaxed);
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let host_bpm = context.transport().tempo;
        match host_bpm {
            Some(bpm) if bpm.is_finite() && bpm > 1.0 => {
                self.shared
                    .host_bpm_x100
                    .store((bpm * 100.0).round() as u32, Ordering::Relaxed);
            }
            _ => self.shared.host_bpm_x100.store(0, Ordering::Relaxed),
        }

        let mut next_event = context.next_event();
        let root = self.params.root.value();
        let keytrack = self.params.keytrack.value();
        let follow = self.params.sync.value();
        let manual = sanitize_bpm(self.params.tempo.value());
        let sample_bpm = sanitize_bpm(self.params.sample_bpm.value());
        let target = if follow {
            host_bpm
                .filter(|bpm| bpm.is_finite() && *bpm > 1.0)
                .map(|bpm| bpm as f32)
                .unwrap_or(manual)
        } else {
            manual
        };
        let tempo_ratio = (target / sample_bpm).clamp(0.05, 8.0);
        let transport = context.transport();
        self.engine
            .locate(target, transport.pos_beats(), transport.playing);
        self.engine.set_global(GlobalRoll {
            swell: self.params.swell.value(),
            crush: self.params.crush.value(),
            pass: self.params.pass.value(),
            stutter: self.params.stutter.value(),
            smear: self.params.smear.value(),
            splatter: self.params.splatter.value(),
            escape: self.params.escape.value(),
            drift: self.params.drift.value(),
            b_side: self.params.b_side.value(),
            stereoentropic: self.params.stereoentropic.value(),
            reverse_floor: self.params.reverse_floor.value(),
            duck: self.params.duck.value(),
            detune: self.params.detune.value(),
            duck_quarter: self.params.duck_quarter.value(),
            duck_half: self.params.duck_half.value(),
            duck_whole: self.params.duck_whole.value(),
        });
        self.poll_preview(buffer.samples());

        let block = buffer.samples().min(self.dry_l.len());
        for sample_id in 0..block {
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }
                match event {
                    NoteEvent::NoteOn {
                        channel,
                        note,
                        velocity,
                        voice_id,
                        ..
                    } => {
                        self.begin_note(channel, note, velocity, voice_id, None);
                    }
                    NoteEvent::NoteOff {
                        channel,
                        note,
                        voice_id,
                        ..
                    } => self.end_note(channel, note, voice_id),
                    NoteEvent::Choke {
                        channel,
                        note,
                        voice_id,
                        ..
                    } => self.engine.choke(channel, note, voice_id),
                    NoteEvent::PolyTuning {
                        channel,
                        note,
                        tuning,
                        voice_id,
                        ..
                    } => self.engine.set_tuning(channel, note, voice_id, tuning),
                    _ => (),
                }
                next_event = context.next_event();
            }

            let (dry_l, dry_r, wet_l, wet_r) = self.engine.render_buses(root, tempo_ratio, keytrack);
            self.dry_l[sample_id] = dry_l;
            self.dry_r[sample_id] = dry_r;
            self.wet_l[sample_id] = wet_l;
            self.wet_r[sample_id] = wet_r;
        }

        let splatter = self.params.splatter.value();
        SpectralGate::process_block(
            &mut self.gate,
            &mut self.wet_l[..block],
            &mut self.wet_r[..block],
            &self.dry_l[..block],
            &self.dry_r[..block],
            splatter,
        );

        for (sample_id, channel_samples) in buffer.iter_samples().enumerate() {
            if sample_id >= block {
                break;
            }
            let gain = util::db_to_gain(self.params.gain.smoothed.next());
            let left = shape(self.wet_l[sample_id] * gain * BUS_TRIM);
            let right = shape(self.wet_r[sample_id] * gain * BUS_TRIM);
            let mut channel_index = 0;
            for sample in channel_samples {
                *sample = if channel_index == 0 { left } else { right };
                channel_index += 1;
            }
            let amp = left.abs().max(right.abs());
            if amp > self.meter {
                self.meter = amp;
            } else {
                self.meter = self.meter * self.peak_decay + amp * (1.0 - self.peak_decay);
            }
        }

        self.shared
            .peak_bits
            .store(self.meter.to_bits(), Ordering::Relaxed);
        self.shared
            .voices
            .store(self.engine.voice_count() as u32, Ordering::Relaxed);
        self.shared
            .active_mask
            .store(self.engine.active_mask(), Ordering::Relaxed);
        self.shared
            .b_side_mask
            .store(self.engine.b_side_mask(), Ordering::Relaxed);
        let mut heads = [u32::MAX; SAMPLE_SLOTS];
        let mut weather = [0u32; SAMPLE_SLOTS];
        let mut serials = [0u32; SAMPLE_SLOTS];
        self.engine
            .write_playheads(&mut heads, &mut weather, &mut serials);
        for (slot, head) in self.shared.playhead_ppm.iter().zip(heads) {
            slot.store(head, Ordering::Relaxed);
        }
        for (slot, amount) in self.shared.weather_ppm.iter().zip(weather) {
            slot.store(amount, Ordering::Relaxed);
        }

        if self.engine.any_active() || self.gate.is_active() || self.params.editor_state.is_open() {
            ProcessStatus::KeepAlive
        } else {
            ProcessStatus::Normal
        }
    }
}

impl Hyperchroma {
    /// How long the editor's play button holds a note before releasing it.
    const PREVIEW_SECONDS: f32 = 2.0;
    const PREVIEW_VOICE: i32 = -1_000_001;
    const PREVIEW_NOTE: u8 = 60;

    fn poll_preview(&mut self, block_samples: usize) {
        let requested = self.shared.preview_slot.swap(u32::MAX, Ordering::AcqRel);
        if requested != u32::MAX {
            self.stop_preview();
            self.start_preview(requested as usize);
            return;
        }
        if !self.preview_on {
            return;
        }
        let block = block_samples as u32;
        if self.preview_left <= block {
            self.stop_preview();
        } else {
            self.preview_left -= block;
        }
    }

    fn start_preview(&mut self, slot: usize) {
        let started = self.begin_note(
            0,
            Self::PREVIEW_NOTE,
            1.0,
            Some(Self::PREVIEW_VOICE),
            Some(slot),
        );
        if !started {
            return;
        }
        self.preview_on = true;
        self.preview_left = (self.engine.sample_rate() * Self::PREVIEW_SECONDS)
            .round()
            .max(1.0) as u32;
    }

    fn stop_preview(&mut self) {
        if !self.preview_on {
            return;
        }
        let release = ms_to_samples(self.params.release.value(), self.engine.sample_rate());
        self.engine.release_id(Self::PREVIEW_VOICE, release);
        self.preview_on = false;
        self.preview_left = 0;
    }

    fn begin_note(
        &mut self,
        channel: u8,
        note: u8,
        velocity: f32,
        voice_id: Option<i32>,
        forced: Option<usize>,
    ) -> bool {
        if !(0..128).contains(&note) {
            return false;
        }
        if velocity <= 0.0 {
            self.end_note(channel, note, voice_id);
            return false;
        }
        let mut weights = [0.0f32; 32];
        let mut rolls = [SliceRolls::neutral(); 32];
        let count = self.engine.bank_len().min(32);
        for index in 0..count {
            match self.params.slice(self.engine.sample_name(index)) {
                Some(slice) => {
                    weights[index] = slice.probability.value().clamp(0.0, 1.0);
                    rolls[index] = SliceRolls {
                        reverse: slice.reverse.value(),
                        pan: slice.pan.value(),
                        octave: slice.octave.value(),
                        charm: slice.charm.value(),
                    };
                }
                None => weights[index] = 1.0,
            }
        }
        if let Some(slot) = forced {
            if slot < weights.len() {
                weights[slot] = 1.0;
            }
        }
        let attack = ms_to_samples(self.params.attack.value(), self.engine.sample_rate());
        if let Some(triggered) = self.engine.note_on(
            NoteOn {
                note,
                velocity,
                channel,
                voice_id,
                sample: forced,
                start_norm: None,
            },
            &weights,
            &rolls,
            attack,
        ) {
            self.shared
                .last_index
                .store(triggered.sample as u32, Ordering::Relaxed);
            let ppm = (triggered.start_norm.clamp(0.0, 1.0) * 1000.0).round() as u32;
            self.shared.last_ppm.store(ppm.min(1000), Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn end_note(&mut self, channel: u8, note: u8, voice_id: Option<i32>) {
        let release = ms_to_samples(self.params.release.value(), self.engine.sample_rate());
        self.engine.note_off(channel, note, voice_id, release);
    }
}

impl ClapPlugin for Hyperchroma {
    const CLAP_ID: &'static str = "com.raadscapes.hyperchroma";
    const CLAP_DESCRIPTION: Option<&'static str> = Some(
        "Polyphonic random-slice instrument. Each note plays a random moment from a random recording.",
    );
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for Hyperchroma {
    // Rdsc2 replaces Rdsc1, which Cubase blocklisted when the first build failed to scan.
    const VST3_CLASS_ID: [u8; 16] = *b"HyperchromaRdsc2";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
    ];
}

#[cfg(not(windows))]
nih_export_clap!(Hyperchroma);
nih_export_vst3!(Hyperchroma);

fn sanitize_bpm(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(20.0, 400.0)
    } else {
        120.0
    }
}

fn ms_to_samples(ms: f32, sample_rate: f32) -> f32 {
    if ms.is_finite() && sample_rate.is_finite() {
        (ms.max(0.0) * 0.001 * sample_rate).max(0.0)
    } else {
        1.0
    }
}

fn shape(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-1.15, 1.15)
    } else {
        0.0
    }
}
