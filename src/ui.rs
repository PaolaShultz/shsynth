use crate::audio_recorder::AudioRecorder;
use crate::chord::HeldNotes;
use crate::config::RuntimeConfig;
use crate::control::{parameter_color, CONTROLS};
use crate::engine::{self, Engine, MidiEvent};
use crate::geometry::{contains, rect, visible_index};
use crate::navigation::{self, Action, Screen};
use crate::pads::TapTempo;
use crate::preset::{BackendKind, Catalog, Preset};
use crate::recording::{self, Recorder, TimedEvent};
use crate::sequencer::{self, Cell, GestureCapture, Note, PageTarget, Song, LANES_PER_PAGE};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEvent, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame, Terminal,
};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Stdout};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CPU_TEMPERATURE_REFRESH: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default)]
struct Hits {
    list: Rect,
    actions: Vec<(Rect, Action)>,
    primary: Rect,
    stop: Rect,
    exit: Rect,
    back: Rect,
    record: Rect,
    stop_record: Rect,
    playback: Rect,
    save: Rect,
    load: Rect,
    delete: Rect,
    inspect: Rect,
}
impl Hits {
    fn action(&self, x: u16, y: u16) -> Option<Action> {
        self.actions
            .iter()
            .find(|(r, _)| contains(*r, x, y))
            .copied()
            .map(|(_, a)| a)
    }
}

struct Playback {
    events: Vec<TimedEvent>,
    index: usize,
    started: Instant,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PageManagerMode {
    #[default]
    Pages,
    Target,
    Channel,
}
struct App {
    catalogs: Vec<Catalog>,
    backend_index: usize,
    presets: Vec<Preset>,
    selected: usize,
    offset: usize,
    screen: Screen,
    engine: Option<Engine>,
    playing: Option<Preset>,
    values: HashMap<u8, f32>,
    original_values: HashMap<u8, f32>,
    held_notes: HeldNotes,
    status: String,
    hits: Hits,
    recorder: Recorder,
    last: Vec<TimedEvent>,
    playback: Option<Playback>,
    tap: TapTempo,
    ideas: Vec<String>,
    idea_selected: usize,
    idea_offset: usize,
    confirm_delete: Option<String>,
    confirm_load: Option<String>,
    midi_output: engine::SharedOutput,
    pickup: engine::SharedPickup,
    midi_backend: engine::SharedBackend,
    tracker_route: engine::SharedTrackerRoute,
    config: RuntimeConfig,
    cpu_temperature: Option<f32>,
    cpu_temperature_read_at: Option<Instant>,
    pad_locked: bool,
    song: Song,
    song_list: Vec<String>,
    song_selected: usize,
    tracker_order: usize,
    tracker_row: usize,
    tracker_page: usize,
    tracker_track: usize,
    tracker_edit: bool,
    tracker_octave: u8,
    tracker_gesture: GestureCapture,
    tracker_gesture_anchor: Option<(usize, usize, usize, usize)>,
    tap_held: bool,
    confirm_song_save: bool,
    confirm_song_delete: Option<String>,
    confirm_pattern_clear: bool,
    pattern_clear_beats: u8,
    song_previewing: bool,
    sequencer: sequencer::Sequencer,
    tracker_live_input: sequencer::LiveInput,
    page_manager_original: Option<Song>,
    page_manager_mode: PageManagerMode,
    page_target_candidates: Vec<PageTarget>,
    available_page_outputs: Vec<String>,
    page_target_selected: usize,
    page_channel_draft: u8,
    audio_recorder: AudioRecorder,
}
impl App {
    fn new(
        catalogs: &[Catalog],
        midi_output: engine::SharedOutput,
        pickup: engine::SharedPickup,
        midi_backend: engine::SharedBackend,
        tracker_route: engine::SharedTrackerRoute,
        tracker_input: engine::SharedTrackerInput,
        config: RuntimeConfig,
    ) -> Self {
        let backend_index = catalogs
            .iter()
            .position(|catalog| catalog.backend == BackendKind::Synthv1)
            .unwrap_or(0);
        let presets = catalogs
            .get(backend_index)
            .map(|catalog| catalog.presets.clone())
            .unwrap_or_default();
        let song = Song::new(&config.external_midi);
        let sequencer =
            sequencer::Sequencer::start(&config.external_midi, Arc::clone(&midi_output));
        let tracker_live_input = sequencer.live_input();
        if let Ok(mut input) = tracker_input.lock() {
            *input = Some(tracker_live_input.clone());
        }
        let audio_recorder = AudioRecorder::new(config.capture.clone());
        Self {
            catalogs: catalogs.to_vec(),
            backend_index,
            presets,
            selected: 0,
            offset: 0,
            screen: Screen::Presets,
            engine: None,
            playing: None,
            values: HashMap::new(),
            original_values: HashMap::new(),
            held_notes: HeldNotes::default(),
            status: "Ready".into(),
            hits: Hits::default(),
            recorder: Recorder::default(),
            last: vec![],
            playback: None,
            tap: TapTempo::default(),
            ideas: recording::list(&recording::ideas_dir()).unwrap_or_default(),
            idea_selected: 0,
            idea_offset: 0,
            confirm_delete: None,
            confirm_load: None,
            midi_output,
            pickup,
            midi_backend,
            tracker_route,
            config,
            cpu_temperature: None,
            cpu_temperature_read_at: None,
            pad_locked: false,
            song,
            song_list: sequencer::list(&sequencer::songs_dir()),
            song_selected: 0,
            tracker_order: 0,
            tracker_row: 0,
            tracker_page: 0,
            tracker_track: 0,
            tracker_edit: false,
            tracker_octave: 4,
            tracker_gesture: GestureCapture::default(),
            tracker_gesture_anchor: None,
            tap_held: false,
            confirm_song_save: false,
            confirm_song_delete: None,
            confirm_pattern_clear: false,
            pattern_clear_beats: 4,
            song_previewing: false,
            sequencer,
            tracker_live_input,
            page_manager_original: None,
            page_manager_mode: PageManagerMode::Pages,
            page_target_candidates: Vec::new(),
            available_page_outputs: Vec::new(),
            page_target_selected: 0,
            page_channel_draft: 0,
            audio_recorder,
        }
    }

    fn selected_backend(&self) -> BackendKind {
        self.catalogs
            .get(self.backend_index)
            .map(|catalog| catalog.backend)
            .unwrap_or(BackendKind::Synthv1)
    }

    fn cycle_engine(&mut self, direction: i8) {
        if self.catalogs.is_empty() {
            return;
        }
        let next = self.selected_backend().next(direction);
        if let Some(index) = self
            .catalogs
            .iter()
            .position(|catalog| catalog.backend == next)
        {
            self.backend_index = index;
            self.presets = self.catalogs[index].presets.clone();
            self.selected = 0;
            self.offset = 0;
            self.status = self.catalogs[index]
                .unavailable
                .as_ref()
                .map(|reason| format!("{} unavailable · {reason}", next.label()))
                .unwrap_or_else(|| format!("{} · {} sounds", next.label(), self.presets.len()));
        }
    }
    fn arm_pickup(&self) {
        if let Ok(mut pickup) = self.pickup.lock() {
            pickup.arm(&self.values);
        }
    }
    fn ensure_visible(&mut self, rows: usize) {
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if rows > 0 && self.selected >= self.offset + rows {
            self.offset = self.selected + 1 - rows;
        }
    }
    fn stop_recording(&mut self) {
        if self.recorder.is_recording() {
            self.recorder.stop();
            self.last = self.recorder.events.clone();
            self.status = format!(
                "recorded {} MIDI events · Playback to review",
                self.last.len()
            );
        }
    }
    fn stop_playback(&mut self) {
        self.playback = None;
        if let Some(e) = &self.engine {
            e.panic();
        }
        self.status = "recording playback stopped · all notes off".into();
    }
    fn stop_all(&mut self, state: &Path) {
        self.cancel_tracker_gesture();
        self.sequencer.stop();
        let _ = self.audio_recorder.stop();
        self.stop_recording();
        self.stop_playback();
        self.engine.take();
        let _ = engine::stop_managed(state);
        self.playing = None;
        self.status = "synth stopped".into();
    }

