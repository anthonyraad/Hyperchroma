/// Recordings shipped with Hyperchroma, in grid order.
pub const SAMPLE_NAMES: [&str; 24] = [
    "bug", "dark", "dragon", "electric", "fairy", "mulberry", "fighting", "fire", "flying",
    "ghost", "grass", "olive", "ground", "ice", "normal", "poison", "psychic", "aloe", "rock",
    "steel", "water", "sage", "clay", "cinnamon",
];

const PITCH_CLASS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// MIDI note number to a name. Middle C (60) is C4.
pub fn note_name(note: i32) -> String {
    let note = note.clamp(0, 127);
    let pc = (note as usize) % 12;
    let octave = note / 12 - 1;
    format!("{}{}", PITCH_CLASS[pc], octave)
}

/// Parse `C4`, `C#4`, `Db4`, or a raw MIDI number.
pub fn parse_note(text: &str) -> Option<i32> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(midi) = trimmed.parse::<i32>() {
        if (0..128).contains(&midi) {
            return Some(midi);
        }
    }

    let s = trimmed.replace('♯', "#").replace('♭', "b");
    let bytes = s.as_bytes();
    let letter = bytes.first()?.to_ascii_uppercase();
    let mut pc: i32 = match letter {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let mut index = 1;
    if index < bytes.len() && (bytes[index] == b'#' || bytes[index] == b's' || bytes[index] == b'S')
    {
        pc += 1;
        index += 1;
    } else if index < bytes.len() && (bytes[index] == b'b' || bytes[index] == b'B') {
        pc -= 1;
        index += 1;
    }
    let octave: i32 = s[index..].parse().ok()?;
    let midi = (octave + 1) * 12 + pc;
    (0..128).contains(&midi).then_some(midi)
}

pub fn display_name(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_c_round_trips() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(parse_note("C4"), Some(60));
        assert_eq!(parse_note("C#4"), Some(61));
        assert_eq!(note_name(61), "C#4");
        assert_eq!(parse_note("Db4"), Some(61));
        assert_eq!(parse_note("60"), Some(60));
        assert_eq!(parse_note("B3"), Some(59));
    }
}
