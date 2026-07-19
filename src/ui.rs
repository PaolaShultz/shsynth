use crate::audio_graph::{
    EffectId, EffectKind, InsertRack, ProjectAuxRouting, SendPoint, MAX_AUX_BUSES,
};
use crate::audio_recorder::{AudioRecorder, RecorderStatus};
use crate::chord::HeldNotes;
use crate::config::{BankSelectMode, ExternalMidiConfig, RuntimeConfig};
use crate::control::{parameter_color, CONTROLS, VOLUME_CC};
use crate::device_profile::{DeviceProfile, Registry as DeviceProfiles};
use crate::drum_pattern::{self, DrumPattern};
use crate::engine::{self, Engine, MidiEvent};
use crate::geometry::{contains, rect, visible_index};
use crate::help::{self, HelpKind};
use crate::navigation::{self, Action, MenuContext, Screen, SlotState};
use crate::pads::{ControllerLayout, MenuInput, TapTempo};
use crate::performance_meter::{
    self, AudioAvailability, AudioLevel, BarCell, MeterColor, PerformanceMeter, VISIBLE_CPU_CORES,
};
use crate::preset::{BackendKind, Catalog, Preset};
use crate::recording::{self, Recorder, TimedEvent};
use crate::scale::{Scale, ScaleKind};
use crate::sequencer::{
    self, Cell, Command, GestureCapture, Note, PageTarget, Song, LANES_PER_PAGE,
};
use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEvent, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend, TestBackend},
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame, Terminal,
};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CPU_TEMPERATURE_REFRESH: Duration = Duration::from_secs(10);
const HELP_TEXT_WIDTH: usize = 38;
const INSERT_EFFECTS: [EffectKind; 13] = [
    EffectKind::Eq,
    EffectKind::Compressor,
    EffectKind::Distortion,
    EffectKind::Delay,
    EffectKind::Chorus,
    EffectKind::Flanger,
    EffectKind::Phaser,
    EffectKind::TremoloPan,
    EffectKind::Reverb,
    EffectKind::Gate,
    EffectKind::Filter,
    EffectKind::Crusher,
    EffectKind::Utility,
];
const FIRST_AUX_EFFECT_INDEX: usize = 3;

fn effect_kind_label(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Utility => "UTILITY",
        EffectKind::Eq => "EQ",
        EffectKind::Compressor => "COMPRESSOR",
        EffectKind::Distortion => "DISTORTION",
        EffectKind::Delay => "DELAY",
        EffectKind::Chorus => "CHORUS",
        EffectKind::Flanger => "FLANGER",
        EffectKind::Phaser => "PHASER",
        EffectKind::TremoloPan => "TREM/PAN",
        EffectKind::Reverb => "REVERB",
        EffectKind::Filter => "FILTER",
        EffectKind::Gate => "GATE",
        EffectKind::Crusher => "CRUSHER",
    }
}

fn fx_target_label(target: usize) -> &'static str {
    match target {
        0 => "SOURCE",
        1 => "AUX 1",
        2 => "AUX 2",
        _ => "MASTER",
    }
}

fn send_point_label(point: SendPoint) -> &'static str {
    match point {
        SendPoint::PreInsert => "PRE",
        SendPoint::PostInsert => "POST",
    }
}

fn project_fx_rack<'a>(
    source: &'a InsertRack,
    aux: &'a ProjectAuxRouting,
    target: usize,
) -> Option<&'a InsertRack> {
    if target == 0 {
        Some(source)
    } else if target <= MAX_AUX_BUSES {
        aux.buses
            .iter()
            .find(|bus| usize::from(bus.id) == target)
            .map(|bus| &bus.rack)
    } else {
        Some(&aux.master_rack)
    }
}

fn project_fx_rack_mut<'a>(
    source: &'a mut InsertRack,
    aux: &'a mut ProjectAuxRouting,
    target: usize,
) -> Option<&'a mut InsertRack> {
    if target == 0 {
        Some(source)
    } else if target <= MAX_AUX_BUSES {
        aux.buses
            .iter_mut()
            .find(|bus| usize::from(bus.id) == target)
            .map(|bus| &mut bus.rack)
    } else {
        Some(&mut aux.master_rack)
    }
}

fn is_aux_target(target: usize) -> bool {
    (1..=MAX_AUX_BUSES).contains(&target)
}