    fn tracker_pattern_number(&self) -> u16 {
        self.song
            .order
            .get(self.tracker_order)
            .copied()
            .unwrap_or(0)
    }
    fn tracker_rows(&self) -> usize {
        self.song
            .patterns
            .get(&self.tracker_pattern_number())
            .map(|p| p.rows.len())
            .unwrap_or(0)
    }
    fn tracker_cell_mut(&mut self) -> Option<&mut Cell> {
        let pattern = self.song.order.get(self.tracker_order).copied()?;
        self.song
            .patterns
            .get_mut(&pattern)?
            .rows
            .get_mut(self.tracker_row)?
            .get_mut(self.tracker_page * LANES_PER_PAGE + self.tracker_track)
    }
    fn advance_tracker_row(&mut self) {
        let rows = self.tracker_rows();
        if rows > 0 {
            self.tracker_row = (self.tracker_row + 1) % rows;
        }
    }
    fn tracker_skip(&mut self) {
        if self.tracker_edit {
            self.cancel_tracker_gesture();
            self.advance_tracker_row();
            self.status = "BLANK/SKIP · row advanced".into();
        }
    }
    fn tracker_erase(&mut self) {
        self.cancel_tracker_gesture();
        if let Some(cell) = self.tracker_cell_mut() {
            *cell = Cell::default();
            self.advance_tracker_row();
            self.status = "ERASE · cell cleared · row advanced".into();
        }
    }
    fn cancel_tracker_gesture(&mut self) {
        self.tracker_gesture.cancel();
        self.tracker_gesture_anchor = None;
        if let Some(page) = self.song.pages.get(self.tracker_page) {
            self.tracker_live_input.cancel(&page.target, page.channel);
        }
    }
    fn tracker_single_note(&mut self, note: u8, velocity: u8) {
        if !self.tracker_edit {
            return;
        }
        if let Some(cell) = self.tracker_cell_mut() {
            *cell = Cell {
                note: Note::On(note),
                velocity: Some(velocity),
                ..Cell::default()
            };
        }
        self.advance_tracker_row();
    }
    fn commit_tracker_gesture(&mut self, now: Instant) {
        let Some(gesture) = self
            .tracker_gesture
            .finish(now, self.config.external_midi.gesture_settle)
        else {
            return;
        };
        if gesture.overflowed {
            self.tracker_gesture_anchor = None;
            self.status = "gesture rejected · maximum four distinct notes".into();
            return;
        }
        let (order, row_index, page_index, first_lane) =
            self.tracker_gesture_anchor.take().unwrap_or((
                self.tracker_order,
                self.tracker_row,
                self.tracker_page,
                self.tracker_track,
            ));
        let pattern_number = self.song.order.get(order).copied().unwrap_or(0);
        if let Some(row) = self
            .song
            .patterns
            .get_mut(&pattern_number)
            .and_then(|pattern| pattern.rows.get_mut(row_index))
        {
            let start = page_index * LANES_PER_PAGE;
            for (lane, (note, velocity)) in gesture.notes.into_iter().enumerate() {
                let destination = start + (first_lane + lane) % LANES_PER_PAGE;
                row[destination] = Cell {
                    note: Note::On(note),
                    velocity: Some(velocity),
                    ..Cell::default()
                };
            }
        }
        self.tracker_order = order;
        self.tracker_row = row_index;
        self.advance_tracker_row();
        self.status = "gesture entered · row advanced".into();
    }
    fn set_tracker_edit(&mut self, enabled: bool) {
        if !enabled {
            self.cancel_tracker_gesture();
        }
        self.tracker_edit = enabled;
        self.sync_tracker_route();
    }
    fn sync_tracker_route(&self) {
        let Some(page) = self.song.pages.get(self.tracker_page) else {
            return;
        };
        if let Ok(mut route) = self.tracker_route.lock() {
            route.configure(
                self.screen == Screen::Tracker && self.tracker_edit,
                page.target.clone(),
                page.channel,
                page.percussion,
                page.program,
                &self.config.external_midi,
            );
        }
    }
    fn move_tracker_lane(&mut self, direction: i8) {
        let current = self.tracker_page * LANES_PER_PAGE + self.tracker_track;
        let next = if direction < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(self.song.total_lanes().saturating_sub(1))
        };
        let page = next / LANES_PER_PAGE;
        self.cancel_tracker_gesture();
        self.tracker_page = page;
        self.tracker_track = next % LANES_PER_PAGE;
        self.sync_tracker_route();
    }
    fn switch_tracker_page(&mut self) {
        self.cancel_tracker_gesture();
        self.tracker_page = (self.tracker_page + 1) % self.song.pages.len().max(1);
        self.sync_tracker_route();
        self.status = format!("{} page", self.song.pages[self.tracker_page].name);
    }
    fn refresh_page_targets(&mut self) {
        let mut targets = vec![PageTarget::ActiveInstrument];
        if !self.config.external_midi.output_match.is_empty() {
            targets.push(PageTarget::ConfiguredExternal);
        }
        if let Ok(outputs) =
            sequencer::available_midi_outputs(&self.config.external_midi.client_name)
        {
            let managed = [
                self.config.midi_output_match.as_str(),
                self.config.yoshimi.backend.midi_output_match.as_str(),
                self.config.fluidsynth.backend.midi_output_match.as_str(),
            ];
            self.available_page_outputs = outputs
                .into_iter()
                .filter(|name| {
                    let name = name.to_lowercase();
                    !managed
                        .iter()
                        .any(|needle| !needle.is_empty() && name.contains(&needle.to_lowercase()))
                })
                .collect();
            targets.extend(
                self.available_page_outputs
                    .iter()
                    .cloned()
                    .map(PageTarget::Midi),
            );
        } else {
            self.available_page_outputs.clear();
        }
        if let Some(target) = self
            .song
            .pages
            .get(self.tracker_page)
            .map(|page| page.target.clone())
        {
            targets.push(target);
        }
        targets.sort();
        targets.dedup();
        self.page_target_candidates = targets;
    }
    fn target_online(&self, target: &PageTarget) -> bool {
        match target {
            PageTarget::ActiveInstrument => self.engine.is_some(),
            PageTarget::ConfiguredExternal => {
                self.config.external_midi.enabled
                    && self.available_page_outputs.iter().any(|name| {
                        name.to_lowercase()
                            .contains(&self.config.external_midi.output_match.to_lowercase())
                    })
            }
            PageTarget::Midi(name) => self.available_page_outputs.contains(name),
        }
    }
    fn open_page_manager(&mut self) {
        self.tracker_stop();
        self.set_tracker_edit(false);
        self.page_manager_original = Some(self.song.clone());
        self.page_manager_mode = PageManagerMode::Pages;
        self.refresh_page_targets();
        self.screen = Screen::TrackerPages;
        self.status = "select page · TARGET or CHANNEL · DONE saves changes".into();
    }
    fn cancel_page_manager(&mut self) {
        if let Some(song) = self.page_manager_original.take() {
            self.song = song;
        }
        self.tracker_page = self
            .tracker_page
            .min(self.song.pages.len().saturating_sub(1));
        self.tracker_track = self.tracker_track.min(LANES_PER_PAGE - 1);
        self.page_manager_mode = PageManagerMode::Pages;
        self.screen = Screen::Tracker;
        self.sync_tracker_route();
        self.status = "page changes cancelled".into();
    }
    fn confirm_page_manager(&mut self) {
        if self.page_manager_mode != PageManagerMode::Pages {
            self.confirm_page_field();
            return;
        }
        self.page_manager_original = None;
        self.screen = Screen::Tracker;
        self.sync_tracker_route();
        self.status = format!("{} pages ready", self.song.pages.len());
    }
    fn move_page_selection(&mut self, direction: i8) {
        if self.page_manager_mode != PageManagerMode::Pages {
            return;
        }
        self.tracker_page = if direction < 0 {
            self.tracker_page.saturating_sub(1)
        } else {
            (self.tracker_page + 1).min(self.song.pages.len().saturating_sub(1))
        };
        self.refresh_page_targets();
    }
    fn add_tracker_page(&mut self) {
        if self.page_manager_mode != PageManagerMode::Pages {
            return;
        }
        let target = self
            .song
            .pages
            .get(self.tracker_page)
            .map(|page| page.target.clone())
            .unwrap_or(PageTarget::ConfiguredExternal);
        let channel = self
            .song
            .pages
            .get(self.tracker_page)
            .map_or(0, |page| page.channel);
        match self.song.add_page(target, channel) {
            Ok(page) => {
                self.tracker_page = page;
                self.tracker_track = 0;
                self.refresh_page_targets();
                self.status = "page added · four empty lanes · choose target/channel".into();
            }
            Err(error) => self.status = format!("add page: {error}"),
        }
    }
    fn edit_page_target(&mut self) {
        if self.page_manager_mode != PageManagerMode::Pages {
            return;
        }
        self.refresh_page_targets();
        let current = &self.song.pages[self.tracker_page].target;
        self.page_target_selected = self
            .page_target_candidates
            .iter()
            .position(|target| target == current)
            .unwrap_or(0);
        self.page_manager_mode = PageManagerMode::Target;
        self.status = "turn encoder for target · press to confirm · STOP cancels field".into();
    }
    fn edit_page_channel(&mut self) {
        if self.page_manager_mode != PageManagerMode::Pages {
            return;
        }
        self.page_channel_draft = self.song.pages[self.tracker_page].channel;
        self.page_manager_mode = PageManagerMode::Channel;
        self.status = "turn encoder for channel 1–16 · press to confirm".into();
    }
    fn confirm_page_field(&mut self) {
        if let Some(page) = self.song.pages.get_mut(self.tracker_page) {
            match self.page_manager_mode {
                PageManagerMode::Target => {
                    if let Some(target) = self.page_target_candidates.get(self.page_target_selected)
                    {
                        page.target = target.clone();
                    }
                }
                PageManagerMode::Channel => page.channel = self.page_channel_draft,
                PageManagerMode::Pages => return,
            }
        }
        self.page_manager_mode = PageManagerMode::Pages;
        self.status = "page route updated · DONE to keep or CANCEL to restore".into();
    }
    fn cancel_page_field(&mut self) {
        self.page_manager_mode = PageManagerMode::Pages;
        self.status = "field change cancelled".into();
    }
    fn turn_page_manager(&mut self, direction: i8) {
        match self.page_manager_mode {
            PageManagerMode::Pages => self.move_page_selection(direction),
            PageManagerMode::Target => {
                let last = self.page_target_candidates.len().saturating_sub(1);
                self.page_target_selected = if direction < 0 {
                    self.page_target_selected.saturating_sub(1)
                } else {
                    (self.page_target_selected + 1).min(last)
                };
            }
            PageManagerMode::Channel => {
                self.page_channel_draft = if direction < 0 {
                    self.page_channel_draft.saturating_sub(1)
                } else {
                    self.page_channel_draft.saturating_add(1).min(15)
                };
            }
        }
    }
    fn toggle_tracker_page_mute(&mut self) {
        if let Some(page) = self.song.pages.get_mut(self.tracker_page) {
            page.enabled = !page.enabled;
            let muted = !page.enabled;
            let name = page.name.clone();
            self.sequencer.mute_page(self.tracker_page, muted);
            self.status = format!("{name} page {}", if muted { "muted" } else { "enabled" });
        }
    }
    fn set_tracker_tempo(&mut self, bpm: u16) {
        self.song.tempo = bpm.clamp(20, 300);
        self.sequencer.tempo(self.song.tempo);
        self.status = format!("tracker tempo {} BPM", self.song.tempo);
    }
    fn tracker_keyboard_note(&self, semitone: u8) -> u8 {
        let percussion = self
            .song
            .pages
            .get(self.tracker_page)
            .is_some_and(|page| page.percussion);
        let base = if percussion {
            if let Some(&note) = self
                .config
                .external_midi
                .percussion_notes
                .get(usize::from(semitone))
            {
                return note;
            }
            36
        } else {
            self.tracker_octave.saturating_add(1).saturating_mul(12)
        };
        base.saturating_add(semitone).min(127)
    }
    fn tracker_stop(&mut self) {
        self.cancel_tracker_gesture();
        self.sequencer.stop();
        self.song_previewing = false;
        self.status = "tracker stopped · STOP again goes back".into();
    }
    fn tracker_play(&mut self, from_start: bool) {
        let status = self.sequencer.status();
        if status.playing {
            self.tracker_stop();
            return;
        }
        self.cancel_tracker_gesture();
        let (order, row) = if from_start {
            (0, 0)
        } else {
            (self.tracker_order, self.tracker_row)
        };
        let notes = match sequencer::schedule(&self.song, &self.config.external_midi, order, row) {
            Ok(messages) => messages
                .iter()
                .filter(|message| {
                    message
                        .bytes
                        .first()
                        .is_some_and(|status| status & 0xf0 == 0x90)
                })
                .count(),
            Err(error) => {
                self.status = format!("tracker cannot play: {error}");
                return;
            }
        };
        if notes == 0 {
            self.status =
                "tracker has no notes from this position · enable EDIT and enter notes".into();
            return;
        }
        self.sequencer.play(&self.song, order, row);
        let offline = self
            .song
            .pages
            .iter()
            .filter(|page| page.enabled && !self.target_online(&page.target))
            .count();
        self.status = if offline == 0 {
            format!("tracker playing · {notes} note events")
        } else {
            format!("tracker playing · {notes} events · {offline} target(s) offline")
        };
    }
    fn save_song(&mut self) {
        match sequencer::save(&sequencer::songs_dir(), &self.song, self.confirm_song_save) {
            Ok(path) => {
                self.status = format!("saved {}", path.display());
                self.confirm_song_save = false;
                self.song_list = sequencer::list(&sequencer::songs_dir());
                self.song_selected = self
                    .song_list
                    .iter()
                    .position(|name| name == &sequencer::safe_name(&self.song.name))
                    .unwrap_or(0);
            }
            Err(error) if !self.confirm_song_save && error.to_string().contains("confirm") => {
                self.confirm_song_save = true;
                self.status = "song exists · SAVE again to overwrite".into();
            }
            Err(error) => {
                self.status = format!("song save: {error}");
                self.confirm_song_save = false;
            }
        }
    }
    fn load_song(&mut self) {
        let Some(name) = self.song_list.get(self.song_selected).cloned() else {
            self.status = "no saved songs".into();
            return;
        };
        self.tracker_stop();
        self.song_previewing = false;
        match sequencer::load(&sequencer::songs_dir(), &name) {
            Ok(song) => {
                self.song = song;
                self.tracker_order = 0;
                self.tracker_row = 0;
                self.tracker_page = 0;
                self.tracker_track = 0;
                self.screen = Screen::Tracker;
                self.refresh_page_targets();
                self.sync_tracker_route();
                self.status = format!("loaded {name}");
            }
            Err(e) => self.status = format!("song load: {e}"),
        }
    }
    fn preview_song(&mut self) {
        if self.song_previewing {
            self.sequencer.stop();
            self.song_previewing = false;
            self.status = "song preview stopped".into();
            return;
        }
        let Some(name) = self.song_list.get(self.song_selected) else {
            self.status = "no saved song selected".into();
            return;
        };
        match sequencer::load(&sequencer::songs_dir(), name) {
            Ok(song) => {
                let notes =
                    sequencer::schedule(&song, &self.config.external_midi, 0, 0).map(|messages| {
                        messages
                            .iter()
                            .filter(|message| {
                                message
                                    .bytes
                                    .first()
                                    .is_some_and(|status| status & 0xf0 == 0x90)
                            })
                            .count()
                    });
                match notes {
                    Ok(0) => self.status = format!("{name} has no notes to preview"),
                    Ok(notes) => {
                        self.sequencer.play(&song, 0, 0);
                        self.song_previewing = true;
                        self.status = format!("previewing {name} · {notes} notes");
                    }
                    Err(error) => self.status = format!("song preview: {error}"),
                }
            }
            Err(error) => self.status = format!("song preview: {error}"),
        }
    }
    fn delete_song(&mut self) {
        let Some(name) = self.song_list.get(self.song_selected).cloned() else {
            self.status = "no saved song selected".into();
            return;
        };
        if self.confirm_song_delete.as_deref() != Some(&name) {
            self.confirm_song_delete = Some(name.clone());
            self.status = format!("confirm DELETE {name}: press DELETE again");
            return;
        }
        if self.song_previewing {
            self.sequencer.stop();
            self.song_previewing = false;
        }
        match sequencer::delete(&sequencer::songs_dir(), &name) {
            Ok(()) => {
                self.song_list = sequencer::list(&sequencer::songs_dir());
                self.song_selected = self
                    .song_selected
                    .min(self.song_list.len().saturating_sub(1));
                self.confirm_song_delete = None;
                self.status = format!("deleted {name}");
            }
            Err(error) => self.status = format!("song delete: {error}"),
        }
    }
    fn choose_pattern_clear(&mut self) {
        self.pattern_clear_beats = if self.tracker_rows() == 24 { 3 } else { 4 };
        self.confirm_pattern_clear = true;
        self.status = "choose 3/4 or 4/4 · press master rotary to clear".into();
    }
    fn apply_pattern_clear(&mut self) {
        self.tracker_stop();
        let number = self.tracker_pattern_number();
        let rows = if self.pattern_clear_beats == 3 {
            24
        } else {
            32
        };
        let lanes = self.song.total_lanes();
        if let Some(pattern) = self.song.patterns.get_mut(&number) {
            *pattern = sequencer::Pattern::empty(rows, lanes);
        }
        self.tracker_row = 0;
        self.confirm_pattern_clear = false;
        self.status = format!(
            "cleared pattern {number} · {}/4 · {rows} rows",
            self.pattern_clear_beats
        );
    }
    fn new_pattern(&mut self) {
        self.tracker_stop();
        let number = self.song.patterns.keys().next_back().copied().unwrap_or(0) + 1;
        self.song.patterns.insert(
            number,
            sequencer::Pattern::empty(
                self.config.external_midi.default_pattern_rows,
                self.song.total_lanes(),
            ),
        );
        self.song.order.push(number);
        self.tracker_order = self.song.order.len() - 1;
        self.tracker_row = 0;
        self.confirm_pattern_clear = false;
        self.status = format!(
            "new pattern {number} · order {:02}/{:02} · BACK to edit, SAVE stores all patterns",
            self.tracker_order + 1,
            self.song.order.len()
        );
    }
    fn toggle_audio_recording(&mut self) {
        let result = if self.audio_recorder.status().recording {
            self.audio_recorder.stop()
        } else {
            self.audio_recorder.start(None)
        };
        self.status = match result {
            Ok(()) => {
                if self.audio_recorder.status().recording {
                    "audio recording started".into()
                } else {
                    "audio recording finalized".into()
                }
            }
            Err(e) => format!("audio recorder: {e}"),
        };
    }
    fn load(&mut self, state: &Path, _tx: std::sync::mpsc::Sender<MidiEvent>) {
        if let Some(reason) = self
            .catalogs
            .get(self.backend_index)
            .and_then(|catalog| catalog.unavailable.as_ref())
        {
            self.status = format!("{} unavailable · {reason}", self.selected_backend().label());
            return;
        }
        if self.presets.is_empty() {
            return;
        }
        self.stop_recording();
        self.stop_playback();
        let p = self.presets[self.selected].clone();
        self.original_values = engine::initial_values(&p).unwrap_or_default();
        self.values = self.original_values.clone();
        self.arm_pickup();
        if let Some(engine) = self.engine.as_mut() {
            match engine.load_in_place(&p) {
                Ok(true) => {
                    self.playing = Some(p);
                    self.screen = Screen::Playback;
                    self.status = format!(
                        "{} sound loaded in place · MIDI ready",
                        engine.backend().label()
                    );
                    return;
                }
                Ok(false) => {}
                Err(error) => {
                    self.status = format!("IN-PLACE LOAD FAILED: {error:#}");
                    return;
                }
            }
        }
        self.engine.take();
        if let Ok(mut backend) = self.midi_backend.lock() {
            *backend = p.backend;
        }
        self.status = format!("starting JACK/{}…", p.backend.label());
        let backend_label = p.backend.label();
        match Engine::start(&p, state, Arc::clone(&self.midi_output), &self.config) {
            Ok(e) => {
                self.engine = Some(e);
                self.playing = Some(p);
                self.screen = Screen::Playback;
                self.status = format!("{backend_label} running · MIDI ready");
            }
            Err(e) => {
                self.status = format!("START FAILED: {e:#} · check JACK/log; select Play to retry");
            }
        }
    }
    fn reset_parameters(&mut self) {
        if self.playing.is_none() {
            self.status = "load a preset before resetting it".into();
            return;
        }
        let Some(engine) = &self.engine else {
            self.status = "engine is not running · reload the preset from Presets".into();
            return;
        };
        if !engine.supports_parameter_reset() {
            self.status = format!(
                "{} has no mapped-parameter reset · sound unchanged",
                engine.backend().label()
            );
            return;
        }
        if let Err(error) = engine.set_mapped_parameters(&self.original_values) {
            self.status = format!("PARAMETER RESET FAILED: {error:#}");
            return;
        }
        self.values = self.original_values.clone();
        self.arm_pickup();
        self.status = "parameters reset · controls waiting for pickup".into();
    }
    fn begin_record(&mut self) {
        if self.engine.is_none() {
            self.status = "load a preset before recording".into();
            return;
        }
        self.stop_playback();
        self.recorder.start(Instant::now());
        self.status = "● RECORDING musical MIDI".into();
    }
    fn toggle_playback(&mut self) {
        if self.playback.is_some() {
            self.stop_playback();
        } else if self.last.is_empty() {
            self.status = "no recording yet".into();
        } else {
            self.playback = Some(Playback {
                events: self.last.clone(),
                index: 0,
                started: Instant::now(),
            });
            self.status = "▶ playing recording".into();
        }
    }
    fn open_ideas(&mut self) {
        if self.screen == Screen::Tracker {
            self.set_tracker_edit(false);
        }
        self.ideas = recording::list(&recording::ideas_dir()).unwrap_or_default();
        self.idea_selected = self.idea_selected.min(self.ideas.len().saturating_sub(1));
        self.confirm_delete = None;
        self.screen = Screen::Ideas;
        self.status = "ideas · select an action".into();
    }
    fn save_new(&mut self) {
        let Some(p) = &self.playing else {
            self.status = "load a preset before saving an idea".into();
            return;
        };
        if self.last.is_empty() {
            self.status = "nothing recorded · record an idea first".into();
            return;
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let name = format!("{}-{stamp}", recording::safe_name(&p.name));
        match recording::save(&recording::ideas_dir(), &name, p, &self.values, &self.last) {
            Ok(path) => {
                self.status = format!(
                    "saved idea {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                self.ideas = recording::list(&recording::ideas_dir()).unwrap_or_default();
            }
            Err(e) => self.status = format!("save failed: {e:#}"),
        }
    }
    fn finish_and_save(&mut self) {
        if !self.recorder.is_recording() {
            return;
        }
        self.stop_recording();
        let Some(preset) = &self.playing else { return };
        let stem = recording::safe_name(&preset.name);
        let base = recording::ideas_dir();
        let name = (1..=9999)
            .map(|n| format!("{stem}-{n:03}"))
            .find(|name| !base.join(name).exists());
        let Some(name) = name else {
            self.status = "Save failed: idea numbers exhausted".into();
            return;
        };
        match recording::save(&base, &name, preset, &self.values, &self.last) {
            Ok(_) => {
                self.status = format!("Saved {name}");
                self.ideas = recording::list(&base).unwrap_or_default();
            }
            Err(e) => self.status = format!("Save failed: {e:#}"),
        }
    }
    fn inspect_idea(&mut self) {
        let Some(name) = self.ideas.get(self.idea_selected) else {
            self.status = "no saved idea selected".into();
            return;
        };
        match recording::inspect(&recording::ideas_dir(), name) {
            Ok(text) => self.status = truncate(&text.replace('\n', " "), 120),
            Err(e) => self.status = format!("inspect failed: {e:#}"),
        }
    }
    fn delete_idea(&mut self) {
        let Some(name) = self.ideas.get(self.idea_selected).cloned() else {
            self.status = "no saved idea selected".into();
            return;
        };
        if self.confirm_delete.as_deref() != Some(&name) {
            self.confirm_delete = Some(name.clone());
            self.status = format!("CONFIRM DELETE {name}: choose Delete again; Back cancels");
            return;
        }
        match recording::delete(&recording::ideas_dir(), &name) {
            Ok(()) => {
                self.status = format!("deleted {name}");
                self.ideas = recording::list(&recording::ideas_dir()).unwrap_or_default();
                self.idea_selected = self.idea_selected.min(self.ideas.len().saturating_sub(1));
                self.confirm_delete = None;
            }
            Err(e) => self.status = format!("delete failed: {e:#}"),
        }
    }
    fn load_idea(&mut self, state: &Path, _tx: std::sync::mpsc::Sender<MidiEvent>) {
        let Some(name) = self.ideas.get(self.idea_selected).cloned() else {
            self.status = "no saved idea selected".into();
            return;
        };
        if self.playing.is_some() && self.confirm_load.as_deref() != Some(&name) {
            self.confirm_load = Some(name.clone());
            self.status = format!("CONFIRM REPLACE current preset with {name}: choose Load again");
            return;
        }
        match recording::load(&recording::ideas_dir(), &name) {
            Ok((preset, events)) => {
                self.stop_recording();
                self.stop_playback();
                self.original_values = engine::initial_values(&preset).unwrap_or_default();
                self.values = self.original_values.clone();
                self.arm_pickup();
                if let Some(engine) = self.engine.as_mut() {
                    match engine.load_in_place(&preset) {
                        Ok(true) => {
                            self.playing = Some(preset);
                            self.last = events;
                            self.screen = Screen::Playback;
                            self.status = format!("loaded idea {name} in place · recording ready");
                            self.confirm_load = None;
                            return;
                        }
                        Ok(false) => {}
                        Err(error) => {
                            self.status = format!("idea sound load failed: {error:#}");
                            return;
                        }
                    }
                }
                self.engine.take();
                if let Ok(mut backend) = self.midi_backend.lock() {
                    *backend = preset.backend;
                }
                match Engine::start(&preset, state, Arc::clone(&self.midi_output), &self.config) {
                    Ok(engine) => {
                        self.engine = Some(engine);
                        self.playing = Some(preset);
                        self.last = events;
                        self.screen = Screen::Playback;
                        self.status = format!("loaded idea {name} · recording ready");
                        self.confirm_load = None;
                    }
                    Err(e) => {
                        self.status = format!("idea load failed: {e:#} · check JACK/MIDI and retry")
                    }
                }
            }
            Err(e) => self.status = format!("idea read failed: {e:#}"),
        }
    }
    fn tick(&mut self) {
        let now = Instant::now();
        self.refresh_cpu_temperature(now);
        if self.screen == Screen::Tracker && self.tracker_edit {
            self.commit_tracker_gesture(now);
        }
        if self.screen == Screen::Tracker {
            let tracker = self.sequencer.status();
            self.follow_tracker_transport(&tracker);
            if tracker.playing && !tracker.available {
                self.cancel_tracker_gesture();
                if let Some(error) = tracker.error {
                    self.status = format!("tracker target unavailable: {error}");
                }
            }
        }
        if self.engine.as_mut().is_some_and(|engine| !engine.alive()) {
            self.engine.take();
            self.playing = None;
            self.playback = None;
            self.status = "ENGINE EXITED · select a sound to restart it".into();
        }
        let mut done = false;
        if let Some(pb) = &mut self.playback {
            while pb.index < pb.events.len()
                && Duration::from_micros(pb.events[pb.index].micros) <= pb.started.elapsed()
            {
                if let Some(e) = &self.engine {
                    let _ = e.send(&pb.events[pb.index].bytes);
                }
                pb.index += 1;
            }
            done = pb.index >= pb.events.len();
        }
        if done {
            self.stop_playback();
            self.status = "recording playback complete · all notes off".into();
        }
    }

    fn follow_tracker_transport(&mut self, tracker: &sequencer::SequencerStatus) {
        if tracker.playing {
            self.tracker_order = tracker.order.min(self.song.order.len().saturating_sub(1));
            self.tracker_row = tracker.row.min(self.tracker_rows().saturating_sub(1));
        }
    }

    fn refresh_cpu_temperature(&mut self, now: Instant) {
        if self
            .cpu_temperature_read_at
            .is_some_and(|last| now.duration_since(last) < CPU_TEMPERATURE_REFRESH)
        {
            return;
        }
        self.cpu_temperature_read_at = Some(now);
        self.cpu_temperature = self
            .config
            .cpu_temperature_path
            .as_deref()
            .and_then(read_cpu_temperature);
    }
}

fn read_cpu_temperature(path: &Path) -> Option<f32> {
    let raw = fs::read_to_string(path).ok()?;
    let value = raw.trim().parse::<f32>().ok()?;
    Some(if value.abs() >= 1_000.0 {
        value / 1_000.0
    } else {
        value
    })
}

struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

pub fn run(catalogs: &[Catalog], state: &Path, config: &RuntimeConfig) -> Result<()> {
    enable_raw_mode()?;
    let _restore = Restore;
    execute!(io::stdout(), EnterAlternateScreen, crossterm::cursor::Hide)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    app_loop(&mut terminal, catalogs, state, config)
}

fn app_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    catalogs: &[Catalog],
    state: &Path,
    config: &RuntimeConfig,
) -> Result<()> {
    let stopping = Arc::new(AtomicBool::new(false));
    for sig in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        signal_hook::flag::register(sig, Arc::clone(&stopping))?;
    }
    let (tx, rx) = mpsc::channel();
    let router = engine::MidiRouter::start(state, config, tx.clone());
    let output = router
        .as_ref()
        .map(engine::MidiRouter::output)
        .unwrap_or_else(|_| Arc::new(std::sync::Mutex::new(None)));
    let pickup = router
        .as_ref()
        .map(engine::MidiRouter::pickup)
        .unwrap_or_else(|_| Arc::new(std::sync::Mutex::new(crate::midi::Pickup::default())));
    let midi_backend = router
        .as_ref()
        .map(engine::MidiRouter::backend)
        .unwrap_or_else(|_| Arc::new(std::sync::Mutex::new(BackendKind::Synthv1)));
    let tracker_route = router
        .as_ref()
        .map(engine::MidiRouter::tracker_route)
        .unwrap_or_else(|_| Arc::new(std::sync::Mutex::new(engine::TrackerRoute::default())));
    let tracker_input = router
        .as_ref()
        .map(engine::MidiRouter::tracker_input)
        .unwrap_or_else(|_| Arc::new(std::sync::Mutex::new(None)));
    let mut app = App::new(
        catalogs,
        output,
        pickup,
        midi_backend,
        tracker_route,
        tracker_input,
        config.clone(),
    );
    if let Err(e) = &router {
        app.status = format!("MIDI: {e:#}");
    }
    let _router = router.ok();
    let mut quit = false;
    let mut drawn_screen = None;
    while !quit && !stopping.load(Ordering::Relaxed) {
        drain(&rx, &mut app, state, &tx);
        app.tick();
        if drawn_screen != Some(app.screen) {
            terminal.clear()?;
            drawn_screen = Some(app.screen);
        }
        terminal.draw(|f| draw(f, &mut app))?;
        if !event::poll(Duration::from_millis(30))? {
            continue;
        }
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                quit = key(k.code, &mut app, state, &tx)
            }
            Event::Mouse(m) => quit = mouse(m, &mut app, state, &tx),
            _ => {}
        }
    }
    app.stop_all(state);
    Ok(())
}

