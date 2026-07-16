#[derive(Debug)]
pub struct HeldNotes {
    channels: [u16; 128],
}

impl Default for HeldNotes {
    fn default() -> Self {
        Self { channels: [0; 128] }
    }
}

impl HeldNotes {
    pub fn observe(&mut self, message: &[u8]) {
        if message.len() != 3 || message[1] > 127 || message[2] > 127 {
            return;
        }
        let channel = 1u16 << (message[0] & 0x0f);
        match message[0] & 0xf0 {
            0x90 if message[2] != 0 => self.channels[message[1] as usize] |= channel,
            0x80 | 0x90 => self.channels[message[1] as usize] &= !channel,
            0xb0 if matches!(message[1], 120 | 123) => {
                for active in &mut self.channels {
                    *active &= !channel;
                }
            }
            _ => {}
        }
    }

    pub fn description(&self) -> Option<(String, String)> {
        let notes = self
            .channels
            .iter()
            .enumerate()
            .filter_map(|(note, channels)| (*channels != 0).then_some(note as u8))
            .collect::<Vec<_>>();
        let bass = *notes.first()?;
        let played = notes
            .iter()
            .map(|note| pitch_name(note % 12))
            .collect::<Vec<_>>()
            .join(" ");
        if notes.len() == 1 {
            return Some((pitch_name(notes[0] % 12).into(), played));
        }

        let mut pitch_classes = notes.iter().map(|note| note % 12).collect::<Vec<_>>();
        pitch_classes.sort_unstable();
        pitch_classes.dedup();
        if pitch_classes.len() == 1 {
            return Some((pitch_name(pitch_classes[0]).into(), played));
        }

        let bass_pc = bass % 12;
        let roots = std::iter::once(bass_pc)
            .chain(pitch_classes.iter().copied().filter(|pc| *pc != bass_pc));
        for root in roots {
            let mut intervals = pitch_classes
                .iter()
                .map(|pc| (pc + 12 - root) % 12)
                .collect::<Vec<_>>();
            intervals.sort_unstable();
            if let Some((_, suffix)) = CHORDS.iter().find(|(formula, _)| *formula == intervals) {
                let slash = (root != bass_pc).then(|| format!("/{}", pitch_name(bass_pc)));
                return Some((
                    format!(
                        "{}{}{}",
                        pitch_name(root),
                        suffix,
                        slash.unwrap_or_default()
                    ),
                    played,
                ));
            }
        }

        let intervals = pitch_classes
            .iter()
            .map(|pc| (pc + 12 - bass_pc) % 12)
            .filter(|interval| *interval != 0)
            .map(interval_name)
            .collect::<Vec<_>>()
            .join(" ");
        Some((format!("{} [{intervals}]", pitch_name(bass_pc)), played))
    }
}

const CHORDS: &[(&[u8], &str)] = &[
    (&[0, 4, 7], ""),
    (&[0, 3, 7], "m"),
    (&[0, 3, 6], "dim"),
    (&[0, 4, 8], "aug"),
    (&[0, 2, 7], "sus2"),
    (&[0, 5, 7], "sus4"),
    (&[0, 4, 7, 10], "7"),
    (&[0, 4, 7, 11], "maj7"),
    (&[0, 3, 7, 10], "m7"),
    (&[0, 3, 7, 11], "mMaj7"),
    (&[0, 3, 6, 9], "dim7"),
    (&[0, 3, 6, 10], "m7b5"),
    (&[0, 4, 7, 9], "6"),
    (&[0, 3, 7, 9], "m6"),
    (&[0, 2, 7, 10], "7sus2"),
    (&[0, 2, 7, 11], "maj7sus2"),
    (&[0, 5, 7, 10], "7sus4"),
    (&[0, 5, 7, 11], "maj7sus4"),
    (&[0, 2, 4, 7], "add9"),
    (&[0, 2, 3, 7], "madd9"),
    (&[0, 2, 4, 7, 10], "9"),
    (&[0, 2, 4, 7, 11], "maj9"),
    (&[0, 2, 3, 7, 10], "m9"),
    (&[0, 2, 4, 5, 7, 10], "11"),
    (&[0, 2, 3, 5, 7, 10], "m11"),
    (&[0, 2, 4, 7, 9, 10], "13"),
    (&[0, 2, 3, 7, 9, 10], "m13"),
];

fn pitch_name(pc: u8) -> &'static str {
    [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "B", "H",
    ][pc as usize]
}

fn interval_name(interval: u8) -> &'static str {
    [
        "1", "b2", "2", "m3", "3", "4", "b5", "5", "#5", "6", "b7", "7",
    ][interval as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recognize(notes: &[u8]) -> (String, String) {
        let mut held = HeldNotes::default();
        for note in notes {
            held.observe(&[0x90, *note, 100]);
        }
        held.description().unwrap()
    }

    #[test]
    fn recognizes_note_chord_inversion_and_unusual_set() {
        assert_eq!(recognize(&[61]), ("C#".into(), "C#".into()));
        assert_eq!(
            recognize(&[61, 66, 68, 72]),
            ("C#maj7sus4".into(), "C# F# G# C".into())
        );
        assert_eq!(recognize(&[64, 67, 72]).0, "C/E");
        assert_eq!(recognize(&[60, 61, 66]).0, "C [b2 b5]");
    }

    #[test]
    fn note_off_and_all_notes_off_clear_notes() {
        let mut held = HeldNotes::default();
        held.observe(&[0x91, 60, 100]);
        held.observe(&[0x81, 60, 0]);
        assert!(held.description().is_none());
        held.observe(&[0x92, 60, 100]);
        held.observe(&[0xb2, 123, 0]);
        assert!(held.description().is_none());
    }

    #[test]
    fn malformed_data_bytes_are_ignored() {
        let mut held = HeldNotes::default();
        held.observe(&[0x90, 255, 100]);
        held.observe(&[0x90, 60, 255]);
        held.observe(&[0x90, 60, 100, 0]);
        assert!(held.description().is_none());
    }

    #[test]
    fn uses_central_european_b_and_h_names() {
        assert_eq!(recognize(&[70]).0, "B");
        assert_eq!(recognize(&[71]).0, "H");
    }
}
