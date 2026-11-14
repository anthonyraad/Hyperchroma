#![allow(semicolon_in_expressions_from_non_local_macros)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, EguiState};

use crate::bank::BankSnapshot;
use crate::engine::{Engine, NoteOn};
use crate::names::{note_name, parse_note};

pub mod bank;
mod editor;
pub mod engine;
pub mod names;

use editor::{KbState, UiMidi};

const BUS_TRIM: f32 = 0.5;

pub struct SharedUi {
    pub snapshot: Mutex<BankSnapshot>,
    pub peak_bits: AtomicU32,
    pub last_index: AtomicU32,
    pub last_ppm: AtomicU32,
    pub active_mask: AtomicU32,
    pub voices: AtomicU32,
    pub host_bpm_x100: AtomicU32,
}

impl Default for SharedUi {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(BankSnapshot::default()),
            peak_bits: AtomicU32::new(0),
            last_index: AtomicU32::new(u32::MAX),
            last_ppm: AtomicU32::new(0),
            active_mask: AtomicU32::new(0),
            voices: AtomicU32::new(0),
            host_bpm_x100: AtomicU32::new(0),
        }
    }
}

pub struct Hyperchroma {
    params: Arc<HyperchromaParams>,
    engine: Engine,
    shared: Arc<SharedUi>,
    ui_rx: rtrb::Consumer<UiMidi>,
    ui_tx: Arc<Mutex<rtrb::Producer<UiMidi>>>,
    samples_loaded: bool,
    meter: f32,
    peak_decay: f32,
}

#[derive(Params)]
pub struct HyperchromaParams {
    #[persist = "editor-state"]
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

    #[id = "tempo"]
    pub tempo: FloatParam,

    #[id = "sample_bpm"]
    pub sample_bpm: FloatParam,

    #[id = "sync"]
    pub sync: BoolParam,

    #[id = "en_bug"]
    pub en_bug: BoolParam,
    #[id = "en_dark"]
    pub en_dark: BoolParam,
    #[id = "en_dragon"]
    pub en_dragon: BoolParam,
    #[id = "en_electric"]
    pub en_electric: BoolParam,
    #[id = "en_fairy"]
    pub en_fairy: BoolParam,
    #[id = "en_fighting"]
    pub en_fighting: BoolParam,
    #[id = "en_fire"]
    pub en_fire: BoolParam,
    #[id = "en_flying"]
    pub en_flying: BoolParam,
    #[id = "en_ghost"]
    pub en_ghost: BoolParam,
    #[id = "en_grass"]
    pub en_grass: BoolParam,
    #[id = "en_ground"]
    pub en_ground: BoolParam,
    #[id = "en_ice"]
    pub en_ice: BoolParam,
    #[id = "en_normal"]
    pub en_normal: BoolParam,
    #[id = "en_poison"]
    pub en_poison: BoolParam,
    #[id = "en_psychic"]
    pub en_psychic: BoolParam,
    #[id = "en_rock"]
    pub en_rock: BoolParam,
    #[id = "en_steel"]
    pub en_steel: BoolParam,
    #[id = "en_water"]
    pub en_water: BoolParam,
}

impl HyperchromaParams {
    pub fn sample_toggle(&self, name: &str) -> Option<&BoolParam> {
        Some(match name {
            "bug" => &self.en_bug,
            "dark" => &self.en_dark,
            "dragon" => &self.en_dragon,
            "electric" => &self.en_electric,
            "fairy" => &self.en_fairy,
            "fighting" => &self.en_fighting,
            "fire" => &self.en_fire,
            "flying" => &self.en_flying,
            "ghost" => &self.en_ghost,
            "grass" => &self.en_grass,
            "ground" => &self.en_ground,
            "ice" => &self.en_ice,
            "normal" => &self.en_normal,
            "poison" => &self.en_poison,
            "psychic" => &self.en_psychic,
            "rock" => &self.en_rock,
            "steel" => &self.en_steel,
            "water" => &self.en_water,
            _ => return None,
        })
    }

    /// Known recordings follow their mute toggle. Anything else stays in the draw.
    pub fn is_enabled(&self, name: &str) -> bool {
        self.sample_toggle(name)
            .map(|param| param.value())
            .unwrap_or(true)
    }
}

