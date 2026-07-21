#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoteNaming {
    German,
    English,
}

impl NoteNaming {
    pub fn config_value(self) -> &'static str {
        match self {
            Self::German => "german",
            Self::English => "english",
        }
    }

    pub(crate) fn pitch_name(self, pc: u8) -> &'static str {
        match self {
            Self::German => [
                "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "B", "H",
            ][pc as usize],
            Self::English => [
                "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
            ][pc as usize],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeldNoteDisplay {
    pub midi_note: u8,
    pub name: &'static str,
    pub velocity: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeldNoteDisplayData {
    pub chord: String,
    pub notes: Vec<HeldNoteDisplay>,
}

#[derive(Debug)]
pub struct HeldNotes {
    velocities: [[u8; 16]; 128],
}

impl Default for HeldNotes {
    fn default() -> Self {
        Self {
            velocities: [[0; 16]; 128],
        }
    }
}

impl HeldNotes {
    pub fn is_held(&self, note: u8) -> bool {
        self.velocities
            .get(usize::from(note))
            .is_some_and(|velocities| velocities.iter().any(|velocity| *velocity != 0))
    }

    pub fn observe(&mut self, message: &[u8]) {
        if message.len() != 3 || message[1] > 127 || message[2] > 127 {
            return;
        }
        let channel = usize::from(message[0] & 0x0f);
        match message[0] & 0xf0 {
            0x90 if message[2] != 0 => self.velocities[message[1] as usize][channel] = message[2],
            0x80 | 0x90 => self.velocities[message[1] as usize][channel] = 0,
            0xb0 if matches!(message[1], 120 | 123) => {
                for velocities in &mut self.velocities {
                    velocities[channel] = 0;
                }
            }
            _ => {}
        }
    }

    pub fn display(&self, naming: NoteNaming) -> Option<HeldNoteDisplayData> {
        let notes = self
            .velocities
            .iter()
            .enumerate()
            .filter_map(|(note, velocities)| {
                velocities
                    .iter()
                    .copied()
                    .max()
                    .filter(|velocity| *velocity != 0)
                    .map(|velocity| HeldNoteDisplay {
                        midi_note: note as u8,
                        name: naming.pitch_name(note as u8 % 12),
                        velocity,
                    })
            })
            .collect::<Vec<_>>();
        let bass = notes.first()?.midi_note;
        if notes.len() == 1 {
            return Some(HeldNoteDisplayData {
                chord: notes[0].name.into(),
                notes,
            });
        }

        let mut pitch_classes = notes
            .iter()
            .map(|note| note.midi_note % 12)
            .collect::<Vec<_>>();
        pitch_classes.sort_unstable();
        pitch_classes.dedup();
        if pitch_classes.len() == 1 {
            return Some(HeldNoteDisplayData {
                chord: naming.pitch_name(pitch_classes[0]).into(),
                notes,
            });
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
                let slash = (root != bass_pc).then(|| format!("/{}", naming.pitch_name(bass_pc)));
                return Some(HeldNoteDisplayData {
                    chord: format!(
                        "{}{}{}",
                        naming.pitch_name(root),
                        suffix,
                        slash.unwrap_or_default()
                    ),
                    notes,
                });
            }
        }

        let intervals = pitch_classes
            .iter()
            .map(|pc| (pc + 12 - bass_pc) % 12)
            .filter(|interval| *interval != 0)
            .map(interval_name)
            .collect::<Vec<_>>()
            .join(" ");
        Some(HeldNoteDisplayData {
            chord: format!("{} [{intervals}]", naming.pitch_name(bass_pc)),
            notes,
        })
    }
}

const CHORDS: &[(&[u8], &str)] = &[
    (&[0, 4, 7], " maj"),
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
        let display = held.display(NoteNaming::German).unwrap();
        (
            display.chord,
            display
                .notes
                .iter()
                .map(|note| note.name)
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    #[test]
    fn recognizes_note_chord_inversion_and_unusual_set() {
        assert_eq!(recognize(&[61]), ("C#".into(), "C#".into()));
        assert_eq!(recognize(&[60, 64, 67]), ("C maj".into(), "C E G".into()));
        assert_eq!(
            recognize(&[61, 66, 68, 72]),
            ("C#maj7sus4".into(), "C# F# G# C".into())
        );
        assert_eq!(recognize(&[64, 67, 72]).0, "C maj/E");
        assert_eq!(recognize(&[60, 61, 66]).0, "C [b2 b5]");
    }

    #[test]
    fn note_off_and_all_notes_off_clear_notes() {
        let mut held = HeldNotes::default();
        held.observe(&[0x91, 60, 100]);
        held.observe(&[0x81, 60, 0]);
        assert!(held.display(NoteNaming::German).is_none());
        held.observe(&[0x92, 60, 100]);
        held.observe(&[0xb2, 123, 0]);
        assert!(held.display(NoteNaming::German).is_none());
    }

    #[test]
    fn malformed_data_bytes_are_ignored() {
        let mut held = HeldNotes::default();
        held.observe(&[0x90, 255, 100]);
        held.observe(&[0x90, 60, 255]);
        held.observe(&[0x90, 60, 100, 0]);
        assert!(held.display(NoteNaming::German).is_none());
    }

    #[test]
    fn uses_central_european_b_and_h_names() {
        assert_eq!(recognize(&[70]).0, "B");
        assert_eq!(recognize(&[71]).0, "H");
    }

    #[test]
    fn english_naming_uses_a_sharp_and_b() {
        let mut held = HeldNotes::default();
        held.observe(&[0x90, 70, 100]);
        held.observe(&[0x90, 74, 100]);
        held.observe(&[0x90, 77, 100]);
        let display = held.display(NoteNaming::English).unwrap();
        assert_eq!(display.chord, "A# maj");
        assert_eq!(
            display
                .notes
                .iter()
                .map(|note| note.name)
                .collect::<Vec<_>>(),
            ["A#", "D", "F"]
        );
    }

    #[test]
    fn held_notes_retain_velocity_and_zero_velocity_is_note_off() {
        let mut held = HeldNotes::default();
        held.observe(&[0x90, 60, 37]);
        held.observe(&[0x90, 60, 55]);
        held.observe(&[0x90, 64, 92]);
        held.observe(&[0x90, 67, 127]);
        let display = held.display(NoteNaming::German).unwrap();
        assert_eq!(display.chord, "C maj");
        assert_eq!(
            display
                .notes
                .iter()
                .map(|note| (note.midi_note, note.velocity))
                .collect::<Vec<_>>(),
            [(60, 55), (64, 92), (67, 127)]
        );

        held.observe(&[0x90, 64, 0]);
        let display = held.display(NoteNaming::German).unwrap();
        assert_eq!(
            display
                .notes
                .iter()
                .map(|note| (note.midi_note, note.velocity))
                .collect::<Vec<_>>(),
            [(60, 55), (67, 127)]
        );
    }

    #[test]
    fn channel_ownership_and_duplicate_pitch_use_highest_held_velocity() {
        let mut held = HeldNotes::default();
        held.observe(&[0x90, 60, 48]);
        held.observe(&[0x91, 60, 110]);
        held.observe(&[0x91, 64, 76]);

        let display = held.display(NoteNaming::German).unwrap();
        assert_eq!(display.notes[0].velocity, 110);
        held.observe(&[0x81, 60, 0]);
        let display = held.display(NoteNaming::German).unwrap();
        assert_eq!(display.notes[0].velocity, 48);
        assert_eq!(display.notes[1].velocity, 76);

        held.observe(&[0xb0, 120, 0]);
        let display = held.display(NoteNaming::German).unwrap();
        assert_eq!(display.notes.len(), 1);
        assert_eq!(
            (display.notes[0].midi_note, display.notes[0].velocity),
            (64, 76)
        );
    }
}