fn drain(
    rx: &Receiver<MidiEvent>,
    app: &mut App,
    state: &Path,
    tx: &std::sync::mpsc::Sender<MidiEvent>,
) {
    while let Ok(ev) = rx.try_recv() {
        match ev {
            MidiEvent::Value(cc, v) => {
                app.values.insert(cc, v);
            }
            MidiEvent::Raw(bytes) => {
                let now = Instant::now();
                app.held_notes.observe(&bytes);
                app.recorder.capture(now, &bytes);
                if !(app.screen == Screen::Tracker && app.tracker_edit) {
                    app.sequencer.thru(&bytes);
                }
                if app.screen == Screen::Tracker && app.tracker_edit {
                    if !app.tracker_gesture.is_active()
                        && bytes.len() >= 3
                        && bytes[0] & 0xf0 == 0x90
                        && bytes[2] > 0
                    {
                        app.tracker_gesture_anchor = Some((
                            app.tracker_order,
                            app.tracker_row,
                            app.tracker_page,
                            app.tracker_track,
                        ));
                    }
                    app.tracker_gesture.observe(now, &bytes);
                }
            }
            MidiEvent::Pad(pad, pressed) => {
                if pad == crate::pads::PadAction::TapTempo {
                    app.tap_held = pressed;
                }
                if pressed {
                    if app.screen == Screen::Tracker
                        && app.tracker_edit
                        && pad == crate::pads::PadAction::TapTempo
                    {
                        app.tracker_erase();
                    } else if let Some(action) = navigation::pad_action(app.screen, pad) {
                        perform(action, app, state, Some(tx));
                    }
                }
            }
            MidiEvent::Encoder(action) => {
                if app.screen == Screen::Tracker && app.tap_held {
                    let delta = match action {
                        crate::pads::EncoderAction::Up => -1,
                        crate::pads::EncoderAction::Down => 1,
                        crate::pads::EncoderAction::Select => 0,
                    };
                    match delta.cmp(&0) {
                        std::cmp::Ordering::Less => {
                            app.set_tracker_tempo(app.song.tempo.saturating_sub(1))
                        }
                        std::cmp::Ordering::Greater => {
                            app.set_tracker_tempo(app.song.tempo.saturating_add(1))
                        }
                        std::cmp::Ordering::Equal => {}
                    }
                    continue;
                }
                let action = match action {
                    crate::pads::EncoderAction::Up => Action::Up,
                    crate::pads::EncoderAction::Down => Action::Down,
                    crate::pads::EncoderAction::Select => Action::Activate,
                };
                perform(action, app, state, Some(tx));
            }
            MidiEvent::PadLock(locked) => {
                app.pad_locked = locked;
                if locked {
                    app.tap_held = false;
                }
                app.status = if locked {
                    "pad lock on · command pads play as notes".into()
                } else {
                    "pad lock off · command pads restored".into()
                };
            }
            MidiEvent::Error(e) => app.status = e,
        }
    }
}
fn perform(
    action: Action,
    a: &mut App,
    state: &Path,
    tx: Option<&std::sync::mpsc::Sender<MidiEvent>>,
) -> bool {
    if a.screen == Screen::TrackerFiles && a.confirm_pattern_clear {
        match action {
            Action::Up => {
                a.pattern_clear_beats = 3;
                a.status = "3/4 · 24 rows · press master rotary to clear".into();
            }
            Action::Down => {
                a.pattern_clear_beats = 4;
                a.status = "4/4 · 32 rows · press master rotary to clear".into();
            }
            Action::Activate => a.apply_pattern_clear(),
            Action::Back => {
                a.confirm_pattern_clear = false;
                a.status = "pattern clear cancelled".into();
            }
            _ => {}
        }
        return false;
    }
    match action {
        Action::Noop => {}
        Action::Arp => a.status = "ARP · future".into(),
        Action::Up => {
            if a.screen == Screen::Ideas {
                a.idea_selected = a.idea_selected.saturating_sub(1);
            } else if a.screen == Screen::TrackerFiles {
                a.song_selected = a.song_selected.saturating_sub(1);
                a.confirm_song_delete = None;
            } else if a.screen == Screen::Tracker {
                a.cancel_tracker_gesture();
                a.tracker_row = a.tracker_row.saturating_sub(1);
            } else if a.screen == Screen::TrackerPages {
                a.turn_page_manager(-1);
            } else if a.screen == Screen::Presets {
                a.selected = a.selected.saturating_sub(1);
            }
        }
        Action::Down => {
            if a.screen == Screen::Ideas {
                a.idea_selected = (a.idea_selected + 1).min(a.ideas.len().saturating_sub(1));
            } else if a.screen == Screen::TrackerFiles {
                a.song_selected = (a.song_selected + 1).min(a.song_list.len().saturating_sub(1));
                a.confirm_song_delete = None;
            } else if a.screen == Screen::Tracker {
                a.cancel_tracker_gesture();
                a.tracker_row = (a.tracker_row + 1).min(a.tracker_rows().saturating_sub(1));
            } else if a.screen == Screen::TrackerPages {
                a.turn_page_manager(1);
            } else if a.screen == Screen::Presets {
                a.selected = (a.selected + 1).min(a.presets.len().saturating_sub(1));
            }
        }
        Action::PreviousEngine => {
            if a.screen == Screen::Presets {
                a.cycle_engine(-1);
            }
        }
        Action::NextEngine => {
            if a.screen == Screen::Presets {
                a.cycle_engine(1);
            }
        }
        Action::Activate => match a.screen {
            Screen::Presets => {
                if let Some(tx) = tx {
                    a.load(state, tx.clone())
                }
            }
            Screen::Playback => {
                a.reset_parameters();
            }
            Screen::Ideas => {
                if let Some(tx) = tx {
                    a.load_idea(state, tx.clone())
                }
            }
            Screen::Tracker => a.tracker_skip(),
            Screen::TrackerFiles => a.load_song(),
            Screen::TrackerPages => a.confirm_page_manager(),
            Screen::AudioRecorder => a.toggle_audio_recording(),
        },
        Action::Cancel => {
            if a.recorder.is_recording() {
                a.finish_and_save();
            } else if a.playback.is_some() {
                a.stop_playback();
            } else {
                perform(Action::Back, a, state, tx);
            }
        }
        Action::OpenIdeas => a.open_ideas(),
        Action::OpenTracker => {
            a.screen = Screen::Tracker;
            a.refresh_page_targets();
            a.sync_tracker_route();
            let page_online = a
                .song
                .pages
                .get(a.tracker_page)
                .is_some_and(|page| a.target_online(&page.target));
            a.status = if page_online {
                "tracker ready · EDIT toggles entry · encoder press skips".into()
            } else {
                "tracker page target offline · PAGES to change it".into()
            };
        }
        Action::OpenTrackerFiles => {
            if a.screen == Screen::TrackerPages {
                a.confirm_page_manager();
            }
            a.set_tracker_edit(false);
            a.song_list = sequencer::list(&sequencer::songs_dir());
            a.song_selected = a.song_selected.min(a.song_list.len().saturating_sub(1));
            a.confirm_song_delete = None;
            a.confirm_pattern_clear = false;
            a.song_previewing = false;
            a.screen = Screen::TrackerFiles;
            a.status = "song files · select an action".into();
        }
        Action::OpenTrackerPages => a.open_page_manager(),
        Action::Back => {
            if a.screen == Screen::TrackerPages {
                if a.page_manager_mode == PageManagerMode::Pages {
                    a.cancel_page_manager();
                } else {
                    a.cancel_page_field();
                }
                return false;
            }
            if a.screen == Screen::TrackerFiles && a.song_previewing {
                a.sequencer.stop();
                a.song_previewing = false;
            }
            a.confirm_delete = None;
            a.confirm_load = None;
            a.set_tracker_edit(false);
            a.screen = if a.screen == Screen::TrackerFiles {
                Screen::Tracker
            } else if matches!(
                a.screen,
                Screen::Playback | Screen::Tracker | Screen::AudioRecorder
            ) {
                Screen::Presets
            } else if a.playing.is_some() {
                Screen::Playback
            } else {
                Screen::Presets
            };
        }
        Action::TapTempo => {
            if let Some(b) = a.tap.tap(Instant::now()) {
                if a.screen == Screen::Tracker {
                    a.set_tracker_tempo(b.round().clamp(20.0, 300.0) as u16);
                } else {
                    a.status = format!("tap {b:.1} BPM · display only")
                }
            }
        }
        Action::SaveNew => a.save_new(),
        Action::SaveRecord => {
            if a.recorder.is_recording() {
                a.status = "Recording".into()
            } else {
                a.begin_record()
            }
        }
        Action::InspectIdea => a.inspect_idea(),
        Action::DeleteIdea => a.delete_idea(),
        Action::PlaybackRecording => a.toggle_playback(),
        Action::ToggleTracker => a.tracker_play(true),
        Action::TrackerStop => {
            if a.sequencer.status().playing {
                a.tracker_stop();
            } else {
                perform(Action::Back, a, state, tx);
            }
        }
        Action::TrackerMute => {
            let global_lane = a.tracker_page * LANES_PER_PAGE + a.tracker_track;
            if let Some(lane) = a
                .song
                .pages
                .get_mut(a.tracker_page)
                .and_then(|page| page.lanes.get_mut(a.tracker_track))
            {
                lane.enabled = !lane.enabled;
                let muted = !lane.enabled;
                a.sequencer.mute(global_lane, muted);
                a.status = format!("{} {}", lane.name, if muted { "muted" } else { "enabled" });
            }
        }
        Action::PreviewSong => a.preview_song(),
        Action::DeleteSong => a.delete_song(),
        Action::NewPattern => a.new_pattern(),
        Action::ClearPattern => a.choose_pattern_clear(),
        Action::TrackerEdit => {
            a.set_tracker_edit(!a.tracker_edit);
            a.status = format!("step edit {}", if a.tracker_edit { "on" } else { "off" });
        }
        Action::PreviousTrack => {
            if a.screen == Screen::TrackerPages {
                a.move_page_selection(-1);
            } else {
                a.move_tracker_lane(-1);
            }
        }
        Action::NextTrack => {
            if a.screen == Screen::TrackerPages {
                a.move_page_selection(1);
            } else {
                a.move_tracker_lane(1);
            }
        }
        Action::AddPage => a.add_tracker_page(),
        Action::EditPageTarget => a.edit_page_target(),
        Action::EditPageChannel => a.edit_page_channel(),
        Action::ConfirmPageManager => a.confirm_page_manager(),
        Action::SaveSong => a.save_song(),
        Action::AudioRecord => a.toggle_audio_recording(),
        Action::AudioStop => match a.audio_recorder.stop() {
            Ok(()) => a.status = "audio recording finalized".into(),
            Err(error) => a.status = format!("audio recorder: {error}"),
        },
    }
    false
}
fn key(code: KeyCode, a: &mut App, state: &Path, tx: &std::sync::mpsc::Sender<MidiEvent>) -> bool {
    if a.screen == Screen::Tracker {
        if a.tracker_edit {
            if let Some(semitone) = tracker_key_note(code) {
                a.tracker_single_note(a.tracker_keyboard_note(semitone), 96);
                return false;
            }
        }
        match code {
            KeyCode::Left => {
                a.move_tracker_lane(-1);
                return false;
            }
            KeyCode::Right => {
                a.move_tracker_lane(1);
                return false;
            }
            KeyCode::Tab => {
                a.switch_tracker_page();
                return false;
            }
            KeyCode::PageUp => {
                a.cancel_tracker_gesture();
                a.tracker_order = a.tracker_order.saturating_sub(1);
                a.tracker_row = 0;
                return false;
            }
            KeyCode::PageDown => {
                a.cancel_tracker_gesture();
                a.tracker_order = (a.tracker_order + 1).min(a.song.order.len().saturating_sub(1));
                a.tracker_row = 0;
                return false;
            }
            KeyCode::Char('-') => {
                if let Some(c) = a.tracker_cell_mut() {
                    c.note = Note::Off;
                }
                return false;
            }
            KeyCode::Delete => {
                a.tracker_erase();
                return false;
            }
            KeyCode::Char('.') | KeyCode::Insert => {
                a.tracker_skip();
                return false;
            }
            KeyCode::Char('m') => {
                perform(Action::TrackerMute, a, state, Some(tx));
                return false;
            }
            KeyCode::Char('M') => {
                a.toggle_tracker_page_mute();
                return false;
            }
            KeyCode::Char('p') => {
                a.tracker_play(false);
                return false;
            }
            KeyCode::Char('P') => {
                a.tracker_play(true);
                return false;
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if let Some(page) = a.song.pages.get_mut(a.tracker_page) {
                    page.program = (page.program + 1).min(127);
                }
                return false;
            }
            KeyCode::Char('_') => {
                if let Some(page) = a.song.pages.get_mut(a.tracker_page) {
                    page.program = page.program.saturating_sub(1);
                }
                return false;
            }
            KeyCode::Char('<') => {
                a.set_tracker_tempo(a.song.tempo.saturating_sub(1));
                return false;
            }
            KeyCode::Char('>') => {
                a.set_tracker_tempo(a.song.tempo.saturating_add(1));
                return false;
            }
            KeyCode::Char('v') => {
                a.save_song();
                return false;
            }
            KeyCode::Char('l') => {
                a.load_song();
                return false;
            }
            KeyCode::Char('N') => {
                a.new_pattern();
                return false;
            }
            KeyCode::Char('C') => {
                let old = a.tracker_pattern_number();
                let number = a.song.patterns.keys().next_back().copied().unwrap_or(0) + 1;
                if let Some(p) = a.song.patterns.get(&old).cloned() {
                    a.song.patterns.insert(number, p);
                    a.song.order.push(number);
                    a.tracker_order = a.song.order.len() - 1;
                }
                return false;
            }
            KeyCode::Char('X') => {
                let n = a.tracker_pattern_number();
                let lanes = a.song.total_lanes();
                if let Some(p) = a.song.patterns.get_mut(&n) {
                    *p = crate::sequencer::Pattern::empty(p.rows.len(), lanes);
                }
                return false;
            }
            KeyCode::Char('O') => {
                let n = a.tracker_pattern_number();
                a.song.order.insert(a.tracker_order + 1, n);
                a.tracker_order += 1;
                return false;
            }
            KeyCode::Backspace if a.song.order.len() > 1 => {
                a.song.order.remove(a.tracker_order);
                a.tracker_order = a.tracker_order.min(a.song.order.len() - 1);
                a.tracker_row = 0;
                return false;
            }
            _ => {}
        }
    }
    if a.screen == Screen::AudioRecorder && matches!(code, KeyCode::Char('r')) {
        a.toggle_audio_recording();
        return false;
    }
    match code {
        KeyCode::Char('q') => return true,
        KeyCode::Esc => {
            if a.screen != Screen::Presets {
                perform(Action::Back, a, state, Some(tx));
            } else {
                return true;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if a.screen == Screen::Ideas {
                a.idea_selected = a.idea_selected.saturating_sub(1)
            } else if a.screen == Screen::TrackerFiles {
                perform(Action::Up, a, state, Some(tx));
            } else {
                a.selected = a.selected.saturating_sub(1)
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if a.screen == Screen::Ideas {
                a.idea_selected = (a.idea_selected + 1).min(a.ideas.len().saturating_sub(1))
            } else if a.screen == Screen::TrackerFiles {
                perform(Action::Down, a, state, Some(tx));
            } else {
                a.selected = (a.selected + 1).min(a.presets.len().saturating_sub(1))
            }
        }
        KeyCode::PageUp => a.selected = a.selected.saturating_sub(10),
        KeyCode::PageDown => a.selected = (a.selected + 10).min(a.presets.len().saturating_sub(1)),
        KeyCode::Home => a.selected = 0,
        KeyCode::End => a.selected = a.presets.len().saturating_sub(1),
        KeyCode::Char('[') if a.screen == Screen::Presets => a.cycle_engine(-1),
        KeyCode::Char(']') if a.screen == Screen::Presets => a.cycle_engine(1),
        KeyCode::Enter => {
            if a.screen == Screen::Presets {
                a.load(state, tx.clone())
            } else if a.screen == Screen::TrackerFiles {
                perform(Action::Activate, a, state, Some(tx));
            } else {
                perform(
                    if a.screen == Screen::Ideas {
                        Action::InspectIdea
                    } else {
                        Action::Back
                    },
                    a,
                    state,
                    Some(tx),
                );
            }
        }
        KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Char(' ') => a.stop_all(state),
        KeyCode::Char('b') => {
            perform(Action::Back, a, state, Some(tx));
        }
        KeyCode::Char('r') => {
            if a.recorder.is_recording() {
                a.stop_recording()
            } else {
                a.begin_record()
            }
        }
        KeyCode::Char('p') => a.toggle_playback(),
        KeyCode::Char('w') => a.open_ideas(),
        KeyCode::Char('t') => {
            a.screen = Screen::Tracker;
            a.sync_tracker_route();
            a.status = a.sequencer.unavailable_label();
        }
        KeyCode::Char('a') => {
            if a.screen == Screen::Tracker {
                a.set_tracker_edit(false);
            }
            a.screen = Screen::AudioRecorder;
            a.status = "stereo audio recorder".into();
        }
        KeyCode::Char('d') if a.screen == Screen::Ideas => a.delete_idea(),
        KeyCode::Char('i') if a.screen == Screen::Ideas => a.inspect_idea(),
        _ => {}
    }
    false
}

fn tracker_key_note(code: KeyCode) -> Option<u8> {
    match code {
        KeyCode::Char('z') => Some(0),
        KeyCode::Char('s') => Some(1),
        KeyCode::Char('x') => Some(2),
        KeyCode::Char('d') => Some(3),
        KeyCode::Char('c') => Some(4),
        KeyCode::Char('v') => Some(5),
        KeyCode::Char('g') => Some(6),
        KeyCode::Char('b') => Some(7),
        KeyCode::Char('h') => Some(8),
        KeyCode::Char('n') => Some(9),
        KeyCode::Char('j') => Some(10),
        KeyCode::Char('m') => Some(11),
        _ => None,
    }
}
fn mouse(
    m: MouseEvent,
    a: &mut App,
    state: &Path,
    tx: &std::sync::mpsc::Sender<MidiEvent>,
) -> bool {
    if matches!(m.kind, MouseEventKind::Down(MouseButton::Right)) {
        if a.screen != Screen::Presets {
            perform(Action::Back, a, state, Some(tx));
        } else {
            return true;
        }
        return false;
    }
    match m.kind {
        MouseEventKind::ScrollUp => {
            if a.screen == Screen::Ideas {
                a.idea_selected = a.idea_selected.saturating_sub(1)
            } else {
                a.selected = a.selected.saturating_sub(3)
            }
        }
        MouseEventKind::ScrollDown => {
            if a.screen == Screen::Ideas {
                a.idea_selected = (a.idea_selected + 1).min(a.ideas.len().saturating_sub(1))
            } else {
                a.selected = (a.selected + 3).min(a.presets.len().saturating_sub(1))
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if a.screen == Screen::Presets && contains(a.hits.list, m.column, m.row) {
                let i = visible_index(a.hits.list, a.offset, m.column, m.row).unwrap();
                if i < a.presets.len() {
                    if i == a.selected {
                        a.load(state, tx.clone())
                    } else {
                        a.selected = i;
                    }
                }
            } else if a.screen == Screen::Ideas && contains(a.hits.list, m.column, m.row) {
                let i = visible_index(a.hits.list, a.idea_offset, m.column, m.row).unwrap();
                if i < a.ideas.len() {
                    if i == a.idea_selected {
                        a.inspect_idea()
                    } else {
                        a.idea_selected = i;
                        a.confirm_delete = None;
                    }
                }
            } else if a.screen == Screen::TrackerFiles && contains(a.hits.list, m.column, m.row) {
                let offset = a
                    .song_selected
                    .saturating_sub(a.hits.list.height.saturating_sub(1) as usize);
                let i = visible_index(a.hits.list, offset, m.column, m.row).unwrap();
                if i < a.song_list.len() {
                    a.song_selected = i;
                    a.confirm_song_delete = None;
                }
            } else if let Some(action) = a.hits.action(m.column, m.row) {
                if perform(action, a, state, Some(tx)) {
                    return true;
                }
            }
        }
        _ => {}
    }
    false
}

fn draw<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let area = f.size();
    a.hits = Hits::default();
    f.render_widget(Clear, area);
    if area.width < 38 || area.height < 10 {
        f.render_widget(
            Paragraph::new(
                "SHSYNTH\nterminal too small\nresize to at least 38×10\nRight-click/Esc: back/exit",
            )
            .style(Style::default().fg(Color::Green))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green)),
            ),
            area,
        );
        return;
    }
    match a.screen {
        Screen::Presets => draw_list(f, a),
        Screen::Playback => draw_playing(f, a),
        Screen::Ideas => draw_ideas(f, a),
        Screen::Tracker => draw_tracker(f, a),
        Screen::TrackerFiles => draw_tracker_files(f, a),
        Screen::TrackerPages => draw_tracker_pages(f, a),
        Screen::AudioRecorder => draw_audio_recorder(f, a),
    }
    draw_pad_lock(f, a);
    draw_pad_buttons(f, a);
    draw_status_bar(f, a);
}
fn draw_pad_lock<B: Backend>(f: &mut Frame<B>, a: &App) {
    if !a.pad_locked {
        return;
    }
    let z = f.size();
    f.render_widget(
        Paragraph::new("LCK").style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        rect(z.x + z.width.saturating_sub(3), z.y, z.width.min(3), 1),
    );
}
fn button<B: Backend>(f: &mut Frame<B>, r: Rect, label: &str) {
    let _ = (f, r, label);
}
fn pad_line(screen: Screen) -> String {
    let _ = screen;
    String::new()
}
fn draw_pad_buttons<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    if z.height < 4 {
        return;
    }
    a.hits.actions.clear();
    for (i, assignment) in navigation::assignments(a.screen).iter().enumerate() {
        let col = (i % 4) as u16;
        let row = (i / 4) as u16;
        let width = z.width / 4;
        let x0 = z.x + col * width;
        let r = rect(x0, z.y + z.height - 3 + row, width, 1);
        let label = if a.screen == Screen::TrackerFiles && a.confirm_pattern_clear {
            if assignment.pad == crate::pads::PadAction::Stop {
                "BACK"
            } else {
                ""
            }
        } else if a.screen == Screen::TrackerPages && a.page_manager_mode != PageManagerMode::Pages
        {
            if assignment.pad == crate::pads::PadAction::Stop {
                "CANCEL"
            } else if assignment.pad == crate::pads::PadAction::TapTempo {
                "CONFIRM"
            } else {
                ""
            }
        } else if a.screen == Screen::Tracker
            && a.tracker_edit
            && assignment.pad == crate::pads::PadAction::TapTempo
        {
            "ERASE"
        } else {
            assignment.label
        };
        f.render_widget(
            Paragraph::new(format!(
                "[{:^width$}]",
                label,
                width = usize::from(r.width.saturating_sub(2))
            ))
            .style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            r,
        );
        a.hits.actions.push((r, assignment.action));
    }
}
fn draw_status_bar<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    if z.height == 0 {
        return;
    }
    let bpm = if matches!(
        a.screen,
        Screen::Tracker | Screen::TrackerFiles | Screen::TrackerPages
    ) {
        format!("{} BPM", a.song.tempo)
    } else {
        a.tap
            .bpm()
            .map(|v| format!("{v:.1} BPM"))
            .unwrap_or_else(|| "--- BPM".into())
    };
    let engine = if a.engine.is_some() { "RUN" } else { "STOP" };
    let rec = if a.recorder.is_recording() {
        "REC"
    } else if a.playback.is_some() {
        "PLAY"
    } else {
        "IDLE"
    };
    let temperature = a.config.cpu_temperature_path.as_ref().map(|_| {
        a.cpu_temperature
            .map(|value| format!("CPU {value:.0}°C"))
            .unwrap_or_else(|| "CPU --°C".into())
    });
    let area = rect(z.x, z.y + z.height - 1, z.width, 1);
    let style = Style::default().fg(Color::Gray).bg(Color::Rgb(32, 32, 32));
    let left = format!(" {engine}  MIDI  {rec}");
    let right = temperature
        .map(|temperature| format!("{temperature}  {bpm}"))
        .unwrap_or(bpm);
    let gap = usize::from(z.width)
        .saturating_sub(left.chars().count() + right.chars().count())
        .max(1);
    f.render_widget(
        Paragraph::new(truncate(
            &format!("{left}{:gap$}{right}", ""),
            z.width as usize,
        ))
        .style(style),
        area,
    );
}
fn draw_list<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let head = rect(z.x, z.y, z.width, 2);
    let foot = rect(z.x, z.y + z.height - 3, z.width, 3);
    let list = rect(z.x, z.y + 2, z.width, z.height - 5);
    let rows = list.height.saturating_sub(2) as usize;
    a.ensure_visible(rows);
    let inner = rect(list.x + 1, list.y + 1, list.width - 2, list.height - 2);
    a.hits.list = inner;
    a.hits.primary = rect(z.x + 1, foot.y, 8, 1);
    a.hits.stop = rect(z.x + 10, foot.y, 8, 1);
    a.hits.exit = rect(z.x + 19, foot.y, 8, 1);
    let now = a
        .playing
        .as_ref()
        .map(|p| format!("{}: {}", p.backend.label(), p.name))
        .unwrap_or_else(|| "stopped".into());
    let selected_engine = a.selected_backend().label();
    f.render_widget(
        Paragraph::new(format!(
            " SHSYNTH//PRESETS  < {selected_engine} >\n running: {now}"
        ))
        .style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        head,
    );
    let lines = (a.offset..(a.offset + rows).min(a.presets.len()))
        .map(|i| {
            let mark = if i == a.selected { "▶" } else { " " };
            Spans::from(Span::styled(
                format!("{mark} {:02} {}", i + 1, a.presets[i].display_name()),
                if i == a.selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            ))
        })
        .collect::<Vec<_>>();
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(format!(
                    " sounds {}–{} / {} ",
                    if a.presets.is_empty() {
                        0
                    } else {
                        a.offset + 1
                    },
                    (a.offset + rows).min(a.presets.len()),
                    a.presets.len()
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        ),
        list,
    );
    button(f, a.hits.primary, "Play");
    button(f, a.hits.stop, "Stop");
    button(f, a.hits.exit, "Exit");
    f.render_widget(
        Paragraph::new(format!(
            "{}\n{}",
            truncate(&a.status, z.width as usize - 2),
            truncate(&pad_line(Screen::Presets), z.width as usize - 2)
        ))
        .style(Style::default().fg(Color::DarkGray)),
        rect(z.x + 1, foot.y + 1, z.width - 2, 2),
    );
}
fn draw_playing<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let header = rect(z.x, z.y, z.width, 1);
    let actions = rect(z.x, z.y + z.height - 3, z.width, 3);
    let params = rect(z.x, z.y + 1, z.width, z.height.saturating_sub(5));
    let name = a
        .playing
        .as_ref()
        .map(|p| format!("{} · {}", p.backend.label(), p.name))
        .unwrap_or_else(|| "none".into());
    f.render_widget(
        Paragraph::new(name).alignment(Alignment::Center).style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        header,
    );
    let inner = params;
    if a.playing
        .as_ref()
        .is_some_and(|preset| preset.backend == BackendKind::Synthv1)
    {
        for (i, c) in CONTROLS.iter().enumerate() {
            let col = (i % 4) as u16;
            let control_row = (i / 4) as u16;
            let label_y = inner.y + control_row * 2;
            if label_y >= inner.y + inner.height {
                break;
            }
            let x = inner.x + col * inner.width / 4;
            let next_x = inner.x + (col + 1) * inner.width / 4;
            let w = next_x - x;
            let v = a.values.get(&c.cc).copied().unwrap_or(c.min);
            let original = a.original_values.get(&c.cc).copied().unwrap_or(c.min);
            let label = truncate(c.name, w as usize);
            f.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::White)),
                rect(x, label_y, w, 1),
            );
            if label_y + 1 >= inner.y + inner.height {
                continue;
            }
            f.render_widget(
                Paragraph::new(Spans::from(vec![
                    Span::styled(format!("{v:>5.2}"), Style::default().fg(Color::Yellow)),
                    Span::raw(" "),
                    Span::styled("●", Style::default().fg(parameter_color(v, original))),
                ]))
                .alignment(Alignment::Center),
                rect(x, label_y + 1, w, 1),
            );
        }
    } else {
        f.render_widget(
            Paragraph::new("SoundFont/instrument engine\n\nMusical MIDI is routed normally.\nNo synthv1 parameter mapping is imposed.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray)),
            inner,
        );
    }
    let chord_area = rect(
        z.x,
        params.y + 6,
        z.width,
        actions.y.saturating_sub(params.y + 6),
    );
    if let Some((chord, notes)) = a.held_notes.description() {
        let top = chord_area.y + chord_area.height.saturating_sub(2) / 2;
        f.render_widget(
            Paragraph::new(chord)
                .style(
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
                .alignment(Alignment::Center),
            rect(chord_area.x, top, chord_area.width, 1),
        );
        if chord_area.height >= 2 {
            f.render_widget(
                Paragraph::new(truncate(&notes, chord_area.width as usize))
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::DarkGray)),
                rect(chord_area.x, top + 1, chord_area.width, 1),
            );
        }
    }
    a.hits.back = rect(z.x + 1, actions.y, 6, 1);
    a.hits.stop = rect(z.x + 7, actions.y, 6, 1);
    a.hits.record = rect(z.x + 13, actions.y, 7, 1);
    a.hits.stop_record = rect(z.x + 20, actions.y, 9, 1);
    a.hits.playback = rect(z.x + 29, actions.y, 10.min(z.width.saturating_sub(29)), 1);
    a.hits.save = rect(z.x + 1, actions.y + 1, 11, 1);
    button(f, a.hits.back, "Back");
    button(f, a.hits.stop, "Stop");
    button(f, a.hits.record, "Record");
    button(f, a.hits.stop_record, "Stop Rec");
    button(
        f,
        a.hits.playback,
        if a.playback.is_some() {
            "Stop Play"
        } else {
            "Playback"
        },
    );
    button(f, a.hits.save, "Save Idea");
    f.render_widget(
        Paragraph::new(truncate(&a.status, z.width as usize - 2)).style(Style::default().fg(
            if a.recorder.is_recording() {
                Color::Red
            } else {
                Color::DarkGray
            },
        )),
        rect(z.x + 12, actions.y + 1, z.width.saturating_sub(13), 1),
    );
    f.render_widget(
        Paragraph::new(truncate(&pad_line(Screen::Playback), z.width as usize - 2))
            .style(Style::default().fg(Color::DarkGray)),
        rect(z.x + 1, actions.y + 2, z.width - 2, 1),
    );
}
fn draw_ideas<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let list = rect(z.x, z.y + 2, z.width, z.height.saturating_sub(6));
    let rows = list.height.saturating_sub(2) as usize;
    if a.idea_selected < a.idea_offset {
        a.idea_offset = a.idea_selected;
    }
    if rows > 0 && a.idea_selected >= a.idea_offset + rows {
        a.idea_offset = a.idea_selected + 1 - rows;
    }
    let inner = rect(
        list.x + 1,
        list.y + 1,
        list.width.saturating_sub(2),
        list.height.saturating_sub(2),
    );
    a.hits.list = inner;
    f.render_widget(
        Paragraph::new(" SHSYNTH//IDEAS").style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        rect(z.x, z.y, z.width, 2),
    );
    let lines = (a.idea_offset..(a.idea_offset + rows).min(a.ideas.len()))
        .map(|i| {
            let style = if i == a.idea_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Spans::from(Span::styled(
                format!(
                    "{} {}",
                    if i == a.idea_selected { "▶" } else { " " },
                    a.ideas[i]
                ),
                style,
            ))
        })
        .collect::<Vec<_>>();
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(format!(" saved ideas · {} ", a.ideas.len()))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        ),
        list,
    );
    let y = z.y + z.height - 4;
    a.hits.save = rect(z.x + 1, y, 10, 1);
    a.hits.load = rect(z.x + 11, y, 7, 1);
    a.hits.delete = rect(z.x + 18, y, 9, 1);
    a.hits.inspect = rect(z.x + 27, y, 10.min(z.width.saturating_sub(27)), 1);
    a.hits.back = rect(z.x + 1, y + 1, 7, 1);
    button(f, a.hits.save, "Save New");
    button(f, a.hits.load, "Load");
    button(f, a.hits.delete, "Delete");
    button(f, a.hits.inspect, "Inspect");
    button(f, a.hits.back, "Back");
    f.render_widget(
        Paragraph::new(truncate(&a.status, z.width as usize - 10)).style(Style::default().fg(
            if a.confirm_delete.is_some() || a.confirm_load.is_some() {
                Color::Yellow
            } else {
                Color::DarkGray
            },
        )),
        rect(z.x + 9, y + 1, z.width.saturating_sub(10), 1),
    );
    f.render_widget(
        Paragraph::new(truncate(&pad_line(Screen::Ideas), z.width as usize - 2))
            .style(Style::default().fg(Color::DarkGray)),
        rect(z.x + 1, y + 2, z.width - 2, 1),
    );
}
fn draw_tracker<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let transport = a.sequencer.status();
    let pattern_number = a.tracker_pattern_number();
    let state = if transport.playing {
        "PLAY"
    } else if a.tracker_edit {
        "EDIT"
    } else {
        "STOP"
    };
    f.render_widget(
        Paragraph::new(truncate(
            &format!(
                "{} · {} {state}",
                a.song.pages[a.tracker_page].name, a.song.name
            ),
            z.width as usize,
        ))
        .style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        rect(z.x, z.y, z.width, 1),
    );
    let page = &a.song.pages[a.tracker_page];
    let page_online = a.target_online(&page.target);
    let available = if page_online { "ONLINE" } else { "OFFLINE" };
    f.render_widget(
        Paragraph::new(truncate(
            &format!(
                "ord {:02}/{:02} pat {:02} · {available}",
                a.tracker_order + 1,
                a.song.order.len(),
                pattern_number
            ),
            z.width as usize,
        ))
        .style(Style::default().fg(if page_online {
            Color::DarkGray
        } else {
            Color::Yellow
        })),
        rect(z.x, z.y + 1, z.width, 1),
    );
    let grid = rect(z.x, z.y + 2, z.width, z.height.saturating_sub(6));
    let visible_tracks = LANES_PER_PAGE;
    let first_track = a.tracker_page * LANES_PER_PAGE;
    let row_width = 3u16;
    let column_width = grid.width.saturating_sub(row_width) / visible_tracks.max(1) as u16;
    let rows = grid.height.saturating_sub(1) as usize;
    let start = a.tracker_row.saturating_sub(rows / 2);
    let mut header = String::from("ROW");
    let page = &a.song.pages[a.tracker_page];
    for lane in &page.lanes {
        header.push_str(&format!(
            "{:^w$}",
            truncate(&lane.name, usize::from(column_width)),
            w = usize::from(column_width)
        ));
    }
    f.render_widget(
        Paragraph::new(header).style(Style::default().fg(Color::Yellow)),
        rect(grid.x, grid.y, grid.width, 1),
    );
    let Some(pattern) = a.song.patterns.get(&pattern_number) else {
        return;
    };
    for (screen_row, row_index) in (start..(start + rows).min(pattern.rows.len())).enumerate() {
        let y = grid.y + 1 + screen_row as u16;
        let selected = row_index == a.tracker_row;
        let beat_stride = if pattern.rows.len() == 24 { 6 } else { 8 };
        let beat_start = row_index % beat_stride == 0;
        let mut spans = vec![Span::styled(
            format!("{:02X} ", row_index),
            if selected {
                Style::default().fg(Color::Black).bg(Color::Green)
            } else if beat_start {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        )];
        for (track_index, cell) in pattern.rows[row_index]
            .iter()
            .enumerate()
            .skip(first_track)
            .take(visible_tracks)
        {
            let velocity = cell.velocity.map_or("..".into(), |v| format!("{:02X}", v));
            let text = format!("{} {velocity}", sequencer::note_name(cell.note));
            let cursor = selected && track_index == first_track + a.tracker_track;
            spans.push(Span::styled(
                format!("{text:<w$}", w = usize::from(column_width)),
                if cursor {
                    // Do not combine bold with black here. Some ANSI terminals
                    // render bold black as bright-black/gray, and incremental
                    // redraws can then leave a note with mixed foregrounds.
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else if selected {
                    Style::default().bg(Color::DarkGray)
                } else if beat_start {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                },
            ));
        }
        f.render_widget(
            Paragraph::new(Spans::from(spans)),
            rect(grid.x, y, grid.width, 1),
        );
    }
    let page = &a.song.pages[a.tracker_page];
    let lane = &page.lanes[a.tracker_track];
    f.render_widget(
        Paragraph::new(truncate(
            &format!(
                "P{}/{} {} L{} ch{} {} {}",
                a.tracker_page + 1,
                a.song.pages.len(),
                page.name,
                a.tracker_track + 1,
                page.channel + 1,
                truncate(page.target.label(), 10),
                if !page.enabled {
                    "PAGE MUTE"
                } else if !lane.enabled {
                    "MUTE"
                } else if page.percussion {
                    "DRUM"
                } else {
                    "ON"
                }
            ),
            z.width as usize,
        ))
        .style(Style::default().fg(Color::DarkGray)),
        rect(z.x, z.y + z.height.saturating_sub(4), z.width, 1),
    );
}

fn draw_tracker_pages<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    f.render_widget(
        Paragraph::new("TRACKER PAGES · 4 LANES EACH").style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        rect(z.x, z.y, z.width, 1),
    );
    let body_height = z.height.saturating_sub(5);
    match a.page_manager_mode {
        PageManagerMode::Pages => {
            let rows = usize::from(body_height);
            let start = a.tracker_page.saturating_sub(rows.saturating_sub(1));
            let lines = a
                .song
                .pages
                .iter()
                .enumerate()
                .skip(start)
                .take(rows)
                .map(|(index, page)| {
                    let online = a.target_online(&page.target);
                    let text = format!(
                        "{}{:02} {:<8} ch{:02} {} {}",
                        if index == a.tracker_page { "▶" } else { " " },
                        index + 1,
                        truncate(&page.name, 8),
                        page.channel + 1,
                        truncate(page.target.label(), 12),
                        if online { "" } else { "OFFLINE" }
                    );
                    Spans::from(Span::styled(
                        truncate(&text, usize::from(z.width)),
                        if index == a.tracker_page {
                            Style::default().fg(Color::Black).bg(Color::Yellow)
                        } else if online {
                            Style::default()
                        } else {
                            Style::default().fg(Color::Yellow)
                        },
                    ))
                })
                .collect::<Vec<_>>();
            f.render_widget(
                Paragraph::new(lines).block(
                    Block::default()
                        .title(format!(" {} pages ", a.song.pages.len()))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Green)),
                ),
                rect(z.x, z.y + 1, z.width, body_height),
            );
        }
        PageManagerMode::Target => {
            let target = a
                .page_target_candidates
                .get(a.page_target_selected)
                .map(PageTarget::label)
                .unwrap_or("no MIDI outputs");
            let online = a
                .page_target_candidates
                .get(a.page_target_selected)
                .is_some_and(|target| a.target_online(target));
            f.render_widget(
                Paragraph::new(vec![
                    Spans::from("TARGET DEVICE"),
                    Spans::from(""),
                    Spans::from(Span::styled(
                        format!(
                            "▶ {}",
                            truncate(target, usize::from(z.width.saturating_sub(6)))
                        ),
                        Style::default().fg(Color::Black).bg(Color::Yellow),
                    )),
                    Spans::from(if online {
                        "ONLINE"
                    } else {
                        "OFFLINE · data is kept"
                    }),
                    Spans::from("turn encoder · press to confirm"),
                ])
                .block(Block::default().borders(Borders::ALL)),
                rect(z.x, z.y + 1, z.width, body_height),
            );
        }
        PageManagerMode::Channel => {
            f.render_widget(
                Paragraph::new(vec![
                    Spans::from("MIDI CHANNEL"),
                    Spans::from(""),
                    Spans::from(Span::styled(
                        format!("▶ {:02}", a.page_channel_draft + 1),
                        Style::default().fg(Color::Black).bg(Color::Yellow),
                    )),
                    Spans::from("turn encoder 1–16"),
                    Spans::from("press to confirm"),
                ])
                .block(Block::default().borders(Borders::ALL)),
                rect(z.x, z.y + 1, z.width, body_height),
            );
        }
    }
}

