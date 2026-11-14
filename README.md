# Hyperchroma

Hyperchroma is a polyphonic instrument for ambient and botanica music. Play a note and it picks one of eighteen recordings at random, starts at a random moment inside that recording, and holds the slice until you let go.

The recordings are in C at 120 BPM. Pitch and tempo are separate. With keytracking off, every note plays the recording transposed to Original pitch, so a loop in C stays in C no matter which key you hold. Turn keytracking on and the note you play transposes it. The DAW tempo time-stretches the slice either way, so a loop that pulsed at 120 still lands on the grid at 90 or 140.

## Play

Load **Hyperchroma** as an instrument. It is a VST3 and a CLAP plugin, and it also builds as a standalone app.

- **Keytracking** is off by default. Every note plays at **Original pitch**, and the key you hold only picks a slice. **C4** leaves the loops as recorded. **D4** raises every note a whole step. **C3** drops every note an octave.
- Turn **Keytracking** on and the note you play transposes around Original pitch. That note is the one that leaves a recording untransposed. It defaults to **C4**.
- **Follow the host tempo** reads the session BPM. If the host has not sent a tempo, or you turn the switch off, the **Tempo** knob is used instead.
- **Sample BPM** is the tempo the recordings were played at. Leave it at 120 unless that turns out to be wrong.
- **Attack** and **Release** shape the swell. Release defaults to 800 ms so a lifted key still breathes.
- Each recording tile is a mute. A muted recording drops out of the random draw. A glowing tile is sounding right now.
- Click the keys, or type: the **Z row starts at C3**, the **Q row starts at C4**.

A note that is held past the end of a file wraps to the start of that same file, so a long chord can drone. Sixteen notes can sound at once. The newest matching note-off releases one voice.

## Cubase

Cubase loads this as a VST3 instrument. The Windows 64-bit plugin is `release/Hyperchroma.vst3`, in the same place Sparkletop keeps `release/Sparkletop.vst3`.

The first builds never appeared because the DLL imported `api-ms-win-core-synch-l1-2-0.dll`. That name is a Windows API set, not a file in System32. Cubase's scanner points DLL search at the plugin folder and then cannot resolve the API set, so the load fails and the instrument is left off the list. Sparkletop does not fail that lookup. This build imports `WaitOnAddress` from `kernelbase.dll`, a real system DLL the scanner has already loaded, and the bundle includes `Contents/Resources/moduleinfo.json` so Cubase can read the instrument's name and class id.

1. Copy the whole `Hyperchroma.vst3` folder over `C:\Program Files\Common Files\VST3\Hyperchroma.vst3`, next to `Sparkletop.vst3`. Replace the folder that is already there.
2. In Cubase, open Studio → VST Plug-in Manager. Open the blocklist and remove Hyperchroma if it is listed. A failed scan stays hidden until that entry is gone, even after the file on disk is fixed.
3. Update the plug-in list.
4. Add an instrument track and choose Hyperchroma. It is an instrument, not an audio effect.

## Recordings

The eighteen loops are in `samples/` and are baked into the plugin when you build:

`bug.mp3`, `dark.mp3`, `dragon.mp3`, `electric.mp3`, `fairy.mp3`, `fighting.mp3`, `fire.mp3`, `flying.mp3`, `ghost.mp3`, `grass.mp3`, `ground.mp3`, `ice.mp3`, `normal.mp3`, `poison.mp3`, `psychic.mp3`, `rock.mp3`, `steel.mp3`, `water.mp3`.

A hashed upload name such as `fairy_08ef.mp3` is accepted too, if you drop a replacement into that folder. On load, each file is peak-normalized so one recording does not bury the others. Rebuild after replacing a file; the audio is compiled into the plugin.

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
- macOS: `~/Library/Audio/Plug-Ins/VST3` and `~/Library/Audio/Plug-Ins/CLAP`. Build on macOS.
- Windows: `cargo build --release --target x86_64-pc-windows-gnu`, then retarget the sync import and copy the DLL into the bundle:

```bash
python3 tools/retarget_synch_import.py target/x86_64-pc-windows-gnu/release/hyperchroma.dll
cp target/x86_64-pc-windows-gnu/release/hyperchroma.dll \
  release/Hyperchroma.vst3/Contents/x86_64-win/Hyperchroma.vst3
```

The copy already in that folder is the 64-bit Windows build. FL Studio ignores a symlinked VST3; copy the folder.

`render_phrase` writes `renders/hyperchroma-phrase.wav` when the recordings are embedded. It is a way to hear the engine with no audio device.

## What this version does not do yet

Per-recording gain, probability, start range, reverse, and filters. Pitch bend. Those are the next knobs, not this pass.