#[derive(Clone, Debug, Default)]
struct Hits {
    list: Rect,
    actions: Vec<(Rect, Action)>,
    menu_pages: Vec<(Rect, usize)>,
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
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PageManagerMode {
    #[default]
    Pages,
    Target,
    Channel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NoteEditorField {
    Note,
    Gate,
    Velocity,
    Program,
    Effect,
    EffectParameter,
}

impl NoteEditorField {
    const ALL: [Self; 6] = [
        Self::Note,
        Self::Gate,
        Self::Velocity,
        Self::Program,
        Self::Effect,
        Self::EffectParameter,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Note => "NOTE",
            Self::Gate => "GATE",
            Self::Velocity => "VELOCITY",
            Self::Program => "PROGRAM",
            Self::Effect => "EFFECT",
            Self::EffectParameter => "PARAM",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NoteEditor {
    pattern: u16,
    row: usize,
    lane: usize,
    original: Cell,
    draft: Cell,
    field: NoteEditorField,
}

#[derive(Debug)]
struct TrackerRecording {
    pattern: u16,
    order: usize,
    page: usize,
    last_row: usize,
    next_lane: usize,
    active_lanes: HashMap<(u8, u8), Vec<usize>>,
    notes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LaneClipboard {
    lane: sequencer::Lane,
    cells: Vec<Cell>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PageClipboard {
    page: sequencer::Page,
    rows: Vec<Vec<Cell>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum TrackerMode {
    #[default]
    Play,
    Rec,
    Edit,
    Noob,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum TrackerFilesMode {
    #[default]
    Projects,
    Patterns,
    Drums,
}

impl TrackerMode {
    const fn label(self) -> &'static str {
        match self {
            Self::Play => "PLAY",
            Self::Rec => "REC",
            Self::Edit => "EDIT",
            Self::Noob => "N00B",
        }
    }
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
    help_selected: usize,
    help_offset: usize,
    help_previous: Screen,
    web_help: Option<help::WebHelpServer>,
    web_help_status: String,
    web_help_enabled: bool,
    confirm_delete: Option<String>,
    confirm_load: Option<String>,
    midi_output: engine::SharedOutput,
    pickup: engine::SharedPickup,
    midi_backend: engine::SharedBackend,
    tracker_route: engine::SharedTrackerRoute,
    device_profiles: DeviceProfiles,
    config: RuntimeConfig,
    cpu_temperature: Option<f32>,
    cpu_temperature_read_at: Option<Instant>,
    pad_locked: bool,
    song: Song,
    song_file_stem: Option<String>,
    project_name_input: Option<String>,
    song_list: Vec<String>,
    song_selected: usize,
    tracker_order: usize,
    tracker_row: usize,
    tracker_page: usize,
    tracker_track: usize,
    tracker_advance: usize,
    tracker_mode: TrackerMode,
    tracker_recording: Option<TrackerRecording>,
    note_editor: Option<NoteEditor>,
    tracker_octave: u8,
    noob_scale: Scale,
    noob_draft: Scale,
    tracker_gesture: GestureCapture,
    tracker_gesture_anchor: Option<(usize, usize, usize, usize)>,
    confirm_song_save: Option<String>,
    confirm_new_project: bool,
    confirm_loop_remove: bool,
    confirm_song_delete: Option<String>,
    confirm_pattern_clear: bool,
    pattern_clear_beats: u8,
    pattern_setup_rows: usize,
    pattern_setup_new: bool,
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
    loop_player: crate::loop_player::LoopPlayer,
    loop_imports: Vec<PathBuf>,
    loop_selected: usize,
    loop_edit_bars: bool,
    pattern_clipboard: Option<sequencer::Pattern>,
    lane_clipboard: Option<LaneClipboard>,
    page_clipboard: Option<PageClipboard>,
    confirm_pattern_paste_over: Option<u16>,
    confirm_pattern_delete: Option<u16>,
    tracker_files_mode: TrackerFilesMode,
    drum_patterns: Vec<drum_pattern::Entry>,
    drum_pattern_selected: usize,
    drum_genre_selected: usize,
    drum_meter: u8,
    drum_target_rows: usize,
    confirm_drum_pattern_delete: Option<PathBuf>,
    loop_library_mode: bool,
    loop_library: Vec<crate::loop_player::LibraryEntry>,
    loop_library_selected: usize,
    confirm_loop_delete: Option<String>,
    arrange_selected: usize,
    menu_page_by_screen: [usize; Screen::COUNT],
    page_select_mode: bool,
    controller_layout: ControllerLayout,
    fx_selected: usize,
    fx_parameter: usize,
    fx_add_kind: usize,
    fx_target: usize,
    performance_meter: PerformanceMeter,
    last_mapped_volume: Option<f32>,
}
impl App {
    fn new(
        catalogs: &[Catalog],
        midi_output: engine::SharedOutput,
        pickup: engine::SharedPickup,
        midi_backend: engine::SharedBackend,
        tracker_route: engine::SharedTrackerRoute,
        tracker_input: engine::SharedTrackerInput,
        mut config: RuntimeConfig,
    ) -> Self {
        let backend_index = catalogs
            .iter()
            .position(|catalog| catalog.backend == BackendKind::Synthv1)
            .unwrap_or(0);
        let presets = catalogs
            .get(backend_index)
            .map(|catalog| catalog.presets.clone())
            .unwrap_or_default();
        let device_profiles = DeviceProfiles::discover();
        if let Some(profile) = device_profiles.by_id(&config.external_midi.profile) {
            profile.apply_midi_selection(&mut config.external_midi);
        }
        let song = Song::new(&config.external_midi);
        let transport_clock = Arc::new(crate::loop_player::TransportClock::default());
        let sequencer = sequencer::Sequencer::start_with_clock(
            &config.external_midi,
            Arc::clone(&midi_output),
            Arc::clone(&transport_clock),
        );
        let tracker_live_input = sequencer.live_input();
        if let Ok(mut input) = tracker_input.lock() {
            *input = Some(tracker_live_input.clone());
        }
        let audio_recorder = AudioRecorder::new(config.capture.clone());
        let loop_player = crate::loop_player::LoopPlayer::new(&config.loop_player, transport_clock);
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
            help_selected: 0,
            help_offset: 0,
            help_previous: Screen::Presets,
            web_help: None,
            web_help_status: String::new(),
            web_help_enabled: true,
            confirm_delete: None,
            confirm_load: None,
            midi_output,
            pickup,
            midi_backend,
            tracker_route,
            device_profiles,
            config,
            cpu_temperature: None,
            cpu_temperature_read_at: None,
            pad_locked: false,
            song,
            song_file_stem: None,
            project_name_input: None,
            song_list: sequencer::list(&sequencer::songs_dir()),
            song_selected: 0,
            tracker_order: 0,
            tracker_row: 0,
            tracker_page: 0,
            tracker_track: 0,
            tracker_advance: 1,
            tracker_mode: TrackerMode::Play,
            tracker_recording: None,
            note_editor: None,
            tracker_octave: 4,
            noob_scale: Scale::default(),
            noob_draft: Scale::default(),
            tracker_gesture: GestureCapture::default(),
            tracker_gesture_anchor: None,
            confirm_song_save: None,
            confirm_new_project: false,
            confirm_loop_remove: false,
            confirm_song_delete: None,
            confirm_pattern_clear: false,
            pattern_clear_beats: 4,
            pattern_setup_rows: 32,
            pattern_setup_new: false,
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
            loop_player,
            loop_imports: Vec::new(),
            loop_selected: 0,
            loop_edit_bars: false,
            pattern_clipboard: None,
            lane_clipboard: None,
            page_clipboard: None,
            confirm_pattern_paste_over: None,
            confirm_pattern_delete: None,
            tracker_files_mode: TrackerFilesMode::Projects,
            drum_patterns: drum_pattern::discover(),
            drum_pattern_selected: 0,
            drum_genre_selected: 0,
            drum_meter: 4,
            drum_target_rows: 32,
            confirm_drum_pattern_delete: None,
            loop_library_mode: false,
            loop_library: Vec::new(),
            loop_library_selected: 0,
            confirm_loop_delete: None,
            arrange_selected: 0,
            menu_page_by_screen: [0; Screen::COUNT],
            page_select_mode: false,
            controller_layout: ControllerLayout::Eight,
            fx_selected: 0,
            fx_parameter: 0,
            fx_add_kind: 0,
            fx_target: 0,
            performance_meter: PerformanceMeter::default(),
            last_mapped_volume: None,
        }
    }

    fn menu_context(&self) -> MenuContext {
        if self.screen == Screen::Tracker && self.note_editor.is_some() {
            MenuContext::TrackerNoteEdit
        } else if self.screen == Screen::Tracker && self.tracker_recording.is_some() {
            MenuContext::TrackerRecord
        } else if self.screen == Screen::Tracker && self.tracker_mode == TrackerMode::Edit {
            MenuContext::TrackerEdit
        } else if self.screen == Screen::TrackerFiles && self.confirm_pattern_clear {
            MenuContext::PatternClear
        } else if self.screen == Screen::TrackerFiles {
            match self.tracker_files_mode {
                TrackerFilesMode::Projects => MenuContext::Normal,
                TrackerFilesMode::Patterns => MenuContext::PatternTools,
                TrackerFilesMode::Drums => MenuContext::DrumPatterns,
            }
        } else if self.screen == Screen::TrackerLoop && self.loop_library_mode {
            MenuContext::LoopLibrary
        } else if self.screen == Screen::TrackerPages {
            match self.page_manager_mode {
                PageManagerMode::Target => MenuContext::PageTarget,
                PageManagerMode::Channel => MenuContext::PageChannel,
                PageManagerMode::Pages => MenuContext::Normal,
            }
        } else {
            MenuContext::Normal
        }
    }

    fn set_screen(&mut self, screen: Screen) {
        if self.screen != screen {
            if self.screen == Screen::TrackerFiles && self.song_previewing {
                self.stop_song_preview();
            }
            self.page_select_mode = false;
            self.prepare_confirmation_action(Action::Noop);
        }
        self.screen = screen;
    }

    fn prepare_confirmation_action(&mut self, action: Action) {
        let loading_selected_idea = action == Action::LoadIdea
            || (action == Action::Activate && self.screen == Screen::Ideas);
        if !loading_selected_idea {
            self.confirm_load = None;
        }
        if action != Action::DeleteIdea {
            self.confirm_delete = None;
        }
        if action != Action::SaveSong {
            self.confirm_song_save = None;
        }
        if action != Action::DeleteSong {
            self.confirm_song_delete = None;
        }
        if action != Action::PastePatternOver {
            self.confirm_pattern_paste_over = None;
        }
        if action != Action::DeleteUnusedPattern {
            self.confirm_pattern_delete = None;
        }
        if action != Action::DeleteDrumPattern {
            self.confirm_drum_pattern_delete = None;
        }
        if action != Action::NewProject {
            self.confirm_new_project = false;
        }
        if action != Action::LoopRemove {
            self.confirm_loop_remove = false;
        }
        if action != Action::DeleteLoopFile {
            self.confirm_loop_delete = None;
        }
    }

    fn menu_page(&self) -> usize {
        self.menu_page_by_screen[self.screen.index()].min(3)
    }

    fn select_menu_page(&mut self, page: usize) {
        self.prepare_confirmation_action(Action::Noop);
        let page = page.min(3);
        if navigation::pages(self.screen, self.menu_context())[page].available() {
            self.menu_page_by_screen[self.screen.index()] = page;
        }
        self.page_select_mode = false;
    }

    fn cycle_menu_page(&mut self, direction: i8) {
        self.prepare_confirmation_action(Action::Noop);
        let pages = navigation::pages(self.screen, self.menu_context());
        let mut next = self.menu_page();
        for _ in 0..4 {
            next = if direction < 0 {
                (next + 3) % 4
            } else {
                (next + 1) % 4
            };
            if pages[next].available() {
                self.menu_page_by_screen[self.screen.index()] = next;
                break;
            }
        }
    }

    fn reset_context_page(&mut self) {
        self.menu_page_by_screen[self.screen.index()] = 0;
        self.page_select_mode = false;
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
        self.playback.take();
        if let Some(e) = &self.engine {
            e.panic();
        }
        self.status = "recording playback stopped · all notes off".into();
    }
    fn stop_all(&mut self, state: &Path) {
        self.cancel_note_editor();
        self.cancel_tracker_gesture();
        self.stop_tracker_recording();
        if !self.stop_song_preview() {
            self.sequencer.stop();
        }
        self.loop_player.stop();
        let _ = self.audio_recorder.stop();
        self.stop_recording();
        self.stop_playback();
        self.engine.take();
        self.performance_meter
            .set_audio_unavailable(AudioAvailability::Stopped);
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
        self.current_pattern().map(|p| p.rows.len()).unwrap_or(0)
    }
    fn current_pattern(&self) -> Option<&sequencer::Pattern> {
        self.song.patterns.get(&self.tracker_pattern_number())
    }
    fn current_pattern_mut(&mut self) -> Option<&mut sequencer::Pattern> {
        let pattern = self.tracker_pattern_number();
        self.song.patterns.get_mut(&pattern)
    }
    fn current_pages(&self) -> &[sequencer::Page] {
        self.current_pattern()
            .map_or(&[], |pattern| pattern.pages.as_slice())
    }
    fn current_page(&self) -> Option<&sequencer::Page> {
        self.current_pages().get(self.tracker_page)
    }
    fn current_page_mut(&mut self) -> Option<&mut sequencer::Page> {
        let page = self.tracker_page;
        self.current_pattern_mut()?.pages.get_mut(page)
    }
    fn current_column(&self) -> Option<&sequencer::ColumnSetup> {
        self.current_page()
            .map(|page| page.column(self.tracker_track))
    }
    fn current_column_mut(&mut self) -> Option<&mut sequencer::ColumnSetup> {
        let track = self.tracker_track;
        self.current_page_mut().map(|page| page.column_mut(track))
    }
    fn current_tempo(&self) -> u16 {
        self.current_pattern()
            .map_or(self.config.external_midi.default_tempo, |pattern| {
                pattern.tempo
            })
    }
    fn current_meter(&self) -> u8 {
        self.current_pattern().map_or(4, |pattern| pattern.meter)
    }
    fn current_total_lanes(&self) -> usize {
        self.current_pattern()
            .map_or(0, sequencer::Pattern::total_lanes)
    }
    fn clamp_tracker_cursor(&mut self) {
        let pages = self.current_pages().len();
        self.tracker_page = self.tracker_page.min(pages.saturating_sub(1));
        self.tracker_track = self.tracker_track.min(LANES_PER_PAGE - 1);
        self.tracker_row = self.tracker_row.min(self.tracker_rows().saturating_sub(1));
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
    fn open_note_editor(&mut self) {
        self.cancel_tracker_gesture();
        let pattern = self.tracker_pattern_number();
        let row = self.tracker_row;
        let lane = self.tracker_page * LANES_PER_PAGE + self.tracker_track;
        let Some(original) = self
            .song
            .patterns
            .get(&pattern)
            .and_then(|pattern| pattern.rows.get(row))
            .and_then(|row| row.get(lane))
            .copied()
        else {
            self.status = "selected cell is unavailable".into();
            return;
        };
        self.note_editor = Some(NoteEditor {
            pattern,
            row,
            lane,
            original,
            draft: original,
            field: NoteEditorField::Note,
        });
        self.sync_tracker_route();
        self.reset_context_page();
        self.status = "CELL EDIT · NOTE selected · Confirm commits, Cancel restores".into();
    }
    fn select_note_editor_field(&mut self, field: NoteEditorField) {
        let program = if let Some(editor) = self.note_editor.as_mut() {
            editor.field = field;
            self.status = format!("CELL EDIT · {} selected", field.label());
            (field == NoteEditorField::Program).then_some(
                editor
                    .draft
                    .program
                    .unwrap_or_else(|| self.current_column().map_or(0, |column| column.program)),
            )
        } else {
            None
        };
        if let Some(program) = program {
            self.preview_tracker_program(program);
        }
    }
    fn move_note_editor_field(&mut self, direction: i8) {
        let Some(editor) = self.note_editor.as_mut() else {
            return;
        };
        let current = NoteEditorField::ALL
            .iter()
            .position(|field| *field == editor.field)
            .unwrap_or(0);
        let next = if direction < 0 {
            (current + NoteEditorField::ALL.len() - 1) % NoteEditorField::ALL.len()
        } else {
            (current + 1) % NoteEditorField::ALL.len()
        };
        editor.field = NoteEditorField::ALL[next];
        self.status = format!("CELL EDIT · {} selected", editor.field.label());
        if editor.field == NoteEditorField::Program {
            let program = editor
                .draft
                .program
                .unwrap_or_else(|| self.current_column().map_or(0, |column| column.program));
            self.preview_tracker_program(program);
        }
    }
    fn adjust_note_editor(&mut self, direction: i8) {
        let page_velocity = self.current_page().map_or(96, |page| page.velocity);
        let page_program = self.current_column().map_or(0, |column| column.program);
        let song_gate = self.song.gate_percent;
        let pattern_tempo = self.current_tempo();
        let changed_field = {
            let Some(editor) = self.note_editor.as_mut() else {
                return;
            };
            let increase = direction >= 0;
            match editor.field {
                NoteEditorField::Note => {
                    editor.draft.note = match (editor.draft.note, increase) {
                        (Note::Empty, true) => Note::On(60),
                        (Note::Empty, false) => Note::Off,
                        (Note::On(127), true) => Note::Off,
                        (Note::On(0), false) => Note::Empty,
                        (Note::On(note), true) => Note::On(note + 1),
                        (Note::On(note), false) => Note::On(note - 1),
                        (Note::Off, true) => Note::Empty,
                        (Note::Off, false) => Note::On(127),
                    };
                }
                NoteEditorField::Gate => {
                    let value = editor.draft.gate.unwrap_or(song_gate);
                    editor.draft.gate = Some(if increase {
                        value.saturating_add(1).min(100)
                    } else {
                        value.saturating_sub(1).max(1)
                    });
                }
                NoteEditorField::Velocity => {
                    let value = editor.draft.velocity.unwrap_or(page_velocity);
                    editor.draft.velocity = Some(if increase {
                        value.saturating_add(1).min(127)
                    } else {
                        value.saturating_sub(1)
                    });
                }
                NoteEditorField::Program => {
                    let value = editor.draft.program.unwrap_or(page_program);
                    editor.draft.program = Some(if increase {
                        value.saturating_add(1).min(127)
                    } else {
                        value.saturating_sub(1)
                    });
                }
                NoteEditorField::Effect => {
                    let effects = [
                        Command::None,
                        Command::Cut(0),
                        Command::Delay(0),
                        Command::Retrigger(2),
                        Command::Tempo(pattern_tempo),
                    ];
                    let current = match editor.draft.command {
                        Command::None => 0,
                        Command::Cut(_) => 1,
                        Command::Delay(_) => 2,
                        Command::Retrigger(_) => 3,
                        Command::Tempo(_) => 4,
                    };
                    let next = if increase {
                        (current + 1) % effects.len()
                    } else {
                        (current + effects.len() - 1) % effects.len()
                    };
                    editor.draft.command = effects[next];
                }
                NoteEditorField::EffectParameter => {
                    editor.draft.command = match editor.draft.command {
                        Command::None => {
                            self.status = "PARAM unavailable · select an effect first".into();
                            return;
                        }
                        Command::Cut(value) => Command::Cut(if increase {
                            value.saturating_add(1).min(15)
                        } else {
                            value.saturating_sub(1)
                        }),
                        Command::Delay(value) => Command::Delay(if increase {
                            value.saturating_add(1).min(15)
                        } else {
                            value.saturating_sub(1)
                        }),
                        Command::Retrigger(value) => Command::Retrigger(if increase {
                            value.saturating_add(1).min(8)
                        } else {
                            value.saturating_sub(1).max(1)
                        }),
                        Command::Tempo(value) => Command::Tempo(if increase {
                            value.saturating_add(1).min(300)
                        } else {
                            value.saturating_sub(1).max(20)
                        }),
                    };
                }
            }
            editor.field
        };
        if changed_field == NoteEditorField::Program {
            self.sync_tracker_route();
            if let Some(program) = self.note_editor.and_then(|editor| editor.draft.program) {
                self.preview_tracker_program(program);
            }
        }
        let detail = if changed_field == NoteEditorField::Program {
            self.note_editor
                .and_then(|editor| editor.draft.program)
                .map(|program| self.tracker_program_label(program))
        } else {
            None
        };
        self.status = detail.map_or_else(
            || format!("CELL EDIT · {} changed", changed_field.label()),
            |detail| format!("CELL EDIT · {detail} · play MIDI to audition"),
        );
    }
    fn clear_note_editor_field(&mut self) {
        let field = {
            let Some(editor) = self.note_editor.as_mut() else {
                return;
            };
            match editor.field {
                NoteEditorField::Note => editor.draft.note = Note::Empty,
                NoteEditorField::Gate => editor.draft.gate = None,
                NoteEditorField::Velocity => editor.draft.velocity = None,
                NoteEditorField::Program => editor.draft.program = None,
                NoteEditorField::Effect | NoteEditorField::EffectParameter => {
                    editor.draft.command = Command::None
                }
            }
            editor.field
        };
        if field == NoteEditorField::Program {
            self.sync_tracker_route();
            self.preview_tracker_program(self.current_column().map_or(0, |column| column.program));
        }
        self.status = format!("CELL EDIT · {} cleared only", field.label());
    }
    fn confirm_note_editor(&mut self) {
        let Some(editor) = self.note_editor else {
            return;
        };
        if let Err(error) = editor.draft.validate() {
            self.status = format!("CELL EDIT rejected · {error}");
            return;
        }
        if !matches!(editor.draft.note, Note::On(_))
            && (editor.draft.velocity.is_some()
                || editor.draft.program.is_some()
                || editor.draft.gate.is_some())
        {
            self.status =
                "CELL EDIT rejected · velocity, program, and gate require a note-on".into();
            return;
        }
        if matches!(editor.draft.command, Command::Retrigger(_))
            && !matches!(editor.draft.note, Note::On(_))
        {
            self.status = "CELL EDIT rejected · retrigger requires a note-on".into();
            return;
        }
        if let Some(cell) = self
            .song
            .patterns
            .get_mut(&editor.pattern)
            .and_then(|pattern| pattern.rows.get_mut(editor.row))
            .and_then(|row| row.get_mut(editor.lane))
        {
            *cell = editor.draft;
            self.note_editor = None;
            self.sync_tracker_route();
            self.reset_context_page();
            self.status = "CELL EDIT committed".into();
        } else {
            self.status = "CELL EDIT rejected · source cell no longer exists".into();
        }
    }
    fn cancel_note_editor(&mut self) {
        let Some(editor) = self.note_editor.take() else {
            return;
        };
        let restore_program = editor
            .original
            .program
            .unwrap_or_else(|| self.current_column().map_or(0, |column| column.program));
        if let Some(cell) = self
            .song
            .patterns
            .get_mut(&editor.pattern)
            .and_then(|pattern| pattern.rows.get_mut(editor.row))
            .and_then(|row| row.get_mut(editor.lane))
        {
            *cell = editor.original;
        }
        self.preview_tracker_program(restore_program);
        self.sync_tracker_route();
        self.reset_context_page();
        self.status = "CELL EDIT cancelled · original restored".into();
    }
    fn advance_tracker_row(&mut self) {
        let rows = self.tracker_rows();
        if rows > 0 {
            self.tracker_row = (self.tracker_row + self.tracker_advance) % rows;
        }
    }
    fn set_tracker_advance(&mut self, rows: usize) {
        self.tracker_advance = rows;
        self.status = format!("ADD {rows} · note entry and delete advance {rows} row(s)");
    }
    fn tracker_skip(&mut self) {
        if self.tracker_mode == TrackerMode::Edit {
            self.cancel_tracker_gesture();
            self.advance_tracker_row();
            self.status = format!("BLANK/SKIP · advanced {} row(s)", self.tracker_advance);
        }
    }
    fn tracker_erase(&mut self) {
        if self.tracker_mode != TrackerMode::Edit {
            return;
        }
        self.cancel_tracker_gesture();
        if let Some(cell) = self.tracker_cell_mut() {
            *cell = Cell::default();
            self.advance_tracker_row();
            self.status = format!(
                "ERASE · cell cleared · advanced {} row(s)",
                self.tracker_advance
            );
        }
    }
    fn cancel_tracker_gesture(&mut self) {
        self.tracker_gesture.cancel();
        self.tracker_gesture_anchor = None;
        if let Some(page) = self.current_page() {
            for channel in page
                .columns
                .iter()
                .map(|column| column.channel)
                .collect::<std::collections::BTreeSet<_>>()
            {
                self.tracker_live_input.cancel(&page.target, channel);
            }
        }
    }
    fn tracker_single_note(&mut self, note: u8, velocity: u8) {
        if self.tracker_mode != TrackerMode::Edit {
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
        self.status = format!("gesture entered · advanced {} row(s)", self.tracker_advance);
    }
    fn set_tracker_edit(&mut self, enabled: bool) {
        self.set_tracker_mode(if enabled {
            TrackerMode::Edit
        } else {
            TrackerMode::Play
        });
    }

    fn set_tracker_mode(&mut self, next: TrackerMode) {
        let changed = self.tracker_mode != next;
        if next != TrackerMode::Edit {
            self.cancel_tracker_gesture();
        }
        self.tracker_mode = next;
        if changed {
            self.reset_context_page();
        }
        self.sync_tracker_route();
    }

    fn open_noob_setup(&mut self) {
        self.tracker_stop();
        self.noob_draft = self.noob_scale;
        self.set_screen(Screen::TrackerNoob);
        self.reset_context_page();
        self.status = "choose root and MAJOR/MINOR · DONE enables N00B".into();
    }

    fn adjust_noob_root(&mut self, direction: i8) {
        self.noob_draft.root = if direction < 0 {
            (self.noob_draft.root + 11) % 12
        } else {
            (self.noob_draft.root + 1) % 12
        };
        self.status = format!(
            "N00B {} {}",
            crate::scale::note_name(self.noob_draft.root),
            self.noob_draft.kind.label()
        );
    }

    fn toggle_noob_scale(&mut self) {
        self.noob_draft.kind = match self.noob_draft.kind {
            ScaleKind::Major => ScaleKind::NaturalMinor,
            ScaleKind::NaturalMinor => ScaleKind::Major,
        };
        self.status = format!(
            "N00B {} {}",
            crate::scale::note_name(self.noob_draft.root),
            self.noob_draft.kind.label()
        );
    }

    fn confirm_noob(&mut self) {
        self.noob_scale = self.noob_draft;
        self.set_screen(Screen::Tracker);
        self.set_tracker_mode(TrackerMode::Noob);
        self.status = format!(
            "N00B {} {} · nearest note, ties down",
            crate::scale::note_name(self.noob_scale.root),
            self.noob_scale.kind.label()
        );
    }
    fn sync_tracker_route(&self) {
        let Some(page) = self.current_page() else {
            return;
        };
        let column = *page.column(self.tracker_track);
        let external = self.tracker_external_config();
        let program = self
            .note_editor
            .and_then(|editor| editor.draft.program)
            .unwrap_or(column.program);
        let mut columns = page.columns.map(|setup| {
            (
                setup.channel,
                (setup.program, setup.bank_msb, setup.bank_lsb),
            )
        });
        columns[self.tracker_track].1 .0 = program;
        if let Ok(mut route) = self.tracker_route.lock() {
            route.configure(crate::engine::TrackerRouteConfig {
                enabled: self.screen == Screen::Tracker
                    && (matches!(self.tracker_mode, TrackerMode::Edit | TrackerMode::Noob)
                        || self.note_editor.is_some()
                        || self.tracker_recording.is_some()),
                target: page.target.clone(),
                columns,
                start_column: self.tracker_track,
                percussion: page.percussion,
                scale: (self.tracker_mode == TrackerMode::Noob).then_some(self.noob_scale),
                external: &external,
            });
        }
    }

    fn tracker_external_config(&self) -> ExternalMidiConfig {
        let mut external = self.config.external_midi.clone();
        if let Some(profile) = self.tracker_device_profile() {
            profile.apply_midi_selection(&mut external);
        }
        external
    }

    fn tracker_device_profile(&self) -> Option<&DeviceProfile> {
        let page = self.current_page()?;
        match &page.target {
            PageTarget::ConfiguredExternal => self
                .device_profiles
                .by_id(&self.config.external_midi.profile),
            PageTarget::Midi(port) => self.device_profiles.matching_port(port),
            PageTarget::ActiveInstrument => None,
        }
    }

    fn tracker_program_label(&self, program: u8) -> String {
        let Some(page) = self.current_page() else {
            return format!("MIDI program {program}");
        };
        let column = page.column(self.tracker_track);
        self.tracker_device_profile()
            .and_then(|profile| profile.program_label(column.bank_msb, column.bank_lsb, program))
            .unwrap_or_else(|| format!("MIDI program {program}"))
    }

    fn preview_tracker_program(&self, program: u8) {
        let Some(page) = self.current_page() else {
            return;
        };
        for message in self.tracker_program_messages(program) {
            self.tracker_live_input.send(&page.target, &message);
        }
    }

    fn tracker_program_messages(&self, program: u8) -> Vec<Vec<u8>> {
        let external = self.tracker_external_config();
        if !external.program_changes {
            return Vec::new();
        }
        let Some(page) = self.current_page() else {
            return Vec::new();
        };
        let column = page.column(self.tracker_track);
        let mut messages = Vec::new();
        match external.bank_select {
            BankSelectMode::Off => {}
            BankSelectMode::Cc0 => {
                messages.push(vec![0xb0 | column.channel, 0, column.bank_msb]);
            }
            BankSelectMode::Cc0Cc32 => {
                messages.push(vec![0xb0 | column.channel, 0, column.bank_msb]);
                messages.push(vec![0xb0 | column.channel, 32, column.bank_lsb]);
            }
        }
        messages.push(vec![0xc0 | column.channel, program]);
        messages
    }
    fn move_tracker_lane(&mut self, direction: i8) {
        let current = self.tracker_page * LANES_PER_PAGE + self.tracker_track;
        let next = if direction < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(self.current_total_lanes().saturating_sub(1))
        };
        let page = next / LANES_PER_PAGE;
        self.cancel_tracker_gesture();
        self.tracker_page = page;
        self.tracker_track = next % LANES_PER_PAGE;
        self.sync_tracker_route();
    }
    fn switch_tracker_page(&mut self) {
        self.cancel_tracker_gesture();
        self.tracker_page = (self.tracker_page + 1) % self.current_pages().len().max(1);
        self.sync_tracker_route();
        self.status = self
            .current_page()
            .map_or_else(|| "no page".into(), |page| format!("{} page", page.name));
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
        if let Some(target) = self.current_page().map(|page| page.target.clone()) {
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
                    && sequencer::matching_output_index(
                        &self.available_page_outputs,
                        &self.config.external_midi.output_match,
                        true,
                    )
                    .is_ok()
            }
            PageTarget::Midi(name) => {
                sequencer::matching_output_index(&self.available_page_outputs, name, false).is_ok()
            }
        }
    }
    fn open_page_manager(&mut self) {
        self.tracker_stop();
        self.set_tracker_mode(TrackerMode::Play);
        self.page_manager_original = Some(self.song.clone());
        self.page_manager_mode = PageManagerMode::Pages;
        self.refresh_page_targets();
        self.set_screen(Screen::TrackerPages);
        self.reset_context_page();
        self.status = "select page/column · TARGET, CHANNEL, PROGRAM · DONE saves changes".into();
    }
    fn cancel_page_manager(&mut self) {
        if let Some(song) = self.page_manager_original.take() {
            self.song = song;
        }
        self.clamp_tracker_cursor();
        self.tracker_track = self.tracker_track.min(LANES_PER_PAGE - 1);
        self.page_manager_mode = PageManagerMode::Pages;
        self.set_screen(Screen::Tracker);
        self.sync_tracker_route();
        self.status = "page changes cancelled".into();
    }
    fn confirm_page_manager(&mut self) {
        if self.page_manager_mode != PageManagerMode::Pages {
            self.confirm_page_field();
            return;
        }
        if let Err(error) = self.song.validate() {
            self.status = format!("tracks conflict: {error}");
            return;
        }
        self.page_manager_original = None;
        self.set_screen(Screen::Tracker);
        self.sync_tracker_route();
        self.status = format!("{} pages ready", self.current_pages().len());
    }
    fn move_page_selection(&mut self, direction: i8) {
        if self.page_manager_mode != PageManagerMode::Pages {
            return;
        }
        self.tracker_page = if direction < 0 {
            self.tracker_page.saturating_sub(1)
        } else {
            (self.tracker_page + 1).min(self.current_pages().len().saturating_sub(1))
        };
        self.refresh_page_targets();
    }
    fn add_tracker_page(&mut self) {
        if self.page_manager_mode != PageManagerMode::Pages {
            return;
        }
        let target = self
            .current_page()
            .map(|page| page.target.clone())
            .unwrap_or(PageTarget::ConfiguredExternal);
        let channel = self.current_column().map_or(0, |column| column.channel);
        match self
            .song
            .add_page_to_pattern(self.tracker_pattern_number(), target, channel)
        {
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
        let Some(current) = self.current_page().map(|page| page.target.clone()) else {
            return;
        };
        self.page_target_selected = self
            .page_target_candidates
            .iter()
            .position(|target| target == &current)
            .unwrap_or(0);
        self.page_manager_mode = PageManagerMode::Target;
        self.reset_context_page();
        self.status = format!(
            "turn encoder for target · {} · EXIT cancels field",
            self.page_field_confirm_hint()
        );
    }
    fn edit_page_channel(&mut self) {
        if self.page_manager_mode != PageManagerMode::Pages {
            return;
        }
        self.page_channel_draft = self.current_column().map_or(0, |column| column.channel);
        self.page_manager_mode = PageManagerMode::Channel;
        self.reset_context_page();
        self.status = format!(
            "turn encoder for channel 1–16 · {}",
            self.page_field_confirm_hint()
        );
    }
    fn confirm_page_field(&mut self) {
        let mode = self.page_manager_mode;
        let selected_target = self
            .page_target_candidates
            .get(self.page_target_selected)
            .cloned();
        let channel = self.page_channel_draft;
        let track = self.tracker_track;
        if let Some(page) = self.current_page_mut() {
            match mode {
                PageManagerMode::Target => {
                    if let Some(target) = selected_target {
                        page.target = target;
                    }
                }
                PageManagerMode::Channel => page.column_mut(track).channel = channel,
                PageManagerMode::Pages => return,
            }
        }
        self.page_manager_mode = PageManagerMode::Pages;
        self.reset_context_page();
        self.status = "page route updated · DONE to keep or CANCEL to restore".into();
    }
    fn cancel_page_field(&mut self) {
        self.page_manager_mode = PageManagerMode::Pages;
        self.reset_context_page();
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
    fn page_field_confirm_hint(&self) -> &'static str {
        if self.controller_layout == ControllerLayout::Four {
            "use CONFIRM button"
        } else {
            "press encoder to confirm"
        }
    }
    fn toggle_tracker_page_mute(&mut self) {
        let page_index = self.tracker_page;
        if let Some(page) = self.current_page_mut() {
            page.enabled = !page.enabled;
            let muted = !page.enabled;
            let name = page.name.clone();
            self.sequencer.mute_page(page_index, muted);
            self.status = format!("{name} page {}", if muted { "muted" } else { "enabled" });
        }
    }
    fn set_tracker_tempo(&mut self, bpm: u16) {
        let tempo = self.apply_tracker_tempo(bpm);
        self.status = format!("pattern tempo {tempo} BPM");
    }
    fn apply_tracker_tempo(&mut self, bpm: u16) -> u16 {
        let tempo = bpm.clamp(20, 300);
        if let Some(pattern) = self.current_pattern_mut() {
            pattern.tempo = tempo;
        }
        self.sequencer.tempo(tempo);
        tempo
    }
    fn loop_project_tempo(settings: &sequencer::LoopSettings) -> u16 {
        settings.interpreted_bpm().round().clamp(20.0, 300.0) as u16
    }
    fn tracker_keyboard_note(&self, semitone: u8) -> u8 {
        let percussion = self.current_page().is_some_and(|page| page.percussion);
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
        if self.stop_tracker_recording() {
            return;
        }
        if !self.stop_song_preview() {
            self.sequencer.stop();
        }
        self.status = "tracker stopped".into();
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
        if notes == 0 && self.song.audio_loop.is_none() {
            self.status =
                "tracker has no notes from this position · enable EDIT and enter notes".into();
            return;
        }
        self.sequencer.play(&self.song, order, row);
        let offline = self
            .song
            .patterns
            .values()
            .flat_map(|pattern| pattern.pages.iter())
            .filter(|page| page.enabled && !self.target_online(&page.target))
            .count();
        self.status = if offline == 0 {
            format!(
                "tracker playing · {notes} MIDI · loop {}",
                if self.song.audio_loop.is_some() {
                    "on"
                } else {
                    "off"
                }
            )
        } else {
            format!("tracker playing · {notes} events · {offline} target(s) offline")
        };
    }

    fn tracker_record_song(&self, pattern: u16, page: usize) -> Song {
        let mut song = self.song.clone();
        song.order = vec![pattern];
        if let Some(pattern) = song.patterns.get_mut(&pattern) {
            for (index, candidate) in pattern.pages.iter_mut().enumerate() {
                candidate.enabled = index == page;
            }
        }
        song
    }

    fn begin_tracker_recording(&mut self) {
        if self.tracker_recording.is_some() {
            self.stop_tracker_recording();
            return;
        }
        let Some(page) = self.current_page() else {
            self.status = "REC unavailable · current page is missing".into();
            return;
        };
        if matches!(page.target, PageTarget::ActiveInstrument) {
            self.status = "REC needs a MIDI output page · loaded synth stays isolated".into();
            return;
        }
        self.cancel_note_editor();
        self.set_tracker_mode(TrackerMode::Play);
        self.sequencer.stop();
        let pattern = self.tracker_pattern_number();
        let order = self.tracker_order;
        let page_index = self.tracker_page;
        self.tracker_row = 0;
        self.tracker_recording = Some(TrackerRecording {
            pattern,
            order,
            page: page_index,
            last_row: 0,
            next_lane: self.tracker_track,
            active_lanes: HashMap::new(),
            notes: 0,
        });
        self.tracker_mode = TrackerMode::Rec;
        self.sync_tracker_route();
        self.reset_context_page();
        self.sequencer
            .play(&self.tracker_record_song(pattern, page_index), 0, 0);
        self.status = format!(
            "REC pattern {pattern} · {} only · MIDI output",
            self.current_pages()
                .get(page_index)
                .map_or("page", |page| page.name.as_str())
        );
    }

    fn stop_tracker_recording(&mut self) -> bool {
        let Some(recording) = self.tracker_recording.take() else {
            return false;
        };
        let pattern = recording.pattern;
        if let Some(page) = self
            .song
            .patterns
            .get(&pattern)
            .and_then(|pattern| pattern.pages.get(recording.page))
        {
            for channel in page
                .columns
                .iter()
                .map(|column| column.channel)
                .collect::<std::collections::BTreeSet<_>>()
            {
                self.tracker_live_input.cancel(&page.target, channel);
            }
        }
        self.sequencer.stop();
        self.tracker_mode = TrackerMode::Play;
        self.sync_tracker_route();
        self.reset_context_page();
        self.status = format!(
            "REC stopped · {} notes in pattern {} page {}",
            recording.notes,
            recording.pattern,
            recording.page + 1
        );
        true
    }

    fn record_tracker_midi(&mut self, bytes: &[u8]) {
        if bytes.len() < 3 || !matches!(bytes[0] & 0xf0, 0x80 | 0x90) {
            return;
        }
        let channel = bytes[0] & 0x0f;
        let note = bytes[1];
        let note_on = bytes[0] & 0xf0 == 0x90 && bytes[2] > 0;
        if !note_on {
            if let Some(recording) = self.tracker_recording.as_mut() {
                let key = (channel, note);
                let empty = recording.active_lanes.get_mut(&key).is_none_or(|lanes| {
                    lanes.pop();
                    lanes.is_empty()
                });
                if empty {
                    recording.active_lanes.remove(&key);
                }
            }
            return;
        }
        let row = self.sequencer.status().row;
        let Some(recording) = self.tracker_recording.as_mut() else {
            return;
        };
        let Some(pattern) = self.song.patterns.get_mut(&recording.pattern) else {
            return;
        };
        let row = row.min(pattern.rows.len().saturating_sub(1));
        let first_lane = recording.page * LANES_PER_PAGE;
        let lane = (0..LANES_PER_PAGE)
            .map(|offset| (recording.next_lane + offset) % LANES_PER_PAGE)
            .find(|lane| {
                !recording
                    .active_lanes
                    .values()
                    .flatten()
                    .any(|active| active == lane)
                    && matches!(pattern.rows[row][first_lane + lane].note, Note::Empty)
            })
            .or_else(|| {
                (0..LANES_PER_PAGE)
                    .map(|offset| (recording.next_lane + offset) % LANES_PER_PAGE)
                    .find(|lane| {
                        !recording
                            .active_lanes
                            .values()
                            .flatten()
                            .any(|active| active == lane)
                    })
            });
        let Some(lane) = lane else {
            self.status = format!("REC row {row:02X} full · note ignored");
            return;
        };
        pattern.rows[row][first_lane + lane] = Cell {
            note: Note::On(note),
            velocity: Some(bytes[2]),
            ..Cell::default()
        };
        recording
            .active_lanes
            .entry((channel, note))
            .or_default()
            .push(lane);
        recording.next_lane = (lane + 1) % LANES_PER_PAGE;
        recording.notes += 1;
        self.tracker_track = lane;
        self.tracker_row = row;
        self.status = format!(
            "REC pattern {} · row {row:02X} · lane {}",
            recording.pattern,
            lane + 1
        );
    }

    fn refresh_loop_imports(&mut self) {
        self.loop_imports =
            crate::loop_player::list_wavs(&self.config.loop_player.import_directory);
        self.loop_selected = self
            .loop_selected
            .min(self.loop_imports.len().saturating_sub(1));
    }

    fn load_current_loop(&mut self) -> bool {
        self.load_loop_settings(self.song.audio_loop.clone())
    }

    fn load_loop_settings(&mut self, settings: Option<sequencer::LoopSettings>) -> bool {
        self.unload_loop_player();
        let Some(settings) = settings else {
            return false;
        };
        let path = crate::loop_player::loops_dir().join(&settings.file);
        match crate::loop_player::DecodedLoop::open(&path)
            .and_then(|decoded| self.loop_player.load(decoded, &settings))
        {
            Ok(()) => {
                self.status = format!("loop ready · {}", settings.file);
                true
            }
            Err(error) => {
                self.status = format!("loop load: {error}");
                false
            }
        }
    }

    fn unload_loop_player(&mut self) {
        self.loop_player.unload();
    }

    fn remove_project_loop(&mut self) {
        if self.song.audio_loop.is_none() {
            self.confirm_loop_remove = false;
            self.status = "project has no loop".into();
            return;
        }
        if !self.confirm_loop_remove {
            self.confirm_loop_remove = true;
            self.status =
                "REMOVE LOOP detaches it from the Project · press again · WAV is kept".into();
            return;
        }
        self.tracker_stop();
        self.song.audio_loop = None;
        self.unload_loop_player();
        self.confirm_loop_remove = false;
        self.status = "loop removed from Project · private WAV kept".into();
    }

    fn open_loop_library(&mut self) {
        self.tracker_stop();
        self.loop_library_mode = true;
        self.refresh_loop_library();
        self.set_screen(Screen::TrackerLoop);
        self.reset_context_page();
        self.status = "private loop library · DELETE requires confirmation".into();
    }

    fn refresh_loop_library(&mut self) {
        match crate::loop_player::library_entries(
            &crate::loop_player::loops_dir(),
            self.song.audio_loop.as_ref(),
            &sequencer::songs_dir(),
        ) {
            Ok(entries) => {
                self.loop_library = entries;
                self.loop_library_selected = self
                    .loop_library_selected
                    .min(self.loop_library.len().saturating_sub(1));
            }
            Err(error) => {
                self.loop_library.clear();
                self.status = format!("loop library: {error}");
            }
        }
    }

    fn delete_selected_loop_file(&mut self) {
        let Some(entry) = self.loop_library.get(self.loop_library_selected).cloned() else {
            self.status = "private loop library is empty".into();
            return;
        };
        if self.confirm_loop_delete.as_deref() != Some(&entry.file) {
            self.confirm_loop_delete = Some(entry.file.clone());
            self.status = if entry.current || entry.saved_references != 0 {
                format!(
                    "refusing {} · current {} · {} saved reference(s)",
                    entry.file, entry.current, entry.saved_references
                )
            } else {
                format!("DELETE {} physically · press again to confirm", entry.file)
            };
            return;
        }
        match crate::loop_player::delete_library_file(
            &crate::loop_player::loops_dir(),
            &entry.file,
            self.song.audio_loop.as_ref(),
            &sequencer::songs_dir(),
        ) {
            Ok(()) => {
                self.confirm_loop_delete = None;
                self.refresh_loop_library();
                self.status = format!("deleted private WAV {}", entry.file);
            }
            Err(error) => {
                self.confirm_loop_delete = None;
                self.status = format!("loop delete: {error}");
            }
        }
    }

    fn stop_song_preview(&mut self) -> bool {
        if !self.song_previewing {
            return false;
        }
        self.sequencer.stop();
        self.song_previewing = false;
        self.load_current_loop();
        true
    }

    fn import_selected_loop(&mut self) {
        let Some(source) = self.loop_imports.get(self.loop_selected).cloned() else {
            self.status = format!(
                "no WAV in {}",
                self.config.loop_player.import_directory.display()
            );
            return;
        };
        match crate::loop_player::import(&source, &crate::loop_player::loops_dir()) {
            Ok((path, decoded)) => {
                let alignment = crate::loop_player::analyze_alignment(
                    &decoded,
                    self.current_tempo(),
                    self.current_meter(),
                );
                let candidates = crate::loop_player::bpm_candidates(alignment.source_bpm);
                let settings = sequencer::LoopSettings {
                    file: path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("loop.wav")
                        .into(),
                    source_bpm_x100: (alignment.source_bpm * 100.0).round() as u32,
                    interpretation: sequencer::BpmInterpretation::Normal,
                    start_beat: 0,
                    length_beats: alignment.length_beats,
                    offset_beats: 0,
                };
                self.song.audio_loop = Some(settings.clone());
                let tempo = self.apply_tracker_tempo(Self::loop_project_tempo(&settings));
                match self.loop_player.load(decoded, &settings) {
                    Ok(()) => {
                        self.status = format!(
                            "imported {} · {} bar(s) · project {} BPM · source {:.0}/{:.0}/{:.0}",
                            settings.file,
                            alignment.bars,
                            tempo,
                            candidates[0],
                            candidates[1],
                            candidates[2]
                        )
                    }
                    Err(error) => {
                        self.status = format!("imported privately · JACK loop offline: {error}")
                    }
                }
            }
            Err(error) => self.status = format!("loop import: {error}"),
        }
    }

    fn auto_align_loop(&mut self) {
        let Some(settings) = self.song.audio_loop.clone() else {
            self.status = "import a loop first".into();
            return;
        };
        let path = crate::loop_player::loops_dir().join(&settings.file);
        match crate::loop_player::DecodedLoop::open(&path) {
            Ok(decoded) => {
                let alignment = crate::loop_player::analyze_alignment(
                    &decoded,
                    self.current_tempo(),
                    self.current_meter(),
                );
                if let Some(settings) = self.song.audio_loop.as_mut() {
                    settings.source_bpm_x100 = (alignment.source_bpm * 100.0).round() as u32;
                    settings.interpretation = sequencer::BpmInterpretation::Normal;
                    settings.start_beat = 0;
                    settings.length_beats = alignment.length_beats;
                    settings.offset_beats = 0;
                }
                let tempo = self
                    .song
                    .audio_loop
                    .as_ref()
                    .map(Self::loop_project_tempo)
                    .map(|tempo| self.apply_tracker_tempo(tempo))
                    .unwrap_or_else(|| self.current_tempo());
                if self.load_current_loop() {
                    self.status = format!(
                        "auto aligned {} bar(s) · project {} BPM{}",
                        alignment.bars,
                        tempo,
                        if alignment.transient_detected {
                            ""
                        } else {
                            " (duration)"
                        }
                    );
                }
            }
            Err(error) => self.status = format!("auto align: {error}"),
        }
    }

    fn adjust_loop_offset_bars(&mut self, direction: i8) {
        let unit = i32::from(self.current_meter().clamp(1, 16));
        if let Some(settings) = self.song.audio_loop.as_mut() {
            let delta = if direction < 0 { -unit } else { unit };
            settings.offset_beats = (settings.offset_beats + delta).clamp(-16_384, 16_384);
            let bars = f64::from(settings.offset_beats) / f64::from(unit);
            self.load_current_loop();
            self.status = format!("loop offset {bars:+.0} bar(s)");
        } else {
            self.status = "import a loop first".into();
        }
    }

    fn adjust_loop_source_bpm(&mut self, direction: i8) {
        let tempo = if let Some(settings) = self.song.audio_loop.as_mut() {
            settings.source_bpm_x100 = if direction < 0 {
                settings.source_bpm_x100.saturating_sub(100).max(2_000)
            } else {
                settings.source_bpm_x100.saturating_add(100).min(30_000)
            };
            Some(Self::loop_project_tempo(settings))
        } else {
            None
        };
        let Some(tempo) = tempo else {
            self.status = "import a loop first".into();
            return;
        };
        let tempo = self.apply_tracker_tempo(tempo);
        if self.load_current_loop() {
            self.status = format!("loop source BPM · project {tempo} BPM");
        }
    }

    fn cycle_loop_bpm_mode(&mut self) {
        let tempo = if let Some(settings) = self.song.audio_loop.as_mut() {
            settings.interpretation = match settings.interpretation {
                sequencer::BpmInterpretation::Half => sequencer::BpmInterpretation::Normal,
                sequencer::BpmInterpretation::Normal => sequencer::BpmInterpretation::Double,
                sequencer::BpmInterpretation::Double => sequencer::BpmInterpretation::Half,
            };
            Some(Self::loop_project_tempo(settings))
        } else {
            None
        };
        let Some(tempo) = tempo else {
            self.status = "import a loop first".into();
            return;
        };
        let tempo = self.apply_tracker_tempo(tempo);
        if self.load_current_loop() {
            self.status = format!("loop BPM interpretation · project {tempo} BPM");
        }
    }

    fn adjust_loop_region(&mut self, start: bool, direction: i8) {
        let unit = if self.loop_edit_bars {
            crate::loop_player::bar_to_beat(1, self.current_meter())
        } else {
            1
        };
        if let Some(settings) = self.song.audio_loop.as_mut() {
            let value = if start {
                &mut settings.start_beat
            } else {
                &mut settings.length_beats
            };
            *value = if direction < 0 {
                value.saturating_sub(unit)
            } else {
                value.saturating_add(unit)
            };
            if !start {
                *value = (*value).max(1);
            }
            self.load_current_loop();
        }
    }
    fn save_song(&mut self) {
        let stem = sequencer::safe_name(&self.song.name);
        let confirmed = self.confirm_song_save.as_deref() == Some(&stem);
        match sequencer::save(&sequencer::songs_dir(), &self.song, confirmed) {
            Ok(path) => {
                self.status = format!("saved {}", path.display());
                self.song_file_stem = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned);
                self.confirm_song_save = None;
                self.song_list = sequencer::list(&sequencer::songs_dir());
                self.song_selected = self
                    .song_list
                    .iter()
                    .position(|name| name == &sequencer::safe_name(&self.song.name))
                    .unwrap_or(0);
            }
            Err(error) if !confirmed && error.to_string().contains("confirm") => {
                self.confirm_song_save = Some(stem);
                self.status = "song exists · SAVE again to overwrite".into();
            }
            Err(error) => {
                self.status = format!("song save: {error}");
                self.confirm_song_save = None;
            }
        }
    }
    fn new_project(&mut self) {
        if !self.confirm_new_project {
            self.confirm_new_project = true;
            self.status =
                "NEW PROJECT clears the current unsaved work · press again to confirm".into();
            return;
        }
        let Some(name) = next_numbered_song_name(&self.song_list, "project") else {
            self.confirm_new_project = false;
            self.status = "new project: project numbers exhausted".into();
            return;
        };
        self.cancel_note_editor();
        self.cancel_tracker_gesture();
        self.stop_tracker_recording();
        self.sequencer.stop();
        self.song_previewing = false;
        self.unload_loop_player();
        let mut song = Song::new(&self.config.external_midi);
        song.name = name.clone();
        if let Err(status) = self.publish_fx_routing_runtime(&song.insert_rack, &song.aux_routing) {
            self.confirm_new_project = false;
            self.status = status;
            return;
        }
        self.song = song;
        self.song_file_stem = None;
        self.tracker_order = 0;
        self.tracker_row = 0;
        self.tracker_page = 0;
        self.tracker_track = 0;
        self.tracker_mode = TrackerMode::Play;
        self.tracker_recording = None;
        self.page_manager_original = None;
        self.page_manager_mode = PageManagerMode::Pages;
        self.arrange_selected = 0;
        self.confirm_new_project = false;
        self.confirm_song_save = None;
        self.confirm_song_delete = None;
        self.confirm_pattern_clear = false;
        self.confirm_pattern_paste_over = None;
        self.set_screen(Screen::Tracker);
        self.refresh_page_targets();
        self.sync_tracker_route();
        self.project_name_input = Some(name.clone());
        self.status = format!("new project {name} · type a name or confirm the quick default");
    }

    fn begin_project_rename(&mut self) {
        self.project_name_input = Some(self.song.name.clone());
        self.status = "PROJECT NAME · type, Enter confirms, Esc cancels".into();
    }

    fn commit_project_rename(&mut self) {
        let Some(input) = self.project_name_input.clone() else {
            return;
        };
        let display = input.trim();
        if display.is_empty() {
            self.status = "project name cannot be empty".into();
            return;
        }
        if let Some(old_stem) = self.song_file_stem.clone() {
            self.tracker_stop();
            match sequencer::rename_project(&sequencer::songs_dir(), &old_stem, display) {
                Ok((song, path)) => {
                    self.song = song;
                    self.song_file_stem = path
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .map(str::to_owned);
                    self.song_list = sequencer::list(&sequencer::songs_dir());
                    self.song_selected = self
                        .song_file_stem
                        .as_ref()
                        .and_then(|stem| self.song_list.iter().position(|name| name == stem))
                        .unwrap_or(0);
                    self.project_name_input = None;
                    self.status = format!("Project renamed · {}", self.song.name);
                }
                Err(error) => self.status = format!("rename: {error}"),
            }
        } else {
            let mut candidate = self.song.clone();
            candidate.name = display.to_owned();
            match candidate.validate() {
                Ok(()) => {
                    self.song = candidate;
                    self.project_name_input = None;
                    self.status = format!("Project named {} · unsaved", self.song.name);
                }
                Err(error) => self.status = format!("name: {error}"),
            }
        }
    }

    fn delete_unused_pattern(&mut self) {
        let candidate = self
            .song
            .patterns
            .keys()
            .copied()
            .find(|number| self.song.pattern_reference_count(*number) == 0);
        let Some(number) = candidate else {
            self.confirm_pattern_delete = None;
            let current = self.tracker_pattern_number();
            let references = self.song.pattern_reference_count(current);
            self.status = format!(
                "no unused patterns · pattern {current} has {references} arrangement reference(s)"
            );
            return;
        };
        if self.confirm_pattern_delete != Some(number) {
            self.confirm_pattern_delete = Some(number);
            self.status = format!("DELETE unused pattern {number} · press again to confirm");
            return;
        }
        match self.song.delete_unused_pattern(number) {
            Ok(()) => {
                self.confirm_pattern_delete = None;
                self.clamp_tracker_cursor();
                self.status = format!("deleted unused pattern {number} · arrangement unchanged");
            }
            Err(error) => {
                self.confirm_pattern_delete = None;
                self.status = format!("pattern delete: {error}");
            }
        }
    }
    fn save_song_as(&mut self) {
        self.song_list = sequencer::list(&sequencer::songs_dir());
        let stem = sequencer::safe_name(&self.song.name);
        let prefix = format!("{}-copy", stem.chars().take(51).collect::<String>());
        let Some(name) = next_numbered_song_name(&self.song_list, &prefix) else {
            self.status = "save as: copy numbers exhausted".into();
            return;
        };
        let mut copy = self.song.clone();
        copy.name = name.clone();
        match sequencer::save(&sequencer::songs_dir(), &copy, false) {
            Ok(path) => {
                self.song = copy;
                self.song_file_stem = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned);
                self.confirm_song_save = None;
                self.song_list = sequencer::list(&sequencer::songs_dir());
                self.song_selected = self
                    .song_list
                    .iter()
                    .position(|candidate| candidate == &name)
                    .unwrap_or(0);
                self.status = format!("saved as {}", path.display());
            }
            Err(error) => self.status = format!("save as: {error}"),
        }
    }
    fn load_song(&mut self) {
        let Some(name) = self.song_list.get(self.song_selected).cloned() else {
            self.status = "no saved songs".into();
            return;
        };
        self.tracker_stop();
        match sequencer::load(&sequencer::songs_dir(), &name) {
            Ok(song) => {
                if let Err(status) =
                    self.publish_fx_routing_runtime(&song.insert_rack, &song.aux_routing)
                {
                    self.status = status;
                    return;
                }
                self.song = song;
                self.song_file_stem = Some(name.clone());
                self.tracker_order = 0;
                self.tracker_row = 0;
                self.tracker_page = 0;
                self.tracker_track = 0;
                self.set_screen(Screen::Tracker);
                self.refresh_page_targets();
                self.sync_tracker_route();
                if !self.load_current_loop() && self.song.audio_loop.is_none() {
                    self.status = format!("loaded {name}");
                }
            }
            Err(e) => self.status = format!("song load: {e}"),
        }
    }
    fn preview_song(&mut self) {
        if self.song_previewing {
            self.stop_song_preview();
            self.status = "song preview stopped".into();
            return;
        }
        let Some(name) = self.song_list.get(self.song_selected).cloned() else {
            self.status = "no saved song selected".into();
            return;
        };
        match sequencer::load(&sequencer::songs_dir(), &name) {
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
                    Ok(0) if song.audio_loop.is_none() => {
                        self.status = format!("{name} has no notes or loop to preview")
                    }
                    Ok(notes) => {
                        self.sequencer.stop();
                        if song.audio_loop.is_some()
                            && !self.load_loop_settings(song.audio_loop.clone())
                        {
                            let preview_error = self.status.clone();
                            if self.song.audio_loop.is_some() && !self.load_current_loop() {
                                let restore_error = self.status.clone();
                                self.status =
                                    format!("{preview_error}; current Project {restore_error}");
                            } else {
                                self.status = preview_error;
                            }
                            return;
                        }
                        if song.audio_loop.is_none() {
                            self.load_loop_settings(None);
                        }
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
            self.stop_song_preview();
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
        let rows = self.tracker_rows();
        self.pattern_clear_beats = if [6, 12, 24, 48, 96].contains(&rows) {
            3
        } else {
            4
        };
        self.pattern_setup_rows = rows;
        self.pattern_setup_new = false;
        self.confirm_pattern_clear = true;
        self.reset_context_page();
        self.pattern_setup_status();
    }

    fn choose_new_pattern(&mut self) {
        self.pattern_clear_beats = 4;
        self.pattern_setup_rows =
            nearest_pattern_rows(4, self.config.external_midi.default_pattern_rows);
        self.pattern_setup_new = true;
        self.confirm_pattern_clear = true;
        self.set_screen(Screen::TrackerFiles);
        self.reset_context_page();
        self.pattern_setup_status();
    }

    fn select_pattern_meter(&mut self, beats: u8) {
        let old = pattern_sizes(self.pattern_clear_beats);
        let tier = old
            .iter()
            .position(|rows| *rows == self.pattern_setup_rows)
            .unwrap_or(2);
        self.pattern_clear_beats = beats;
        self.pattern_setup_rows = pattern_sizes(beats)[tier];
        self.pattern_setup_status();
    }

    fn change_pattern_size(&mut self, direction: i8) {
        let sizes = pattern_sizes(self.pattern_clear_beats);
        let current = sizes
            .iter()
            .position(|rows| *rows == self.pattern_setup_rows)
            .unwrap_or(2);
        let next = if direction < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(sizes.len() - 1)
        };
        self.pattern_setup_rows = sizes[next];
        self.pattern_setup_status();
    }

    fn pattern_setup_status(&mut self) {
        self.status = format!(
            "{} pattern · {}/4 · {} rows · confirm",
            if self.pattern_setup_new {
                "new"
            } else {
                "clear"
            },
            self.pattern_clear_beats,
            self.pattern_setup_rows
        );
    }

    fn apply_pattern_clear(&mut self) {
        self.tracker_stop();
        if self.pattern_setup_new {
            self.create_pattern(self.pattern_setup_rows);
            return;
        }
        let number = self.tracker_pattern_number();
        let rows = self.pattern_setup_rows;
        if let Some(pattern) = self.song.patterns.get(&number) {
            let mut replacement = sequencer::Pattern::empty_like_setup(rows, pattern);
            replacement.meter = self.pattern_clear_beats;
            if let Err(error) = self.song.replace_pattern(number, replacement) {
                self.status = format!("clear pattern: {error}");
                return;
            }
        }
        self.tracker_row = 0;
        self.confirm_pattern_clear = false;
        self.status = format!(
            "cleared pattern {number} · {}/4 · {rows} rows",
            self.pattern_clear_beats
        );
    }
    fn new_pattern(&mut self) {
        self.choose_new_pattern();
    }

    fn create_pattern(&mut self, rows: usize) {
        self.tracker_stop();
        let pattern = sequencer::Pattern::from_config(
            &self.config.external_midi,
            rows,
            self.pattern_clear_beats,
        );
        let number = match self.song.append_pattern(pattern) {
            Ok(number) => number,
            Err(error) => {
                self.status = format!("new pattern: {error}");
                return;
            }
        };
        self.tracker_order = self.song.order.len() - 1;
        self.clamp_tracker_cursor();
        self.tracker_row = 0;
        self.confirm_pattern_clear = false;
        self.pattern_setup_new = false;
        self.set_screen(Screen::Tracker);
        self.status = format!(
            "new pattern {number} · {rows} rows · order {:02}/{:02}",
            self.tracker_order + 1,
            self.song.order.len()
        );
    }
    fn clone_pattern(&mut self) {
        let old = self.tracker_pattern_number();
        let Some(pattern) = self.song.patterns.get(&old).cloned() else {
            self.status = "no pattern to clone".into();
            return;
        };
        let number = match self.song.append_pattern(pattern) {
            Ok(number) => number,
            Err(error) => {
                self.status = format!("clone pattern: {error}");
                return;
            }
        };
        self.tracker_order = self.song.order.len() - 1;
        self.tracker_row = 0;
        self.status = format!("cloned pattern {old} as {number}");
    }
    fn copy_pattern(&mut self) {
        let number = self.tracker_pattern_number();
        let Some(pattern) = self.song.patterns.get(&number).cloned() else {
            self.status = "no pattern to copy".into();
            return;
        };
        self.pattern_clipboard = Some(pattern);
        self.confirm_pattern_paste_over = None;
        self.status = format!("copied pattern {number}");
    }
    fn open_pattern_tools(&mut self) {
        self.tracker_files_mode = TrackerFilesMode::Patterns;
        self.reset_context_page();
        self.status = format!("pattern {} tools", self.tracker_pattern_number());
    }
    fn open_drum_patterns(&mut self) {
        self.drum_patterns = drum_pattern::discover();
        self.drum_meter = self.current_meter();
        let sizes = drum_sizes(self.drum_meter);
        self.drum_target_rows = sizes
            .into_iter()
            .min_by_key(|rows| rows.abs_diff(self.tracker_rows()))
            .unwrap_or(sizes[0]);
        self.drum_genre_selected = 0;
        self.clamp_drum_selection();
        self.tracker_files_mode = TrackerFilesMode::Drums;
        self.reset_context_page();
        self.drum_filter_status();
    }
    fn drum_genres(&self) -> Vec<String> {
        let mut genres = self
            .drum_patterns
            .iter()
            .filter(|entry| entry.meter == self.drum_meter)
            .map(|entry| entry.genre.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        genres.insert(0, "ALL".into());
        genres
    }
    fn drum_genre(&self) -> String {
        self.drum_genres()
            .get(self.drum_genre_selected)
            .cloned()
            .unwrap_or_else(|| "ALL".into())
    }
    fn filtered_drum_indices(&self) -> Vec<usize> {
        let genre = self.drum_genre();
        self.drum_patterns
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.meter == self.drum_meter
                    && (genre == "ALL" || entry.genre == genre)
                    && entry.rows <= self.drum_target_rows
                    && self.drum_target_rows % entry.rows == 0
            })
            .map(|(index, _)| index)
            .collect()
    }
    fn clamp_drum_selection(&mut self) {
        let filtered = self.filtered_drum_indices();
        if !filtered.contains(&self.drum_pattern_selected) {
            self.drum_pattern_selected = filtered.first().copied().unwrap_or(0);
        }
    }
    fn move_drum_selection(&mut self, amount: isize) {
        let filtered = self.filtered_drum_indices();
        if filtered.is_empty() {
            return;
        }
        let current = filtered
            .iter()
            .position(|index| *index == self.drum_pattern_selected)
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(amount)
            .min(filtered.len().saturating_sub(1));
        self.drum_pattern_selected = filtered[next];
        self.confirm_drum_pattern_delete = None;
    }
    fn cycle_drum_genre(&mut self, direction: isize) {
        let len = self.drum_genres().len();
        self.drum_genre_selected = if direction < 0 {
            (self.drum_genre_selected + len - 1) % len
        } else {
            (self.drum_genre_selected + 1) % len
        };
        self.clamp_drum_selection();
        self.drum_filter_status();
    }
    fn toggle_drum_meter(&mut self) {
        let old_sizes = drum_sizes(self.drum_meter);
        let tier = old_sizes
            .iter()
            .position(|rows| *rows == self.drum_target_rows)
            .unwrap_or(0);
        self.drum_meter = if self.drum_meter == 4 { 3 } else { 4 };
        self.drum_target_rows = drum_sizes(self.drum_meter)[tier];
        self.drum_genre_selected = 0;
        self.clamp_drum_selection();
        self.drum_filter_status();
    }
    fn cycle_drum_size(&mut self) {
        let sizes = drum_sizes(self.drum_meter);
        let tier = sizes
            .iter()
            .position(|rows| *rows == self.drum_target_rows)
            .unwrap_or(0);
        self.drum_target_rows = sizes[(tier + 1) % sizes.len()];
        self.drum_filter_status();
    }
    fn drum_filter_status(&mut self) {
        self.status = format!(
            "{} · {}/4 · {} rows · {} groove(s)",
            self.drum_genre(),
            self.drum_meter,
            self.drum_target_rows,
            self.filtered_drum_indices().len()
        );
    }
    fn transpose_pattern(&mut self, semitones: i8) {
        self.tracker_stop();
        let number = self.tracker_pattern_number();
        let result = self
            .song
            .patterns
            .get_mut(&number)
            .context("current pattern missing")
            .and_then(|pattern| pattern.transpose_melodic(semitones));
        match result {
            Ok(0) => self.status = "transpose: no melodic notes in current pattern".into(),
            Ok(notes) => {
                self.status =
                    format!("transposed {notes} melodic note(s) {semitones:+} · drums unchanged")
            }
            Err(error) => self.status = format!("transpose: {error}"),
        }
    }
    fn percussion_page_index(&self) -> Option<usize> {
        self.current_pages().iter().position(|page| page.percussion)
    }
    fn load_drum_pattern(&mut self) {
        let Some(entry) = self.drum_patterns.get(self.drum_pattern_selected).cloned() else {
            self.status = "no drum pattern selected".into();
            return;
        };
        let pattern = match drum_pattern::load(&entry)
            .and_then(|pattern| drum_pattern::arrange(&pattern, self.drum_target_rows))
        {
            Ok(pattern) => pattern,
            Err(error) => {
                self.status = format!("drum load: {error}");
                return;
            }
        };
        let Some(page) = self.percussion_page_index() else {
            self.status = "drum load: current pattern has no percussion page".into();
            return;
        };
        let number = self.tracker_pattern_number();
        let target_rows = self.drum_target_rows;
        let shape_changes =
            self.current_meter() != pattern.meter || self.tracker_rows() != target_rows;
        if shape_changes && self.current_pattern_has_other_page_data(page) {
            self.status = format!(
                "drum load: {}/4 {target_rows} rows would resize existing page data",
                pattern.meter
            );
            return;
        }
        self.tracker_stop();
        let start = page * LANES_PER_PAGE;
        if let Some(target) = self.song.patterns.get_mut(&number) {
            if shape_changes {
                let lanes = target.total_lanes();
                target
                    .rows
                    .resize(target_rows, vec![Cell::default(); lanes]);
                target.meter = pattern.meter;
            }
            for (row, cells) in target.rows.iter_mut().enumerate() {
                cells[start..start + LANES_PER_PAGE].copy_from_slice(&pattern.rows[row]);
            }
        }
        self.tracker_page = page;
        self.tracker_row = 0;
        self.tracker_track = 0;
        self.status = format!(
            "loaded {} · {}/4 · {target_rows} rows · routing unchanged",
            entry.name, pattern.meter
        );
    }
    fn current_pattern_has_other_page_data(&self, drum_page: usize) -> bool {
        let Some(pattern) = self.current_pattern() else {
            return false;
        };
        pattern.pages.iter().enumerate().any(|(page, _)| {
            page != drum_page
                && pattern.rows.iter().any(|row| {
                    let start = page * LANES_PER_PAGE;
                    row[start..start + LANES_PER_PAGE]
                        .iter()
                        .any(|cell| *cell != Cell::default())
                })
        })
    }
    fn save_drum_pattern(&mut self) {
        let Some(page) = self.percussion_page_index() else {
            self.status = "drum save: current pattern has no percussion page".into();
            return;
        };
        let start = page * LANES_PER_PAGE;
        let Some(pattern) = self.current_pattern() else {
            self.status = "drum save: current pattern missing".into();
            return;
        };
        let existing = self
            .drum_patterns
            .iter()
            .filter(|entry| entry.user)
            .filter_map(|entry| entry.path.file_stem()?.to_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let prefix = format!("{}-drums", sequencer::safe_name(&self.song.name));
        let Some(stem) = next_numbered_song_name(&existing, &prefix) else {
            self.status = "drum save: pattern numbers exhausted".into();
            return;
        };
        let drum = DrumPattern {
            name: stem.replace('-', " "),
            genre: "User".into(),
            meter: pattern.meter,
            rows: pattern
                .rows
                .iter()
                .map(|row| {
                    let mut cells = [Cell::default(); LANES_PER_PAGE];
                    cells.copy_from_slice(&row[start..start + LANES_PER_PAGE]);
                    cells
                })
                .collect(),
        };
        match drum_pattern::save_user(&drum, &stem) {
            Ok(path) => {
                self.drum_patterns = drum_pattern::discover();
                self.drum_pattern_selected = self
                    .drum_patterns
                    .iter()
                    .position(|entry| entry.path == path)
                    .unwrap_or(0);
                if let Some(index) = self.drum_genres().iter().position(|genre| genre == "User") {
                    self.drum_genre_selected = index;
                }
                self.status = format!("saved drum pattern {}", path.display());
            }
            Err(error) => self.status = format!("drum save: {error}"),
        }
    }
    fn delete_drum_pattern(&mut self) {
        let Some(entry) = self.drum_patterns.get(self.drum_pattern_selected).cloned() else {
            self.status = "no drum pattern selected".into();
            return;
        };
        if !entry.user {
            self.status = "bundled drum patterns cannot be deleted".into();
            return;
        }
        if self.confirm_drum_pattern_delete.as_ref() != Some(&entry.path) {
            self.confirm_drum_pattern_delete = Some(entry.path);
            self.status = format!("DELETE {} · press again to confirm", entry.name);
            return;
        }
        match drum_pattern::delete_user(&entry) {
            Ok(()) => {
                self.drum_patterns = drum_pattern::discover();
                self.clamp_drum_selection();
                self.confirm_drum_pattern_delete = None;
                self.status = format!("deleted drum pattern {}", entry.name);
            }
            Err(error) => self.status = format!("drum delete: {error}"),
        }
    }
    fn paste_pattern_new(&mut self) {
        let Some(pattern) = self.pattern_clipboard.clone() else {
            self.status = "pattern clipboard empty".into();
            return;
        };
        self.tracker_stop();
        let number = match self.song.append_pattern(pattern) {
            Ok(number) => number,
            Err(error) => {
                self.status = format!("paste pattern: {error}");
                return;
            }
        };
        self.tracker_order = self.song.order.len() - 1;
        self.tracker_row = 0;
        self.clamp_tracker_cursor();
        self.status = format!("pasted clipboard as pattern {number}");
    }
    fn paste_pattern_over(&mut self) {
        let Some(pattern) = self.pattern_clipboard.clone() else {
            self.status = "pattern clipboard empty".into();
            return;
        };
        let number = self.tracker_pattern_number();
        if self.confirm_pattern_paste_over != Some(number) {
            self.confirm_pattern_paste_over = Some(number);
            self.status = format!("confirm paste over pattern {number}");
            return;
        }
        self.tracker_stop();
        if let Err(error) = self.song.replace_pattern(number, pattern) {
            self.confirm_pattern_paste_over = None;
            self.status = format!("paste pattern: {error}");
            return;
        }
        self.confirm_pattern_paste_over = None;
        self.clamp_tracker_cursor();
        self.status = format!("pasted over pattern {number}");
    }
    fn copy_lane(&mut self) {
        let global_lane = self.tracker_page * LANES_PER_PAGE + self.tracker_track;
        let Some(pattern) = self.current_pattern() else {
            self.status = "lane copy unavailable".into();
            return;
        };
        let Some(lane) = pattern
            .pages
            .get(self.tracker_page)
            .and_then(|page| page.lanes.get(self.tracker_track))
            .cloned()
        else {
            self.status = "lane copy unavailable".into();
            return;
        };
        self.lane_clipboard = Some(LaneClipboard {
            lane,
            cells: pattern
                .rows
                .iter()
                .filter_map(|row| row.get(global_lane).copied())
                .collect(),
        });
        self.status = format!("copied lane {}", self.tracker_track + 1);
    }
    fn paste_lane(&mut self) {
        let Some(clipboard) = self.lane_clipboard.clone() else {
            self.status = "lane clipboard empty".into();
            return;
        };
        let page_index = self.tracker_page;
        let track = self.tracker_track;
        let global_lane = page_index * LANES_PER_PAGE + track;
        let Some(pattern) = self.current_pattern_mut() else {
            self.status = "lane paste unavailable".into();
            return;
        };
        let Some(page) = pattern.pages.get_mut(page_index) else {
            self.status = "lane paste destination missing".into();
            return;
        };
        if let Some(lane) = page.lanes.get_mut(track) {
            *lane = clipboard.lane;
        }
        let rows = pattern.rows.len().min(clipboard.cells.len());
        for (row, cell) in pattern
            .rows
            .iter_mut()
            .take(rows)
            .zip(clipboard.cells.iter().copied())
        {
            if let Some(destination) = row.get_mut(global_lane) {
                *destination = cell;
            }
        }
        self.status = if rows < clipboard.cells.len() {
            format!("pasted lane · truncated to {rows} row(s)")
        } else {
            format!("pasted lane · {rows} row(s)")
        };
    }
    fn copy_page_block(&mut self) {
        let Some(pattern) = self.current_pattern() else {
            self.status = "page copy unavailable".into();
            return;
        };
        let Some(page) = pattern.pages.get(self.tracker_page).cloned() else {
            self.status = "page copy unavailable".into();
            return;
        };
        let start = self.tracker_page * LANES_PER_PAGE;
        self.page_clipboard = Some(PageClipboard {
            page,
            rows: pattern
                .rows
                .iter()
                .map(|row| {
                    row.iter()
                        .skip(start)
                        .take(LANES_PER_PAGE)
                        .copied()
                        .collect()
                })
                .collect(),
        });
        self.status = format!("copied page {}", self.tracker_page + 1);
    }
    fn paste_page_block(&mut self) {
        let Some(clipboard) = self.page_clipboard.clone() else {
            self.status = "page clipboard empty".into();
            return;
        };
        let page_index = self.tracker_page;
        let start = page_index * LANES_PER_PAGE;
        let Some(pattern) = self.current_pattern_mut() else {
            self.status = "page paste unavailable".into();
            return;
        };
        let Some(page) = pattern.pages.get_mut(page_index) else {
            self.status = "page paste destination missing".into();
            return;
        };
        *page = clipboard.page;
        let rows = pattern.rows.len().min(clipboard.rows.len());
        let mut lane_truncated = false;
        for (row, source) in pattern
            .rows
            .iter_mut()
            .take(rows)
            .zip(clipboard.rows.iter())
        {
            let lanes = LANES_PER_PAGE.min(source.len());
            lane_truncated |= lanes < source.len();
            for (offset, cell) in source.iter().copied().take(lanes).enumerate() {
                if let Some(destination) = row.get_mut(start + offset) {
                    *destination = cell;
                } else {
                    lane_truncated = true;
                }
            }
        }
        let row_truncated = rows < clipboard.rows.len();
        self.status = match (row_truncated, lane_truncated) {
            (true, true) => format!("pasted page · truncated rows/lanes to {rows} row(s)"),
            (true, false) => format!("pasted page · truncated to {rows} row(s)"),
            (false, true) => "pasted page · lane truncation".into(),
            (false, false) => format!("pasted page · {rows} row(s)"),
        };
    }
    fn clear_pattern_now(&mut self) {
        let number = self.tracker_pattern_number();
        let Some(pattern) = self.song.patterns.get(&number) else {
            return;
        };
        let replacement = sequencer::Pattern::empty_like_setup(pattern.rows.len(), pattern);
        match self.song.replace_pattern(number, replacement) {
            Ok(()) => {
                self.tracker_row = 0;
                self.status = format!("cleared pattern {number}");
            }
            Err(error) => self.status = format!("clear pattern: {error}"),
        }
    }
    fn move_order(&mut self, direction: i8) {
        self.cancel_tracker_gesture();
        self.tracker_order = if direction < 0 {
            self.tracker_order.saturating_sub(1)
        } else {
            (self.tracker_order + 1).min(self.song.order.len().saturating_sub(1))
        };
        self.clamp_tracker_cursor();
        self.tracker_row = 0;
        self.status = format!(
            "order {:02}/{:02}",
            self.tracker_order + 1,
            self.song.order.len()
        );
    }
    fn repeat_order(&mut self) {
        let number = self.tracker_pattern_number();
        let index = self.tracker_order + 1;
        if let Err(error) = self.song.insert_arrangement_step(index, number) {
            self.status = format!("repeat pattern: {error}");
            return;
        }
        self.tracker_order = index;
        self.clamp_tracker_cursor();
        self.tracker_row = 0;
        self.status = format!("repeated pattern {number} in order");
    }
    fn delete_order(&mut self) {
        if self.song.order.len() <= 1 {
            self.status = "cannot remove the only order entry".into();
            return;
        }
        self.song.order.remove(self.tracker_order);
        self.tracker_order = self.tracker_order.min(self.song.order.len() - 1);
        self.clamp_tracker_cursor();
        self.tracker_row = 0;
        self.status = "order entry removed".into();
    }
    fn open_arrange(&mut self) {
        self.set_tracker_edit(false);
        self.arrange_selected = self
            .tracker_order
            .min(self.song.order.len().saturating_sub(1));
        self.set_screen(Screen::TrackerArrange);
        self.reset_context_page();
        self.status = "FT2 arrangement · chain pattern steps".into();
    }
    fn select_arrangement_step(&mut self, direction: i8) {
        self.arrange_selected = if direction < 0 {
            self.arrange_selected.saturating_sub(1)
        } else {
            (self.arrange_selected + 1).min(self.song.order.len().saturating_sub(1))
        };
        self.status = format!(
            "arrangement step {:02}/{:02}",
            self.arrange_selected + 1,
            self.song.order.len()
        );
    }
    fn arrangement_append_current(&mut self) {
        let number = self.tracker_pattern_number();
        let index = self.song.order.len();
        match self.song.insert_arrangement_step(index, number) {
            Ok(index) => self.arrange_selected = index,
            Err(error) => {
                self.status = format!("append pattern: {error}");
                return;
            }
        }
        self.status = format!("appended pattern {number}");
    }
    fn arrangement_insert_current(&mut self) {
        let number = self.tracker_pattern_number();
        let index = self.arrange_selected.min(self.song.order.len());
        match self.song.insert_arrangement_step(index, number) {
            Ok(index) => self.arrange_selected = index,
            Err(error) => {
                self.status = format!("insert pattern: {error}");
                return;
            }
        }
        self.status = format!("inserted pattern {number}");
    }
    fn arrangement_duplicate_step(&mut self) {
        let Some(number) = self.song.order.get(self.arrange_selected).copied() else {
            return;
        };
        let index = self.arrange_selected + 1;
        match self.song.insert_arrangement_step(index, number) {
            Ok(index) => self.arrange_selected = index,
            Err(error) => {
                self.status = format!("duplicate step: {error}");
                return;
            }
        }
        self.status = format!("duplicated step for pattern {number}");
    }
    fn arrangement_remove_step(&mut self) {
        if self.song.order.len() <= 1 {
            self.status = "cannot remove the only arrangement step".into();
            return;
        }
        self.song.order.remove(self.arrange_selected);
        self.arrange_selected = self.arrange_selected.min(self.song.order.len() - 1);
        self.tracker_order = self.tracker_order.min(self.song.order.len() - 1);
        self.status = "arrangement step removed".into();
    }
    fn arrangement_move_step(&mut self, direction: i8) {
        let len = self.song.order.len();
        if len < 2 {
            return;
        }
        let next = if direction < 0 {
            self.arrange_selected.saturating_sub(1)
        } else {
            (self.arrange_selected + 1).min(len - 1)
        };
        if next != self.arrange_selected {
            self.song.order.swap(self.arrange_selected, next);
            self.arrange_selected = next;
            self.status = format!("moved step to {:02}", next + 1);
        }
    }
    fn arrangement_jump_to_pattern(&mut self) {
        self.tracker_order = self
            .arrange_selected
            .min(self.song.order.len().saturating_sub(1));
        self.tracker_row = 0;
        self.clamp_tracker_cursor();
        self.set_screen(Screen::Tracker);
        self.status = format!("editing pattern {}", self.tracker_pattern_number());
    }
    fn arrangement_play_from_step(&mut self) {
        self.tracker_order = self
            .arrange_selected
            .min(self.song.order.len().saturating_sub(1));
        self.tracker_row = 0;
        self.tracker_play(false);
    }
    fn tracker_note_off(&mut self) {
        if self.tracker_mode != TrackerMode::Edit {
            return;
        }
        self.cancel_tracker_gesture();
        if let Some(cell) = self.tracker_cell_mut() {
            cell.note = Note::Off;
            cell.velocity = None;
            cell.program = None;
            cell.gate = None;
            if matches!(cell.command, Command::Retrigger(_)) {
                cell.command = Command::None;
            }
            self.advance_tracker_row();
            self.status = format!("NOTE OFF · advanced {} row(s)", self.tracker_advance);
        }
    }
    fn change_program(&mut self, direction: i8) {
        let track = self.tracker_track;
        if let Some(page) = self.current_page_mut() {
            let name = page.name.clone();
            let column = page.column_mut(track);
            column.program = if direction < 0 {
                column.program.saturating_sub(1)
            } else {
                column.program.saturating_add(1).min(127)
            };
            self.status = format!("{name} column {} program {}", track + 1, column.program);
            self.sync_tracker_route();
        }
    }
    fn change_bank(&mut self, msb: bool, direction: i8) {
        let track = self.tracker_track;
        if let Some(column) = self.current_column_mut() {
            let value = if msb {
                &mut column.bank_msb
            } else {
                &mut column.bank_lsb
            };
            *value = if direction < 0 {
                value.saturating_sub(1)
            } else {
                value.saturating_add(1).min(127)
            };
            let (bank_msb, bank_lsb) = (column.bank_msb, column.bank_lsb);
            self.status = format!("column {} bank {bank_msb}/{bank_lsb}", track + 1);
            self.sync_tracker_route();
        }
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
    fn commit_loaded_preset(
        &mut self,
        preset: Preset,
        original_values: HashMap<u8, f32>,
        values: HashMap<u8, f32>,
    ) {
        self.performance_meter
            .set_audio_unavailable(AudioAvailability::Stopped);
        self.playing = Some(preset);
        self.original_values = original_values;
        self.values = values;
        self.arm_pickup();
        self.set_screen(Screen::Playback);
    }

    fn observe_mapped_control(&mut self, cc: u8, value: f32) {
        if cc != VOLUME_CC {
            return;
        }
        if self
            .last_mapped_volume
            .is_some_and(|previous| value < previous)
        {
            self.performance_meter.clear_numeric_peaks();
        }
        self.last_mapped_volume = Some(value);
    }

    fn apply_control_value(&mut self, cc: u8, value: f32) {
        if cc == VOLUME_CC
            && self
                .values
                .get(&cc)
                .is_some_and(|previous| value < *previous)
        {
            self.performance_meter.clear_numeric_peaks();
        }
        self.values.insert(cc, value);
    }
    fn audio_graph_edit_blocker(&self) -> Option<&'static str> {
        if !self.config.audio_graph.enabled {
            return None;
        }
        if self.audio_recorder.status().recording || self.recorder.is_recording() {
            return Some("stop recording before changing the insert rack");
        }
        if self.playback.is_some()
            || self.sequencer.status().playing
            || self.loop_player.status().playing
            || self.song_previewing
        {
            return Some("stop transport before changing the insert rack");
        }
        None
    }

    fn selected_effect_id(&self) -> Option<EffectId> {
        project_fx_rack(
            &self.song.insert_rack,
            &self.song.aux_routing,
            self.fx_target,
        )?
        .order
        .get(self.fx_selected)
        .copied()
    }

    fn selected_effect(&self) -> Option<&crate::audio_graph::EffectInstance> {
        let id = self.selected_effect_id()?;
        project_fx_rack(
            &self.song.insert_rack,
            &self.song.aux_routing,
            self.fx_target,
        )?
        .effect(id)
    }

    fn commit_fx_routing(
        &mut self,
        rack: InsertRack,
        aux_routing: ProjectAuxRouting,
        success: String,
    ) -> bool {
        if let Err(status) = self.publish_fx_routing_runtime(&rack, &aux_routing) {
            self.status = status;
            return false;
        }
        self.song.insert_rack = rack;
        self.song.aux_routing = aux_routing;
        let length = project_fx_rack(
            &self.song.insert_rack,
            &self.song.aux_routing,
            self.fx_target,
        )
        .map(|rack| rack.order.len())
        .unwrap_or(0);
        self.fx_selected = self.fx_selected.min(length.saturating_sub(1));
        self.status = success;
        true
    }

    fn publish_fx_routing_runtime(
        &mut self,
        rack: &InsertRack,
        aux_routing: &ProjectAuxRouting,
    ) -> std::result::Result<(), String> {
        if let Some(reason) = self.audio_graph_edit_blocker() {
            return Err(reason.into());
        }
        if let Err(error) = aux_routing.validate(rack) {
            return Err(format!("FX routing: {error}"));
        }
        if let Some(engine) = self.engine.as_mut() {
            match engine.publish_fx_routing(rack, aux_routing) {
                Ok(_) => {}
                Err(error) => {
                    return Err(format!("FX publication: {error:#}"));
                }
            }
        }
        Ok(())
    }

    fn add_effect(&mut self) {
        let kind = INSERT_EFFECTS[self.fx_add_kind];
        let mut rack = self.song.insert_rack.clone();
        let mut aux = self.song.aux_routing.clone();
        let result = if self.fx_target == 0 {
            aux.next_effect_id(&rack)
                .and_then(|id| rack.add_with_id(kind, id).map(|()| id))
        } else if is_aux_target(self.fx_target) {
            let aux_id = self.fx_target as u8;
            while aux.buses.iter().all(|bus| bus.id != aux_id) {
                if let Err(error) = aux.add_bus() {
                    self.status = format!("FX add: {error}");
                    return;
                }
            }
            let id = match aux.add_effect(&rack, aux_id, kind) {
                Ok(id) => id,
                Err(error) => {
                    self.status = format!("FX add: {error}");
                    return;
                }
            };
            if aux.sends.iter().all(|send| send.aux_id != aux_id) {
                if let Err(error) = aux.set_send(&rack, aux_id, -18.0, SendPoint::PostInsert) {
                    self.status = format!("FX send: {error}");
                    return;
                }
            }
            Ok(id)
        } else {
            aux.next_effect_id(&rack).and_then(|id| {
                aux.master_rack.add_with_id(kind, id)?;
                aux.validate(&rack)?;
                Ok(id)
            })
        };
        match result {
            Ok(id) => {
                let index = project_fx_rack(&rack, &aux, self.fx_target)
                    .map(|rack| rack.order.len().saturating_sub(1))
                    .unwrap_or(0);
                if self.commit_fx_routing(
                    rack,
                    aux,
                    format!("added {} #{id}", effect_kind_label(kind)),
                ) {
                    self.fx_selected = index;
                    self.fx_parameter = 0;
                }
            }
            Err(error) => self.status = format!("FX add: {error}"),
        }
    }

    fn remove_effect(&mut self) {
        let Some(id) = self.selected_effect_id() else {
            self.status = "FX rack is empty".into();
            return;
        };
        let mut rack = self.song.insert_rack.clone();
        let mut aux = self.song.aux_routing.clone();
        let result = project_fx_rack_mut(&mut rack, &mut aux, self.fx_target)
            .expect("selected effect has a rack")
            .remove(id);
        if is_aux_target(self.fx_target)
            && project_fx_rack(&rack, &aux, self.fx_target)
                .is_some_and(|rack| rack.effects.is_empty())
        {
            aux.clear_send(self.fx_target as u8);
        }
        match result {
            Ok(effect) => {
                self.commit_fx_routing(
                    rack,
                    aux,
                    format!("removed {} #{id}", effect_kind_label(effect.kind)),
                );
            }
            Err(error) => self.status = format!("FX remove: {error}"),
        }
    }

    fn move_effect(&mut self, direction: i8) {
        let Some(id) = self.selected_effect_id() else {
            self.status = "FX rack is empty".into();
            return;
        };
        let destination = if direction < 0 {
            self.fx_selected.saturating_sub(1)
        } else {
            let length = project_fx_rack(
                &self.song.insert_rack,
                &self.song.aux_routing,
                self.fx_target,
            )
            .map(|rack| rack.order.len())
            .unwrap_or(0);
            (self.fx_selected + 1).min(length - 1)
        };
        if destination == self.fx_selected {
            return;
        }
        let mut rack = self.song.insert_rack.clone();
        let mut aux = self.song.aux_routing.clone();
        if let Err(error) = project_fx_rack_mut(&mut rack, &mut aux, self.fx_target)
            .expect("selected effect has a rack")
            .move_to(id, destination)
        {
            self.status = format!("FX reorder: {error}");
            return;
        }
        if self.commit_fx_routing(rack, aux, format!("moved FX #{id}")) {
            self.fx_selected = destination;
        }
    }

    fn toggle_effect_bypass(&mut self) {
        let Some(id) = self.selected_effect_id() else {
            self.status = "FX rack is empty".into();
            return;
        };
        let mut rack = self.song.insert_rack.clone();
        let mut aux = self.song.aux_routing.clone();
        let effect = project_fx_rack_mut(&mut rack, &mut aux, self.fx_target)
            .and_then(|rack| rack.effect_mut(id))
            .expect("rack order was validated");
        effect.bypass = !effect.bypass;
        let bypass = effect.bypass;
        self.commit_fx_routing(
            rack,
            aux,
            format!("FX #{id} {}", if bypass { "bypassed" } else { "active" }),
        );
    }

    fn cycle_effect_kind(&mut self, direction: i8) {
        let mut next = if direction < 0 {
            self.fx_add_kind
                .checked_sub(1)
                .unwrap_or(INSERT_EFFECTS.len() - 1)
        } else {
            (self.fx_add_kind + 1) % INSERT_EFFECTS.len()
        };
        if is_aux_target(self.fx_target) {
            while !matches!(
                INSERT_EFFECTS[next],
                EffectKind::Delay
                    | EffectKind::Reverb
                    | EffectKind::Chorus
                    | EffectKind::Flanger
                    | EffectKind::Phaser
            ) {
                next = if direction < 0 {
                    next.checked_sub(1).unwrap_or(INSERT_EFFECTS.len() - 1)
                } else {
                    (next + 1) % INSERT_EFFECTS.len()
                };
            }
        }
        self.fx_add_kind = next;
        self.status = format!(
            "next FX to add · {}",
            effect_kind_label(INSERT_EFFECTS[self.fx_add_kind])
        );
    }

    fn cycle_fx_target(&mut self) {
        self.fx_target = (self.fx_target + 1) % (MAX_AUX_BUSES + 2);
        self.fx_selected = 0;
        self.fx_parameter = 0;
        if is_aux_target(self.fx_target)
            && !matches!(
                INSERT_EFFECTS[self.fx_add_kind],
                EffectKind::Delay
                    | EffectKind::Reverb
                    | EffectKind::Chorus
                    | EffectKind::Flanger
                    | EffectKind::Phaser
            )
        {
            self.fx_add_kind = FIRST_AUX_EFFECT_INDEX;
        }
        self.status = format!("FX target · {}", fx_target_label(self.fx_target));
    }

    fn adjust_aux_send(&mut self, direction: i8) {
        if !is_aux_target(self.fx_target) {
            self.status = "select AUX 1 or AUX 2 for send level".into();
            return;
        }
        let aux_id = self.fx_target as u8;
        let mut aux = self.song.aux_routing.clone();
        let Some(bus) = aux.buses.iter().find(|bus| bus.id == aux_id) else {
            self.status = "add an aux effect before enabling its send".into();
            return;
        };
        if bus.rack.effects.is_empty() {
            self.status = "add an aux effect before enabling its send".into();
            return;
        }
        let existing = aux.sends.iter().find(|send| send.aux_id == aux_id);
        let point = existing
            .map(|send| send.point)
            .unwrap_or(SendPoint::PostInsert);
        let current = existing.map(|send| send.level_db).unwrap_or(-27.0);
        if direction < 0 && current <= -60.0 {
            aux.clear_send(aux_id);
            self.commit_fx_routing(
                self.song.insert_rack.clone(),
                aux,
                format!("AUX {aux_id} send · OFF"),
            );
            return;
        }
        let value = (current + 3.0 * f32::from(direction.signum())).clamp(-60.0, 12.0);
        if let Err(error) = aux.set_send(&self.song.insert_rack, aux_id, value, point) {
            self.status = format!("FX send: {error}");
            return;
        }
        self.commit_fx_routing(
            self.song.insert_rack.clone(),
            aux,
            format!("AUX {aux_id} send · {value:.0} dB"),
        );
    }

    fn toggle_aux_send_point(&mut self) {
        if !is_aux_target(self.fx_target) {
            self.status = "select AUX 1 or AUX 2".into();
            return;
        }
        let aux_id = self.fx_target as u8;
        let mut aux = self.song.aux_routing.clone();
        let Some(send) = aux.sends.iter().find(|send| send.aux_id == aux_id).cloned() else {
            self.status = "enable the aux send first".into();
            return;
        };
        let point = match send.point {
            SendPoint::PreInsert => SendPoint::PostInsert,
            SendPoint::PostInsert => SendPoint::PreInsert,
        };
        if let Err(error) = aux.set_send(&self.song.insert_rack, aux_id, send.level_db, point) {
            self.status = format!("FX send: {error}");
            return;
        }
        self.commit_fx_routing(
            self.song.insert_rack.clone(),
            aux,
            format!("AUX {aux_id} send · {}", send_point_label(point)),
        );
    }

    fn cycle_aux_return(&mut self) {
        if !is_aux_target(self.fx_target) {
            self.status = "select AUX 1 or AUX 2".into();
            return;
        }
        let aux_id = self.fx_target as u8;
        let mut aux = self.song.aux_routing.clone();
        let Some(bus) = aux.buses.iter_mut().find(|bus| bus.id == aux_id) else {
            self.status = "aux bus is empty".into();
            return;
        };
        bus.return_gain_db = if bus.return_gain_db <= -60.0 {
            12.0
        } else {
            (bus.return_gain_db - 3.0).max(-60.0)
        };
        let value = bus.return_gain_db;
        self.commit_fx_routing(
            self.song.insert_rack.clone(),
            aux,
            format!("AUX {aux_id} return · {value:.0} dB"),
        );
    }

    fn adjust_effect_parameter(&mut self, direction: i8) {
        let Some(id) = self.selected_effect_id() else {
            self.status = "FX rack is empty".into();
            return;
        };
        let mut rack = self.song.insert_rack.clone();
        let mut aux = self.song.aux_routing.clone();
        let effect = project_fx_rack_mut(&mut rack, &mut aux, self.fx_target)
            .and_then(|rack| rack.effect_mut(id))
            .expect("rack order was validated");
        let schema = crate::effect_schema::schema(effect.kind);
        self.fx_parameter = self.fx_parameter.min(schema.len().saturating_sub(1));
        let spec = schema[self.fx_parameter];
        if is_aux_target(self.fx_target)
            && matches!(spec.name, "dry_percent" | "wet_percent" | "mix_percent")
        {
            self.status = "aux wet/dry controls are fixed at 100% wet".into();
            return;
        }
        let current = effect
            .parameters
            .get(spec.name)
            .copied()
            .unwrap_or(spec.default);
        let value = match spec.value_type {
            crate::effect_schema::ParameterType::Toggle => 1.0 - current,
            crate::effect_schema::ParameterType::Integer => {
                (current + f32::from(direction.signum())).clamp(spec.minimum, spec.maximum)
            }
            crate::effect_schema::ParameterType::Choices(choices) => {
                let current_index = choices
                    .iter()
                    .position(|choice| f32::from(*choice) == current)
                    .unwrap_or(0);
                let index = if direction < 0 {
                    current_index.saturating_sub(1)
                } else {
                    (current_index + 1).min(choices.len() - 1)
                };
                f32::from(choices[index])
            }
            crate::effect_schema::ParameterType::Continuous => {
                let step = match spec.unit {
                    "dB" | "dBFS" => 0.5,
                    "%" => 1.0,
                    "Hz" => (current * (2.0_f32.powf(1.0 / 24.0) - 1.0)).max(0.1),
                    "ms" => ((spec.maximum - spec.minimum) / 200.0).max(0.1),
                    _ => ((spec.maximum - spec.minimum) / 100.0).max(0.01),
                };
                (current + step * f32::from(direction.signum())).clamp(spec.minimum, spec.maximum)
            }
        };
        effect.parameters.insert(spec.name.into(), value);
        self.commit_fx_routing(
            rack,
            aux,
            format!("{} · {value:.2} {}", spec.name, spec.unit),
        );
    }
    fn load(&mut self, state: &Path, _tx: std::sync::mpsc::Sender<MidiEvent>) {
        if let Some(reason) = self.audio_graph_edit_blocker() {
            self.status = reason.into();
            return;
        }
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
        let p = self.presets[self.selected].clone();
        let original_values = match engine::initial_values(&p) {
            Ok(values) => values,
            Err(error) => {
                self.status = format!("INVALID PRESET: {error:#}");
                return;
            }
        };
        self.stop_recording();
        self.stop_playback();
        if let Some(engine) = self.engine.as_mut() {
            match engine.load_in_place(&p) {
                Ok(true) => {
                    let backend = engine.backend();
                    self.commit_loaded_preset(p, original_values.clone(), original_values);
                    self.status = format!("{} sound loaded in place · MIDI ready", backend.label());
                    return;
                }
                Ok(false) => {}
                Err(error) => {
                    self.status = format!("IN-PLACE LOAD FAILED: {error:#}");
                    return;
                }
            }
        }
        if let Err(error) = engine::validate_start(&p, state, &self.config) {
            self.status = format!("START PRECHECK FAILED: {error:#}");
            return;
        }
        self.engine.take();
        self.performance_meter
            .set_audio_unavailable(AudioAvailability::Stopped);
        self.playing = None;
        self.status = format!("starting JACK/{}…", p.backend.label());
        let backend_label = p.backend.label();
        match Engine::start_with_routing(
            &p,
            state,
            Arc::clone(&self.midi_output),
            &self.config,
            &self.song.insert_rack,
            &self.song.aux_routing,
        ) {
            Ok(e) => {
                let audio_route = e.audio_route_status();
                self.engine = Some(e);
                if let Ok(mut backend) = self.midi_backend.lock() {
                    *backend = p.backend;
                }
                self.commit_loaded_preset(p, original_values.clone(), original_values);
                self.status = format!(
                    "{backend_label} running · MIDI ready{}",
                    audio_route.map_or_else(String::new, |route| format!(" · {route}"))
                );
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
        } else if self.engine.is_none() {
            self.status = "load the idea preset before playing its recording".into();
        } else if self.last.is_empty() {
            self.status = "no recording yet".into();
        } else {
            let events = self.last.clone();
            let output = Arc::clone(&self.midi_output);
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let worker = std::thread::spawn(move || {
                recording::play_events(
                    &events,
                    |message| {
                        if let Ok(mut output) = output.lock() {
                            if let Some(output) = output.as_mut() {
                                let _ = output.send(message);
                            }
                        }
                    },
                    &worker_stop,
                );
            });
            self.playback = Some(Playback {
                stop,
                worker: Some(worker),
            });
            self.status = "▶ playing recording".into();
        }
    }
    fn open_ideas(&mut self) {
        if self.screen == Screen::Tracker {
            self.set_tracker_mode(TrackerMode::Play);
        }
        self.ideas = recording::list(&recording::ideas_dir()).unwrap_or_default();
        self.idea_selected = self.idea_selected.min(self.ideas.len().saturating_sub(1));
        self.confirm_delete = None;
        self.set_screen(Screen::Ideas);
        self.status = "ideas · select an action".into();
    }
    fn open_help(&mut self) {
        self.stop_tracker_recording();
        if self.screen != Screen::Help {
            self.help_previous = self.screen;
        }
        self.set_screen(Screen::Help);
        self.start_web_help();
        self.sync_tracker_route();
        self.reset_context_page();
        self.status = format!("HELP · {} · EXIT closes", self.web_help_status);
    }
    fn close_help(&mut self) {
        self.web_help = None;
        self.web_help_status.clear();
        self.set_screen(self.help_previous);
        self.sync_tracker_route();
        self.status = format!("returned to {}", self.screen.label());
    }
    fn start_web_help(&mut self) {
        if self.web_help.is_some() {
            return;
        }
        if !self.web_help_enabled {
            self.web_help_status = "web help unavailable".into();
            return;
        }
        match help::start_web_help() {
            Ok(server) => {
                self.web_help_status = server.url().to_owned();
                self.web_help = Some(server);
            }
            Err(error) => {
                self.web_help_status = error.label().into();
            }
        }
    }
    fn move_help(&mut self, delta: isize) {
        let last = help::lines(HELP_TEXT_WIDTH).len().saturating_sub(1);
        self.help_selected = if delta < 0 {
            self.help_selected.saturating_sub(delta.unsigned_abs())
        } else {
            (self.help_selected + delta as usize).min(last)
        };
    }
    fn activate_help(&mut self) {
        let lines = help::lines(HELP_TEXT_WIDTH);
        let Some(line) = lines.get(self.help_selected) else {
            return;
        };
        let Some(target) = line.target.as_deref() else {
            self.status = "HELP · select a row ending in > to jump".into();
            return;
        };
        if let Some(index) = help::target_index(&lines, target) {
            self.help_selected = index;
            self.status = format!("HELP · {}", lines[index].text);
        } else {
            self.status = format!("HELP · missing section #{target}");
        }
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
        if let Some(reason) = self.audio_graph_edit_blocker() {
            self.status = reason.into();
            return;
        }
        let Some(name) = self.ideas.get(self.idea_selected).cloned() else {
            self.status = "no saved idea selected".into();
            return;
        };
        if self.playing.is_some() && self.confirm_load.as_deref() != Some(&name) {
            self.confirm_load = Some(name.clone());
            self.status = format!("CONFIRM REPLACE current preset with {name}: choose Load again");
            return;
        }
        match recording::load_with_parameters(&recording::ideas_dir(), &name) {
            Ok((preset, saved_values, events)) => {
                let original_values = match engine::initial_values(&preset) {
                    Ok(values) => values,
                    Err(error) => {
                        self.status = format!("idea preset is invalid: {error:#}");
                        return;
                    }
                };
                self.stop_recording();
                self.stop_playback();
                if let Some(engine) = self.engine.as_mut() {
                    match engine.load_in_place(&preset) {
                        Ok(true) => {
                            if let Ok(mut backend) = self.midi_backend.lock() {
                                *backend = preset.backend;
                            }
                            self.finish_idea_load(
                                preset,
                                original_values,
                                saved_values,
                                events,
                                &name,
                                true,
                            );
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
                if let Err(error) = engine::validate_start(&preset, state, &self.config) {
                    self.status = format!("idea sound precheck failed: {error:#}");
                    return;
                }
                self.engine.take();
                self.performance_meter
                    .set_audio_unavailable(AudioAvailability::Stopped);
                self.playing = None;
                match Engine::start_with_routing(
                    &preset,
                    state,
                    Arc::clone(&self.midi_output),
                    &self.config,
                    &self.song.insert_rack,
                    &self.song.aux_routing,
                ) {
                    Ok(engine) => {
                        let audio_route = engine.audio_route_status();
                        self.engine = Some(engine);
                        if let Ok(mut backend) = self.midi_backend.lock() {
                            *backend = preset.backend;
                        }
                        self.finish_idea_load(
                            preset,
                            original_values,
                            saved_values,
                            events,
                            &name,
                            false,
                        );
                        if let Some(route) = audio_route {
                            self.status.push_str(&format!(" · {route}"));
                        }
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
    fn finish_idea_load(
        &mut self,
        preset: Preset,
        original_values: HashMap<u8, f32>,
        saved_values: HashMap<u8, f32>,
        events: Vec<TimedEvent>,
        name: &str,
        in_place: bool,
    ) {
        let mut values = original_values.clone();
        values.extend(saved_values.iter().map(|(&cc, &value)| (cc, value)));
        let restore_error = if saved_values.is_empty() {
            None
        } else {
            self.engine
                .as_ref()
                .and_then(|engine| engine.set_mapped_parameters(&values).err())
        };
        self.last = events;
        if let Some(error) = restore_error {
            if let Some(engine) = self.engine.as_ref() {
                let _ = engine.set_mapped_parameters(&original_values);
            }
            self.commit_loaded_preset(preset, original_values.clone(), original_values);
            self.status = format!(
                "loaded idea {name}, but parameter restore failed: {error:#} · preset defaults active"
            );
        } else {
            self.commit_loaded_preset(preset, original_values, values);
            self.status = format!(
                "loaded idea {name}{} · recording ready",
                if in_place { " in place" } else { "" }
            );
        }
    }
    fn tick(&mut self) {
        let now = Instant::now();
        self.refresh_cpu_temperature(now);
        if self.screen == Screen::Meter {
            self.performance_meter
                .poll_cpu(now, Path::new("/proc/stat"));
            if let Some(engine) = self.engine.as_ref() {
                if let Some(meter) = engine.master_meter() {
                    self.performance_meter.update_audio(meter.output, now);
                } else {
                    self.performance_meter
                        .set_audio_unavailable(AudioAvailability::DirectUnavailable);
                }
            } else {
                self.performance_meter
                    .set_audio_unavailable(AudioAvailability::Stopped);
            }
        }
        if self.screen == Screen::Tracker
            && self.tracker_mode == TrackerMode::Edit
            && self.note_editor.is_none()
        {
            self.commit_tracker_gesture(now);
        }
        if self.screen == Screen::Tracker {
            let tracker = self.sequencer.status();
            self.follow_tracker_transport(&tracker);
            let restart = self.tracker_recording.as_mut().and_then(|recording| {
                let wrapped = tracker.playing && tracker.row < recording.last_row;
                recording.last_row = tracker.row;
                wrapped.then_some((recording.pattern, recording.page))
            });
            if let Some((pattern, page)) = restart {
                self.sequencer
                    .play(&self.tracker_record_song(pattern, page), 0, 0);
            }
            if tracker.playing && !tracker.available {
                self.cancel_tracker_gesture();
                if let Some(error) = tracker.error {
                    self.status = format!("tracker target unavailable: {error}");
                }
            }
        }
        if let Some(status) = self.engine.as_mut().and_then(Engine::poll_audio_graph) {
            self.performance_meter
                .set_audio_unavailable(AudioAvailability::DirectUnavailable);
            self.status = status;
        }
        if self.engine.as_mut().is_some_and(|engine| !engine.alive()) {
            self.playback.take();
            self.engine.take();
            self.performance_meter
                .set_audio_unavailable(AudioAvailability::Stopped);
            self.playing = None;
            self.status = "ENGINE EXITED · select a sound to restart it".into();
        }
        let done = self.playback.as_ref().is_some_and(|playback| {
            playback
                .worker
                .as_ref()
                .is_none_or(std::thread::JoinHandle::is_finished)
        });
        if done {
            self.stop_playback();
            self.status = "recording playback complete · all notes off".into();
        }
    }

    fn follow_tracker_transport(&mut self, tracker: &sequencer::SequencerStatus) {
        if tracker.playing {
            self.tracker_order = self.tracker_recording.as_ref().map_or_else(
                || tracker.order.min(self.song.order.len().saturating_sub(1)),
                |recording| recording.order,
            );
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
    app.controller_layout = crate::pads::PadConfig::load(&state.join("controller.conf"))
        .unwrap_or_default()
        .layout;
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
            MidiEvent::MappedControl(cc, v) => {
                app.observe_mapped_control(cc, v);
            }
            MidiEvent::Value(cc, v) => {
                app.apply_control_value(cc, v);
            }
            MidiEvent::Raw { received, bytes } => {
                app.held_notes.observe(&bytes);
                app.recorder.capture(received, &bytes);
                let tracker_preview = app.screen == Screen::Tracker
                    && (matches!(app.tracker_mode, TrackerMode::Edit | TrackerMode::Noob)
                        || app.note_editor.is_some()
                        || app.tracker_recording.is_some());
                if !tracker_preview {
                    app.sequencer.thru(&bytes);
                }
                if app.screen == Screen::Tracker && app.tracker_recording.is_some() {
                    app.record_tracker_midi(&bytes);
                }
                if app.screen == Screen::Tracker
                    && app.tracker_mode == TrackerMode::Edit
                    && app.note_editor.is_none()
                {
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
                    app.tracker_gesture.observe(received, &bytes);
                }
            }
            MidiEvent::Pad(pad, pressed) => {
                if pressed {
                    match pad.menu_input() {
                        MenuInput::SelectPage(page) => app.select_menu_page(page),
                        MenuInput::CyclePage => app.cycle_menu_page(1),
                        MenuInput::ActivateItem(item) => {
                            let slot = navigation::slot(
                                app.screen,
                                app.menu_context(),
                                app.menu_page(),
                                item,
                            );
                            if let Some(action) = slot.and_then(|slot| slot.dispatch()) {
                                perform(action, app, state, Some(tx));
                            }
                        }
                    }
                }
            }
            MidiEvent::Encoder(action) => {
                if app.controller_layout == ControllerLayout::Four {
                    match action {
                        crate::pads::EncoderAction::Select => {
                            app.prepare_confirmation_action(Action::Noop);
                            app.page_select_mode = !app.page_select_mode;
                            app.status = if app.page_select_mode {
                                "PAGE SELECT · turn encoder · press to return".into()
                            } else {
                                "encoder returned to screen control".into()
                            };
                        }
                        crate::pads::EncoderAction::Up if app.page_select_mode => {
                            app.cycle_menu_page(-1)
                        }
                        crate::pads::EncoderAction::Down if app.page_select_mode => {
                            app.cycle_menu_page(1)
                        }
                        crate::pads::EncoderAction::Up => {
                            perform(Action::Up, app, state, Some(tx));
                        }
                        crate::pads::EncoderAction::Down => {
                            perform(Action::Down, app, state, Some(tx));
                        }
                    }
                } else {
                    let action = match action {
                        crate::pads::EncoderAction::Up => Action::Up,
                        crate::pads::EncoderAction::Down => Action::Down,
                        crate::pads::EncoderAction::Select => Action::Activate,
                    };
                    perform(action, app, state, Some(tx));
                }
            }
            MidiEvent::PadLock(locked) => {
                app.pad_locked = locked;
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
    a.prepare_confirmation_action(action);
    // EXIT is the same physical page/item on every normal and contextual
    // menu, so it must not be swallowed by an active editor or confirmation.
    if action == Action::Quit {
        return true;
    }
    // PANIC is the other global system action. It remains live even while a
    // modal confirmation owns the rest of the input dispatch.
    if action == Action::StopAll {
        a.stop_all(state);
        return false;
    }
    if a.project_name_input.is_some() {
        match action {
            Action::Activate => a.commit_project_rename(),
            Action::Back => {
                a.project_name_input = None;
                a.status = "project naming cancelled".into();
            }
            _ => {}
        }
        return false;
    }
    if action == Action::OpenHelp && a.screen != Screen::Help {
        a.open_help();
        return false;
    }
    if a.screen == Screen::Help {
        match action {
            Action::Up => a.move_help(-1),
            Action::Down => a.move_help(1),
            Action::PageUp => a.move_help(-10),
            Action::PageDown => a.move_help(10),
            Action::Home => a.help_selected = 0,
            Action::End => a.help_selected = help::lines(HELP_TEXT_WIDTH).len().saturating_sub(1),
            Action::Activate => a.activate_help(),
            Action::Back => a.close_help(),
            Action::OpenHelp => {}
            Action::Quit | Action::StopAll => unreachable!("handled before help dispatch"),
            _ => {}
        }
        return false;
    }
    if a.note_editor.is_some() {
        match action {
            Action::Up | Action::NoteEditorDecrease => a.adjust_note_editor(-1),
            Action::Down | Action::NoteEditorIncrease => a.adjust_note_editor(1),
            Action::Activate | Action::NoteEditorConfirm => a.confirm_note_editor(),
            Action::Back | Action::NoteEditorCancel => a.cancel_note_editor(),
            Action::NoteField => a.select_note_editor_field(NoteEditorField::Note),
            Action::GateField => a.select_note_editor_field(NoteEditorField::Gate),
            Action::VelocityField => a.select_note_editor_field(NoteEditorField::Velocity),
            Action::ProgramField => a.select_note_editor_field(NoteEditorField::Program),
            Action::EffectField => a.select_note_editor_field(NoteEditorField::Effect),
            Action::EffectParameterField => {
                a.select_note_editor_field(NoteEditorField::EffectParameter)
            }
            Action::NoteEditorClearField => a.clear_note_editor_field(),
            Action::NoteEditorPreviousField => a.move_note_editor_field(-1),
            Action::NoteEditorNextField => a.move_note_editor_field(1),
            Action::TrackerEdit => {
                a.cancel_note_editor();
                a.set_tracker_edit(true);
                a.status = "step edit on".into();
            }
            Action::TrackerStop => {
                a.tracker_stop();
            }
            Action::StopAll => unreachable!("panic is handled before contextual dispatch"),
            _ => {}
        }
        return false;
    }
    if a.tracker_recording.is_some() {
        match action {
            Action::TrackerRecord | Action::TrackerStop => {
                a.stop_tracker_recording();
                return false;
            }
            Action::Back => {
                a.stop_tracker_recording();
                return false;
            }
            Action::StopAll => unreachable!("panic is handled before contextual dispatch"),
            _ => return false,
        }
    }
    if a.screen == Screen::TrackerFiles && a.confirm_pattern_clear {
        match action {
            Action::Up | Action::PatternSizeDown => a.change_pattern_size(-1),
            Action::Down | Action::PatternSizeUp => a.change_pattern_size(1),
            Action::SelectThreeFour => a.select_pattern_meter(3),
            Action::SelectFourFour => a.select_pattern_meter(4),
            Action::Activate | Action::ConfirmPatternClear => a.apply_pattern_clear(),
            Action::ClearPatternNow => {
                if a.pattern_setup_new {
                    a.create_pattern(a.pattern_setup_rows);
                } else {
                    a.confirm_pattern_clear = false;
                    a.clear_pattern_now();
                }
            }
            Action::Back => {
                a.confirm_pattern_clear = false;
                a.reset_context_page();
                a.status = "pattern clear cancelled".into();
            }
            _ => {}
        }
        return false;
    }
    match action {
        Action::Noop => {}
        Action::Up => {
            if a.screen == Screen::Ideas {
                a.idea_selected = a.idea_selected.saturating_sub(1);
            } else if a.screen == Screen::TrackerFiles {
                match a.tracker_files_mode {
                    TrackerFilesMode::Projects => {
                        a.song_selected = a.song_selected.saturating_sub(1);
                        a.confirm_song_delete = None;
                    }
                    TrackerFilesMode::Drums => {
                        a.move_drum_selection(-1);
                    }
                    TrackerFilesMode::Patterns => {}
                }
            } else if a.screen == Screen::TrackerArrange {
                a.select_arrangement_step(-1);
            } else if a.screen == Screen::Tracker {
                a.cancel_tracker_gesture();
                a.tracker_row = a.tracker_row.saturating_sub(1);
            } else if a.screen == Screen::TrackerPages {
                a.turn_page_manager(-1);
            } else if a.screen == Screen::TrackerLoop {
                if a.loop_library_mode {
                    a.loop_library_selected = a.loop_library_selected.saturating_sub(1);
                } else {
                    a.loop_selected = a.loop_selected.saturating_sub(1);
                }
            } else if a.screen == Screen::Presets {
                a.selected = a.selected.saturating_sub(1);
            } else if a.screen == Screen::FxRack {
                a.fx_selected = a.fx_selected.saturating_sub(1);
            } else if a.screen == Screen::FxEditor {
                a.adjust_effect_parameter(-1);
            }
        }
        Action::Down => {
            if a.screen == Screen::Ideas {
                a.idea_selected = (a.idea_selected + 1).min(a.ideas.len().saturating_sub(1));
            } else if a.screen == Screen::TrackerFiles {
                match a.tracker_files_mode {
                    TrackerFilesMode::Projects => {
                        a.song_selected =
                            (a.song_selected + 1).min(a.song_list.len().saturating_sub(1));
                        a.confirm_song_delete = None;
                    }
                    TrackerFilesMode::Drums => {
                        a.move_drum_selection(1);
                    }
                    TrackerFilesMode::Patterns => {}
                }
            } else if a.screen == Screen::TrackerArrange {
                a.select_arrangement_step(1);
            } else if a.screen == Screen::Tracker {
                a.cancel_tracker_gesture();
                a.tracker_row = (a.tracker_row + 1).min(a.tracker_rows().saturating_sub(1));
            } else if a.screen == Screen::TrackerPages {
                a.turn_page_manager(1);
            } else if a.screen == Screen::TrackerLoop {
                if a.loop_library_mode {
                    a.loop_library_selected =
                        (a.loop_library_selected + 1).min(a.loop_library.len().saturating_sub(1));
                } else {
                    a.loop_selected =
                        (a.loop_selected + 1).min(a.loop_imports.len().saturating_sub(1));
                }
            } else if a.screen == Screen::Presets {
                a.selected = (a.selected + 1).min(a.presets.len().saturating_sub(1));
            } else if a.screen == Screen::FxRack {
                let length = project_fx_rack(&a.song.insert_rack, &a.song.aux_routing, a.fx_target)
                    .map(|rack| rack.order.len())
                    .unwrap_or(0);
                a.fx_selected = (a.fx_selected + 1).min(length.saturating_sub(1));
            } else if a.screen == Screen::FxEditor {
                a.adjust_effect_parameter(1);
            }
        }
        Action::PageUp => {
            if a.screen == Screen::Presets {
                a.selected = a.selected.saturating_sub(10);
            } else if a.screen == Screen::TrackerFiles
                && a.tracker_files_mode == TrackerFilesMode::Drums
            {
                a.move_drum_selection(-10);
            } else if a.screen == Screen::TrackerLoop && a.loop_library_mode {
                a.loop_library_selected = a.loop_library_selected.saturating_sub(10);
            }
        }
        Action::PageDown => {
            if a.screen == Screen::Presets {
                a.selected = (a.selected + 10).min(a.presets.len().saturating_sub(1));
            } else if a.screen == Screen::TrackerFiles
                && a.tracker_files_mode == TrackerFilesMode::Drums
            {
                a.move_drum_selection(10);
            } else if a.screen == Screen::TrackerLoop && a.loop_library_mode {
                a.loop_library_selected =
                    (a.loop_library_selected + 10).min(a.loop_library.len().saturating_sub(1));
            }
        }
        Action::Home => {
            if a.screen == Screen::Ideas {
                a.idea_selected = 0;
            } else if a.screen == Screen::TrackerFiles
                && a.tracker_files_mode == TrackerFilesMode::Drums
            {
                if let Some(index) = a.filtered_drum_indices().first() {
                    a.drum_pattern_selected = *index;
                }
            } else {
                a.selected = 0;
            }
        }
        Action::End => {
            if a.screen == Screen::Ideas {
                a.idea_selected = a.ideas.len().saturating_sub(1);
            } else if a.screen == Screen::TrackerFiles
                && a.tracker_files_mode == TrackerFilesMode::Drums
            {
                if let Some(index) = a.filtered_drum_indices().last() {
                    a.drum_pattern_selected = *index;
                }
            } else {
                a.selected = a.presets.len().saturating_sub(1);
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
            Screen::Help => a.activate_help(),
            Screen::Tracker => a.tracker_skip(),
            Screen::TrackerFiles => match a.tracker_files_mode {
                TrackerFilesMode::Projects => a.load_song(),
                TrackerFilesMode::Patterns => {}
                TrackerFilesMode::Drums => a.load_drum_pattern(),
            },
            Screen::TrackerArrange => a.arrangement_jump_to_pattern(),
            Screen::TrackerPages => a.confirm_page_manager(),
            Screen::TrackerTools
            | Screen::TrackerNoob
            | Screen::TrackerLoop
            | Screen::TrackerLoopAlign => {}
            Screen::AudioRecorder => a.toggle_audio_recording(),
            Screen::Meter => a.performance_meter.clear_holds(),
            Screen::FxRack => {
                if a.selected_effect_id().is_some() {
                    a.fx_parameter = 0;
                    a.set_screen(Screen::FxEditor);
                    a.status = "effect editor · turn to adjust".into();
                } else {
                    a.status = "FX rack is empty · choose a kind and ADD".into();
                }
            }
            Screen::FxEditor => a.adjust_effect_parameter(1),
        },
        Action::Quit => unreachable!("quit is handled before contextual dispatch"),
        Action::StopAll => unreachable!("panic is handled before contextual dispatch"),
        Action::OpenPresets => {
            a.set_tracker_edit(false);
            a.set_screen(Screen::Presets);
        }
        Action::OpenIdeas => a.open_ideas(),
        Action::OpenHelp => a.open_help(),
        Action::OpenTracker => {
            a.set_screen(Screen::Tracker);
            a.refresh_page_targets();
            a.sync_tracker_route();
            let page_online = a
                .current_page()
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
            a.tracker_files_mode = TrackerFilesMode::Projects;
            a.stop_song_preview();
            a.set_screen(Screen::TrackerFiles);
            a.status = "song files · select an action".into();
        }
        Action::OpenTrackerArrange => a.open_arrange(),
        Action::OpenTrackerPages => a.open_page_manager(),
        Action::OpenTrackerTools => {
            a.set_screen(Screen::TrackerTools);
            a.reset_context_page();
            a.status = "FT2 tools · pages, files, loop, mute".into();
        }
        Action::OpenTrackerLoop => {
            a.loop_library_mode = false;
            a.set_screen(Screen::TrackerLoop);
            a.refresh_loop_imports();
            a.reset_context_page();
            a.status = format!("loop inbox · {} WAV file(s)", a.loop_imports.len());
        }
        Action::OpenTrackerLoopAlign => {
            a.set_screen(Screen::TrackerLoopAlign);
            a.reset_context_page();
            a.status = "loop align · AUTO or move by one bar".into();
        }
        Action::OpenAudioRecorder => {
            a.set_tracker_edit(false);
            a.set_screen(Screen::AudioRecorder);
            a.status = "stereo audio recorder".into();
        }
        Action::OpenFxRack => {
            let length = project_fx_rack(&a.song.insert_rack, &a.song.aux_routing, a.fx_target)
                .map(|rack| rack.order.len())
                .unwrap_or(0);
            a.fx_selected = a.fx_selected.min(length.saturating_sub(1));
            a.set_screen(Screen::FxRack);
            a.status = format!(
                "{} rack · next {}",
                fx_target_label(a.fx_target),
                effect_kind_label(INSERT_EFFECTS[a.fx_add_kind])
            );
        }
        Action::OpenFxEditor => {
            if a.selected_effect_id().is_some() {
                a.fx_parameter = 0;
                a.set_screen(Screen::FxEditor);
                a.status = "effect editor · PARAM chooses · VALUE adjusts".into();
            } else {
                a.status = "FX rack is empty".into();
            }
        }
        Action::OpenMeter => {
            a.set_tracker_edit(false);
            a.set_screen(Screen::Meter);
            a.reset_context_page();
            a.status = "passive performance meters".into();
        }
        Action::ResetMeter => {
            a.performance_meter.clear_holds();
            a.status = "meter MAX, short peak, and clip holds cleared".into();
        }
        Action::Back => {
            if a.screen == Screen::FxEditor {
                a.set_screen(Screen::FxRack);
                a.status = "insert rack".into();
                return false;
            }
            if a.screen == Screen::TrackerFiles {
                match a.tracker_files_mode {
                    TrackerFilesMode::Drums => {
                        a.tracker_files_mode = TrackerFilesMode::Patterns;
                        a.confirm_drum_pattern_delete = None;
                        a.reset_context_page();
                        a.status = format!("pattern {} tools", a.tracker_pattern_number());
                        return false;
                    }
                    TrackerFilesMode::Patterns => {
                        a.tracker_files_mode = TrackerFilesMode::Projects;
                        a.reset_context_page();
                        a.status = "Project files".into();
                        return false;
                    }
                    TrackerFilesMode::Projects => {}
                }
            }
            if a.screen == Screen::TrackerLoop && a.loop_library_mode {
                a.loop_library_mode = false;
                a.confirm_loop_delete = None;
                a.reset_context_page();
                a.status = "loop editor".into();
                return false;
            }
            if a.screen == Screen::TrackerPages {
                if a.page_manager_mode == PageManagerMode::Pages {
                    a.cancel_page_manager();
                } else {
                    a.cancel_page_field();
                }
                return false;
            }
            if a.screen == Screen::TrackerFiles && a.song_previewing {
                a.stop_song_preview();
            }
            a.confirm_delete = None;
            a.confirm_load = None;
            a.set_tracker_edit(false);
            let current_screen = a.screen;
            let next_screen = if matches!(
                current_screen,
                Screen::TrackerFiles
                    | Screen::TrackerTools
                    | Screen::TrackerArrange
                    | Screen::TrackerNoob
                    | Screen::TrackerLoop
                    | Screen::TrackerLoopAlign
            ) {
                if current_screen == Screen::TrackerLoopAlign {
                    Screen::TrackerLoop
                } else {
                    Screen::Tracker
                }
            } else if matches!(
                a.screen,
                Screen::Playback
                    | Screen::Tracker
                    | Screen::AudioRecorder
                    | Screen::FxRack
                    | Screen::Meter
            ) {
                Screen::Presets
            } else if a.playing.is_some() {
                Screen::Playback
            } else {
                Screen::Presets
            };
            a.set_screen(next_screen);
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
        Action::ResetParameters => a.reset_parameters(),
        Action::BeginRecord => {
            if a.recorder.is_recording() {
                a.status = "already recording".into();
            } else {
                a.begin_record();
            }
        }
        Action::StopRecord => a.stop_recording(),
        Action::FinishSaveRecord => a.finish_and_save(),
        Action::SaveNew => a.save_new(),
        Action::InspectIdea => a.inspect_idea(),
        Action::DeleteIdea => a.delete_idea(),
        Action::LoadIdea => {
            if let Some(tx) = tx {
                a.load_idea(state, tx.clone());
            }
        }
        Action::PlaybackRecording => a.toggle_playback(),
        Action::StopPlayback => a.stop_playback(),
        Action::TrackerPlayCursor => a.tracker_play(false),
        Action::TrackerPlayStart => a.tracker_play(true),
        Action::TrackerRecord => a.begin_tracker_recording(),
        Action::TrackerModePlay => {
            a.set_tracker_mode(TrackerMode::Play);
            a.status = "PLAY mode · normal performance and transport".into();
        }
        Action::TrackerModeEdit => {
            a.set_tracker_mode(TrackerMode::Edit);
            a.status = "EDIT mode · step entry on".into();
        }
        Action::TrackerModeNoob => a.open_noob_setup(),
        Action::NoobRootDown => a.adjust_noob_root(-1),
        Action::NoobRootUp => a.adjust_noob_root(1),
        Action::NoobScale => a.toggle_noob_scale(),
        Action::ConfirmNoob => a.confirm_noob(),
        Action::LoopImport => a.import_selected_loop(),
        Action::LoopRemove => a.remove_project_loop(),
        Action::LoopSourceDown => a.adjust_loop_source_bpm(-1),
        Action::LoopSourceUp => a.adjust_loop_source_bpm(1),
        Action::LoopBpmMode => a.cycle_loop_bpm_mode(),
        Action::LoopEditUnit => {
            a.loop_edit_bars = !a.loop_edit_bars;
            a.status = format!(
                "loop cut unit: {}",
                if a.loop_edit_bars { "BAR" } else { "BEAT" }
            );
        }
        Action::LoopStartDown => a.adjust_loop_region(true, -1),
        Action::LoopStartUp => a.adjust_loop_region(true, 1),
        Action::LoopLengthDown => a.adjust_loop_region(false, -1),
        Action::LoopLengthUp => a.adjust_loop_region(false, 1),
        Action::LoopAutoAlign => a.auto_align_loop(),
        Action::LoopOffsetDown => a.adjust_loop_offset_bars(-1),
        Action::LoopOffsetUp => a.adjust_loop_offset_bars(1),
        Action::LoopAlignDone => {
            a.set_screen(Screen::TrackerLoop);
            a.status = "loop alignment set".into();
        }
        Action::OpenLoopLibrary => a.open_loop_library(),
        Action::DeleteLoopFile => a.delete_selected_loop_file(),
        Action::TrackerStop => {
            a.tracker_stop();
        }
        Action::TrackerMute => {
            let global_lane = a.tracker_page * LANES_PER_PAGE + a.tracker_track;
            let track = a.tracker_track;
            if let Some(lane) = a
                .current_page_mut()
                .and_then(|page| page.lanes.get_mut(track))
            {
                lane.enabled = !lane.enabled;
                let muted = !lane.enabled;
                let name = lane.name.clone();
                a.sequencer.mute(global_lane, muted);
                a.status = format!("{} {}", name, if muted { "muted" } else { "enabled" });
            }
        }
        Action::TrackerPageMute => a.toggle_tracker_page_mute(),
        Action::NextTrackerPage => a.switch_tracker_page(),
        Action::PreviewSong => a.preview_song(),
        Action::DeleteSong => a.delete_song(),
        Action::RenameProject => a.begin_project_rename(),
        Action::OpenPatternTools => a.open_pattern_tools(),
        Action::OpenDrumPatterns => a.open_drum_patterns(),
        Action::NewPattern => a.new_pattern(),
        Action::ClearPattern => a.choose_pattern_clear(),
        Action::ClearPatternNow => {
            a.confirm_pattern_clear = false;
            a.clear_pattern_now();
        }
        Action::ClonePattern => a.clone_pattern(),
        Action::CopyPattern => a.copy_pattern(),
        Action::PastePatternNew => a.paste_pattern_new(),
        Action::PastePatternOver => a.paste_pattern_over(),
        Action::DeleteUnusedPattern => a.delete_unused_pattern(),
        Action::TransposeDownOctave => a.transpose_pattern(-12),
        Action::TransposeDownSemitone => a.transpose_pattern(-1),
        Action::TransposeUpSemitone => a.transpose_pattern(1),
        Action::TransposeUpOctave => a.transpose_pattern(12),
        Action::LoadDrumPattern => a.load_drum_pattern(),
        Action::SaveDrumPattern => a.save_drum_pattern(),
        Action::DeleteDrumPattern => a.delete_drum_pattern(),
        Action::DrumGenreDown => a.cycle_drum_genre(-1),
        Action::DrumGenreUp => a.cycle_drum_genre(1),
        Action::DrumMeter => a.toggle_drum_meter(),
        Action::DrumSize => a.cycle_drum_size(),
        Action::CopyLane => a.copy_lane(),
        Action::PasteLane => a.paste_lane(),
        Action::CopyPage => a.copy_page_block(),
        Action::PastePage => a.paste_page_block(),
        Action::ArrangementAppend => a.arrangement_append_current(),
        Action::ArrangementInsert => a.arrangement_insert_current(),
        Action::ArrangementRemove => a.arrangement_remove_step(),
        Action::ArrangementDuplicate => a.arrangement_duplicate_step(),
        Action::ArrangementMoveEarlier => a.arrangement_move_step(-1),
        Action::ArrangementMoveLater => a.arrangement_move_step(1),
        Action::ArrangementJumpToPattern => a.arrangement_jump_to_pattern(),
        Action::ArrangementPlayFromStep => a.arrangement_play_from_step(),
        Action::PreviousOrder => a.move_order(-1),
        Action::NextOrder => a.move_order(1),
        Action::TrackerEdit => {
            let enabled = a.tracker_mode != TrackerMode::Edit;
            a.set_tracker_edit(enabled);
            a.status = format!("step edit {}", if enabled { "on" } else { "off" });
        }
        Action::TrackerSkip => a.tracker_skip(),
        Action::TrackerErase => a.tracker_erase(),
        Action::TrackerNoteOff => a.tracker_note_off(),
        Action::TrackerAdvance1 => a.set_tracker_advance(1),
        Action::TrackerAdvance2 => a.set_tracker_advance(2),
        Action::TrackerAdvance4 => a.set_tracker_advance(4),
        Action::TrackerAdvance8 => a.set_tracker_advance(8),
        Action::OpenNoteEditor => a.open_note_editor(),
        Action::NoteField
        | Action::GateField
        | Action::VelocityField
        | Action::ProgramField
        | Action::EffectField
        | Action::EffectParameterField
        | Action::NoteEditorClearField
        | Action::NoteEditorPreviousField
        | Action::NoteEditorNextField
        | Action::NoteEditorDecrease
        | Action::NoteEditorIncrease
        | Action::NoteEditorConfirm
        | Action::NoteEditorCancel => {}
        Action::PreviousTrack => {
            if a.screen == Screen::TrackerPages {
                a.tracker_track = a.tracker_track.saturating_sub(1);
            } else {
                a.move_tracker_lane(-1);
            }
        }
        Action::NextTrack => {
            if a.screen == Screen::TrackerPages {
                a.tracker_track = (a.tracker_track + 1).min(LANES_PER_PAGE - 1);
            } else {
                a.move_tracker_lane(1);
            }
        }
        Action::PreviousProgram => a.change_program(-1),
        Action::NextProgram => a.change_program(1),
        Action::BankMsbDown => a.change_bank(true, -1),
        Action::BankMsbUp => a.change_bank(true, 1),
        Action::BankLsbDown => a.change_bank(false, -1),
        Action::BankLsbUp => a.change_bank(false, 1),
        Action::AddPage => a.add_tracker_page(),
        Action::EditPageTarget => a.edit_page_target(),
        Action::EditPageChannel => a.edit_page_channel(),
        Action::ConfirmPageManager => a.confirm_page_manager(),
        Action::SaveSong => a.save_song(),
        Action::SaveSongAs => a.save_song_as(),
        Action::LoadSong => a.load_song(),
        Action::NewProject => a.new_project(),
        Action::SelectThreeFour => {
            a.select_pattern_meter(3);
        }
        Action::SelectFourFour => {
            a.select_pattern_meter(4);
        }
        Action::PatternSizeDown => a.change_pattern_size(-1),
        Action::PatternSizeUp => a.change_pattern_size(1),
        Action::ConfirmPatternClear => a.apply_pattern_clear(),
        Action::AudioRecord => a.toggle_audio_recording(),
        Action::AudioStop => match a.audio_recorder.stop() {
            Ok(()) => a.status = "audio recording finalized".into(),
            Err(error) => a.status = format!("audio recorder: {error}"),
        },
        Action::FxAdd => a.add_effect(),
        Action::FxRemove => a.remove_effect(),
        Action::FxMoveUp => a.move_effect(-1),
        Action::FxMoveDown => a.move_effect(1),
        Action::FxBypass => a.toggle_effect_bypass(),
        Action::FxKindPrevious => a.cycle_effect_kind(-1),
        Action::FxKindNext => a.cycle_effect_kind(1),
        Action::FxTargetNext => a.cycle_fx_target(),
        Action::FxSendDecrease => a.adjust_aux_send(-1),
        Action::FxSendIncrease => a.adjust_aux_send(1),
        Action::FxSendPoint => a.toggle_aux_send_point(),
        Action::FxReturnCycle => a.cycle_aux_return(),
        Action::FxParameterPrevious => {
            let len = a
                .selected_effect()
                .map(|effect| crate::effect_schema::schema(effect.kind).len())
                .unwrap_or(0);
            a.fx_parameter = a.fx_parameter.saturating_sub(1).min(len.saturating_sub(1));
        }
        Action::FxParameterNext => {
            let len = a
                .selected_effect()
                .map(|effect| crate::effect_schema::schema(effect.kind).len())
                .unwrap_or(0);
            a.fx_parameter = (a.fx_parameter + 1).min(len.saturating_sub(1));
        }
        Action::FxValueDecrease => a.adjust_effect_parameter(-1),
        Action::FxValueIncrease => a.adjust_effect_parameter(1),
    }
    false
}
fn key(code: KeyCode, a: &mut App, state: &Path, tx: &std::sync::mpsc::Sender<MidiEvent>) -> bool {
    if a.project_name_input.is_some() {
        match code {
            KeyCode::Enter => a.commit_project_rename(),
            KeyCode::Esc => {
                a.project_name_input = None;
                a.status = "project naming cancelled".into();
            }
            KeyCode::Backspace => {
                if let Some(input) = a.project_name_input.as_mut() {
                    input.pop();
                }
            }
            KeyCode::Char(character)
                if !character.is_control()
                    && a.project_name_input
                        .as_ref()
                        .is_some_and(|input| input.chars().count() < 64) =>
            {
                if let Some(input) = a.project_name_input.as_mut() {
                    input.push(character);
                }
            }
            _ => {}
        }
        return false;
    }
    if a.screen == Screen::TrackerLoop && a.loop_library_mode {
        let action = match code {
            KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
            KeyCode::PageUp => Some(Action::PageUp),
            KeyCode::PageDown => Some(Action::PageDown),
            KeyCode::Delete | KeyCode::Char('d') | KeyCode::Char('D') => {
                Some(Action::DeleteLoopFile)
            }
            KeyCode::Esc | KeyCode::Char('b') => Some(Action::Back),
            _ => None,
        };
        if let Some(action) = action {
            return perform(action, a, state, Some(tx));
        }
        return false;
    }
    let confirmation_action = match (a.screen, &code) {
        (Screen::Ideas, KeyCode::Char('d')) => Action::DeleteIdea,
        (Screen::Tracker, KeyCode::Char('v'))
            if a.tracker_mode != TrackerMode::Edit
                && a.tracker_recording.is_none()
                && a.note_editor.is_none() =>
        {
            Action::SaveSong
        }
        (Screen::Tracker, KeyCode::Char('W'))
            if a.tracker_recording.is_none() && a.note_editor.is_none() =>
        {
            Action::PastePatternOver
        }
        _ => Action::Noop,
    };
    a.prepare_confirmation_action(confirmation_action);
    if matches!(code, KeyCode::F(1) | KeyCode::Char('?')) && a.screen != Screen::Help {
        perform(Action::OpenHelp, a, state, Some(tx));
        return false;
    }
    if a.screen == Screen::Help {
        let action = match code {
            KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
            KeyCode::PageUp => Some(Action::PageUp),
            KeyCode::PageDown => Some(Action::PageDown),
            KeyCode::Home => Some(Action::Home),
            KeyCode::End => Some(Action::End),
            KeyCode::Enter => Some(Action::Activate),
            KeyCode::Esc | KeyCode::Char('b') => Some(Action::Back),
            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Char(' ') => Some(Action::StopAll),
            _ => None,
        };
        if let Some(action) = action {
            perform(action, a, state, Some(tx));
        }
        return false;
    }
    if a.screen == Screen::TrackerLoopAlign {
        let action = match code {
            KeyCode::Left | KeyCode::Char('-') => Some(Action::LoopOffsetDown),
            KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=') => Some(Action::LoopOffsetUp),
            KeyCode::Char('a') | KeyCode::Char('A') => Some(Action::LoopAutoAlign),
            KeyCode::Enter => Some(Action::LoopAlignDone),
            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Char(' ') => {
                Some(Action::TrackerStop)
            }
            KeyCode::Esc | KeyCode::Char('b') => Some(Action::Back),
            _ => None,
        };
        if let Some(action) = action {
            perform(action, a, state, Some(tx));
        }
        return false;
    }
    if a.screen == Screen::TrackerLoop {
        let action = match code {
            KeyCode::Up => Some(Action::Up),
            KeyCode::Down => Some(Action::Down),
            KeyCode::Enter | KeyCode::Char('i') | KeyCode::Char('I') => Some(Action::LoopImport),
            KeyCode::Char('p') => Some(Action::TrackerPlayCursor),
            KeyCode::Char('P') => Some(Action::TrackerPlayStart),
            KeyCode::Char('a') | KeyCode::Char('A') => Some(Action::LoopAutoAlign),
            KeyCode::Char('-') => Some(Action::LoopSourceDown),
            KeyCode::Char('+') | KeyCode::Char('=') => Some(Action::LoopSourceUp),
            KeyCode::Char('x') | KeyCode::Char('X') => Some(Action::LoopBpmMode),
            KeyCode::Char('u') | KeyCode::Char('U') => Some(Action::LoopEditUnit),
            KeyCode::Char('[') => Some(Action::LoopStartDown),
            KeyCode::Char(']') => Some(Action::LoopStartUp),
            KeyCode::Char('{') => Some(Action::LoopLengthDown),
            KeyCode::Char('}') => Some(Action::LoopLengthUp),
            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Char(' ') => {
                Some(Action::TrackerStop)
            }
            KeyCode::Esc | KeyCode::Char('b') => Some(Action::Back),
            _ => None,
        };
        if let Some(action) = action {
            perform(action, a, state, Some(tx));
        }
        return false;
    }
    if a.screen == Screen::TrackerNoob {
        let action = match code {
            KeyCode::Left | KeyCode::Up | KeyCode::Char('-') => Some(Action::NoobRootDown),
            KeyCode::Right | KeyCode::Down | KeyCode::Char('+') | KeyCode::Char('=') => {
                Some(Action::NoobRootUp)
            }
            KeyCode::Tab | KeyCode::Char('m') | KeyCode::Char('M') => Some(Action::NoobScale),
            KeyCode::Enter => Some(Action::ConfirmNoob),
            KeyCode::Esc | KeyCode::Char('b') => Some(Action::Back),
            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Char(' ') => Some(Action::StopAll),
            _ => None,
        };
        if let Some(action) = action {
            perform(action, a, state, Some(tx));
        }
        return false;
    }
    if a.screen == Screen::TrackerArrange {
        let action = match code {
            KeyCode::Up => Some(Action::Up),
            KeyCode::Down => Some(Action::Down),
            KeyCode::Enter => Some(Action::ArrangementJumpToPattern),
            KeyCode::Char('p') | KeyCode::Char('P') => Some(Action::ArrangementPlayFromStep),
            KeyCode::Char('a') | KeyCode::Char('A') => Some(Action::ArrangementAppend),
            KeyCode::Char('i') | KeyCode::Char('I') => Some(Action::ArrangementInsert),
            KeyCode::Char('d') | KeyCode::Char('D') => Some(Action::ArrangementDuplicate),
            KeyCode::Delete | KeyCode::Backspace => Some(Action::ArrangementRemove),
            KeyCode::Char('<') => Some(Action::ArrangementMoveEarlier),
            KeyCode::Char('>') => Some(Action::ArrangementMoveLater),
            KeyCode::Esc | KeyCode::Char('b') => Some(Action::Back),
            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Char(' ') => Some(Action::StopAll),
            _ => None,
        };
        if let Some(action) = action {
            perform(action, a, state, Some(tx));
        }
        return false;
    }
    if a.screen == Screen::Tracker {
        if a.tracker_recording.is_some() {
            match code {
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    a.stop_tracker_recording();
                }
                KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Char(' ') => a.stop_all(state),
                KeyCode::Esc | KeyCode::Char('b') => {
                    perform(Action::Back, a, state, Some(tx));
                }
                _ => {}
            }
            return false;
        }
        if a.note_editor.is_some() {
            let action = match code {
                KeyCode::Up | KeyCode::Char('-') => Some(Action::NoteEditorDecrease),
                KeyCode::Down | KeyCode::Char('+') | KeyCode::Char('=') => {
                    Some(Action::NoteEditorIncrease)
                }
                KeyCode::Left => Some(Action::NoteEditorPreviousField),
                KeyCode::Right => Some(Action::NoteEditorNextField),
                KeyCode::Delete | KeyCode::Backspace => Some(Action::NoteEditorClearField),
                KeyCode::Enter => Some(Action::NoteEditorConfirm),
                KeyCode::Esc | KeyCode::Char('b') => Some(Action::NoteEditorCancel),
                KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Char(' ') => {
                    Some(Action::StopAll)
                }
                _ => None,
            };
            if let Some(action) = action {
                perform(action, a, state, Some(tx));
            }
            return false;
        }
        if a.tracker_mode == TrackerMode::Edit {
            let advance = match code {
                KeyCode::Char('1') => Some(1),
                KeyCode::Char('2') => Some(2),
                KeyCode::Char('4') => Some(4),
                KeyCode::Char('8') => Some(8),
                _ => None,
            };
            if let Some(advance) = advance {
                a.set_tracker_advance(advance);
                return false;
            }
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
                perform(Action::NextTrackerPage, a, state, Some(tx));
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
                a.tracker_note_off();
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
            KeyCode::Char('y') => {
                a.copy_lane();
                return false;
            }
            KeyCode::Char('u') => {
                a.paste_lane();
                return false;
            }
            KeyCode::Char('Y') => {
                a.copy_page_block();
                return false;
            }
            KeyCode::Char('U') => {
                a.paste_page_block();
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
            KeyCode::Char('r') | KeyCode::Char('R') => {
                a.begin_tracker_recording();
                return false;
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if let Some(column) = a.current_column_mut() {
                    column.program = (column.program + 1).min(127);
                }
                a.sync_tracker_route();
                return false;
            }
            KeyCode::Char('_') => {
                if let Some(column) = a.current_column_mut() {
                    column.program = column.program.saturating_sub(1);
                }
                a.sync_tracker_route();
                return false;
            }
            KeyCode::Char('<') => {
                a.set_tracker_tempo(a.current_tempo().saturating_sub(1));
                return false;
            }
            KeyCode::Char('>') => {
                a.set_tracker_tempo(a.current_tempo().saturating_add(1));
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
                a.clone_pattern();
                return false;
            }
            KeyCode::Char('B') => {
                a.copy_pattern();
                return false;
            }
            KeyCode::Char('V') => {
                a.paste_pattern_new();
                return false;
            }
            KeyCode::Char('W') => {
                a.paste_pattern_over();
                return false;
            }
            KeyCode::Char('X') => {
                perform(Action::OpenTrackerFiles, a, state, Some(tx));
                perform(Action::ClearPattern, a, state, Some(tx));
                return false;
            }
            KeyCode::Char('O') => {
                a.repeat_order();
                return false;
            }
            KeyCode::Backspace if a.song.order.len() > 1 => {
                a.delete_order();
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
            } else if a.screen == Screen::Presets {
                a.selected = a.selected.saturating_sub(1)
            } else {
                perform(Action::Up, a, state, Some(tx));
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if a.screen == Screen::Ideas {
                a.idea_selected = (a.idea_selected + 1).min(a.ideas.len().saturating_sub(1))
            } else if a.screen == Screen::Presets {
                a.selected = (a.selected + 1).min(a.presets.len().saturating_sub(1))
            } else {
                perform(Action::Down, a, state, Some(tx));
            }
        }
        KeyCode::PageUp => {
            perform(Action::PageUp, a, state, Some(tx));
        }
        KeyCode::PageDown => {
            perform(Action::PageDown, a, state, Some(tx));
        }
        KeyCode::Home => {
            perform(Action::Home, a, state, Some(tx));
        }
        KeyCode::End => {
            perform(Action::End, a, state, Some(tx));
        }
        KeyCode::Char('[') if a.screen == Screen::Presets => a.cycle_engine(-1),
        KeyCode::Char(']') if a.screen == Screen::Presets => a.cycle_engine(1),
        KeyCode::Enter => {
            if a.screen == Screen::Presets {
                a.load(state, tx.clone())
            } else if matches!(a.screen, Screen::TrackerFiles | Screen::TrackerPages) {
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
        KeyCode::Char('m') if a.screen == Screen::Presets => {
            perform(Action::OpenMeter, a, state, Some(tx));
        }
        KeyCode::Char('t') => {
            a.set_screen(Screen::Tracker);
            a.sync_tracker_route();
            a.status = a.sequencer.unavailable_label();
        }
        KeyCode::Char('a') => {
            if a.screen == Screen::Tracker {
                a.set_tracker_edit(false);
            }
            a.set_screen(Screen::AudioRecorder);
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

fn next_numbered_song_name(existing: &[String], prefix: &str) -> Option<String> {
    let prefix = sequencer::safe_name(prefix);
    (1..=9999).find_map(|number| {
        let suffix = format!("-{number:03}");
        let max_prefix = 64usize.saturating_sub(suffix.len());
        let name = format!(
            "{}{}",
            prefix.chars().take(max_prefix).collect::<String>(),
            suffix
        );
        (!existing.iter().any(|candidate| candidate == &name)).then_some(name)
    })
}

const fn pattern_sizes(beats: u8) -> [usize; 5] {
    if beats == 3 {
        [6, 12, 24, 48, 96]
    } else {
        [8, 16, 32, 64, 128]
    }
}

const fn drum_sizes(meter: u8) -> [usize; 3] {
    if meter == 3 {
        [24, 48, 96]
    } else {
        [32, 64, 128]
    }
}

fn nearest_pattern_rows(beats: u8, wanted: usize) -> usize {
    pattern_sizes(beats)
        .into_iter()
        .min_by_key(|rows| rows.abs_diff(wanted))
        .unwrap_or(32)
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
            a.prepare_confirmation_action(Action::Noop);
            if a.screen == Screen::Ideas {
                a.idea_selected = a.idea_selected.saturating_sub(1)
            } else if a.screen == Screen::Help {
                a.move_help(-3);
            } else if a.screen == Screen::Presets {
                a.selected = a.selected.saturating_sub(3)
            } else {
                perform(Action::Up, a, state, Some(tx));
            }
        }
        MouseEventKind::ScrollDown => {
            a.prepare_confirmation_action(Action::Noop);
            if a.screen == Screen::Ideas {
                a.idea_selected = (a.idea_selected + 1).min(a.ideas.len().saturating_sub(1))
            } else if a.screen == Screen::Help {
                a.move_help(3);
            } else if a.screen == Screen::Presets {
                a.selected = (a.selected + 3).min(a.presets.len().saturating_sub(1))
            } else {
                perform(Action::Down, a, state, Some(tx));
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if a.screen == Screen::Presets && contains(a.hits.list, m.column, m.row) {
                a.prepare_confirmation_action(Action::Noop);
                let i = visible_index(a.hits.list, a.offset, m.column, m.row).unwrap();
                if i < a.presets.len() {
                    if i == a.selected {
                        a.load(state, tx.clone())
                    } else {
                        a.selected = i;
                    }
                }
            } else if a.screen == Screen::Ideas && contains(a.hits.list, m.column, m.row) {
                a.prepare_confirmation_action(Action::Noop);
                let i = visible_index(a.hits.list, a.idea_offset, m.column, m.row).unwrap();
                if i < a.ideas.len() {
                    if i == a.idea_selected {
                        a.inspect_idea()
                    } else {
                        a.idea_selected = i;
                    }
                }
            } else if a.screen == Screen::Help && contains(a.hits.list, m.column, m.row) {
                a.prepare_confirmation_action(Action::Noop);
                let i = visible_index(a.hits.list, a.help_offset, m.column, m.row).unwrap();
                if i < help::lines(a.hits.list.width as usize).len() {
                    if i == a.help_selected {
                        a.activate_help();
                    } else {
                        a.help_selected = i;
                    }
                }
            } else if a.screen == Screen::TrackerFiles && contains(a.hits.list, m.column, m.row) {
                a.prepare_confirmation_action(Action::Noop);
                let filtered = a.filtered_drum_indices();
                let (selected, len) = if a.tracker_files_mode == TrackerFilesMode::Drums {
                    (
                        filtered
                            .iter()
                            .position(|index| *index == a.drum_pattern_selected)
                            .unwrap_or(0),
                        filtered.len(),
                    )
                } else {
                    (a.song_selected, a.song_list.len())
                };
                let offset = selected.saturating_sub(a.hits.list.height.saturating_sub(1) as usize);
                let i = visible_index(a.hits.list, offset, m.column, m.row).unwrap();
                if i < len {
                    if a.tracker_files_mode == TrackerFilesMode::Drums {
                        a.drum_pattern_selected = filtered[i];
                    } else {
                        a.song_selected = i;
                    }
                }
            } else if a.screen == Screen::TrackerLoop
                && a.loop_library_mode
                && contains(a.hits.list, m.column, m.row)
            {
                let rows = usize::from(a.hits.list.height.saturating_sub(2));
                let start = a.loop_library_selected.saturating_sub(rows / 2);
                let i = visible_index(a.hits.list, start, m.column, m.row).unwrap_or(0);
                if i < a.loop_library.len() {
                    a.loop_library_selected = i;
                }
            } else if let Some((_, page)) = a
                .hits
                .menu_pages
                .iter()
                .find(|(area, _)| contains(*area, m.column, m.row))
                .copied()
            {
                a.select_menu_page(page);
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
        Screen::Help => draw_help(f, a),
        Screen::Tracker => draw_tracker(f, a),
        Screen::TrackerFiles => draw_tracker_files(f, a),
        Screen::TrackerArrange => draw_tracker_arrange(f, a),
        Screen::TrackerPages => draw_tracker_pages(f, a),
        Screen::TrackerTools => draw_tracker_child(f, "FT2 TOOLS", "Pages · Files · Loop · Mute"),
        Screen::TrackerNoob => draw_noob_setup(f, a),
        Screen::TrackerLoop => draw_tracker_loop(f, a),
        Screen::TrackerLoopAlign => draw_tracker_loop_align(f, a),
        Screen::AudioRecorder => draw_audio_recorder(f, a),
        Screen::FxRack => draw_fx_rack(f, a),
        Screen::FxEditor => draw_fx_editor(f, a),
        Screen::Meter => draw_performance_meter(f, a),
    }
    draw_pad_lock(f, a);
    draw_pad_buttons(f, a);
    if a.screen != Screen::Playback {
        draw_status_bar(f, a);
    }
    if let Some(input) = a.project_name_input.as_deref() {
        let z = f.size();
        let area = rect(z.x + 2, z.y + 4, z.width.saturating_sub(4), 5);
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new(format!(
                "PROJECT NAME\n{input}_\nEnter confirm · Esc cancel"
            ))
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL)),
            area,
        );
    }
}

fn draw_fx_rack<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let body = rect(z.x, z.y, z.width, z.height.saturating_sub(4));
    a.hits.list = body;
    let rack = project_fx_rack(&a.song.insert_rack, &a.song.aux_routing, a.fx_target);
    let rack_length = rack.map(|rack| rack.order.len()).unwrap_or(0);
    let mut lines = vec![Spans::from(vec![
        Span::styled(
            format!("FX {}", fx_target_label(a.fx_target)),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  ADD: {}  {}/8",
            effect_kind_label(INSERT_EFFECTS[a.fx_add_kind]),
            rack_length
        )),
    ])];
    if is_aux_target(a.fx_target) {
        let aux_id = a.fx_target as u8;
        let send = a
            .song
            .aux_routing
            .sends
            .iter()
            .find(|send| send.aux_id == aux_id);
        let return_gain = a
            .song
            .aux_routing
            .buses
            .iter()
            .find(|bus| bus.id == aux_id)
            .map(|bus| bus.return_gain_db)
            .unwrap_or(0.0);
        lines.push(Spans::from(format!(
            "SEND {}  {}  RETURN {return_gain:.0} dB",
            send.map(|send| format!("{:.0} dB", send.level_db))
                .unwrap_or_else(|| "OFF".into()),
            send.map(|send| send_point_label(send.point))
                .unwrap_or("POST")
        )));
        if let Some(meter) = a
            .engine
            .as_ref()
            .and_then(|engine| engine.aux_meter(aux_id))
        {
            let peak = meter.output.peak.left.max(meter.output.peak.right);
            let rms = meter.output.rms.left.max(meter.output.rms.right);
            lines.push(Spans::from(format!(
                "RETURN pk {:>5.1} rms {:>5.1} dBFS",
                meter_db(peak),
                meter_db(rms)
            )));
        }
    } else if a.fx_target > MAX_AUX_BUSES {
        if let Some(meter) = a.engine.as_ref().and_then(Engine::master_meter) {
            let peak = meter.output.peak.left.max(meter.output.peak.right);
            let rms = meter.output.rms.left.max(meter.output.rms.right);
            lines.push(Spans::from(format!(
                "MASTER pk {:>5.1} rms {:>5.1} dBFS",
                meter_db(peak),
                meter_db(rms)
            )));
        }
    }
    if rack_length == 0 {
        lines.push(Spans::from("  Empty · choose KIND then ADD"));
    } else if let Some(rack) = rack {
        for (index, id) in rack.order.iter().copied().enumerate() {
            let effect = rack.effect(id).expect("validated rack order");
            let selected = index == a.fx_selected;
            let marker = if selected { ">" } else { " " };
            let state = if effect.bypass { "BYP" } else { "ON " };
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else if effect.bypass {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Spans::from(Span::styled(
                format!(
                    "{marker} {:>2}. {:<12} #{id:<3} {state}",
                    index + 1,
                    effect_kind_label(effect.kind)
                ),
                style,
            )));
        }
    }
    lines.push(Spans::from(""));
    lines.push(Spans::from("Structural edits require stopped transport"));
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL)),
        body,
    );
}

fn meter_db(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        (20.0 * value.log10()).max(-120.0)
    } else {
        -120.0
    }
}

fn performance_color(color: MeterColor) -> Color {
    match color {
        MeterColor::Green => Color::Green,
        MeterColor::Yellow => Color::LightYellow,
        MeterColor::Red => Color::Red,
    }
}

fn styled_meter_bar(cells: Vec<BarCell>) -> Vec<Span<'static>> {
    cells
        .into_iter()
        .map(|cell| {
            let active = cell.symbol != '·';
            Span::styled(
                cell.symbol.to_string(),
                Style::default()
                    .fg(performance_color(cell.color))
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            )
        })
        .collect()
}

fn cpu_meter_line(
    index: usize,
    value: Option<f32>,
    detected: bool,
    width: usize,
) -> Spans<'static> {
    let bar_width = width.saturating_sub(10).max(1);
    let mut spans = vec![Span::styled(
        format!("{index} ["),
        Style::default().fg(Color::Gray),
    )];
    spans.extend(styled_meter_bar(performance_meter::cpu_bar(
        bar_width,
        value.unwrap_or(0.0),
    )));
    spans.push(Span::styled(
        match value {
            Some(value) => format!("] {value:>3.0}%"),
            None if detected => "]  --%".into(),
            None => "]  n/a".into(),
        },
        Style::default().fg(if value.is_some() {
            Color::White
        } else {
            Color::DarkGray
        }),
    ));
    Spans::from(spans)
}

fn audio_meter_line(
    label: char,
    level: AudioLevel,
    numeric_peak_dbfs: f32,
    width: usize,
) -> Spans<'static> {
    let bar_width = width.saturating_sub(14).max(1);
    let mut spans = vec![Span::styled(
        format!("{label} ["),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )];
    spans.extend(styled_meter_bar(performance_meter::audio_bar(
        bar_width,
        level.rms_dbfs,
        level.peak_dbfs,
    )));
    spans.push(Span::styled(
        format!("] MAX {numeric_peak_dbfs:>5.1}"),
        Style::default().fg(performance_color(performance_meter::audio_color(
            numeric_peak_dbfs,
        ))),
    ));
    Spans::from(spans)
}

fn audio_scale_line(width: usize) -> String {
    let bar_width = width.saturating_sub(14).max(1);
    let mut chars = vec![' '; width];
    let start = 3;
    let right = "-12  -3  0";
    for (x, label) in [
        (start, "-60"),
        ((start + bar_width + 1).saturating_sub(right.len()), right),
    ] {
        let x = x.min(width.saturating_sub(label.len()));
        for (offset, character) in label.chars().enumerate() {
            if x + offset < chars.len() {
                chars[x + offset] = character;
            }
        }
    }
    chars.into_iter().collect()
}

const fn performance_audio_route(availability: AudioAvailability) -> &'static str {
    match availability {
        AudioAvailability::GraphActive => "Owned graph master",
        AudioAvailability::DirectUnavailable => "Direct · meter unavailable",
        AudioAvailability::Stopped => "Engine stopped · meter unavailable",
        AudioAvailability::Presentation => "Presentation · no live audio",
    }
}

fn draw_performance_meter<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let body = rect(z.x, z.y, z.width, z.height.saturating_sub(3));
    let inner = rect(
        body.x.saturating_add(1),
        body.y.saturating_add(1),
        body.width.saturating_sub(2),
        body.height.saturating_sub(2),
    );
    let width = usize::from(inner.width);
    let cpu_count = a.performance_meter.cpu_cores();
    let temperature = a.config.cpu_temperature_path.as_ref().map(|_| {
        a.cpu_temperature
            .map(|value| format!(" · {value:.0}°C"))
            .unwrap_or_else(|| " · --°C".into())
    });
    let cpu_title = if a.performance_meter.cpu_available() {
        format!(
            "CPU LOAD · {}/{} core{}{}",
            cpu_count.min(VISIBLE_CPU_CORES),
            cpu_count,
            if cpu_count == 1 { "" } else { "s" },
            temperature.unwrap_or_default()
        )
    } else {
        format!("CPU LOAD · unavailable{}", temperature.unwrap_or_default())
    };
    let mut lines = vec![Spans::from(Span::styled(
        truncate(&cpu_title, width),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ))];
    let detailed = inner.height >= 13;
    if detailed {
        lines.push(Spans::from(vec![
            Span::styled("green <60%  ", Style::default().fg(Color::Green)),
            Span::styled("yellow 60-85%  ", Style::default().fg(Color::LightYellow)),
            Span::styled("red >85%", Style::default().fg(Color::Red)),
        ]));
    }
    let loads = a.performance_meter.cpu_loads();
    for (index, value) in loads.into_iter().enumerate() {
        lines.push(cpu_meter_line(index, value, index < cpu_count, width));
    }
    if detailed {
        lines.push(Spans::from(""));
    }

    let availability = a.performance_meter.audio_availability();
    let clipping = a.performance_meter.clipping(Instant::now());
    let signal_fault = a.performance_meter.non_finite() > 0;
    let mut audio_heading = vec![Span::styled(
        "STEREO VU · FINAL OUT · dBFS",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )];
    if clipping {
        audio_heading.push(Span::styled(
            "  CLIP!",
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        ));
    } else if signal_fault {
        audio_heading.push(Span::styled(
            "  FAULT!",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Spans::from(audio_heading));
    if detailed {
        lines.push(Spans::from(Span::styled(
            audio_scale_line(width),
            Style::default().fg(Color::DarkGray),
        )));
    }
    let levels = if matches!(
        availability,
        AudioAvailability::GraphActive | AudioAvailability::Presentation
    ) {
        a.performance_meter.audio_levels()
    } else {
        [AudioLevel::default(); 2]
    };
    let numeric_peaks = a.performance_meter.numeric_peak_dbfs();
    lines.push(audio_meter_line('L', levels[0], numeric_peaks[0], width));
    lines.push(audio_meter_line('R', levels[1], numeric_peaks[1], width));
    let route = performance_audio_route(availability);
    lines.push(Spans::from(Span::styled(
        truncate(route, width),
        Style::default().fg(match availability {
            AudioAvailability::GraphActive => Color::Green,
            AudioAvailability::Presentation => Color::LightYellow,
            AudioAvailability::Stopped | AudioAvailability::DirectUnavailable => Color::DarkGray,
        }),
    )));
    if detailed {
        lines.push(Spans::from(vec![
            Span::styled("█ RMS smoothed  ", Style::default().fg(Color::Green)),
            Span::styled("│ short peak  ", Style::default().fg(Color::LightYellow)),
            Span::styled("· scale", Style::default().fg(Color::Red)),
        ]));
        lines.push(Spans::from(Span::styled(
            "MAX = highest peak since reset",
            Style::default().fg(Color::White),
        )));
    }
    if detailed && a.performance_meter.is_presentation() {
        lines.push(Spans::from(Span::styled(
            "Deterministic screenshot preview",
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.truncate(usize::from(inner.height));
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" MTR · PERFORMANCE ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        ),
        body,
    );
}

fn draw_fx_editor<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let body = rect(z.x, z.y, z.width, z.height.saturating_sub(4));
    let Some(id) = a.selected_effect_id() else {
        f.render_widget(
            Paragraph::new("FX EDIT\nNo effect selected")
                .block(Block::default().borders(Borders::ALL)),
            body,
        );
        return;
    };
    let effect = project_fx_rack(&a.song.insert_rack, &a.song.aux_routing, a.fx_target)
        .and_then(|rack| rack.effect(id))
        .expect("validated rack order");
    let schema = crate::effect_schema::schema(effect.kind);
    a.fx_parameter = a.fx_parameter.min(schema.len().saturating_sub(1));
    let visible_rows = body.height.saturating_sub(7) as usize;
    let offset = a
        .fx_parameter
        .saturating_sub(visible_rows.saturating_sub(1) / 2)
        .min(schema.len().saturating_sub(visible_rows));
    let mut lines = vec![Spans::from(vec![
        Span::styled(
            format!(
                "{} · {} #{id}",
                fx_target_label(a.fx_target),
                effect_kind_label(effect.kind)
            ),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(if effect.bypass {
            "  BYPASSED"
        } else {
            "  ACTIVE"
        }),
    ])];
    for (index, spec) in schema.iter().enumerate().skip(offset).take(visible_rows) {
        let value = effect
            .parameters
            .get(spec.name)
            .copied()
            .unwrap_or(spec.default);
        let style = if index == a.fx_parameter {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Spans::from(Span::styled(
            format!(
                "{} {:<22} {:>8.2} {}",
                if index == a.fx_parameter { ">" } else { " " },
                spec.name,
                value,
                spec.unit
            ),
            style,
        )));
    }
    let meter = a.engine.as_ref().and_then(|engine| engine.effect_meter(id));
    lines.push(Spans::from(""));
    if let Some(meter) = meter {
        let input_peak = meter.input.peak.left.max(meter.input.peak.right);
        let output_peak = meter.output.peak.left.max(meter.output.peak.right);
        let input_rms = meter.input.rms.left.max(meter.input.rms.right);
        let output_rms = meter.output.rms.left.max(meter.output.rms.right);
        lines.push(Spans::from(format!(
            "IN  pk {:>6.1} rms {:>6.1} dBFS",
            meter_db(input_peak),
            meter_db(input_rms)
        )));
        lines.push(Spans::from(format!(
            "OUT pk {:>6.1} rms {:>6.1} dBFS",
            meter_db(output_peak),
            meter_db(output_rms)
        )));
        lines.push(Spans::from(Span::styled(
            format!(
                "CLIP {}  NONFINITE {}{}",
                meter.output.clips,
                meter.output.non_finite,
                meter
                    .gain_reduction_db
                    .map(|value| format!("  GR {value:.1} dB"))
                    .unwrap_or_default()
            ),
            if meter.output.clips > 0 || meter.output.non_finite > 0 {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Green)
            },
        )));
    } else {
        lines.push(Spans::from("Meters unavailable · graph inactive"));
    }
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL)),
        body,
    );
}
fn draw_help<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let header = rect(z.x, z.y, z.width, 1);
    let help_width = z.width.min(HELP_TEXT_WIDTH as u16);
    let body = rect(
        z.x + z.width.saturating_sub(help_width) / 2,
        z.y + 1,
        help_width,
        z.height.saturating_sub(4),
    );
    let rows = body.height as usize;
    let lines = help::lines(HELP_TEXT_WIDTH);
    a.help_selected = a.help_selected.min(lines.len().saturating_sub(1));
    if a.help_selected < a.help_offset {
        a.help_offset = a.help_selected;
    } else if rows > 0 && a.help_selected >= a.help_offset + rows {
        a.help_offset = a.help_selected + 1 - rows;
    }
    a.hits.list = body;
    let web_status = if a.web_help_status.is_empty() {
        "web help unavailable"
    } else {
        &a.web_help_status
    };
    f.render_widget(
        Paragraph::new(truncate(web_status, z.width as usize)).style(
            Style::default()
                .fg(if a.web_help.is_some() {
                    Color::LightYellow
                } else {
                    Color::DarkGray
                })
                .add_modifier(Modifier::BOLD),
        ),
        header,
    );
    let visible = lines
        .iter()
        .enumerate()
        .skip(a.help_offset)
        .take(rows)
        .map(|(index, line)| {
            let style = if index == a.help_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                match line.kind {
                    HelpKind::Heading => Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                    HelpKind::Link => Style::default().fg(Color::Yellow),
                    HelpKind::Blank | HelpKind::Text => Style::default().fg(Color::Gray),
                }
            };
            Spans::from(Span::styled(
                truncate(&line.text, body.width as usize),
                style,
            ))
        })
        .collect::<Vec<_>>();
    f.render_widget(Paragraph::new(visible), body);
}
fn draw_tracker_child<B: Backend>(f: &mut Frame<B>, title: &str, details: &str) {
    let z = f.size();
    f.render_widget(
        Paragraph::new(format!("{title}\n\n{details}"))
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        rect(z.x, z.y + 1, z.width, z.height.saturating_sub(5)),
    );
}

fn draw_noob_setup<B: Backend>(f: &mut Frame<B>, a: &App) {
    let z = f.size();
    let root = crate::scale::note_name(a.noob_draft.root);
    f.render_widget(
        Paragraph::new(format!(
            "N00B MODE\n\nRoot  {root}\nScale {}\n\nNearest scale tone\nEqual ties map downward\n\nDONE enables safe input",
            a.noob_draft.kind.label()
        ))
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Green)),
        rect(z.x, z.y, z.width, z.height.saturating_sub(4)),
    );
}

fn draw_tracker_loop<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    if a.loop_library_mode {
        let body = rect(z.x, z.y, z.width, z.height.saturating_sub(4));
        a.hits.list = body;
        let rows = usize::from(body.height.saturating_sub(2));
        let start = a.loop_library_selected.saturating_sub(rows / 2);
        let lines = a
            .loop_library
            .iter()
            .enumerate()
            .skip(start)
            .take(rows)
            .map(|(index, entry)| {
                let marker = if entry.current {
                    "CURRENT"
                } else if entry.saved_references != 0 {
                    "SAVED"
                } else {
                    "FREE"
                };
                Spans::from(Span::styled(
                    truncate(
                        &format!(
                            "{} {:<24} {marker}",
                            if index == a.loop_library_selected {
                                "▶"
                            } else {
                                " "
                            },
                            entry.file
                        ),
                        usize::from(z.width.saturating_sub(2)),
                    ),
                    if index == a.loop_library_selected {
                        Style::default().fg(Color::Black).bg(Color::Yellow)
                    } else if marker == "FREE" {
                        Style::default()
                    } else {
                        Style::default().fg(Color::Green)
                    },
                ))
            })
            .collect::<Vec<_>>();
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .title(format!(" PRIVATE LOOPS · {} ", a.loop_library.len()))
                    .borders(Borders::ALL),
            ),
            body,
        );
        return;
    }
    let player = a.loop_player.status();
    let selected = a
        .loop_imports
        .get(a.loop_selected)
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "(inbox empty)".into());
    let details = if let Some(settings) = &a.song.audio_loop {
        let bar_unit = i32::from(a.current_meter().clamp(1, 16));
        let offset_bars = f64::from(settings.offset_beats) / f64::from(bar_unit);
        format!(
            "FT2 WAV LOOP\n{}\n\nSource {:>6.2} BPM  {}\nProject {:>3} BPM\nRegion beat {} +{}\nOffset {:+.0} bar(s)\nCut {} · meter {}/4\n\n{}  {} / {}\n{} Hz · {}ch\nNative pitch playback",
            truncate(
                player.file.as_deref().unwrap_or(&settings.file),
                z.width.saturating_sub(2) as usize
            ),
            settings.source_bpm(),
            settings.interpretation.label(),
            a.current_tempo(),
            settings.start_beat,
            settings.length_beats,
            offset_bars,
            if a.loop_edit_bars { "BAR" } else { "BEAT" },
            a.current_meter(),
            if player.playing { "PLAY" } else { "STOP" },
            short_time(player.elapsed),
            short_time(player.duration),
            player.source_rate,
            player.source_channels,
        )
    } else {
        format!(
            "FT2 WAV LOOP\n\nInbox: {}\nSelected: {}\n\nTurn encoder to choose\nIMPORT copies to private storage\n\nAUTO estimates beat length.\nProject tempo follows WAV.",
            a.config.loop_player.import_directory.display(),
            truncate(&selected, z.width.saturating_sub(2) as usize)
        )
    };
    f.render_widget(
        Paragraph::new(details)
            .alignment(Alignment::Center)
            .style(Style::default().fg(if player.error.is_some() {
                Color::Yellow
            } else {
                Color::Green
            })),
        rect(z.x, z.y, z.width, z.height.saturating_sub(4)),
    );
}

fn draw_tracker_loop_align<B: Backend>(f: &mut Frame<B>, a: &App) {
    let z = f.size();
    let details = if let Some(settings) = &a.song.audio_loop {
        let bar_unit = i32::from(a.current_meter().clamp(1, 16));
        let offset_bars = f64::from(settings.offset_beats) / f64::from(bar_unit);
        format!(
            "LOOP ALIGN\n{}\n\nAUTO measures pulse/length\nand snaps length to bars.\n\nBAR- / BAR+\nmove placement by 1 bar.\n\nLength: {} beat(s)\nOffset: {:+.0} bar(s)\nMeter: {}/4\n\nLeft/right also shift.",
            truncate(&settings.file, z.width.saturating_sub(2) as usize),
            settings.length_beats,
            offset_bars,
            a.current_meter(),
        )
    } else {
        "LOOP ALIGN\n\nImport a WAV first.\n\nAUTO measures a selected loop.\nBAR- / BAR+ move by bars.\n\nEXIT returns to loop.".into()
    };
    f.render_widget(
        Paragraph::new(details)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Green)),
        rect(z.x, z.y, z.width, z.height.saturating_sub(4)),
    );
}

fn short_time(duration: Duration) -> String {
    format!(
        "{:02}:{:02}",
        duration.as_secs() / 60,
        duration.as_secs() % 60
    )
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
// Screen layouts still calculate these rectangles for mouse hit tests. The
// visible controls are rendered by the
// canonical paged menu below.
fn button<B: Backend>(_f: &mut Frame<B>, _r: Rect, _label: &str) {}
fn pad_line(screen: Screen) -> String {
    format!("{} · controller menu below", screen.label())
}
fn draw_pad_buttons<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    if z.height < 4 {
        return;
    }
    a.hits.actions.clear();
    a.hits.menu_pages.clear();
    let pages = navigation::pages(a.screen, a.menu_context());
    // The hardware surface is four compact columns.  Do not turn a five-letter
    // label into a terminal-wide button on larger displays.
    let menu_width = z.width.min(40);
    let menu_x = z.x + z.width.saturating_sub(menu_width) / 2;
    let footer_rows = if a.screen == Screen::Playback { 2 } else { 3 };
    f.render_widget(
        Clear,
        rect(
            menu_x,
            z.y + z.height - footer_rows,
            menu_width,
            footer_rows.saturating_sub(1),
        ),
    );
    for (i, page) in pages.iter().enumerate() {
        let col = i as u16;
        let width = menu_width / 4;
        let x0 = menu_x + col * width;
        let r = rect(x0, z.y + z.height - footer_rows, width, 1);
        if !page.available() {
            continue;
        }
        let active = i == a.menu_page();
        let marker = if active && a.page_select_mode {
            ">"
        } else if active {
            "*"
        } else {
            " "
        };
        f.render_widget(
            Paragraph::new(format!(
                "{}{i}:{}",
                marker,
                truncate(page.label, usize::from(r.width.saturating_sub(3))),
                i = i + 1
            ))
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(if active {
                        Color::LightYellow
                    } else {
                        Color::DarkGray
                    })
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            r,
        );
        a.hits.menu_pages.push((r, i));
    }
    for (i, slot) in pages[a.menu_page()].slots.iter().enumerate() {
        let width = menu_width / 4;
        let x0 = menu_x + i as u16 * width;
        let r = rect(x0, z.y + z.height - footer_rows + 1, width, 1);
        if slot.state != SlotState::Enabled {
            continue;
        }
        let color = Color::Yellow;
        let text = truncate(slot.label, usize::from(r.width.saturating_sub(3)));
        f.render_widget(
            Paragraph::new(format!("[{text}]"))
                .alignment(Alignment::Center)
                .style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
            r,
        );
        if let Some(action) = slot.dispatch() {
            a.hits.actions.push((r, action));
        }
    }
}
fn draw_status_bar<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    if z.height == 0 {
        return;
    }
    let bpm = if a.screen == Screen::Meter {
        "-60..0 dBFS".into()
    } else if matches!(
        a.screen,
        Screen::Tracker
            | Screen::TrackerFiles
            | Screen::TrackerArrange
            | Screen::TrackerPages
            | Screen::TrackerTools
            | Screen::TrackerNoob
            | Screen::TrackerLoop
            | Screen::TrackerLoopAlign
    ) {
        format!("{} BPM", a.current_tempo())
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
    let left = format!(
        " {} P{} {engine} {rec}",
        a.screen.label(),
        a.menu_page() + 1
    );
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
    let actions = rect(z.x, z.y + z.height - 2, z.width, 2);
    let params = rect(z.x, z.y + 1, z.width, z.height.saturating_sub(3));
    let name = a
        .playing
        .as_ref()
        .map(|p| format!("{} · {}", p.backend.label(), p.name))
        .unwrap_or_else(|| "none".into());
    f.render_widget(
        Paragraph::new(truncate(&name, usize::from(z.width.saturating_sub(8))))
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        rect(header.x + 4, header.y, header.width.saturating_sub(8), 1),
    );
    let (mode, color) = if a.pad_locked {
        ("LCK", Color::Red)
    } else if a.recorder.is_recording() {
        ("REC", Color::Red)
    } else {
        ("PLY", Color::Green)
    };
    f.render_widget(
        Paragraph::new(mode).style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
        rect(z.x + z.width.saturating_sub(3), z.y, z.width.min(3), 1),
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
    let content_height = 4.min(chord_area.height);
    let top = chord_area.y + chord_area.height.saturating_sub(content_height) / 2;
    if let Some((chord, notes)) = a.held_notes.description(a.config.note_naming) {
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
    if chord_area.height >= 4 {
        draw_playback_keyboard(f, a, rect(chord_area.x, top + 2, chord_area.width, 2));
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
}

const PLAYBACK_KEY_TOP_GLYPH: &str = "└";
const PLAYBACK_KEYBOARD_FIRST_NOTE: u8 = 36;
const PLAYBACK_NATURAL_PITCHES: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
const PLAYBACK_SHARP_PITCHES: [Option<u8>; 7] =
    [Some(1), Some(3), None, Some(6), Some(8), Some(10), None];

fn draw_playback_keyboard<B: Backend>(f: &mut Frame<B>, a: &App, area: Rect) {
    if area.width == 0 || area.height < 2 {
        return;
    }
    for column in 0..area.width {
        let key = usize::from(column % 7);
        let octave = column / 7;
        let octave_base = u32::from(PLAYBACK_KEYBOARD_FIRST_NOTE) + u32::from(octave) * 12;
        let natural = octave_base + u32::from(PLAYBACK_NATURAL_PITCHES[key]);
        if natural > 127 {
            break;
        }
        let natural = natural as u8;
        let x = area.x + column;
        let natural_held = a.held_notes.is_held(natural);
        let natural_color = if natural_held {
            Color::Red
        } else {
            Color::White
        };
        let sharp = PLAYBACK_SHARP_PITCHES[key]
            .map(|pitch| octave_base + u32::from(pitch))
            .filter(|note| *note <= 127)
            .map(|note| note as u8);
        if let Some(sharp) = sharp {
            f.render_widget(
                Paragraph::new(PLAYBACK_KEY_TOP_GLYPH).style(
                    Style::default()
                        .fg(if a.held_notes.is_held(sharp) {
                            Color::Red
                        } else {
                            Color::Black
                        })
                        .bg(if natural_held {
                            Color::Red
                        } else {
                            Color::White
                        }),
                ),
                rect(x, area.y, 1, 1),
            );
        } else {
            f.render_widget(
                Paragraph::new("█").style(Style::default().fg(natural_color)),
                rect(x, area.y, 1, 1),
            );
        }
        f.render_widget(
            Paragraph::new("█").style(Style::default().fg(natural_color)),
            rect(x, area.y + 1, 1, 1),
        );
    }
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
    a.clamp_tracker_cursor();
    let transport = a.sequencer.status();
    let pattern_number = a.tracker_pattern_number();
    let Some(pattern) = a.current_pattern() else {
        return;
    };
    let Some(page) = pattern.pages.get(a.tracker_page) else {
        return;
    };
    let state = if a.note_editor.is_some() {
        "CELL".into()
    } else if a.tracker_recording.is_some() {
        "REC".into()
    } else if transport.playing {
        "PLAY".into()
    } else if a.tracker_mode == TrackerMode::Edit {
        format!("EDIT +{}", a.tracker_advance)
    } else if a.tracker_mode == TrackerMode::Noob {
        a.tracker_mode.label().into()
    } else {
        "STOP".into()
    };
    f.render_widget(
        Paragraph::new(truncate(
            &format!("{} · {} {state}", page.name, a.song.name),
            z.width as usize,
        ))
        .style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        rect(z.x, z.y, z.width, 1),
    );
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
    if a.note_editor
        .is_some_and(|editor| editor.field == NoteEditorField::Program)
    {
        draw_tracker_program_browser(f, a, grid);
    } else {
        let visible_tracks = LANES_PER_PAGE;
        let first_track = a.tracker_page * LANES_PER_PAGE;
        let row_width = 3u16;
        let column_width = grid.width.saturating_sub(row_width) / visible_tracks.max(1) as u16;
        let rows = grid.height.saturating_sub(1) as usize;
        let start = a.tracker_row.saturating_sub(rows / 2);
        let mut header = String::from("ROW");
        for (index, lane) in page.lanes.iter().enumerate() {
            let setup = page.column(index);
            let compact = format!(
                "{}:{:02}/{:03}",
                index + 1,
                setup.channel + 1,
                setup.program
            );
            header.push_str(&format!(
                "{:^w$}",
                truncate(
                    if column_width >= 8 {
                        &compact
                    } else {
                        &lane.name
                    },
                    usize::from(column_width)
                ),
                w = usize::from(column_width)
            ));
        }
        f.render_widget(
            Paragraph::new(header).style(Style::default().fg(Color::Yellow)),
            rect(grid.x, grid.y, grid.width, 1),
        );
        for (screen_row, row_index) in (start..(start + rows).min(pattern.rows.len())).enumerate() {
            let y = grid.y + 1 + screen_row as u16;
            let selected = row_index == a.tracker_row;
            let beat_stride = if [6, 12, 24, 48, 96].contains(&pattern.rows.len()) {
                (pattern.rows.len() / 4).max(1)
            } else {
                8.min(pattern.rows.len()).max(1)
            };
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
            for (track_index, stored_cell) in pattern.rows[row_index]
                .iter()
                .enumerate()
                .skip(first_track)
                .take(visible_tracks)
            {
                let cursor = selected && track_index == first_track + a.tracker_track;
                let cell = a
                    .note_editor
                    .filter(|editor| {
                        editor.pattern == pattern_number
                            && editor.row == row_index
                            && editor.lane == track_index
                    })
                    .map_or(*stored_cell, |editor| editor.draft);
                let velocity = cell.velocity.map_or("..".into(), |v| format!("{:02X}", v));
                let text = format!(
                    "{} {velocity}{} ",
                    sequencer::note_name(cell.note),
                    cell.command.marker()
                );
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
    }
    let lane = &page.lanes[a.tracker_track];
    let column = page.column(a.tracker_track);
    let footer = if let Some(editor) = a.note_editor {
        let command = match editor.draft.command {
            Command::None => "none".into(),
            Command::Cut(value) => format!("cut {value}/15"),
            Command::Delay(value) => format!("delay {value}/15"),
            Command::Retrigger(value) => format!("retrig {value}/8"),
            Command::Tempo(value) => format!("tempo {value}"),
        };
        let program = editor
            .draft
            .program
            .map_or_else(|| "inherit".into(), |value| a.tracker_program_label(value));
        format!(
            "{} · {} v{} g{} · {} · {command}",
            editor.field.label(),
            sequencer::note_name(editor.draft.note),
            editor
                .draft
                .velocity
                .map_or("inherit".into(), |value| value.to_string()),
            editor
                .draft
                .gate
                .map_or("inherit".into(), |value| format!("{value}%")),
            program,
        )
    } else {
        format!(
            "P{}/{} {} L{} ch{} {} {}",
            a.tracker_page + 1,
            pattern.pages.len(),
            page.name,
            a.tracker_track + 1,
            column.channel + 1,
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
        )
    };
    f.render_widget(
        Paragraph::new(truncate(&footer, z.width as usize))
            .style(Style::default().fg(Color::DarkGray)),
        rect(z.x, z.y + z.height.saturating_sub(4), z.width, 1),
    );
}

fn draw_tracker_program_browser<B: Backend>(f: &mut Frame<B>, a: &App, area: Rect) {
    let Some(page) = a.current_page() else {
        return;
    };
    let selected = a
        .note_editor
        .and_then(|editor| editor.draft.program)
        .unwrap_or(page.column(a.tracker_track).program);
    let title = a
        .tracker_device_profile()
        .map(DeviceProfile::label)
        .unwrap_or_else(|| "Unnamed MIDI device".into());
    let list_height = usize::from(area.height.saturating_sub(3));
    let before = list_height / 2;
    let start = usize::from(selected).saturating_sub(before);
    let end = (start + list_height).min(128);
    let mut lines = vec![
        Spans::from(Span::styled(
            truncate(&format!("PROGRAM · {title}"), usize::from(area.width)),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Spans::from(Span::styled(
            "turn to choose · play MIDI to audition",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    for program in start..end {
        let program = program as u8;
        let text = format!("{:03}  {}", program, a.tracker_program_label(program));
        lines.push(Spans::from(Span::styled(
            truncate(&text, usize::from(area.width)),
            if program == selected {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else {
                Style::default()
            },
        )));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_tracker_pages<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    a.clamp_tracker_cursor();
    let setup_label = a
        .current_column()
        .map(|column| {
            format!(
                "C{} ch{} b{}/{} {}",
                a.tracker_track + 1,
                column.channel + 1,
                column.bank_msb,
                column.bank_lsb,
                a.tracker_program_label(column.program)
            )
        })
        .unwrap_or_else(|| "no column".into());
    f.render_widget(
        Paragraph::new(truncate(
            &format!("FT2 TRACKS · {setup_label}"),
            usize::from(z.width),
        ))
        .style(
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
                .current_pages()
                .iter()
                .enumerate()
                .skip(start)
                .take(rows)
                .map(|(index, page)| {
                    let online = a.target_online(&page.target);
                    let text = format!(
                        "{}{:02} {} {:<6} C{} ch{:02} p{:03} {}",
                        if index == a.tracker_page { "▶" } else { " " },
                        index + 1,
                        if online { "ON" } else { "OFFLINE" },
                        truncate(&page.name, 6),
                        a.tracker_track + 1,
                        page.column(a.tracker_track).channel + 1,
                        page.column(a.tracker_track).program,
                        truncate(page.target.label(), 7),
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
                        .title(format!(" {} pages ", a.current_pages().len()))
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
                    Spans::from(format!("turn encoder · {}", a.page_field_confirm_hint())),
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
                    Spans::from(a.page_field_confirm_hint()),
                ])
                .block(Block::default().borders(Borders::ALL)),
                rect(z.x, z.y + 1, z.width, body_height),
            );
        }
    }
}

fn draw_tracker_arrange<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    a.arrange_selected = a.arrange_selected.min(a.song.order.len().saturating_sub(1));
    f.render_widget(
        Paragraph::new("FT2 ARRANGEMENT").style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        rect(z.x, z.y, z.width, 1),
    );
    let list = rect(z.x, z.y + 1, z.width, z.height.saturating_sub(5));
    let rows = list.height.saturating_sub(2) as usize;
    let offset = a.arrange_selected.saturating_sub(rows.saturating_sub(1));
    let lines = a
        .song
        .order
        .iter()
        .enumerate()
        .skip(offset)
        .take(rows)
        .map(|(index, pattern_number)| {
            let pattern = a.song.patterns.get(pattern_number);
            let detail = pattern.map_or_else(
                || "missing".into(),
                |pattern| {
                    format!(
                        "{:03} rows {} BPM {}/4 {}p",
                        pattern.rows.len(),
                        pattern.tempo,
                        pattern.meter,
                        pattern.pages.len()
                    )
                },
            );
            let text = format!(
                "{} {:02}  pat {:02}  {}",
                if index == a.arrange_selected {
                    ">"
                } else {
                    " "
                },
                index + 1,
                pattern_number,
                detail
            );
            Spans::from(Span::styled(
                truncate(&text, usize::from(z.width.saturating_sub(2))),
                if index == a.arrange_selected {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else {
                    Style::default()
                },
            ))
        })
        .collect::<Vec<_>>();
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(format!(" {} steps ", a.song.order.len()))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        ),
        list,
    );
    f.render_widget(
        Paragraph::new(truncate(&a.status, z.width as usize))
            .style(Style::default().fg(Color::DarkGray)),
        rect(z.x, z.y + z.height.saturating_sub(4), z.width, 1),
    );
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
        let lines = vec![
            Spans::from(if a.pattern_setup_new {
                "NEW PATTERN"
            } else {
                "CLEAR CURRENT PATTERN"
            }),
            Spans::from(""),
            Spans::from(format!("METER  ▶ {}/4", a.pattern_clear_beats)),
            Spans::from(format!("ROWS   ▶ {}", a.pattern_setup_rows)),
            Spans::from(""),
            Spans::from("Turn: size · buttons: meter/confirm"),
            Spans::from("EXIT cancels"),
        ];
        f.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL)),
            rect(z.x, z.y + 1, z.width, z.height.saturating_sub(5)),
        );
        return;
    }
    if a.tracker_files_mode == TrackerFilesMode::Patterns {
        let pattern = a.current_pattern();
        let melodic_notes = pattern.map_or(0, |pattern| {
            pattern
                .rows
                .iter()
                .map(|row| {
                    pattern
                        .pages
                        .iter()
                        .enumerate()
                        .filter(|(_, page)| !page.percussion)
                        .flat_map(|(page, _)| {
                            let start = page * LANES_PER_PAGE;
                            &row[start..start + LANES_PER_PAGE]
                        })
                        .filter(|cell| matches!(cell.note, Note::On(_)))
                        .count()
                })
                .sum()
        });
        let lines = vec![
            Spans::from(format!("PATTERN {}", a.tracker_pattern_number())),
            Spans::from(""),
            Spans::from(format!(
                "{}/4 · {} rows · {} page(s)",
                a.current_meter(),
                a.tracker_rows(),
                a.current_pages().len()
            )),
            Spans::from(format!("{melodic_notes} melodic notes")),
            Spans::from(""),
            Spans::from("DRUMS opens reusable rhythm files"),
            Spans::from("TRANS changes melody, never drums"),
        ];
        f.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL)),
            rect(z.x, z.y + 1, z.width, z.height.saturating_sub(5)),
        );
        f.render_widget(
            Paragraph::new(truncate(&a.status, z.width as usize))
                .style(Style::default().fg(Color::DarkGray)),
            rect(z.x, z.y + z.height.saturating_sub(4), z.width, 1),
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
    let (selected, names, title) = if a.tracker_files_mode == TrackerFilesMode::Drums {
        let filtered = a.filtered_drum_indices();
        let genre = a.drum_genre();
        (
            filtered
                .iter()
                .position(|index| *index == a.drum_pattern_selected)
                .unwrap_or(0),
            filtered
                .iter()
                .map(|index| &a.drum_patterns[*index])
                .map(|entry| {
                    format!(
                        "{}{}{}",
                        if genre == "ALL" {
                            format!("{} · ", entry.genre)
                        } else {
                            String::new()
                        },
                        entry.name,
                        if entry.user { " · USER" } else { "" }
                    )
                })
                .collect::<Vec<_>>(),
            format!(
                " {} · {}/4 · {} · {} ",
                genre,
                a.drum_meter,
                a.drum_target_rows,
                filtered.len()
            ),
        )
    } else {
        (
            a.song_selected,
            a.song_list.clone(),
            format!(" saved songs · {} ", a.song_list.len()),
        )
    };
    let offset = selected.saturating_sub(rows.saturating_sub(1));
    let lines = names
        .iter()
        .enumerate()
        .skip(offset)
        .take(rows)
        .map(|(index, name)| {
            let selected = index == selected;
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
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        ),
        list,
    );
    f.render_widget(
        Paragraph::new(truncate(&a.status, z.width as usize)).style(Style::default().fg(
            if a.confirm_song_delete.is_some()
                || a.confirm_pattern_clear
                || a.confirm_drum_pattern_delete.is_some()
            {
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

#[derive(Serialize)]
struct ScreenshotSet {
    cols: u16,
    rows: u16,
    screens: Vec<ScreenshotFrame>,
}

#[derive(Serialize)]
struct ScreenshotFrame {
    name: String,
    cells: Vec<ScreenshotCell>,
}

#[derive(Serialize)]
struct ScreenshotCell {
    symbol: String,
    fg: [u8; 3],
    bg: [u8; 3],
    bold: bool,
}

pub fn readme_screenshots_json(config: &RuntimeConfig) -> Result<String> {
    let readme_screens = [
        ("shr-daw-presets.png", ScreenshotScenario::Presets),
        ("shr-daw-playback.png", ScreenshotScenario::Playback),
        ("shr-daw-ft2-pattern.png", ScreenshotScenario::TrackerEdit),
        (
            "shr-daw-ft2-arrangement.png",
            ScreenshotScenario::TrackerArrange,
        ),
        ("shr-daw-ft2-pages.png", ScreenshotScenario::TrackerPages),
        (
            "shr-daw-project-files.png",
            ScreenshotScenario::TrackerFiles,
        ),
        (
            "shr-daw-drum-patterns.png",
            ScreenshotScenario::DrumPatterns,
        ),
        ("shr-daw-ft2-loop.png", ScreenshotScenario::TrackerLoop),
        (
            "shr-daw-audio-recorder.png",
            ScreenshotScenario::AudioRecorder,
        ),
        ("shr-daw-performance-meter.png", ScreenshotScenario::Meter),
    ];
    let mut frames = Vec::new();
    for (name, scenario) in readme_screens {
        let mut app = screenshot_app(config.clone());
        configure_screenshot_scenario(&mut app, scenario);
        frames.push(render_screenshot_frame(&mut app, name.into())?);
    }
    for scenario in ScreenshotScenario::ALL {
        let mut app = screenshot_app(config.clone());
        configure_screenshot_scenario(&mut app, scenario);
        let context = app.menu_context();
        for (page_index, page) in navigation::pages(app.screen, context).iter().enumerate() {
            if !page.available() {
                continue;
            }
            app.select_menu_page(page_index);
            let name = format!(
                "menu/{}-{}.png",
                scenario.slug(),
                screenshot_name_slug(page.label)
            );
            frames.push(render_screenshot_frame(&mut app, name)?);
        }
    }
    Ok(serde_json::to_string_pretty(&ScreenshotSet {
        cols: 40,
        rows: 20,
        screens: frames,
    })?)
}

fn render_screenshot_frame(app: &mut App, name: String) -> Result<ScreenshotFrame> {
    let backend = TestBackend::new(40, 20);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| draw(frame, app))?;
    let cells = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| ScreenshotCell {
            symbol: cell.symbol.clone(),
            fg: color_rgb(cell.fg, true),
            bg: color_rgb(cell.bg, false),
            bold: cell.modifier.contains(Modifier::BOLD),
        })
        .collect();
    Ok(ScreenshotFrame { name, cells })
}

fn screenshot_name_slug(label: &str) -> String {
    let mut slug = String::new();
    for character in label.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_owned()
}

#[derive(Clone, Copy, Debug)]
enum ScreenshotScenario {
    Presets,
    Playback,
    Ideas,
    Help,
    TrackerPlay,
    TrackerEdit,
    TrackerRecord,
    TrackerCellEdit,
    TrackerFiles,
    PatternTools,
    DrumPatterns,
    PatternSetup,
    TrackerArrange,
    TrackerPages,
    PageTarget,
    PageChannel,
    TrackerTools,
    TrackerNoob,
    TrackerLoop,
    LoopLibrary,
    TrackerLoopAlign,
    AudioRecorder,
    FxRack,
    FxEditor,
    Meter,
}

impl ScreenshotScenario {
    const ALL: [Self; 25] = [
        Self::Presets,
        Self::Playback,
        Self::Ideas,
        Self::Help,
        Self::TrackerPlay,
        Self::TrackerEdit,
        Self::TrackerRecord,
        Self::TrackerCellEdit,
        Self::TrackerFiles,
        Self::PatternTools,
        Self::DrumPatterns,
        Self::PatternSetup,
        Self::TrackerArrange,
        Self::TrackerPages,
        Self::PageTarget,
        Self::PageChannel,
        Self::TrackerTools,
        Self::TrackerNoob,
        Self::TrackerLoop,
        Self::LoopLibrary,
        Self::TrackerLoopAlign,
        Self::AudioRecorder,
        Self::FxRack,
        Self::FxEditor,
        Self::Meter,
    ];

    const fn screen(self) -> Screen {
        match self {
            Self::Presets => Screen::Presets,
            Self::Playback => Screen::Playback,
            Self::Ideas => Screen::Ideas,
            Self::Help => Screen::Help,
            Self::TrackerPlay | Self::TrackerEdit | Self::TrackerRecord | Self::TrackerCellEdit => {
                Screen::Tracker
            }
            Self::TrackerFiles | Self::PatternTools | Self::DrumPatterns | Self::PatternSetup => {
                Screen::TrackerFiles
            }
            Self::TrackerArrange => Screen::TrackerArrange,
            Self::TrackerPages | Self::PageTarget | Self::PageChannel => Screen::TrackerPages,
            Self::TrackerTools => Screen::TrackerTools,
            Self::TrackerNoob => Screen::TrackerNoob,
            Self::TrackerLoop | Self::LoopLibrary => Screen::TrackerLoop,
            Self::TrackerLoopAlign => Screen::TrackerLoopAlign,
            Self::AudioRecorder => Screen::AudioRecorder,
            Self::FxRack => Screen::FxRack,
            Self::FxEditor => Screen::FxEditor,
            Self::Meter => Screen::Meter,
        }
    }

    const fn slug(self) -> &'static str {
        match self {
            Self::Presets => "presets",
            Self::Playback => "playback",
            Self::Ideas => "ideas",
            Self::Help => "help",
            Self::TrackerPlay => "ft2-play",
            Self::TrackerEdit => "ft2-step-edit",
            Self::TrackerRecord => "ft2-record",
            Self::TrackerCellEdit => "ft2-cell-edit",
            Self::TrackerFiles => "files",
            Self::PatternTools => "pattern-tools",
            Self::DrumPatterns => "drum-patterns",
            Self::PatternSetup => "pattern-setup",
            Self::TrackerArrange => "arrange",
            Self::TrackerPages => "tracks",
            Self::PageTarget => "target-editor",
            Self::PageChannel => "channel-editor",
            Self::TrackerTools => "ft2-tools",
            Self::TrackerNoob => "noob-setup",
            Self::TrackerLoop => "ft2-loop",
            Self::LoopLibrary => "loop-library",
            Self::TrackerLoopAlign => "loop-align",
            Self::AudioRecorder => "audio-recorder",
            Self::FxRack => "fx-rack",
            Self::FxEditor => "fx-editor",
            Self::Meter => "performance-meter",
        }
    }
}

fn screenshot_app(mut config: RuntimeConfig) -> App {
    config.cpu_temperature_path = None;
    config.capture.inputs = vec![crate::config::StereoInputConfig {
        name: "AudioBox USB 96".into(),
        left_port: "system:capture_1".into(),
        right_port: "system:capture_2".into(),
    }];
    let catalogs = [Catalog {
        backend: BackendKind::Synthv1,
        presets: [
            "Velvet Tines",
            "Hollow Brass",
            "Soft Fifths",
            "Juniper Lead",
            "Dust Pad",
            "Square Bass",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, name)| Preset::synthv1(name, format!("demo-{index}.synthv1").into()))
        .collect(),
        unavailable: None,
    }];
    let mut app = App::new(
        &catalogs,
        Arc::new(std::sync::Mutex::new(None)),
        Arc::new(std::sync::Mutex::new(crate::midi::Pickup::default())),
        Arc::new(std::sync::Mutex::new(BackendKind::Synthv1)),
        Arc::new(std::sync::Mutex::new(engine::TrackerRoute::default())),
        Arc::new(std::sync::Mutex::new(None)),
        config,
    );
    app.web_help_enabled = false;
    app.playing = app.presets.first().cloned();
    app.status = "Ready".into();
    app.song.name = "dusk-project".into();
    app
}

fn configure_screenshot(app: &mut App, screen: Screen) {
    app.screen = screen;
    app.select_menu_page(0);
    match screen {
        Screen::Presets => {
            app.selected = 0;
            app.status = "MIDI ready · pickup armed".into();
        }
        Screen::Playback => {
            for note in [62, 66, 69] {
                app.held_notes.observe(&[0x90, note, 100]);
            }
            let originals = [
                0.72, 0.28, 0.34, 0.46, 0.82, 0.18, 0.42, 0.31, 0.03, 0.36, 0.68, 0.22,
            ];
            let values = [
                0.64, 0.28, 0.46, 0.46, 0.78, 0.25, 0.42, 0.20, 0.03, 0.31, 0.68, 0.29,
            ];
            app.original_values = CONTROLS
                .iter()
                .zip(originals)
                .map(|(control, value)| (control.cc, value))
                .collect();
            app.values = CONTROLS
                .iter()
                .zip(values)
                .map(|(control, value)| (control.cc, value))
                .collect();
            app.status = "Playback to review".into();
        }
        Screen::Tracker => {
            fill_demo_song(app);
            app.tracker_mode = TrackerMode::Edit;
            app.status = "step edit on".into();
        }
        Screen::TrackerArrange => {
            fill_demo_song(app);
            app.arrange_selected = 0;
            app.status = "FT2 arrangement · chain pattern steps".into();
        }
        Screen::TrackerPages => {
            fill_demo_song(app);
            app.tracker_page = 0;
            app.status = "page route updated · DONE to keep".into();
        }
        Screen::TrackerFiles => {
            fill_demo_song(app);
            app.song_list = vec![
                "dusk-project".into(),
                "sunday-sketch".into(),
                "mt240-drums".into(),
                "d50-pad-study".into(),
                "live-set-a".into(),
            ];
            app.status = "Project files · select an action".into();
        }
        Screen::TrackerLoop => {
            fill_demo_song(app);
            let settings = sequencer::LoopSettings {
                file: "breakbeat-96.wav".into(),
                source_bpm_x100: 9_600,
                interpretation: sequencer::BpmInterpretation::Normal,
                start_beat: 0,
                length_beats: 16,
                offset_beats: 0,
            };
            app.song.audio_loop = Some(settings);
            if let Some(pattern) = app.current_pattern_mut() {
                pattern.tempo = 96;
            }
            app.loop_edit_bars = true;
            app.loop_player
                .set_preview_status(crate::loop_player::LoopStatus {
                    loaded: true,
                    playing: false,
                    file: Some("breakbeat-96.wav".into()),
                    source_rate: 48_000,
                    source_channels: 2,
                    duration: Duration::from_secs(8),
                    elapsed: Duration::from_secs(3),
                    error: None,
                });
        }
        Screen::AudioRecorder => {
            app.audio_recorder.set_preview_status(RecorderStatus {
                recording: false,
                elapsed: Duration::from_secs(134),
                bytes: 36_800_000,
                sample_rate: 48_000,
                dropped_frames: 0,
                path: Some(PathBuf::from("recordings/dusk-project-001.wav")),
                error: None,
            });
        }
        Screen::Meter => {
            app.performance_meter.seed_presentation(
                [Some(18.0), Some(43.0), Some(67.0), Some(91.0)],
                [
                    AudioLevel {
                        rms_dbfs: -16.8,
                        peak_dbfs: -5.4,
                    },
                    AudioLevel {
                        rms_dbfs: -12.6,
                        peak_dbfs: -1.8,
                    },
                ],
                [-2.7, -0.6],
                Instant::now(),
            );
            app.status = "passive meter · RESET clears held maxima".into();
        }
        _ => {}
    }
}

fn configure_screenshot_scenario(app: &mut App, scenario: ScreenshotScenario) {
    configure_screenshot(app, scenario.screen());
    match scenario {
        ScreenshotScenario::Presets
        | ScreenshotScenario::Playback
        | ScreenshotScenario::TrackerArrange
        | ScreenshotScenario::TrackerPages
        | ScreenshotScenario::TrackerFiles
        | ScreenshotScenario::TrackerLoop
        | ScreenshotScenario::AudioRecorder
        | ScreenshotScenario::Meter => {}
        ScreenshotScenario::Ideas => {
            app.ideas = vec![
                "2026-07-19-dusk-chords".into(),
                "2026-07-19-bass-answer".into(),
                "2026-07-18-d50-cloud".into(),
                "2026-07-18-break-variation".into(),
                "2026-07-17-copper-pluck".into(),
            ];
            app.idea_selected = 1;
            app.status = "Compact Bass · 18.4 s · ready to inspect".into();
        }
        ScreenshotScenario::Help => {
            app.web_help_status = "Local help · LAN page not started for screenshot".into();
            app.help_selected = 2;
            app.help_offset = 0;
            app.status = "turn to move · OPEN follows section links".into();
        }
        ScreenshotScenario::TrackerPlay => {
            fill_demo_song(app);
            app.tracker_mode = TrackerMode::Play;
            app.status = "PLAY mode · encoder moves rows".into();
        }
        ScreenshotScenario::TrackerEdit => {
            fill_demo_song(app);
            app.tracker_mode = TrackerMode::Edit;
            app.tracker_row = 4;
            app.tracker_track = 1;
            app.status = "step edit · ADD 2 rows after entry".into();
        }
        ScreenshotScenario::TrackerRecord => {
            fill_demo_song(app);
            app.tracker_mode = TrackerMode::Rec;
            app.tracker_row = 7;
            app.tracker_recording = Some(TrackerRecording {
                pattern: 0,
                order: 0,
                page: 0,
                last_row: 7,
                next_lane: 2,
                active_lanes: HashMap::new(),
                notes: 11,
            });
            app.status = "REC pattern 0 · EXT only · 11 notes".into();
        }
        ScreenshotScenario::TrackerCellEdit => {
            fill_demo_song(app);
            app.tracker_mode = TrackerMode::Edit;
            app.tracker_row = 4;
            app.tracker_track = 0;
            app.open_note_editor();
            if let Some(editor) = app.note_editor.as_mut() {
                editor.field = NoteEditorField::EffectParameter;
                editor.draft.gate = Some(75);
                editor.draft.command = Command::Retrigger(3);
            }
            app.status = "CELL EDIT · PARAM selected · draft not committed".into();
        }
        ScreenshotScenario::PatternTools => {
            fill_demo_song(app);
            app.open_pattern_tools();
            app.status = "pattern 0 · 12 melodic notes · clipboard ready".into();
        }
        ScreenshotScenario::DrumPatterns => {
            fill_demo_song(app);
            app.open_pattern_tools();
            app.open_drum_patterns();
        }
        ScreenshotScenario::PatternSetup => {
            fill_demo_song(app);
            app.open_pattern_tools();
            app.confirm_pattern_clear = true;
            app.pattern_setup_new = true;
            app.pattern_clear_beats = 3;
            app.pattern_setup_rows = 48;
            app.status = "new pattern setup · confirm creates it".into();
        }
        ScreenshotScenario::PageTarget => {
            fill_demo_song(app);
            configure_demo_page_editor(app);
            app.page_manager_mode = PageManagerMode::Target;
            app.page_target_selected = 1;
            app.status = "target editor · Roland D-50 is online".into();
        }
        ScreenshotScenario::PageChannel => {
            fill_demo_song(app);
            configure_demo_page_editor(app);
            app.page_manager_mode = PageManagerMode::Channel;
            app.page_channel_draft = 9;
            app.status = "channel editor · draft channel 10".into();
        }
        ScreenshotScenario::TrackerTools => {
            fill_demo_song(app);
            app.status = "choose a focused FT2 tool".into();
        }
        ScreenshotScenario::TrackerNoob => {
            fill_demo_song(app);
            app.noob_draft = Scale {
                root: 4,
                kind: ScaleKind::NaturalMinor,
            };
            app.status = "E natural minor · DONE enables note mapping".into();
        }
        ScreenshotScenario::LoopLibrary => {
            configure_demo_loop(app);
            app.loop_library_mode = true;
            app.loop_library = vec![
                crate::loop_player::LibraryEntry {
                    file: "breakbeat-96.wav".into(),
                    current: true,
                    saved_references: 2,
                },
                crate::loop_player::LibraryEntry {
                    file: "tape-drums-92.wav".into(),
                    current: false,
                    saved_references: 1,
                },
                crate::loop_player::LibraryEntry {
                    file: "room-pulse-120.wav".into(),
                    current: false,
                    saved_references: 0,
                },
                crate::loop_player::LibraryEntry {
                    file: "odd-percussion-135.wav".into(),
                    current: false,
                    saved_references: 0,
                },
            ];
            app.loop_library_selected = 2;
            app.status = "FREE loops can be deleted after confirmation".into();
        }
        ScreenshotScenario::TrackerLoopAlign => {
            configure_demo_loop(app);
            app.screen = Screen::TrackerLoopAlign;
            app.status = "AUTO measured 4 bars · offset +1 bar".into();
            if let Some(settings) = app.song.audio_loop.as_mut() {
                settings.offset_beats = 4;
            }
        }
        ScreenshotScenario::FxRack => {
            configure_demo_fx(app);
            app.screen = Screen::FxRack;
        }
        ScreenshotScenario::FxEditor => {
            configure_demo_fx(app);
            app.screen = Screen::FxEditor;
            app.fx_selected = 1;
            app.fx_parameter = 2;
            app.status = "COMPRESSOR · ratio selected · graph inactive".into();
        }
    }
    app.select_menu_page(0);
}

fn configure_demo_page_editor(app: &mut App) {
    app.page_manager_original = Some(app.song.clone());
    app.available_page_outputs = vec![
        "USB MIDI Interface".into(),
        "Roland D-50".into(),
        "Elektron Model:Cycles".into(),
    ];
    app.page_target_candidates = vec![
        PageTarget::ConfiguredExternal,
        PageTarget::Midi("Roland D-50".into()),
        PageTarget::Midi("Elektron Model:Cycles".into()),
        PageTarget::ActiveInstrument,
    ];
}

fn configure_demo_loop(app: &mut App) {
    configure_screenshot(app, Screen::TrackerLoop);
}

fn configure_demo_fx(app: &mut App) {
    fill_demo_song(app);
    app.fx_target = 0;
    for kind in [0, 1, 2, 10] {
        app.fx_add_kind = kind;
        app.add_effect();
    }
    app.fx_selected = 1;
    app.fx_add_kind = 3;
    app.status = "SOURCE · four active inserts · transport stopped".into();
}

fn fill_demo_song(app: &mut App) {
    let config = &app.config.external_midi;
    let mut song = Song::new(config);
    song.name = "dusk-project".into();
    song.order = vec![0, 1, 0, 2];
    if let Some(pattern) = song.patterns.get_mut(&0) {
        pattern.tempo = 120;
        pattern.meter = 4;
        pattern.pages[0].target = PageTarget::ConfiguredExternal;
        pattern.rows[0][0] = demo_cell(60, 0x60, Command::Delay(0));
        pattern.rows[0][1] = demo_cell(64, 0x58, Command::None);
        pattern.rows[0][2] = demo_cell(67, 0x5a, Command::None);
        pattern.rows[2][0] = demo_cell(72, 0x70, Command::None);
        pattern.rows[2][2] = Cell {
            note: Note::Off,
            ..Cell::default()
        };
        pattern.rows[4][0] = demo_cell(62, 0x62, Command::Tempo(120));
        pattern.rows[4][1] = demo_cell(65, 0x50, Command::None);
        pattern.rows[4][2] = demo_cell(69, 0x55, Command::None);
        pattern.rows[7][0] = demo_cell(55, 0x6a, Command::None);
        pattern.rows[8][0] = demo_cell(60, 0x60, Command::None);
        pattern.rows[8][1] = demo_cell(64, 0x60, Command::None);
        pattern.rows[8][2] = demo_cell(67, 0x60, Command::None);
        pattern.rows[8][3] = demo_cell(71, 0x60, Command::None);
        let mut d50 = sequencer::Page::new("D-50", 2, false, 0);
        d50.target = PageTarget::Midi("Roland D-50".into());
        pattern.pages.push(d50);
        for row in &mut pattern.rows {
            row.extend(std::iter::repeat(Cell::default()).take(LANES_PER_PAGE));
        }
    }
    if let Some(setup) = song.patterns.get(&0).cloned() {
        let mut second = sequencer::Pattern::empty_like_setup(32, &setup);
        second.tempo = 92;
        song.patterns.insert(1, second);
        let mut third = sequencer::Pattern::empty_like_setup(24, &setup);
        third.tempo = 135;
        third.meter = 3;
        third.pages.truncate(1);
        for row in &mut third.rows {
            row.truncate(LANES_PER_PAGE);
        }
        song.patterns.insert(2, third);
    }
    app.song = song;
    app.tracker_order = 0;
    app.tracker_row = 0;
    app.tracker_page = 0;
    app.tracker_track = 0;
}

fn demo_cell(note: u8, velocity: u8, command: Command) -> Cell {
    Cell {
        note: Note::On(note),
        velocity: Some(velocity),
        command,
        ..Cell::default()
    }
}

fn color_rgb(color: Color, foreground: bool) -> [u8; 3] {
    match color {
        Color::Reset => {
            if foreground {
                [170, 170, 170]
            } else {
                [0, 0, 0]
            }
        }
        Color::Black => [0, 0, 0],
        Color::Red => [170, 0, 0],
        Color::Green => [0, 170, 0],
        Color::Yellow => [170, 85, 0],
        Color::Blue => [0, 0, 170],
        Color::Magenta => [170, 0, 170],
        Color::Cyan => [0, 170, 170],
        Color::Gray => [170, 170, 170],
        Color::DarkGray => [85, 85, 85],
        Color::LightRed => [255, 85, 85],
        Color::LightGreen => [85, 255, 85],
        Color::LightYellow => [255, 255, 85],
        Color::LightBlue => [85, 85, 255],
        Color::LightMagenta => [255, 85, 255],
        Color::LightCyan => [85, 255, 255],
        Color::White => [255, 255, 255],
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Indexed(index) => ansi_indexed_color(index),
    }
}

fn ansi_indexed_color(index: u8) -> [u8; 3] {
    const ANSI16: [[u8; 3]; 16] = [
        [0, 0, 0],
        [170, 0, 0],
        [0, 170, 0],
        [170, 85, 0],
        [0, 0, 170],
        [170, 0, 170],
        [0, 170, 170],
        [170, 170, 170],
        [85, 85, 85],
        [255, 85, 85],
        [85, 255, 85],
        [255, 255, 85],
        [85, 85, 255],
        [255, 85, 255],
        [85, 255, 255],
        [255, 255, 255],
    ];
    if index < 16 {
        ANSI16[usize::from(index)]
    } else if index < 232 {
        let value = index - 16;
        let channel = |component| {
            if component == 0 {
                0
            } else {
                55 + component * 40
            }
        };
        [
            channel(value / 36),
            channel((value / 6) % 6),
            channel(value % 6),
        ]
    } else {
        let gray = 8 + (index - 232) * 10;
        [gray, gray, gray]
    }
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
        let mut app = App::new(
            &catalogs,
            Arc::new(std::sync::Mutex::new(None)),
            Arc::new(std::sync::Mutex::new(crate::midi::Pickup::default())),
            Arc::new(std::sync::Mutex::new(BackendKind::Synthv1)),
            Arc::new(std::sync::Mutex::new(engine::TrackerRoute::default())),
            Arc::new(std::sync::Mutex::new(None)),
            RuntimeConfig::default(),
        );
        app.web_help_enabled = false;
        app
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
        render(40, 20, Screen::Help);
        render(40, 20, Screen::Tracker);
        render(40, 20, Screen::TrackerFiles);
        render(40, 20, Screen::TrackerArrange);
        render(40, 20, Screen::TrackerPages);
        render(40, 20, Screen::TrackerTools);
        render(40, 20, Screen::TrackerNoob);
        render(40, 20, Screen::TrackerLoop);
        render(40, 20, Screen::TrackerLoopAlign);
        render(40, 20, Screen::AudioRecorder);
        render(40, 20, Screen::FxRack);
        render(40, 20, Screen::FxEditor);
        render(40, 20, Screen::Meter);
    }

    #[test]
    fn meter_renders_readable_deterministic_40x20_regions_and_honest_labels() {
        let p = presets();
        let mut a = app(&p);
        configure_screenshot(&mut a, Screen::Meter);
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        for expected in [
            "MTR · PERFORMANCE",
            "CPU LOAD",
            "STEREO VU",
            "FINAL OUT",
            "dBFS",
            "MAX  -2.7",
            "MAX  -0.6",
            "MAX = highest peak since reset",
            "Presentation · no live audio",
        ] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
        for color in [Color::Green, Color::LightYellow, Color::Red] {
            assert!(buffer.content.iter().any(|cell| cell.fg == color));
        }
    }

    #[test]
    fn meter_controller_entry_and_exit_return_exactly_to_presets() {
        let p = presets();
        let mut a = app(&p);
        let open = navigation::slot(Screen::Presets, MenuContext::Normal, 2, 0)
            .and_then(|slot| slot.dispatch())
            .unwrap();
        assert_eq!(open, Action::OpenMeter);
        assert!(!perform(open, &mut a, Path::new("/none"), None));
        assert_eq!(a.screen, Screen::Meter);
        assert_eq!(
            navigation::pages(a.screen, a.menu_context())[0].label,
            "OPS"
        );
        let exit = navigation::slot(a.screen, a.menu_context(), 3, 3)
            .and_then(|slot| slot.dispatch())
            .unwrap();
        assert_eq!(exit, Action::Back);
        assert!(!perform(exit, &mut a, Path::new("/none"), None));
        assert_eq!(a.screen, Screen::Presets);
    }

    #[test]
    fn meter_compacts_without_losing_cpu_or_stereo_bars_at_38x14() {
        let p = presets();
        let mut a = app(&p);
        configure_screenshot(&mut a, Screen::Meter);
        let backend = TestBackend::new(38, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        for expected in ["CPU LOAD", "0 [", "3 [", "STEREO VU", "L [", "R [", "MAX"] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
    }

    #[test]
    fn meter_labels_direct_output_unavailable_without_inventing_levels() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Meter;
        a.performance_meter
            .set_audio_unavailable(AudioAvailability::DirectUnavailable);
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        assert!(text.contains("Direct · meter unavailable"));
        assert!(!text.contains("Owned graph master"));
    }

    #[test]
    fn meter_route_labels_fit_the_40_column_inner_row_without_truncation() {
        for availability in [
            AudioAvailability::GraphActive,
            AudioAvailability::DirectUnavailable,
            AudioAvailability::Stopped,
            AudioAvailability::Presentation,
        ] {
            assert!(performance_audio_route(availability).chars().count() <= 38);
        }
    }

    fn seed_numeric_meter_peaks(app: &mut App) {
        app.performance_meter.update_audio(
            crate::dsp::MeterSnapshot {
                peak: crate::dsp::StereoFrame::new(0.8, 0.6),
                ..crate::dsp::MeterSnapshot::default()
            },
            Instant::now(),
        );
        assert!(app.performance_meter.numeric_peak_dbfs()[0] > -2.0);
    }

    #[test]
    fn mapped_volume_down_clears_maxima_even_when_pickup_blocks_the_value() {
        let p = presets();
        let mut a = app(&p);
        a.values.insert(VOLUME_CC, 0.5);
        seed_numeric_meter_peaks(&mut a);
        let (tx, rx) = mpsc::channel();
        tx.send(MidiEvent::MappedControl(VOLUME_CC, 0.8)).unwrap();
        tx.send(MidiEvent::MappedControl(VOLUME_CC, 0.7)).unwrap();

        drain(&rx, &mut a, Path::new("/none"), &tx);

        assert_eq!(a.values.get(&VOLUME_CC), Some(&0.5));
        assert_eq!(
            a.performance_meter.numeric_peak_dbfs(),
            [
                performance_meter::AUDIO_FLOOR_DBFS,
                performance_meter::AUDIO_FLOOR_DBFS,
            ]
        );
    }

    #[test]
    fn accepted_volume_decrease_clears_maxima() {
        let p = presets();
        let mut a = app(&p);
        a.values.insert(VOLUME_CC, 0.8);
        seed_numeric_meter_peaks(&mut a);

        a.apply_control_value(VOLUME_CC, 0.7);

        assert_eq!(a.values.get(&VOLUME_CC), Some(&0.7));
        assert_eq!(
            a.performance_meter.numeric_peak_dbfs(),
            [
                performance_meter::AUDIO_FLOOR_DBFS,
                performance_meter::AUDIO_FLOOR_DBFS,
            ]
        );
    }

    #[test]
    fn manual_meter_reset_clears_numeric_maxima() {
        let p = presets();
        let mut a = app(&p);
        seed_numeric_meter_peaks(&mut a);

        assert!(!perform(
            Action::ResetMeter,
            &mut a,
            Path::new("/none"),
            None
        ));

        assert_eq!(
            a.performance_meter.numeric_peak_dbfs(),
            [
                performance_meter::AUDIO_FLOOR_DBFS,
                performance_meter::AUDIO_FLOOR_DBFS,
            ]
        );
    }

    #[test]
    fn loading_a_new_preset_session_clears_numeric_maxima() {
        let p = presets();
        let mut a = app(&p);
        seed_numeric_meter_peaks(&mut a);

        a.commit_loaded_preset(p[0].clone(), HashMap::new(), HashMap::new());

        assert_eq!(
            a.performance_meter.numeric_peak_dbfs(),
            [
                performance_meter::AUDIO_FLOOR_DBFS,
                performance_meter::AUDIO_FLOOR_DBFS,
            ]
        );
    }

    #[test]
    fn volume_increase_equal_value_and_unrelated_controls_keep_maxima() {
        let p = presets();
        let mut a = app(&p);
        a.values.insert(VOLUME_CC, 0.5);
        seed_numeric_meter_peaks(&mut a);
        let expected = a.performance_meter.numeric_peak_dbfs();

        a.observe_mapped_control(VOLUME_CC, 0.4);
        a.observe_mapped_control(VOLUME_CC, 0.4);
        a.observe_mapped_control(VOLUME_CC, 0.6);
        a.observe_mapped_control(74, 0.1);
        a.apply_control_value(VOLUME_CC, 0.5);
        a.apply_control_value(VOLUME_CC, 0.7);
        a.apply_control_value(74, 0.0);

        assert_eq!(a.performance_meter.numeric_peak_dbfs(), expected);
    }

    #[test]
    fn owned_graph_route_changes_require_stopped_transport_and_recording() {
        let p = presets();
        let mut a = app(&p);
        assert_eq!(a.audio_graph_edit_blocker(), None);
        a.config.audio_graph.enabled = true;
        a.recorder.start(Instant::now());
        assert_eq!(
            a.audio_graph_edit_blocker(),
            Some("stop recording before changing the insert rack")
        );
        a.recorder.stop();
        a.song_previewing = true;
        assert_eq!(
            a.audio_graph_edit_blocker(),
            Some("stop transport before changing the insert rack")
        );
    }
    #[test]
    fn renders_smaller_and_tiny_gracefully() {
        render(38, 14, Screen::Presets);
        render(38, 14, Screen::Playback);
        render(38, 14, Screen::Ideas);
        render(38, 14, Screen::Help);
        render(38, 14, Screen::Tracker);
        render(38, 14, Screen::TrackerFiles);
        render(38, 14, Screen::TrackerArrange);
        render(38, 14, Screen::TrackerPages);
        render(38, 14, Screen::TrackerTools);
        render(38, 14, Screen::TrackerNoob);
        render(38, 14, Screen::TrackerLoop);
        render(38, 14, Screen::TrackerLoopAlign);
        render(38, 14, Screen::AudioRecorder);
        render(38, 14, Screen::FxRack);
        render(38, 14, Screen::FxEditor);
        render(38, 14, Screen::Meter);
        render(30, 8, Screen::Presets);
        render(30, 8, Screen::Tracker)
    }

    #[test]
    fn fx_rack_actions_preserve_ids_order_bypass_and_strict_parameters() {
        let p = presets();
        let mut a = app(&p);
        a.fx_add_kind = 0;
        a.add_effect();
        let eq = a.selected_effect_id().unwrap();
        a.fx_add_kind = 1;
        a.add_effect();
        let compressor = a.selected_effect_id().unwrap();
        assert_eq!(a.song.insert_rack.order, [eq, compressor]);
        a.move_effect(-1);
        assert_eq!(a.song.insert_rack.order, [compressor, eq]);
        a.toggle_effect_bypass();
        assert!(a.song.insert_rack.effect(compressor).unwrap().bypass);
        a.fx_parameter = 0;
        a.adjust_effect_parameter(-1);
        let effect = a.song.insert_rack.effect(compressor).unwrap();
        assert!(effect.parameters["threshold_db"].is_finite());
        a.song.insert_rack.validate().unwrap();
    }

    #[test]
    fn fx_aux_workflow_forces_wet_and_keeps_independent_send_return_state() {
        let p = presets();
        let mut a = app(&p);
        a.fx_add_kind = 0;
        a.add_effect();
        let source_id = a.selected_effect_id().unwrap();
        a.cycle_fx_target();
        a.fx_add_kind = FIRST_AUX_EFFECT_INDEX;
        a.add_effect();
        let first_aux_effect = a.selected_effect_id().unwrap();
        assert!(first_aux_effect > source_id);
        let bus = &a.song.aux_routing.buses[0];
        let delay = bus.rack.effect(first_aux_effect).unwrap();
        assert_eq!(delay.parameters["dry_percent"], 0.0);
        assert_eq!(delay.parameters["wet_percent"], 100.0);
        assert_eq!(a.song.aux_routing.sends[0].level_db, -18.0);
        a.adjust_aux_send(1);
        a.toggle_aux_send_point();
        a.cycle_aux_return();
        assert_eq!(a.song.aux_routing.sends[0].level_db, -15.0);
        assert_eq!(a.song.aux_routing.sends[0].point, SendPoint::PreInsert);
        assert_eq!(a.song.aux_routing.buses[0].return_gain_db, -3.0);

        a.cycle_fx_target();
        a.fx_add_kind = 8;
        a.add_effect();
        assert_eq!(a.song.aux_routing.buses.len(), 2);
        assert_ne!(
            a.song.aux_routing.sends[0].level_db,
            a.song.aux_routing.sends[1].level_db
        );
        let second_aux_effect = a.selected_effect_id().unwrap();
        a.cycle_fx_target();
        a.fx_add_kind = 0;
        a.add_effect();
        let master_effect = a.selected_effect_id().unwrap();
        assert!(master_effect > second_aux_effect);
        assert_eq!(a.song.aux_routing.master_rack.order, [master_effect]);
        a.song.aux_routing.validate(&a.song.insert_rack).unwrap();
    }

    #[test]
    fn forty_by_twenty_shows_screen_all_pages_and_four_current_items() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Playback;
        a.select_menu_page(1);
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        for label in ["OPS", "SOUND", "NAV", "SYS", "RESET", "FINISH", "TAP", "FX"] {
            assert!(text.contains(label), "missing {label}: {text}");
        }
        assert!(text.contains("PLY"));
        assert_eq!(a.hits.menu_pages.len(), 4);
        assert_eq!(a.hits.actions.len(), 4);
    }

    #[test]
    fn eight_five_and_four_control_layouts_select_pages_without_losing_encoder_navigation() {
        let p = presets();
        let (tx, rx) = mpsc::channel();
        let mut eight = app(&p);
        eight.screen = Screen::Tracker;
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Page1, true))
            .unwrap();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item4, true))
            .unwrap();
        drain(&rx, &mut eight, Path::new("/none"), &tx);
        assert_eq!(
            eight.menu_page(),
            0,
            "edit context resets predictably to page one"
        );
        assert!(eight.note_editor.is_some());

        let mut five = app(&p);
        five.screen = Screen::Tracker;
        five.controller_layout = ControllerLayout::Five;
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item4, true))
            .unwrap();
        drain(&rx, &mut five, Path::new("/none"), &tx);
        assert!(five.note_editor.is_some());

        let mut four = app(&p);
        four.screen = Screen::Tracker;
        four.controller_layout = ControllerLayout::Four;
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Select))
            .unwrap();
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Down))
            .unwrap();
        drain(&rx, &mut four, Path::new("/none"), &tx);
        assert!(four.page_select_mode);
        assert_eq!(four.menu_page(), 1);
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Select))
            .unwrap();
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Down))
            .unwrap();
        drain(&rx, &mut four, Path::new("/none"), &tx);
        assert!(!four.page_select_mode);
        assert_eq!(
            four.tracker_row, 1,
            "normal encoder operation remains available"
        );
    }

    #[test]
    fn all_item_buttons_use_the_selected_screen_menu_and_pages_are_remembered() {
        let p = presets();
        let mut a = app(&p);
        let (tx, rx) = mpsc::channel();
        a.screen = Screen::Tracker;
        a.select_menu_page(2);
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item4, true))
            .unwrap();
        drain(&rx, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.tracker_track, 1);
        perform(Action::OpenIdeas, &mut a, Path::new("/none"), None);
        assert_eq!(a.menu_page(), 0);
        perform(Action::OpenTracker, &mut a, Path::new("/none"), None);
        assert_eq!(a.menu_page(), 2, "each screen remembers its page");
    }
    #[test]
    fn help_opens_links_and_returns_to_previous_screen() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;

        perform(Action::OpenHelp, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Help);
        assert_eq!(a.help_previous, Screen::Tracker);
        assert!(a.status.contains("HELP"));
        assert_eq!(a.web_help_status, "web help unavailable");

        a.help_selected = help::lines(38)
            .iter()
            .position(|line| line.target.as_deref() == Some("ft2-tracker"))
            .unwrap();
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        let lines = help::lines(38);
        assert_eq!(
            lines[a.help_selected].anchor.as_deref(),
            Some("ft2-tracker")
        );

        perform(Action::Back, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Tracker);
        assert!(a.web_help_status.is_empty());

        a.open_note_editor();
        perform(Action::OpenHelp, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Help);
        assert!(a.note_editor.is_some());
    }

    #[test]
    fn four_button_help_can_select_sys_and_exit() {
        let p = presets();
        let mut a = app(&p);
        a.controller_layout = ControllerLayout::Four;
        a.set_screen(Screen::Tracker);
        a.open_help();
        let (tx, rx) = mpsc::channel();

        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Select))
            .unwrap();
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Down))
            .unwrap();
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Select))
            .unwrap();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item4, true))
            .unwrap();
        drain(&rx, &mut a, Path::new("/none"), &tx);

        assert_eq!(a.screen, Screen::Tracker);
        assert_eq!(a.menu_page_by_screen[Screen::Help.index()], 3);
        assert!(!a.page_select_mode);
    }

    #[test]
    fn changing_screens_clears_four_button_page_selection_mode() {
        let p = presets();
        let mut a = app(&p);
        a.controller_layout = ControllerLayout::Four;
        a.set_screen(Screen::Playback);
        a.page_select_mode = true;

        perform(Action::OpenIdeas, &mut a, Path::new("/none"), None);

        assert_eq!(a.screen, Screen::Ideas);
        assert!(!a.page_select_mode);
    }

    #[test]
    fn help_render_uses_the_same_width_as_link_navigation() {
        let p = presets();
        let mut a = app(&p);
        a.set_screen(Screen::Help);
        let lines = help::lines(HELP_TEXT_WIDTH);
        a.help_selected = lines
            .iter()
            .position(|line| line.target.as_deref() == Some("ft2-tracker"))
            .unwrap();
        a.activate_help();
        let expected = help::lines(HELP_TEXT_WIDTH)[a.help_selected].text.clone();
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
        let row = a.hits.list.y + (a.help_selected - a.help_offset) as u16;
        let rendered = (a.hits.list.x..a.hits.list.right())
            .map(|x| terminal.backend().buffer().get(x, row).symbol.as_str())
            .collect::<String>();

        assert_eq!(a.hits.list.width, HELP_TEXT_WIDTH as u16);
        assert_eq!(rendered.trim_end(), expected);
    }

    #[test]
    fn tracker_record_exit_stops_capture_without_leaving_tracker() {
        let p = presets();
        let mut a = app(&p);
        a.set_screen(Screen::Tracker);
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.begin_tracker_recording();
        assert!(a.tracker_recording.is_some());

        perform(Action::Back, &mut a, Path::new("/none"), None);

        assert_eq!(a.screen, Screen::Tracker);
        assert!(a.tracker_recording.is_none());
        assert_eq!(a.tracker_mode, TrackerMode::Play);
    }

    #[test]
    fn opening_help_stops_tracker_recording_before_disabling_its_route() {
        let p = presets();
        let mut a = app(&p);
        a.set_screen(Screen::Tracker);
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.begin_tracker_recording();

        a.open_help();

        assert_eq!(a.screen, Screen::Help);
        assert!(a.tracker_recording.is_none());
        assert_eq!(a.tracker_mode, TrackerMode::Play);
    }

    #[test]
    fn cell_editor_stop_preserves_the_draft() {
        let p = presets();
        let mut a = app(&p);
        a.set_screen(Screen::Tracker);
        a.open_note_editor();
        a.adjust_note_editor(1);
        let draft = a.note_editor.unwrap().draft;

        perform(Action::TrackerStop, &mut a, Path::new("/none"), None);

        assert_eq!(a.note_editor.unwrap().draft, draft);
        assert_eq!(a.screen, Screen::Tracker);
    }

    #[test]
    fn empty_controller_items_are_silent() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::AudioRecorder;
        a.select_menu_page(0);
        let before = a.status.clone();
        let (tx, rx) = mpsc::channel();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item2, true))
            .unwrap();
        drain(&rx, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.status, before);
        assert!(a.playing.is_none());
        a.screen = Screen::Help;
        a.select_menu_page(1);
        let before = a.status.clone();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item2, true))
            .unwrap();
        drain(&rx, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.status, before);
        assert!(!a.audio_recorder.status().recording);
    }
    #[test]
    fn tracker_renders_both_page_names_and_compact_stop_label() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.song.patterns.get_mut(&0).unwrap().tempo = 137;
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
        assert_eq!(a.current_pages().len(), 3);
        assert_eq!(a.current_pages()[2].lanes.len(), 4);

        perform(Action::EditPageTarget, &mut a, Path::new("/none"), None);
        assert_eq!(a.page_manager_mode, PageManagerMode::Target);
        perform(Action::Down, &mut a, Path::new("/none"), None);
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert_eq!(a.page_manager_mode, PageManagerMode::Pages);

        perform(Action::EditPageChannel, &mut a, Path::new("/none"), None);
        perform(Action::Down, &mut a, Path::new("/none"), None);
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert_eq!(a.current_pages()[2].column(0).channel, 1);

        perform(Action::PreviousTrack, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_page, 2);
        assert_eq!(a.tracker_track, 0);
        perform(Action::NextTrack, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_page, 2);
        assert_eq!(a.tracker_track, 1);
        perform(Action::ConfirmPageManager, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::TrackerPages);
        assert!(a.status.contains("conflict"));
        a.current_page_mut().unwrap().column_mut(0).program = 9;
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
        a.current_page_mut().unwrap().target = PageTarget::Midi("UNPLUGGED DEVICE".into());
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
        assert!(text.contains("OPS"));
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
    fn wheel_and_page_pad_button_clicks_work_at_40x20() {
        let p = presets();
        let mut a = app(&p);
        a.selected = 5;
        let b = TestBackend::new(40, 20);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        assert_eq!(a.hits.actions.len(), 4);
        assert_eq!(a.hits.menu_pages.len(), 4);
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
                column: 12,
                row: 18,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            &mut a,
            Path::new("/nonexistent"),
            &tx,
        );
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn controller_strip_stays_compact_on_wide_terminals() {
        let p = presets();
        let mut a = app(&p);
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();

        assert_eq!(a.hits.menu_pages.len(), 4);
        assert!(a
            .hits
            .menu_pages
            .iter()
            .all(|(area, _)| area.width == 10 && area.x >= 20 && area.x < 60));
        assert!(a
            .hits
            .actions
            .iter()
            .all(|(area, _)| area.width == 10 && area.x >= 20 && area.x < 60));
    }

    #[test]
    fn midi_exit_closes_contexts_but_never_quits_the_app() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.open_note_editor();
        let exit = navigation::slot(a.screen, a.menu_context(), 3, 3)
            .and_then(|slot| slot.dispatch())
            .unwrap();
        assert!(!perform(exit, &mut a, Path::new("/none"), None));
        assert!(a.note_editor.is_none());

        a.screen = Screen::TrackerFiles;
        a.confirm_pattern_clear = true;
        let exit = navigation::slot(a.screen, a.menu_context(), 3, 3)
            .and_then(|slot| slot.dispatch())
            .unwrap();
        assert!(!perform(exit, &mut a, Path::new("/none"), None));
        assert!(!a.confirm_pattern_clear);

        assert!(navigation::pages(Screen::ALL[0], MenuContext::Normal)
            .iter()
            .flat_map(|page| page.slots)
            .all(|slot| slot.dispatch() != Some(Action::Quit)));
    }
    #[test]
    fn playback_shows_all_parameters_centered_title_and_compact_mode() {
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
        assert!(!text.contains("BPM"));
        assert!(!text.contains("CPU 52°C"));
        let title = (0..40)
            .map(|x| b.get(x, 0).symbol.as_str())
            .collect::<String>();
        assert!(title.trim_start().starts_with("synthv1 · Preset 00"));
        assert!(title.ends_with("PLY"));
        let left = title
            .chars()
            .position(|character| character == 's')
            .unwrap();
        let right = 40 - left - "synthv1 · Preset 00".chars().count();
        assert!(left.abs_diff(right) <= 1);
        assert!((37..40).all(|x| b.get(x, 0).fg == Color::Green));
        let buttons = (18..20)
            .flat_map(|y| (0..40).map(move |x| b.get(x, y).symbol.as_str()))
            .collect::<String>();
        assert!(buttons.contains('['));
        assert!(!buttons.contains(&a.status));

        a.recorder.start(Instant::now());
        t.draw(|f| draw(f, &mut a)).unwrap();
        let b = t.backend().buffer();
        assert_eq!(
            (37..40)
                .map(|x| b.get(x, 0).symbol.as_str())
                .collect::<String>(),
            "REC"
        );
        assert!((37..40).all(|x| b.get(x, 0).fg == Color::Red));
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
    fn playback_keyboard_joins_octaves_and_separates_natural_and_sharp_colors() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Playback;
        a.playing = Some(p[0].clone());
        // C, E, and G exercise natural keys; F# exercises a sharp without F.
        for note in [60, 64, 66, 67] {
            a.held_notes.observe(&[0x90, note, 100]);
        }
        let b = TestBackend::new(40, 20);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let b = t.backend().buffer();

        // Every column is a white-key column; octave boundaries have no gaps.
        assert!((0..40).all(|x| b.get(x, 13).symbol == "█"));
        assert_eq!(b.get(6, 12).symbol, "█"); // B2
        assert_eq!(b.get(7, 12).symbol, "└"); // C3 immediately follows

        // C4: the white natural region and lower block are red, not its └ stroke.
        assert_eq!(b.get(14, 12).symbol, "└");
        assert_eq!(b.get(14, 12).fg, Color::Black);
        assert_eq!(b.get(14, 12).bg, Color::Red);
        assert_eq!(b.get(14, 13).fg, Color::Red);

        // E4 has no sharp above it, so both complete blocks are red.
        assert_eq!(b.get(16, 12).symbol, "█");
        assert_eq!(b.get(16, 12).fg, Color::Red);
        assert_eq!(b.get(16, 13).fg, Color::Red);

        // F#4 colours only the └ foreground; the unplayed F stays white.
        assert_eq!(b.get(17, 12).symbol, "└");
        assert_eq!(b.get(17, 12).fg, Color::Red);
        assert_eq!(b.get(17, 12).bg, Color::White);
        assert_eq!(b.get(17, 13).fg, Color::White);
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
        assert!(a.current_pages()[1].percussion);
        assert_eq!(a.tracker_keyboard_note(0), 36);
        assert_eq!(a.tracker_keyboard_note(11), 60);
        a.config.external_midi.percussion_notes = vec![36, 38, 40];
        assert_eq!(a.tracker_keyboard_note(1), 38);
    }

    #[test]
    fn tracker_play_mode_keyboard_cannot_edit_or_clear_pattern_data() {
        let p = presets();
        let mut a = app(&p);
        a.set_screen(Screen::Tracker);
        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        let (tx, _) = mpsc::channel();

        key(KeyCode::Delete, &mut a, Path::new("/none"), &tx);
        key(KeyCode::Char('-'), &mut a, Path::new("/none"), &tx);
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::On(60));
        assert_eq!(a.tracker_row, 0);

        key(KeyCode::Char('X'), &mut a, Path::new("/none"), &tx);
        assert_eq!(a.screen, Screen::TrackerFiles);
        assert!(a.confirm_pattern_clear);
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::On(60));
    }

    #[test]
    fn loop_align_enter_finishes_instead_of_importing() {
        let p = presets();
        let mut a = app(&p);
        a.set_screen(Screen::TrackerLoopAlign);
        let (tx, _) = mpsc::channel();

        key(KeyCode::Enter, &mut a, Path::new("/none"), &tx);

        assert_eq!(a.screen, Screen::TrackerLoop);
        assert_eq!(a.status, "loop alignment set");
    }

    #[test]
    fn callback_timestamps_survive_delayed_ui_drain() {
        let p = presets();
        let mut a = app(&p);
        let started = Instant::now();
        a.recorder.start(started);
        let (tx, rx) = mpsc::channel();
        tx.send(MidiEvent::Raw {
            received: started + Duration::from_millis(17),
            bytes: vec![0x90, 60, 100],
        })
        .unwrap();
        tx.send(MidiEvent::Raw {
            received: started + Duration::from_millis(93),
            bytes: vec![0x80, 60, 0],
        })
        .unwrap();

        drain(&rx, &mut a, Path::new("/none"), &tx);

        assert_eq!(
            a.recorder
                .events
                .iter()
                .map(|event| event.micros)
                .collect::<Vec<_>>(),
            vec![17_000, 93_000]
        );
    }

    #[test]
    fn take_playback_worker_cancels_without_waiting_for_the_last_event() {
        let p = presets();
        let mut a = app(&p);
        let events = vec![TimedEvent {
            micros: 5_000_000,
            bytes: vec![0x90, 60, 100],
        }];
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            recording::play_events(&events, |_| {}, &worker_stop);
        });
        a.playback = Some(Playback {
            stop,
            worker: Some(worker),
        });
        let started = Instant::now();

        a.stop_playback();

        assert!(a.playback.is_none());
        assert!(started.elapsed() < Duration::from_millis(250));
    }

    #[test]
    fn take_playback_requires_the_loaded_idea_engine() {
        let p = presets();
        let mut a = app(&p);
        a.last = vec![TimedEvent {
            micros: 0,
            bytes: vec![0x90, 60, 100],
        }];

        a.toggle_playback();

        assert!(a.playback.is_none());
        assert_eq!(
            a.status,
            "load the idea preset before playing its recording"
        );
    }

    #[test]
    fn files_wheel_moves_the_song_cursor_without_touching_presets() {
        let p = presets();
        let mut a = app(&p);
        a.set_screen(Screen::TrackerFiles);
        a.song_list = vec!["one".into(), "two".into()];
        a.song_selected = 0;
        a.selected = 7;
        let (tx, _) = mpsc::channel();

        mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            &mut a,
            Path::new("/none"),
            &tx,
        );

        assert_eq!(a.song_selected, 1);
        assert_eq!(a.selected, 7);
    }

    #[test]
    fn new_project_is_confirmed_numbered_and_unloads_the_previous_loop() {
        let p = presets();
        let mut a = app(&p);
        a.set_screen(Screen::TrackerFiles);
        a.song_list = vec!["project-001".into()];
        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        a.tracker_row = 12;
        a.loop_player
            .set_preview_status(crate::loop_player::LoopStatus {
                loaded: true,
                file: Some("old.wav".into()),
                ..crate::loop_player::LoopStatus::default()
            });

        perform(Action::NewProject, &mut a, Path::new("/none"), None);
        assert!(a.confirm_new_project);
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::On(60));
        perform(Action::NewProject, &mut a, Path::new("/none"), None);

        assert_eq!(a.song.name, "project-002");
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::Empty);
        assert_eq!((a.tracker_order, a.tracker_row, a.tracker_page), (0, 0, 0));
        assert_eq!(a.screen, Screen::Tracker);
        assert!(!a.loop_player.status().loaded);
    }

    #[test]
    fn loop_removal_is_confirmed_and_only_clears_the_project_reference() {
        let p = presets();
        let mut a = app(&p);
        a.song.audio_loop = Some(sequencer::LoopSettings {
            file: "private.wav".into(),
            source_bpm_x100: 12_000,
            interpretation: sequencer::BpmInterpretation::Normal,
            start_beat: 0,
            length_beats: 4,
            offset_beats: 0,
        });

        perform(Action::LoopRemove, &mut a, Path::new("/none"), None);
        assert!(a.confirm_loop_remove);
        assert!(a.song.audio_loop.is_some());
        perform(Action::Up, &mut a, Path::new("/none"), None);
        assert!(!a.confirm_loop_remove);
        assert!(a.song.audio_loop.is_some());

        perform(Action::LoopRemove, &mut a, Path::new("/none"), None);
        perform(Action::LoopRemove, &mut a, Path::new("/none"), None);
        assert!(a.song.audio_loop.is_none());
        assert_eq!(a.status, "loop removed from Project · private WAV kept");
    }

    #[test]
    fn numbered_project_and_copy_names_skip_existing_files() {
        assert_eq!(
            next_numbered_song_name(&["project-001".into(), "project-002".into()], "project")
                .as_deref(),
            Some("project-003")
        );
        assert_eq!(
            next_numbered_song_name(&["demo-copy-001".into()], "demo-copy").as_deref(),
            Some("demo-copy-002")
        );
    }

    #[test]
    fn note_editor_confirm_commits_and_cancel_restores_every_field() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        let original = Cell {
            note: Note::On(60),
            velocity: Some(70),
            program: Some(2),
            gate: Some(50),
            command: Command::Cut(3),
        };
        a.song.patterns.get_mut(&0).unwrap().rows[0][0] = original;
        perform(Action::OpenNoteEditor, &mut a, Path::new("/none"), None);
        let edited = Cell {
            note: Note::On(72),
            velocity: Some(120),
            program: Some(19),
            gate: Some(33),
            command: Command::Retrigger(4),
        };
        a.note_editor.as_mut().unwrap().draft = edited;
        assert_eq!(a.song.patterns[&0].rows[0][0], original);
        perform(Action::NoteEditorConfirm, &mut a, Path::new("/none"), None);
        assert_eq!(a.song.patterns[&0].rows[0][0], edited);

        perform(Action::OpenNoteEditor, &mut a, Path::new("/none"), None);
        a.note_editor.as_mut().unwrap().draft = Cell::default();
        perform(Action::NoteEditorCancel, &mut a, Path::new("/none"), None);
        assert_eq!(a.song.patterns[&0].rows[0][0], edited);
    }

    #[test]
    fn note_editor_clear_field_does_not_clear_the_cell() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.song.patterns.get_mut(&0).unwrap().rows[0][0] = Cell {
            note: Note::On(60),
            velocity: Some(99),
            program: Some(8),
            gate: Some(75),
            command: Command::Delay(4),
        };
        a.open_note_editor();
        a.select_note_editor_field(NoteEditorField::Velocity);
        a.clear_note_editor_field();
        a.confirm_note_editor();
        let cell = a.song.patterns[&0].rows[0][0];
        assert_eq!(cell.velocity, None);
        assert_eq!(cell.note, Note::On(60));
        assert_eq!(cell.program, Some(8));
        assert_eq!(cell.gate, Some(75));
        assert_eq!(cell.command, Command::Delay(4));
        a.set_tracker_edit(true);
        a.tracker_erase();
        assert_eq!(a.song.patterns[&0].rows[0][0], Cell::default());
    }

    #[test]
    fn cell_program_browser_uses_profile_names_and_live_draft_route() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.config.external_midi.profile = "roland-d-50".into();
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.open_note_editor();
        a.select_note_editor_field(NoteEditorField::Program);

        assert_eq!(
            a.tracker_route.lock().unwrap().preview_state(),
            (true, Some(0), 0, 0)
        );
        a.adjust_note_editor(1);
        assert_eq!(
            a.tracker_route.lock().unwrap().preview_state(),
            (true, Some(1), 0, 0)
        );
        assert_eq!(a.tracker_program_messages(1), vec![vec![0xc0, 1]]);

        a.config.external_midi.bank_select = BankSelectMode::Cc0Cc32;
        a.current_page_mut().unwrap().column_mut(0).bank_msb = 5;
        a.current_page_mut().unwrap().column_mut(0).bank_lsb = 9;
        a.current_page_mut().unwrap().target = PageTarget::Midi("Roland D-50".into());
        assert_eq!(a.tracker_program_messages(7), vec![vec![0xc0, 7]]);

        a.current_page_mut().unwrap().target = PageTarget::ActiveInstrument;
        assert_eq!(
            a.tracker_program_messages(7),
            vec![vec![0xb0, 0, 5], vec![0xb0, 32, 9], vec![0xc0, 7]]
        );
        a.config.external_midi.bank_select = BankSelectMode::Off;
        a.current_page_mut().unwrap().column_mut(0).bank_msb = 0;
        a.current_page_mut().unwrap().column_mut(0).bank_lsb = 0;
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        assert!(rendered.contains("I-12 Metal Harp"));

        a.cancel_note_editor();
        assert!(!a.tracker_route.lock().unwrap().preview_state().0);
    }

    #[test]
    fn cell_editor_audition_does_not_enter_step_edit_notes() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        a.open_note_editor();
        let original = a.song.patterns[&0].rows[0][0];
        let (tx, rx) = mpsc::channel();
        tx.send(MidiEvent::Raw {
            received: Instant::now(),
            bytes: vec![0x90, 60, 100],
        })
        .unwrap();
        tx.send(MidiEvent::Raw {
            received: Instant::now(),
            bytes: vec![0x80, 60, 0],
        })
        .unwrap();
        drain(&rx, &mut a, Path::new("/none"), &tx);
        a.tick();
        assert!(!a.tracker_gesture.is_active());
        assert_eq!(a.song.patterns[&0].rows[0][0], original);
    }

    #[test]
    fn note_editor_rejects_unsupported_combinations_non_destructively() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        let original = Cell::default();
        a.open_note_editor();
        a.note_editor.as_mut().unwrap().draft.command = Command::Retrigger(4);
        a.confirm_note_editor();
        assert!(a.note_editor.is_some());
        assert!(a.status.contains("retrigger requires"));
        assert_eq!(a.song.patterns[&0].rows[0][0], original);
    }

    #[test]
    fn controller_value_cycle_can_enter_note_off() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.open_note_editor();
        perform(Action::NoteEditorDecrease, &mut a, Path::new("/none"), None);
        perform(Action::NoteEditorConfirm, &mut a, Path::new("/none"), None);
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::Off);
    }

    #[test]
    fn all_four_controller_item_buttons_dispatch_in_note_edit_mode() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.open_note_editor();
        a.select_menu_page(1);
        let (tx, rx) = mpsc::channel();
        for (pad, field) in [
            (crate::pads::PadAction::Item1, NoteEditorField::Note),
            (crate::pads::PadAction::Item2, NoteEditorField::Gate),
            (crate::pads::PadAction::Item3, NoteEditorField::Velocity),
            (crate::pads::PadAction::Item4, NoteEditorField::Program),
        ] {
            tx.send(MidiEvent::Pad(pad, true)).unwrap();
            drain(&rx, &mut a, Path::new("/none"), &tx);
            assert_eq!(a.note_editor.unwrap().field, field);
        }
    }

    #[test]
    fn eight_five_and_four_layout_page_selection_survives_note_editing() {
        let p = presets();
        for layout in [ControllerLayout::Eight, ControllerLayout::Five] {
            let mut a = app(&p);
            a.screen = Screen::Tracker;
            a.controller_layout = layout;
            a.open_note_editor();
            let (tx, rx) = mpsc::channel();
            let page_action = if layout == ControllerLayout::Eight {
                crate::pads::PadAction::Page3
            } else {
                crate::pads::PadAction::CyclePage
            };
            tx.send(MidiEvent::Pad(page_action, true)).unwrap();
            drain(&rx, &mut a, Path::new("/none"), &tx);
            assert!(a.menu_page() > 0);
        }
        let mut four = app(&p);
        four.screen = Screen::Tracker;
        four.controller_layout = ControllerLayout::Four;
        four.open_note_editor();
        let (tx, rx) = mpsc::channel();
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Select))
            .unwrap();
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Down))
            .unwrap();
        drain(&rx, &mut four, Path::new("/none"), &tx);
        assert!(four.page_select_mode);
        assert_eq!(four.menu_page(), 1);
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Select))
            .unwrap();
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Down))
            .unwrap();
        drain(&rx, &mut four, Path::new("/none"), &tx);
        assert!(!four.page_select_mode);
        assert_eq!(four.note_editor.unwrap().draft.note, Note::On(60));
    }

    #[test]
    fn ft2_effect_markers_use_the_first_spacer_without_losing_selection_colors() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        let row = &mut a.song.patterns.get_mut(&0).unwrap().rows[0];
        for (lane, command) in [
            Command::Cut(1),
            Command::Delay(2),
            Command::Retrigger(3),
            Command::Tempo(140),
        ]
        .into_iter()
        .enumerate()
        {
            row[lane].note = Note::On(60 + lane as u8);
            row[lane].command = command;
        }
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
        let buffer = terminal.backend().buffer();
        for (x, marker) in [(9, "C"), (18, "D"), (27, "R"), (36, "T")] {
            assert_eq!(buffer.get(x, 3).symbol, marker);
        }
        assert_eq!(buffer.get(9, 3).fg, Color::Black);
        assert_eq!(buffer.get(9, 3).bg, Color::Yellow);
        assert_eq!(buffer.get(18, 3).bg, Color::DarkGray);

        let mut empty = app(&p);
        empty.screen = Screen::Tracker;
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut empty)).unwrap();
        assert_eq!(terminal.backend().buffer().get(9, 3).symbol, " ");
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
        assert_eq!(a.status, "ERASE · cell cleared · advanced 1 row(s)");
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
            navigation::slot(a.screen, a.menu_context(), 0, 2).and_then(|slot| slot.dispatch()),
            Some(Action::PreviewSong)
        );
        perform(Action::ClearPattern, &mut a, Path::new("/none"), None);
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::On(60));
        assert!(a.confirm_pattern_clear);
        perform(Action::SelectThreeFour, &mut a, Path::new("/none"), None);
        assert_eq!(a.pattern_clear_beats, 3);
        perform(Action::PatternSizeDown, &mut a, Path::new("/none"), None);
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
        assert!(a.confirm_pattern_clear);
        perform(Action::PatternSizeUp, &mut a, Path::new("/none"), None);
        perform(
            Action::ConfirmPatternClear,
            &mut a,
            Path::new("/none"),
            None,
        );

        assert_eq!(a.song.order, vec![0, 1]);
        assert_eq!(a.tracker_order, 1);
        assert_eq!(a.tracker_pattern_number(), 1);
        assert_eq!(a.tracker_row, 0);
        assert_eq!(a.screen, Screen::Tracker);
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::On(60));
        assert_eq!(a.song.patterns[&1].rows[0][0].note, Note::Empty);
        assert!(a.status.contains("new pattern 1"));
        let saved = sequencer::encode(&a.song).unwrap();
        assert!(saved.contains("order=0,1\n"));
        assert!(saved.contains("pattern=0|"));
        assert!(saved.contains("pattern=1|"));
    }

    #[test]
    fn pattern_clipboard_pastes_new_and_over_with_confirmation() {
        let p = presets();
        let mut a = app(&p);
        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        a.copy_pattern();
        a.paste_pattern_new();
        assert_eq!(a.tracker_pattern_number(), 1);
        assert_eq!(a.song.patterns[&1].rows[0][0].note, Note::On(60));
        a.song.patterns.get_mut(&1).unwrap().rows[0][0].note = Note::On(61);
        a.paste_pattern_over();
        assert!(a.confirm_pattern_paste_over.is_some());
        assert_eq!(a.song.patterns[&1].rows[0][0].note, Note::On(61));
        a.paste_pattern_over();
        assert_eq!(a.song.patterns[&1].rows[0][0].note, Note::On(60));
    }

    #[test]
    fn drum_library_load_repeats_into_percussion_page_and_preserves_routing() {
        let p = presets();
        let mut a = app(&p);
        let path = std::env::temp_dir().join(format!(
            "shr-drum-load-{}-{}.shdrum",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut drum = DrumPattern {
            name: "Test groove".into(),
            genre: "Test".into(),
            meter: 4,
            rows: vec![[Cell::default(); LANES_PER_PAGE]; 16],
        };
        drum.rows[0][0] = Cell {
            note: Note::On(36),
            velocity: Some(111),
            ..Cell::default()
        };
        fs::write(&path, drum_pattern::encode(&drum).unwrap()).unwrap();
        a.drum_patterns = vec![drum_pattern::Entry {
            name: drum.name,
            genre: drum.genre,
            meter: drum.meter,
            rows: drum.rows.len(),
            path: path.clone(),
            user: false,
            bundled: None,
        }];
        a.drum_target_rows = a.tracker_rows();
        let percussion = 1;
        a.song.patterns.get_mut(&0).unwrap().pages[percussion].columns[0].program = 55;
        a.song.patterns.get_mut(&0).unwrap().rows[3][0].note = Note::On(64);
        let page_before = a.song.patterns[&0].pages[percussion].clone();

        a.load_drum_pattern();

        let pattern = &a.song.patterns[&0];
        let lane = percussion * LANES_PER_PAGE;
        assert_eq!(pattern.rows[0][lane].note, Note::On(36));
        assert_eq!(pattern.rows[16][lane].note, Note::On(36));
        assert_eq!(pattern.rows[3][0].note, Note::On(64));
        assert_eq!(pattern.pages[percussion], page_before);
        assert!(a.status.contains("routing unchanged"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn drum_filters_map_phrase_sizes_and_protect_existing_melody() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::TrackerFiles;
        a.open_drum_patterns();
        assert_eq!((a.drum_meter, a.drum_target_rows), (4, 64));
        assert!(a.filtered_drum_indices().len() >= 40);

        a.toggle_drum_meter();
        assert_eq!((a.drum_meter, a.drum_target_rows), (3, 48));
        assert!(a.filtered_drum_indices().len() >= 20);
        a.cycle_drum_size();
        assert_eq!(a.drum_target_rows, 96);
        a.toggle_drum_meter();
        assert_eq!((a.drum_meter, a.drum_target_rows), (4, 128));

        a.drum_target_rows = 32;
        a.clamp_drum_selection();
        a.load_drum_pattern();
        assert_eq!(a.tracker_rows(), 32);
        assert!(a.status.contains("32 rows"));

        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        a.drum_target_rows = 64;
        a.load_drum_pattern();
        assert_eq!(a.tracker_rows(), 32);
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::On(60));
        assert!(a.status.contains("would resize existing page data"));
    }

    #[test]
    fn unrelated_actions_expire_pending_confirmations() {
        let p = presets();
        let mut a = app(&p);
        a.confirm_load = Some("idea".into());
        a.confirm_delete = Some("idea".into());
        a.confirm_song_save = Some("song".into());
        a.confirm_song_delete = Some("song".into());
        a.confirm_pattern_paste_over = Some(0);
        a.confirm_new_project = true;
        a.confirm_loop_remove = true;

        perform(Action::Up, &mut a, Path::new("/none"), None);

        assert!(a.confirm_load.is_none());
        assert!(a.confirm_delete.is_none());
        assert!(a.confirm_song_save.is_none());
        assert!(a.confirm_song_delete.is_none());
        assert!(a.confirm_pattern_paste_over.is_none());
        assert!(!a.confirm_new_project);
        assert!(!a.confirm_loop_remove);

        a.confirm_load = Some("idea".into());
        a.confirm_delete = Some("idea".into());
        a.confirm_song_save = Some("song".into());
        a.confirm_song_delete = Some("song".into());
        a.confirm_pattern_paste_over = Some(0);
        a.confirm_new_project = true;
        a.confirm_loop_remove = true;
        let (tx, _rx) = std::sync::mpsc::channel();
        key(KeyCode::Up, &mut a, Path::new("/none"), &tx);
        assert!(a.confirm_load.is_none());
        assert!(a.confirm_delete.is_none());
        assert!(a.confirm_song_save.is_none());
        assert!(a.confirm_song_delete.is_none());
        assert!(a.confirm_pattern_paste_over.is_none());
        assert!(!a.confirm_new_project);
        assert!(!a.confirm_loop_remove);
    }

    #[test]
    fn leaving_project_files_stops_hidden_song_preview() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::TrackerFiles;
        a.song_previewing = true;

        a.set_screen(Screen::Tracker);

        assert_eq!(a.screen, Screen::Tracker);
        assert!(!a.song_previewing);
    }

    #[test]
    fn lane_and_page_clipboards_paste_overlap_and_report_truncation() {
        let p = presets();
        let mut a = app(&p);
        a.song.patterns.get_mut(&0).unwrap().rows[0][0] = Cell {
            note: Note::On(60),
            velocity: Some(77),
            program: Some(5),
            gate: Some(40),
            command: Command::Delay(3),
        };
        a.copy_lane();
        a.song.patterns.get_mut(&0).unwrap().rows.truncate(1);
        a.tracker_track = 1;
        a.paste_lane();
        assert_eq!(a.song.patterns[&0].rows[0][1].note, Note::On(60));
        assert_eq!(a.song.patterns[&0].rows[0][1].program, Some(5));
        assert!(a.status.contains("truncated"));

        a.tracker_track = 0;
        a.copy_page_block();
        a.add_tracker_page();
        a.tracker_page = 1;
        a.paste_page_block();
        assert_eq!(
            a.song.patterns[&0].rows[0][LANES_PER_PAGE].velocity,
            Some(77)
        );
    }

    #[test]
    fn arrangement_screen_repeats_reorders_and_jumps_to_pattern() {
        let p = presets();
        let mut a = app(&p);
        a.clone_pattern();
        a.open_arrange();
        assert_eq!(a.screen, Screen::TrackerArrange);
        a.arrange_selected = 0;
        a.arrangement_duplicate_step();
        assert_eq!(a.song.order[1], 0);
        a.arrangement_move_step(1);
        assert_eq!(a.arrange_selected, 2);
        a.arrangement_remove_step();
        assert_eq!(a.song.order.len(), 2);
        a.arrangement_jump_to_pattern();
        assert_eq!(a.screen, Screen::Tracker);
        assert_eq!(a.tracker_order, 1);
    }

    #[test]
    fn pattern_switch_clamps_cursor_to_pattern_shape() {
        let p = presets();
        let mut a = app(&p);
        let mut small = sequencer::Pattern::from_config(&a.config.external_midi, 8, 4);
        small.pages.truncate(1);
        for row in &mut small.rows {
            row.truncate(LANES_PER_PAGE);
        }
        a.song.patterns.insert(1, small);
        a.song.order.push(1);
        a.tracker_page = 1;
        a.tracker_track = 3;
        a.tracker_row = 63;
        a.tracker_order = 1;
        a.clamp_tracker_cursor();
        assert_eq!(a.tracker_page, 0);
        assert_eq!(a.tracker_track, 3);
        assert_eq!(a.tracker_row, 7);
    }

    #[test]
    fn pattern_setup_offers_matching_four_four_and_three_four_sizes() {
        assert_eq!(pattern_sizes(4), [8, 16, 32, 64, 128]);
        assert_eq!(pattern_sizes(3), [6, 12, 24, 48, 96]);

        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::TrackerFiles;
        a.new_pattern();
        a.select_pattern_meter(3);
        assert_eq!(a.pattern_setup_rows, 48);
        a.change_pattern_size(-1);
        assert_eq!(a.pattern_setup_rows, 24);
        a.apply_pattern_clear();
        assert_eq!(a.song.order, vec![0, 1]);
        assert_eq!(a.song.patterns[&1].rows.len(), 24);
    }

    #[test]
    fn tracker_record_is_hardware_only_and_writes_the_current_page_pattern() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.current_page_mut().unwrap().target = PageTarget::ActiveInstrument;
        a.begin_tracker_recording();
        assert!(a.tracker_recording.is_none());
        assert!(a.status.contains("loaded synth stays isolated"));

        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        let setup = a.current_pattern().unwrap().clone();
        a.song
            .patterns
            .insert(1, sequencer::Pattern::empty_like_setup(8, &setup));
        a.song.order.push(1);
        a.tracker_order = 1;
        a.begin_tracker_recording();
        assert_eq!(a.menu_context(), MenuContext::TrackerRecord);
        assert!(a.tracker_route.lock().unwrap().preview_state().0);

        a.record_tracker_midi(&[0x90, 60, 111]);
        assert_eq!(a.song.patterns[&1].rows[0][0].note, Note::On(60));
        assert_eq!(a.song.patterns[&1].rows[0][0].velocity, Some(111));
        assert!(a.song.patterns[&0]
            .rows
            .iter()
            .flatten()
            .all(|cell| cell.note == Note::Empty));
        assert!(a.song.patterns[&1].rows[0][LANES_PER_PAGE..]
            .iter()
            .all(|cell| cell.note == Note::Empty));

        a.stop_tracker_recording();
        assert!(!a.tracker_route.lock().unwrap().preview_state().0);
    }

    #[test]
    fn tracker_record_keeps_overlapping_same_notes_owned_by_channel_and_instance() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.begin_tracker_recording();

        a.record_tracker_midi(&[0x90, 60, 100]);
        a.record_tracker_midi(&[0x90, 60, 101]);
        a.record_tracker_midi(&[0x91, 60, 102]);
        let recording = a.tracker_recording.as_ref().unwrap();
        assert_eq!(recording.active_lanes[&(0, 60)], [0, 1]);
        assert_eq!(recording.active_lanes[&(1, 60)], [2]);

        a.record_tracker_midi(&[0x80, 60, 0]);
        assert_eq!(
            a.tracker_recording.as_ref().unwrap().active_lanes[&(0, 60)],
            [0]
        );
        a.record_tracker_midi(&[0x90, 62, 103]);
        assert_eq!(a.song.patterns[&0].rows[0][3].note, Note::On(62));

        a.record_tracker_midi(&[0x90, 60, 0]);
        a.record_tracker_midi(&[0x81, 60, 0]);
        assert!(!a
            .tracker_recording
            .as_ref()
            .unwrap()
            .active_lanes
            .contains_key(&(0, 60)));
        assert!(!a
            .tracker_recording
            .as_ref()
            .unwrap()
            .active_lanes
            .contains_key(&(1, 60)));
        a.stop_tracker_recording();
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
    fn encoder_skip_and_paged_erase_and_add_actions_remain_available() {
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
        a.tracker_cell_mut().unwrap().note = Note::On(70);
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Page1, true))
            .unwrap();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item2, true))
            .unwrap();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Page3, true))
            .unwrap();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item4, true))
            .unwrap();
        drain(&rx, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.tracker_advance, 8);
        assert_eq!(a.tracker_row, (row + 1) % a.tracker_rows());
        assert_eq!(a.song.patterns[&0].rows[row][0].note, Note::Empty);
        let note_row = a.tracker_row;
        a.tracker_single_note(60, 96);
        assert_eq!(a.tracker_row, (note_row + 8) % a.tracker_rows());
        a.select_menu_page(0);
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
    fn tracker_stop_never_doubles_as_back() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        perform(Action::TrackerStop, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Tracker);
        assert_eq!(a.tracker_mode, TrackerMode::Edit);
    }

    #[test]
    fn mode_page_enters_edit_noob_and_returns_to_play_without_duplicate_state() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        perform(Action::TrackerModeEdit, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_mode, TrackerMode::Edit);
        assert!(a.tracker_recording.is_none());
        perform(Action::TrackerModeNoob, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::TrackerNoob);
        a.noob_draft = Scale {
            root: 3,
            kind: ScaleKind::NaturalMinor,
        };
        perform(Action::ConfirmNoob, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_mode, TrackerMode::Noob);
        assert_eq!(a.noob_scale.root, 3);
        perform(Action::TrackerModePlay, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_mode, TrackerMode::Play);
        assert!(a.tracker_recording.is_none());
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
        a.select_pattern_meter(3);
        a.change_pattern_size(-1);
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
        a.change_pattern_size(-1);
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
            Screen::Help,
            Screen::Tracker,
            Screen::TrackerFiles,
            Screen::TrackerArrange,
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