fn draw_tracker_files<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    f.render_widget(
        Paragraph::new("TRACKER FILES").style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        rect(z.x, z.y, z.width, 1),
    );
    if a.confirm_pattern_clear {
        let choice = |beats| {
            let selected = a.pattern_clear_beats == beats;
            Spans::from(Span::styled(
                format!(
                    "{} {beats}/4 · {} ROWS",
                    if selected { "▶" } else { " " },
                    if beats == 3 { 24 } else { 32 }
                ),
                if selected {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else {
                    Style::default().fg(Color::Gray)
                },
            ))
        };
        let lines = vec![
            Spans::from("CLEAR CURRENT PATTERN"),
            Spans::from(""),
            choice(3),
            choice(4),
            Spans::from(""),
            Spans::from("Turn master rotary · press to confirm"),
            Spans::from("STOP/BACK cancels"),
        ];
        f.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL)),
            rect(z.x, z.y + 1, z.width, z.height.saturating_sub(5)),
        );
        return;
    }
    let list = rect(z.x, z.y + 1, z.width, z.height.saturating_sub(5));
    let inner = rect(
        list.x + 1,
        list.y + 1,
        list.width.saturating_sub(2),
        list.height.saturating_sub(2),
    );
    a.hits.list = inner;
    let rows = inner.height as usize;
    let offset = a.song_selected.saturating_sub(rows.saturating_sub(1));
    let lines = a
        .song_list
        .iter()
        .enumerate()
        .skip(offset)
        .take(rows)
        .map(|(index, name)| {
            let selected = index == a.song_selected;
            Spans::from(Span::styled(
                format!("{} {name}", if selected { "▶" } else { " " }),
                if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            ))
        })
        .collect::<Vec<_>>();
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(format!(" saved songs · {} ", a.song_list.len()))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        ),
        list,
    );
    f.render_widget(
        Paragraph::new(truncate(&a.status, z.width as usize)).style(Style::default().fg(
            if a.confirm_song_delete.is_some() || a.confirm_pattern_clear {
                Color::Yellow
            } else {
                Color::DarkGray
            },
        )),
        rect(z.x, z.y + z.height.saturating_sub(4), z.width, 1),
    );
}

