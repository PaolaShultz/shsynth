//! Exact ownership for notes transformed or redirected by live routing.

/// Owns the output generated for each source channel/note. Stacks preserve
/// repeated note-ons; draining is used by route changes, stop, panic, and exit.
#[derive(Clone, Debug)]
pub struct NoteLifecycle<T> {
    active: Vec<Vec<T>>,
}

impl<T> Default for NoteLifecycle<T> {
    fn default() -> Self {
        Self {
            active: (0..16 * 128).map(|_| Vec::new()).collect(),
        }
    }
}

impl<T> NoteLifecycle<T> {
    fn index(channel: u8, note: u8) -> usize {
        usize::from(channel.min(15)) * 128 + usize::from(note.min(127))
    }

    pub fn note_on(&mut self, channel: u8, note: u8, output: T) {
        self.active[Self::index(channel, note)].push(output);
    }

    pub fn note_off(&mut self, channel: u8, note: u8) -> Option<T> {
        self.active[Self::index(channel, note)].pop()
    }

    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.active.iter_mut().flat_map(|notes| notes.drain(..))
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.active.iter().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_pairs_channels_repeats_velocity_zero_and_all_notes_off() {
        let mut notes = NoteLifecycle::default();
        notes.note_on(0, 61, 60);
        notes.note_on(0, 61, 60);
        notes.note_on(1, 61, 61);
        assert_eq!(notes.note_off(0, 61), Some(60));
        assert_eq!(notes.note_off(1, 61), Some(61));
        // A velocity-zero note-on calls the same note_off path.
        assert_eq!(notes.note_off(0, 61), Some(60));
        assert_eq!(notes.note_off(0, 61), None);
        notes.note_on(2, 63, 62);
        notes.note_on(2, 64, 64);
        assert_eq!(notes.drain().collect::<Vec<_>>(), vec![62, 64]);
        assert_eq!(notes.len(), 0);
    }

    #[test]
    fn route_transition_drain_releases_the_original_destination() {
        let mut notes = NoteLifecycle::default();
        notes.note_on(4, 64, ("old route", 64));
        assert_eq!(notes.drain().collect::<Vec<_>>(), vec![("old route", 64)]);
    }
}