impl Default for Hyperchroma {
    fn default() -> Self {
        let (tx, rx) = rtrb::RingBuffer::new(128);
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.subsec_nanos())
            .unwrap_or(1);
        Self {
            params: Arc::new(HyperchromaParams::default()),
            engine: Engine::new(44_100.0, bank::Bank::empty(), seed),
            shared: Arc::new(SharedUi::default()),
            ui_rx: rx,
            ui_tx: Arc::new(Mutex::new(tx)),
            samples_loaded: false,
            meter: 0.0,
            peak_decay: 0.999,
        }
    }
}

impl Default for HyperchromaParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(1040, 780),
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
            root: IntParam::new("Original pitch", 60, IntRange::Linear { min: 0, max: 127 })
                .with_value_to_string(Arc::new(|value| note_name(value)))
                .with_string_to_value(Arc::new(|text| parse_note(text))),
            keytrack: BoolParam::new("Keytracking", false),
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
            en_bug: BoolParam::new("Bug", true),
            en_dark: BoolParam::new("Dark", true),
            en_dragon: BoolParam::new("Dragon", true),
            en_electric: BoolParam::new("Electric", true),
            en_fairy: BoolParam::new("Fairy", true),
            en_fighting: BoolParam::new("Fighting", true),
            en_fire: BoolParam::new("Fire", true),
            en_flying: BoolParam::new("Flying", true),
            en_ghost: BoolParam::new("Ghost", true),
            en_grass: BoolParam::new("Grass", true),
            en_ground: BoolParam::new("Ground", true),
            en_ice: BoolParam::new("Ice", true),
            en_normal: BoolParam::new("Normal", true),
            en_poison: BoolParam::new("Poison", true),
            en_psychic: BoolParam::new("Psychic", true),
            en_rock: BoolParam::new("Rock", true),
            en_steel: BoolParam::new("Steel", true),
            en_water: BoolParam::new("Water", true),
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
        let tx = self.ui_tx.clone();
        let egui_state = params.editor_state.clone();
        create_egui_editor(
            egui_state.clone(),
            KbState::new(tx),
            |ctx, _| editor::theme(ctx),
            move |ctx, setter, kb| {
                editor::draw(ctx, setter, kb, &params, &shared, egui_state.as_ref());
            },
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        // Decoding logs at info. The scanner has nowhere to send that, so keep it quiet.
        nih_plug::log::set_max_level(nih_plug::log::LevelFilter::Warn);
        self.engine.set_sample_rate(buffer_config.sample_rate);
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
        self.meter = 0.0;
        self.shared.voices.store(0, Ordering::Relaxed);
        self.shared.active_mask.store(0, Ordering::Relaxed);
        self.shared.peak_bits.store(0, Ordering::Relaxed);
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

        loop {
            match self.ui_rx.pop() {
                Ok(UiMidi::On { note, velocity }) => self.begin_note(0, note, velocity, None),
                Ok(UiMidi::Off { note }) => self.end_note(0, note, None),
                Err(_) => break,
            }
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

        for (sample_id, channel_samples) in buffer.iter_samples().enumerate() {
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
                    } => self.begin_note(channel, note, velocity, voice_id),
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

            let gain = util::db_to_gain(self.params.gain.smoothed.next());
            let (left, right) = self.engine.render(root, tempo_ratio, keytrack);
            let left = shape(left * gain * BUS_TRIM);
            let right = shape(right * gain * BUS_TRIM);
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

        if self.engine.any_active() || self.params.editor_state.is_open() {
            ProcessStatus::KeepAlive
        } else {
            ProcessStatus::Normal
        }
    }
}

impl Hyperchroma {
    fn begin_note(&mut self, channel: u8, note: u8, velocity: f32, voice_id: Option<i32>) {
        if !(0..128).contains(&note) {
            return;
        }
        if velocity <= 0.0 {
            self.end_note(channel, note, voice_id);
            return;
        }
        let mut enabled = [false; 32];
        let count = self.engine.bank_len().min(32);
        for index in 0..count {
            enabled[index] = self.params.is_enabled(self.engine.sample_name(index));
        }
        let attack = ms_to_samples(self.params.attack.value(), self.engine.sample_rate());
        if let Some(triggered) = self.engine.note_on(
            NoteOn {
                note,
                velocity,
                channel,
                voice_id,
                sample: None,
                start_norm: None,
            },
            &enabled,
            attack,
        ) {
            self.shared
                .last_index
                .store(triggered.sample as u32, Ordering::Relaxed);
            let ppm = (triggered.start_norm.clamp(0.0, 1.0) * 1000.0).round() as u32;
            self.shared.last_ppm.store(ppm.min(1000), Ordering::Relaxed);
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