fn draw_audio_recorder<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let s = a.audio_recorder.status();
    let state = if s.recording {
        "● RECORDING"
    } else {
        "STEREO RECORDER"
    };
    f.render_widget(
        Paragraph::new(state).alignment(Alignment::Center).style(
            Style::default()
                .fg(if s.recording {
                    Color::Red
                } else {
                    Color::Green
                })
                .add_modifier(Modifier::BOLD),
        ),
        rect(z.x, z.y, z.width, 1),
    );
    let input = a
        .config
        .capture
        .inputs
        .first()
        .map(|i| format!("{}\nL {}\nR {}", i.name, i.left_port, i.right_port))
        .unwrap_or_else(|| "No capture.input configured".into());
    let elapsed = format!(
        "{:02}:{:02}:{:02}",
        s.elapsed.as_secs() / 3600,
        (s.elapsed.as_secs() / 60) % 60,
        s.elapsed.as_secs() % 60
    );
    let path = s
        .path
        .as_deref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "no file".into());
    let details=format!("\n{input}\n\nTime {elapsed}\nRate {} Hz · 24-bit stereo\nSize {:.1} MiB\nDropped {}\n{}\n{}",s.sample_rate,s.bytes as f64/1_048_576.0,s.dropped_frames,truncate(&path,z.width.saturating_sub(2) as usize),s.error.as_deref().unwrap_or("R/REC start · STOP finalize"));
    f.render_widget(
        Paragraph::new(details)
            .alignment(Alignment::Center)
            .style(
                Style::default().fg(if s.error.is_some() || s.dropped_frames > 0 {
                    Color::Yellow
                } else {
                    Color::Gray
                }),
            ),
        rect(z.x, z.y + 1, z.width, z.height.saturating_sub(5)),
    );
}
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.into();
    }
    if n < 2 {
        return s.chars().take(n).collect();
    }
    format!("{}…", s.chars().take(n - 1).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    fn presets() -> Vec<Preset> {
        (0..39)
            .map(|i| Preset::synthv1(format!("Preset {i:02}"), format!("x{i}").into()))
            .collect()
    }
    fn app(presets: &[Preset]) -> App {
        let catalogs = [Catalog {
            backend: BackendKind::Synthv1,
            presets: presets.to_vec(),
            unavailable: None,
        }];
        App::new(
            &catalogs,
            Arc::new(std::sync::Mutex::new(None)),
            Arc::new(std::sync::Mutex::new(crate::midi::Pickup::default())),
            Arc::new(std::sync::Mutex::new(BackendKind::Synthv1)),
            Arc::new(std::sync::Mutex::new(engine::TrackerRoute::default())),
            Arc::new(std::sync::Mutex::new(None)),
            RuntimeConfig::default(),
        )
    }
    fn render(w: u16, h: u16, screen: Screen) {
        let p = presets();
        let mut a = app(&p);
        a.screen = screen;
        a.playing = Some(p[0].clone());
        let b = TestBackend::new(w, h);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        assert_eq!(t.backend().buffer().area, Rect::new(0, 0, w, h));
    }
    #[test]
    fn renders_40x20_all_screens() {
        render(40, 20, Screen::Presets);
        render(40, 20, Screen::Playback);
        render(40, 20, Screen::Ideas);
        render(40, 20, Screen::Tracker);
        render(40, 20, Screen::TrackerFiles);
        render(40, 20, Screen::TrackerPages);
        render(40, 20, Screen::AudioRecorder);
    }
    #[test]
    fn renders_smaller_and_tiny_gracefully() {
        render(38, 14, Screen::Presets);
        render(38, 14, Screen::Playback);
        render(38, 14, Screen::Ideas);
        render(38, 14, Screen::Tracker);
        render(38, 14, Screen::TrackerFiles);
        render(38, 14, Screen::TrackerPages);
        render(38, 14, Screen::AudioRecorder);
        render(30, 8, Screen::Presets);
        render(30, 8, Screen::Tracker)
    }
    #[test]
    fn tracker_renders_both_page_names_and_compact_stop_label() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.song.tempo = 137;
        let now = Instant::now();
        a.tap.tap(now);
        a.tap.tap(now + Duration::from_millis(500));
        let b = TestBackend::new(40, 20);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let text = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol.as_str())
            .collect::<String>();
        assert!(text.contains("MELODY"));
        assert!(text.contains("STOP"));
        assert!(!text.contains("STOP/BACK"));
        assert!(text.contains("137 BPM"));
        assert!(!text.contains("120.0 BPM"));
        a.switch_tracker_page();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let text = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol.as_str())
            .collect::<String>();
        assert!(text.contains("DRUMS"));
    }
    #[test]
    fn page_management_is_fully_reachable_with_pads_and_encoder_actions() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        perform(Action::OpenTrackerPages, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::TrackerPages);
        perform(Action::AddPage, &mut a, Path::new("/none"), None);
        assert_eq!(a.song.pages.len(), 3);
        assert_eq!(a.song.pages[2].lanes.len(), 4);

        perform(Action::EditPageTarget, &mut a, Path::new("/none"), None);
        assert_eq!(a.page_manager_mode, PageManagerMode::Target);
        perform(Action::Down, &mut a, Path::new("/none"), None);
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert_eq!(a.page_manager_mode, PageManagerMode::Pages);

        perform(Action::EditPageChannel, &mut a, Path::new("/none"), None);
        perform(Action::Down, &mut a, Path::new("/none"), None);
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert_eq!(a.song.pages[2].channel, 1);

        perform(Action::PreviousTrack, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_page, 1);
        perform(Action::NextTrack, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_page, 2);
        perform(Action::ConfirmPageManager, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Tracker);
        assert!(a.page_manager_original.is_none());
    }

    #[test]
    fn page_management_cancel_restores_song_and_offline_target_renders_at_40x20() {
        let p = presets();
        let mut a = app(&p);
        let original = a.song.clone();
        a.open_page_manager();
        a.add_tracker_page();
        a.song.pages[a.tracker_page].target = PageTarget::Midi("UNPLUGGED DEVICE".into());
        a.refresh_page_targets();
        let b = TestBackend::new(40, 20);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let text = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        assert!(text.contains("OFFLINE"));
        assert!(text.contains("TARGET"));
        a.cancel_page_manager();
        assert_eq!(a.song, original);
    }
    #[test]
    fn tracker_cursor_uses_readable_black_for_the_entire_note_cell() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_track = 1;
        a.song.patterns.get_mut(&0).unwrap().rows[0][1] = Cell {
            note: Note::On(60),
            velocity: Some(88),
            ..Cell::default()
        };
        let b = TestBackend::new(40, 20);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();

        // At 40 columns each lane is nine cells wide; lane two starts at x=12.
        // Check the complete visible value ("C-4 58"), including the octave.
        let b = t.backend().buffer();
        for x in 12..18 {
            let cell = b.get(x, 3);
            assert_eq!(cell.fg, Color::Black, "unexpected foreground at x={x}");
            assert_eq!(cell.bg, Color::Yellow, "unexpected background at x={x}");
            assert!(!cell.modifier.contains(Modifier::BOLD));
        }
    }
    #[test]
    fn scrolled_hit_test_maps_visible_row() {
        let p = presets();
        let mut a = app(&p);
        a.offset = 10;
        a.selected = 10;
        a.hits.list = Rect::new(1, 3, 78, 10);
        let (tx, _) = mpsc::channel();
        assert!(!mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 2,
                row: 7,
                modifiers: crossterm::event::KeyModifiers::NONE
            },
            &mut a,
            Path::new("/nonexistent"),
            &tx
        ));
        assert_eq!(a.selected, 14);
    }
    #[test]
    fn wheel_and_equal_pad_button_clicks_work_at_40x20() {
        let p = presets();
        let mut a = app(&p);
        a.selected = 5;
        let b = TestBackend::new(40, 20);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        assert_eq!(a.hits.actions.len(), 8);
        assert!(a.hits.actions.iter().all(|(r, _)| r.width == 10));
        let (tx, _) = mpsc::channel();
        mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 2,
                row: 2,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            &mut a,
            Path::new("/nonexistent"),
            &tx,
        );
        assert_eq!(a.selected, 8);
        mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 2,
                row: 17,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            &mut a,
            Path::new("/nonexistent"),
            &tx,
        );
        assert_eq!(a.status, "ARP · future");
    }
    #[test]
    fn playback_shows_all_parameters_centered_title_and_status_bar() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Playback;
        a.playing = Some(p[0].clone());
        a.cpu_temperature = Some(52.4);
        let b = TestBackend::new(40, 20);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let b = t.backend().buffer();
        let text = b
            .content
            .iter()
            .map(|c| c.symbol.as_str())
            .collect::<String>();
        assert!(text.contains("Sus"));
        assert!(text.contains("Rel"));
        assert!(text.contains("BPM"));
        assert!(text.contains("CPU 52°C"));
        let title = &text[..40];
        assert_eq!(title.trim(), "synthv1 · Preset 00");
        let left = title
            .chars()
            .position(|character| character == 's')
            .unwrap();
        let right = 40 - left - "synthv1 · Preset 00".chars().count();
        assert!(left.abs_diff(right) <= 1);
        assert_eq!(b.get(0, 19).bg, Color::Rgb(32, 32, 32));
    }

    #[test]
    fn preset_pad_engine_cycle_changes_catalog_not_sound_selection() {
        let p = presets();
        let catalogs = [
            Catalog {
                backend: BackendKind::Synthv1,
                presets: p.clone(),
                unavailable: None,
            },
            Catalog {
                backend: BackendKind::Yoshimi,
                presets: vec![],
                unavailable: Some("not installed".into()),
            },
            Catalog {
                backend: BackendKind::FluidSynth,
                presets: vec![],
                unavailable: Some("not installed".into()),
            },
        ];
        let mut a = App::new(
            &catalogs,
            Arc::new(std::sync::Mutex::new(None)),
            Arc::new(std::sync::Mutex::new(crate::midi::Pickup::default())),
            Arc::new(std::sync::Mutex::new(BackendKind::Synthv1)),
            Arc::new(std::sync::Mutex::new(engine::TrackerRoute::default())),
            Arc::new(std::sync::Mutex::new(None)),
            RuntimeConfig::default(),
        );
        a.selected = 7;
        perform(Action::NextEngine, &mut a, Path::new("/none"), None);
        assert_eq!(a.selected_backend(), BackendKind::Yoshimi);
        assert_eq!(a.selected, 0);
        assert!(a.status.contains("unavailable"));
    }

    #[test]
    fn cpu_temperature_read_is_cached_for_ten_seconds() {
        let p = presets();
        let mut a = app(&p);
        let path = std::env::temp_dir().join(format!("shsynth-temp-{}", std::process::id()));
        fs::write(&path, "42000\n").unwrap();
        a.config.cpu_temperature_path = Some(path.clone());
        let start = Instant::now();

        a.refresh_cpu_temperature(start);
        assert_eq!(a.cpu_temperature, Some(42.0));
        fs::write(&path, "43000\n").unwrap();
        a.refresh_cpu_temperature(start + Duration::from_secs(9));
        assert_eq!(a.cpu_temperature, Some(42.0));
        a.refresh_cpu_temperature(start + Duration::from_secs(10));
        assert_eq!(a.cpu_temperature, Some(43.0));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn playback_controls_match_three_physical_rows_of_four() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Playback;
        a.playing = Some(p[0].clone());
        a.original_values = HashMap::from([(74, 0.5), (71, 0.5), (76, 0.0), (77, 0.5)]);
        a.values = HashMap::from([(74, 0.4), (71, 0.5), (76, 0.1), (77, 0.5)]);
        let b = TestBackend::new(40, 20);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let b = t.backend().buffer();
        let row_text = |y| {
            (0..40)
                .map(|x| b.get(x, y).symbol.as_str())
                .collect::<String>()
        };

        assert_eq!(
            row_text(1).split_whitespace().collect::<Vec<_>>(),
            ["Flt", "cut", "Flt", "res", "Flt", "env", "LFO", "rate"]
        );
        assert_eq!(
            row_text(3).split_whitespace().collect::<Vec<_>>(),
            ["Volume", "Dly", "amt", "Dly", "time", "Dly", "fb"]
        );
        assert_eq!(
            row_text(5).split_whitespace().collect::<Vec<_>>(),
            ["Atk", "Dec", "Sus", "Rel"]
        );
        for y in [1, 3, 5] {
            assert!((0..40)
                .filter(|x| b.get(*x, y).symbol != " ")
                .all(|x| b.get(x, y).fg == Color::White));
        }

        let indicator_colors = (0..40)
            .filter_map(|x| {
                let cell = b.get(x, 2);
                (cell.symbol == "●").then_some(cell.fg)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            indicator_colors,
            [
                Color::Green,
                Color::LightYellow,
                Color::Red,
                Color::LightYellow
            ]
        );
    }

    #[test]
    fn playback_shows_held_chord_in_middle_area() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Playback;
        a.playing = Some(p[0].clone());
        for note in [61, 66, 68, 72] {
            a.held_notes.observe(&[0x90, note, 100]);
        }
        let b = TestBackend::new(40, 14);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let text = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        assert!(text.contains("C#maj7sus4"));
        assert!(!text.contains("Chord"));
        assert!(!text.contains("MIDI CC"));
    }

    #[test]
    fn tracker_play_refuses_an_empty_pattern_and_pad_actions_move_tracks() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_play(true);
        assert!(a.status.contains("no notes"));
        perform(Action::NextTrack, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_track, 1);
        perform(Action::PreviousTrack, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_track, 0);
    }

    #[test]
    fn tracker_keyboard_uses_drum_range_on_percussion_track() {
        let p = presets();
        let mut a = app(&p);
        assert_eq!(a.tracker_keyboard_note(0), 60);
        a.tracker_page = 1;
        assert!(a.song.pages[1].percussion);
        assert_eq!(a.tracker_keyboard_note(0), 36);
        assert_eq!(a.tracker_keyboard_note(11), 60);
        a.config.external_midi.percussion_notes = vec![36, 38, 40];
        assert_eq!(a.tracker_keyboard_note(1), 38);
    }

    #[test]
    fn tracker_step_entry_wraps_from_last_row_to_first() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        a.tracker_row = a.tracker_rows() - 1;
        a.tracker_single_note(60, 96);
        assert_eq!(a.tracker_row, 0);
        let pattern = a.song.patterns.get(&a.tracker_pattern_number()).unwrap();
        assert_eq!(pattern.rows.last().unwrap()[0].note, Note::On(60));
    }

    #[test]
    fn tracker_gesture_commits_once_sorted_across_four_lanes() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        let start = Instant::now();
        for (note, velocity) in [(67, 70), (60, 90), (64, 80)] {
            a.tracker_gesture.observe(start, &[0x90, note, velocity]);
        }
        for note in [67, 60, 64] {
            a.tracker_gesture
                .observe(start + Duration::from_millis(2), &[0x80, note, 0]);
        }
        a.commit_tracker_gesture(start + Duration::from_millis(60));
        assert_eq!(a.tracker_row, 1);
        let row = &a.song.patterns[&0].rows[0];
        assert_eq!(
            row[..4].iter().map(|cell| cell.note).collect::<Vec<_>>(),
            [Note::On(60), Note::On(64), Note::On(67), Note::Empty]
        );
        assert_eq!(
            row[..3]
                .iter()
                .map(|cell| cell.velocity)
                .collect::<Vec<_>>(),
            [Some(90), Some(80), Some(70)]
        );
        let row_before = a.tracker_row;
        for note in [60, 62, 64, 65, 67] {
            a.tracker_gesture.observe(start, &[0x90, note, 90]);
            a.tracker_gesture
                .observe(start + Duration::from_millis(2), &[0x80, note, 0]);
        }
        a.commit_tracker_gesture(start + Duration::from_millis(60));
        assert_eq!(a.tracker_row, row_before);
        assert!(a.status.contains("rejected"));
    }

    #[test]
    fn blank_skip_wraps_and_off_and_clear_remain_distinct() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        a.tracker_row = a.tracker_rows() - 1;
        a.tracker_cell_mut().unwrap().note = Note::Off;
        a.tracker_skip();
        assert_eq!(a.tracker_row, 0);
        assert_eq!(a.song.patterns[&0].rows.last().unwrap()[0].note, Note::Off);
        *a.tracker_cell_mut().unwrap() = Cell::default();
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::Empty);
    }

    #[test]
    fn tracker_erase_clears_the_cell_and_advances_one_row() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);

        a.tracker_erase();

        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::Empty);
        assert_eq!(a.tracker_row, 1);
        assert_eq!(a.status, "ERASE · cell cleared · row advanced");
    }
    #[test]
    fn tracker_file_screen_exposes_confirmed_pattern_clear() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        perform(Action::OpenTrackerFiles, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::TrackerFiles);
        assert_eq!(
            navigation::pad_action(a.screen, crate::pads::PadAction::Play),
            Some(Action::PreviewSong)
        );
        perform(Action::ClearPattern, &mut a, Path::new("/none"), None);
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::On(60));
        assert!(a.confirm_pattern_clear);
        perform(Action::Up, &mut a, Path::new("/none"), None);
        assert_eq!(a.pattern_clear_beats, 3);
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::Empty);
        assert_eq!(a.song.patterns[&0].rows.len(), 24);
        assert!(!a.confirm_pattern_clear);
        perform(Action::Back, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Tracker);
    }

    #[test]
    fn tracker_file_new_pattern_appends_and_selects_a_distinct_pattern() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::TrackerFiles;
        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);

        perform(Action::NewPattern, &mut a, Path::new("/none"), None);

        assert_eq!(a.song.order, vec![0, 1]);
        assert_eq!(a.tracker_order, 1);
        assert_eq!(a.tracker_pattern_number(), 1);
        assert_eq!(a.tracker_row, 0);
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::On(60));
        assert_eq!(a.song.patterns[&1].rows[0][0].note, Note::Empty);
        assert!(a.status.contains("SAVE stores all patterns"));
        let saved = sequencer::encode(&a.song).unwrap();
        assert!(saved.contains("order=0,1\n"));
        assert!(saved.contains("pattern=0|"));
        assert!(saved.contains("pattern=1|"));
    }

    #[test]
    fn gesture_entry_starts_at_selected_column() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        a.tracker_track = 1;
        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(48);
        a.song.patterns.get_mut(&0).unwrap().rows[0][2].note = Note::On(72);
        let start = Instant::now();
        a.tracker_gesture.observe(start, &[0x90, 65, 88]);
        a.tracker_gesture
            .observe(start + Duration::from_millis(2), &[0x80, 65, 0]);
        a.commit_tracker_gesture(start + Duration::from_millis(60));
        let row = &a.song.patterns[&0].rows[0];
        assert_eq!(row[0].note, Note::On(48));
        assert_eq!(row[1].note, Note::On(65));
        assert_eq!(row[1].velocity, Some(88));
        assert_eq!(row[2].note, Note::On(72));
    }

    #[test]
    fn main_press_skips_and_tap_erase_advances_before_tempo_change() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        let (tx, rx) = mpsc::channel();
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Select))
            .unwrap();
        drain(&rx, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.tracker_row, 1);
        let row = a.tracker_row;
        let tempo = a.song.tempo;
        a.tracker_cell_mut().unwrap().note = Note::On(70);
        tx.send(MidiEvent::Pad(crate::pads::PadAction::TapTempo, true))
            .unwrap();
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Down))
            .unwrap();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::TapTempo, false))
            .unwrap();
        drain(&rx, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.song.tempo, tempo + 1);
        assert_eq!(a.tracker_row, (row + 1) % a.tracker_rows());
        assert_eq!(a.song.patterns[&0].rows[row][0].note, Note::Empty);
        assert!(!a.tap_held);
        let b = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(b).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        assert!(text.contains("ERASE"));
    }

    #[test]
    fn edit_off_page_switch_stop_and_back_cancel_gestures() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        let now = Instant::now();
        a.tracker_gesture.observe(now, &[0x90, 60, 90]);
        a.set_tracker_edit(false);
        assert_eq!(
            a.tracker_gesture
                .finish(now + Duration::from_secs(1), Duration::ZERO),
            None
        );
        a.set_tracker_edit(true);
        a.tracker_gesture.observe(now, &[0x90, 61, 90]);
        a.switch_tracker_page();
        assert_eq!(
            a.tracker_gesture
                .finish(now + Duration::from_secs(1), Duration::ZERO),
            None
        );
        a.tracker_gesture.observe(now, &[0x90, 62, 90]);
        a.tracker_stop();
        assert_eq!(
            a.tracker_gesture
                .finish(now + Duration::from_secs(1), Duration::ZERO),
            None
        );
        a.set_tracker_edit(true);
        a.tracker_gesture.observe(now, &[0x90, 63, 90]);
        perform(Action::Back, &mut a, Path::new("/none"), None);
        assert_eq!(
            a.tracker_gesture
                .finish(now + Duration::from_secs(1), Duration::ZERO),
            None
        );
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        a.tracker_gesture.observe(now, &[0x90, 64, 90]);
        a.tick();
        assert_eq!(
            a.tracker_gesture
                .finish(now + Duration::from_secs(1), Duration::ZERO),
            None
        );
    }

    #[test]
    fn page_switch_preserves_transport_cursor_and_changes_only_visible_page() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_order = 0;
        a.tracker_row = 17;
        a.tracker_track = 2;
        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        a.sequencer.play(&a.song, 0, 0);
        let generation = a.sequencer.status().generation;
        a.switch_tracker_page();
        assert_eq!(a.tracker_page, 1);
        assert_eq!(a.tracker_order, 0);
        assert_eq!(a.tracker_row, 17);
        assert_eq!(a.tracker_track, 2);
        assert_eq!(a.sequencer.status().generation, generation);
    }

    #[test]
    fn tracker_stop_goes_back_when_transport_is_already_stopped() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        perform(Action::TrackerStop, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Presets);
        assert!(!a.tracker_edit);
    }

    #[test]
    fn tracker_marks_human_rows_one_and_nine_as_yellow_beat_starts() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_row = 4;
        let b = TestBackend::new(40, 20);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let b = t.backend().buffer();
        assert_eq!(b.get(0, 3).fg, Color::Yellow);
        assert_eq!(b.get(0, 11).fg, Color::Yellow);
        assert_eq!(b.get(0, 10).fg, Color::DarkGray);
    }

    #[test]
    fn three_four_pattern_has_24_rows_and_marks_one_seven_thirteen() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::TrackerFiles;
        a.choose_pattern_clear();
        a.pattern_clear_beats = 3;
        a.apply_pattern_clear();
        assert_eq!(a.song.patterns[&0].rows.len(), 24);

        a.screen = Screen::Tracker;
        a.tracker_row = 4;
        let b = TestBackend::new(40, 20);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let b = t.backend().buffer();
        assert_eq!(b.get(0, 3).fg, Color::Yellow);
        assert_eq!(b.get(0, 9).fg, Color::Yellow);
        assert_eq!(b.get(0, 15).fg, Color::Yellow);
        assert_eq!(b.get(0, 11).fg, Color::DarkGray);
    }

    #[test]
    fn four_four_pattern_clear_has_32_rows() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::TrackerFiles;
        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        a.choose_pattern_clear();
        assert_eq!(a.pattern_clear_beats, 4);
        a.apply_pattern_clear();
        assert_eq!(a.song.patterns[&0].rows.len(), 32);
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::Empty);
    }

    #[test]
    fn tracker_view_follows_playback_position() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.follow_tracker_transport(&sequencer::SequencerStatus {
            playing: true,
            order: 0,
            row: 24,
            ..sequencer::SequencerStatus::default()
        });
        assert_eq!(a.tracker_row, 24);
    }

    #[test]
    fn pad_lock_indicator_is_red_on_every_screen() {
        let p = presets();
        for screen in [
            Screen::Presets,
            Screen::Playback,
            Screen::Ideas,
            Screen::Tracker,
            Screen::TrackerFiles,
            Screen::TrackerPages,
            Screen::AudioRecorder,
        ] {
            let mut a = app(&p);
            a.screen = screen;
            a.pad_locked = true;
            let b = TestBackend::new(40, 20);
            let mut t = Terminal::new(b).unwrap();
            t.draw(|f| draw(f, &mut a)).unwrap();
            let b = t.backend().buffer();
            assert_eq!(
                (37..40)
                    .map(|x| b.get(x, 0).symbol.as_str())
                    .collect::<String>(),
                "LCK"
            );
            assert!((37..40).all(|x| b.get(x, 0).fg == Color::Red));
        }
    }
}
