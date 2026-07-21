//! Exact ownership for notes transformed or redirected by live routing.

use std::collections::BTreeMap;

/// Owns the output generated for each source channel/note. Stacks preserve
/// repeated note-ons; draining is used by route changes, stop, panic, and exit.
#[derive(Clone, Debug)]
#[cfg(test)]
pub struct NoteLifecycle<T> {
    active: Vec<Vec<T>>,
}

#[cfg(test)]
impl<T> Default for NoteLifecycle<T> {
    fn default() -> Self {
        Self {
            active: (0..16 * 128).map(|_| Vec::new()).collect(),
        }
    }
}

#[cfg(test)]
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

/// Owns transformed output notes by physical MIDI source as well as source
/// channel/note. Two keyboards can therefore play the same channel and pitch
/// without either keyboard's release consuming the other's ownership.
#[derive(Clone, Debug)]
pub struct SourceNoteLifecycle<S, T> {
    active: BTreeMap<(S, u8, u8), Vec<T>>,
}

impl<S, T> Default for SourceNoteLifecycle<S, T> {
    fn default() -> Self {
        Self {
            active: BTreeMap::new(),
        }
    }
}

impl<S: Ord + Clone, T> SourceNoteLifecycle<S, T> {
    pub fn note_on(&mut self, source: &S, channel: u8, note: u8, output: T) {
        self.active
            .entry((source.clone(), channel.min(15), note.min(127)))
            .or_default()
            .push(output);
    }

    pub fn note_off(&mut self, source: &S, channel: u8, note: u8) -> Option<T> {
        let key = (source.clone(), channel.min(15), note.min(127));
        let output = self.active.get_mut(&key)?.pop();
        if self.active.get(&key).is_some_and(Vec::is_empty) {
            self.active.remove(&key);
        }
        output
    }

    pub fn drain_source_channel(&mut self, source: &S, channel: u8) -> Vec<T> {
        self.drain_matching(|(owned_source, owned_channel, _)| {
            owned_source == source && *owned_channel == channel.min(15)
        })
    }

    pub fn drain_source(&mut self, source: &S) -> Vec<T> {
        self.drain_matching(|(owned_source, _, _)| owned_source == source)
    }

    pub fn drain(&mut self) -> Vec<T> {
        self.drain_matching(|_| true)
    }

    fn drain_matching(&mut self, matches: impl Fn(&(S, u8, u8)) -> bool) -> Vec<T> {
        let keys = self
            .active
            .keys()
            .filter(|key| matches(key))
            .cloned()
            .collect::<Vec<_>>();
        keys.into_iter()
            .filter_map(|key| self.active.remove(&key))
            .flatten()
            .collect()
    }

    #[cfg(test)]
    fn source_len(&self) -> usize {
        self.active.values().map(Vec::len).sum()
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

    #[test]
    fn source_lifecycle_keeps_identical_notes_independent() {
        let mut notes = SourceNoteLifecycle::default();
        notes.note_on(&"controller", 0, 60, "controller output");
        notes.note_on(&"keyboard", 0, 60, "keyboard output");
        assert_eq!(
            notes.note_off(&"controller", 0, 60),
            Some("controller output")
        );
        assert_eq!(notes.source_len(), 1);
        assert_eq!(notes.note_off(&"keyboard", 0, 60), Some("keyboard output"));
        assert_eq!(notes.source_len(), 0);
    }

    #[test]
    fn source_channel_and_disconnect_drains_are_exact() {
        let mut notes = SourceNoteLifecycle::default();
        notes.note_on(&"first", 0, 60, 1);
        notes.note_on(&"first", 1, 60, 2);
        notes.note_on(&"second", 0, 60, 3);
        assert_eq!(notes.drain_source_channel(&"first", 0), [1]);
        assert_eq!(notes.drain_source(&"first"), [2]);
        assert_eq!(notes.drain(), [3]);
    }
}
