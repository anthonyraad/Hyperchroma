# Hyperchroma

Hyperchroma is a polyphonic instrument for ambient and botanica music. Play a note and it picks one of eighteen recordings at random, starts at a random moment inside that recording, and holds the slice until you let go.

The recordings are in C at 120 BPM. Pitch and tempo are separate: the note you play transposes the slice, and the DAW tempo time-stretches it, so a loop that pulsed at 120 still lands on the grid at 90 or 140 without the key drifting.

## Play

Load **Hyperchroma** as an instrument. It is a VST3 and a CLAP plugin, and it also builds as a standalone app.

- **Original pitch** is the MIDI note that plays a recording untransposed. It defaults to **C4**. The loops are in C; the octave is a guess, so move this if they sit too high or too low against the rest of the track.
- **Follow the host tempo** reads the session BPM. If the host has not sent a tempo, or you turn the switch off, the **Tempo** knob is used instead.
- **Sample BPM** is the tempo the recordings were played at. Leave it at 120 unless that turns out to be wrong.
- **Attack** and **Release** shape the swell. Release defaults to 800 ms so a lifted key still breathes.
- Each recording tile is a mute. A muted recording drops out of the random draw. A glowing tile is sounding right now.
- Click the keys, or type: the **Z row starts at C3**, the **Q row starts at C4**.

A note that is held past the end of a file wraps to the start of that same file, so a long chord can drone. Sixteen notes can sound at once. The newest matching note-off releases one voice.

## Recordings

Put the loops in `samples/` before building. Wav or mp3. The file name is the recording's name:

`bug`, `dark`, `dragon`, `electric`, `fairy`, `fighting`, `fire`, `flying`, `ghost`, `grass`, `ground`, `ice`, `normal`, `poison`, `psychic`, `rock`, `steel`, `water`.

A hashed upload name such as `fairy_08ef.mp3` is accepted too. On load, each file is peak-normalized so one recording does not bury the others. Rebuild after adding or replacing a file; the audio is baked into the plugin.

```bash
python3 tools/import_uploads.py   # copies /workspace/uploads when that folder exists
```

## Build

Rust 1.88 or newer. The VST3 wrapper uses the Steinberg SDK, so this project is GPL-3.0-or-later.

Linux packages used by the editor and the standalone app:

```bash
sudo apt install pkg-config libgl1-mesa-dev libx11-dev libxcursor-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libasound2-dev libjack-jackd2-dev
```

```bash
cargo test
cargo xtask bundle hyperchroma --release
cargo run --features standalone --bin hyperchroma-standalone --release
cargo run --example render_phrase
```

`cargo xtask bundle` writes `target/bundled/Hyperchroma.vst3` and `target/bundled/Hyperchroma.clap`.

- Linux: copy the bundle to `~/.vst3/` or `~/.clap/`, then rescan.
- macOS: `~/Library/Audio/Plug-Ins/VST3` and `~/Library/Audio/Plug-Ins/CLAP`. Build on macOS. This tree does not cross-compile a Mac or Windows plugin.
- Windows: `C:\Program Files\Common Files\VST3` and the CLAP folder your DAW scans. Build on Windows. FL Studio ignores a symlinked VST3; copy the folder.

`render_phrase` writes `renders/hyperchroma-phrase.wav` when the recordings are embedded. It is a way to hear the engine with no audio device.

## What this version does not do yet

Per-recording gain, probability, start range, reverse, and filters. Pitch bend. Those are the next knobs, not this pass.
