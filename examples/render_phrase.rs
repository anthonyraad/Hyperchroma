//! Render a short phrase through the engine to `renders/understory-phrase.wav`.
//! Useful where there is no audio device. Exits quietly when no recordings are embedded.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use understory::bank::load_embedded;
use understory::engine::{Engine, NoteOn};

fn main() {
    let bank = load_embedded();
    if bank.samples.is_empty() {
        eprintln!("No recordings are embedded, so there is no phrase to render.");
        if !bank.errors.is_empty() {
            for error in &bank.errors {
                eprintln!("  {error}");
            }
        }
        return;
    }

    let sample_rate = 48_000.0;
    let mut engine = Engine::new(sample_rate, bank, 7);
    let attack = 0.045 * sample_rate;
    let release = 0.7 * sample_rate;
    let mut left = Vec::new();
    let mut right = Vec::new();
    let enabled = [true; 32];

    let melody = [60u8, 64, 67, 72, 71, 67, 64, 60];
    for note in melody {
        engine
            .note_on(
                NoteOn {
                    note,
                    velocity: 0.82,
                    channel: 0,
                    voice_id: None,
                    sample: None,
                    start_norm: None,
                },
                &enabled,
                attack,
            )
            .expect("a recording is enabled");
        render(
            &mut engine,
            &mut left,
            &mut right,
            (sample_rate * 0.85) as usize,
        );
        engine.note_off(0, note, None, release);
        render(
            &mut engine,
            &mut left,
            &mut right,
            (sample_rate * 0.75) as usize,
        );
    }

    for (index, note) in [60u8, 64, 67].into_iter().enumerate() {
        engine.note_on(
            NoteOn {
                note,
                velocity: 0.7,
                channel: 0,
                voice_id: Some(index as i32),
                sample: None,
                start_norm: None,
            },
            &enabled,
            attack,
        );
    }
    render(
        &mut engine,
        &mut left,
        &mut right,
        (sample_rate * 2.4) as usize,
    );
    for note in [60u8, 64, 67] {
        engine.note_off(0, note, None, release);
    }
    render(
        &mut engine,
        &mut left,
        &mut right,
        (sample_rate * 0.9) as usize,
    );

    let dir = Path::new("renders");
    std::fs::create_dir_all(dir).expect("create renders directory");
    let path = dir.join("understory-phrase.wav");
    write_wav(&path, sample_rate as u32, &left, &right);
    eprintln!(
        "Wrote {} ({} seconds)",
        path.display(),
        left.len() as f32 / sample_rate
    );
}

fn render(engine: &mut Engine, left: &mut Vec<f32>, right: &mut Vec<f32>, frames: usize) {
    for _ in 0..frames {
        let (l, r) = engine.render(60, 1.0);
        left.push(l * 0.5);
        right.push(r * 0.5);
    }
}

fn write_wav(path: &Path, sample_rate: u32, left: &[f32], right: &[f32]) {
    let mut file = File::create(path).expect("create wav");
    let frames = left.len().min(right.len()) as u32;
    let data_bytes = frames * 4;
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36 + data_bytes).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&2u16.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&(sample_rate * 4).to_le_bytes()).unwrap();
    file.write_all(&4u16.to_le_bytes()).unwrap();
    file.write_all(&16u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_bytes.to_le_bytes()).unwrap();
    for index in 0..frames as usize {
        let l = (left[index].clamp(-1.0, 1.0) * 32767.0) as i16;
        let r = (right[index].clamp(-1.0, 1.0) * 32767.0) as i16;
        file.write_all(&l.to_le_bytes()).unwrap();
        file.write_all(&r.to_le_bytes()).unwrap();
    }
}
