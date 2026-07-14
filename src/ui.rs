use crate::audio_recorder::AudioRecorder;
use crate::chord::HeldNotes;
use crate::config::{BankSelectMode, RuntimeConfig};
use crate::control::{parameter_color, CONTROLS};
use crate::device_profile::{DeviceProfile, Registry as DeviceProfiles};
use crate::engine::{self, Engine, MidiEvent};
use crate::geometry::{contains, rect, visible_index};
use crate::help::{self, HelpKind};
use crate::navigation::{self, Action, MenuContext, Screen, SlotState};
use crate::pads::{ControllerLayout, MenuInput, TapTempo};
use crate::preset::{BackendKind, Catalog, Preset};
use crate::recording::{self, Recorder, TimedEvent};
use crate::scale::{Scale, ScaleKind};
use crate::sequencer::{
    self, Cell, Command, GestureCapture, Note, PageTarget, Song, LANES_PER_PAGE,
};
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
use std::path::{Path, PathBuf};
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
    active_lanes: HashMap<u8, usize>,
    notes: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum TrackerMode {
    #[default]
    Play,
    Rec,
    Edit,
    Noob,
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
    song_list: Vec<String>,
    song_selected: usize,
    tracker_order: usize,
    tracker_row: usize,
    tracker_page: usize,
    tracker_track: usize,
    tracker_mode: TrackerMode,
    tracker_recording: Option<TrackerRecording>,
    note_editor: Option<NoteEditor>,
    tracker_octave: u8,
    noob_scale: Scale,
    noob_draft: Scale,
    tracker_gesture: GestureCapture,
    tracker_gesture_anchor: Option<(usize, usize, usize, usize)>,
    confirm_song_save: bool,
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
    menu_page_by_screen: [usize; Screen::COUNT],
    page_select_mode: bool,
    controller_layout: ControllerLayout,
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
            song_list: sequencer::list(&sequencer::songs_dir()),
            song_selected: 0,
            tracker_order: 0,
            tracker_row: 0,
            tracker_page: 0,
            tracker_track: 0,
            tracker_mode: TrackerMode::Play,
            tracker_recording: None,
            note_editor: None,
            tracker_octave: 4,
            noob_scale: Scale::default(),
            noob_draft: Scale::default(),
            tracker_gesture: GestureCapture::default(),
            tracker_gesture_anchor: None,
            confirm_song_save: false,
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
            menu_page_by_screen: [0; Screen::COUNT],
            page_select_mode: false,
            controller_layout: ControllerLayout::Eight,
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

    fn menu_page(&self) -> usize {
        self.menu_page_by_screen[self.screen.index()].min(3)
    }

    fn select_menu_page(&mut self, page: usize) {
        let page = page.min(3);
        if navigation::pages(self.screen, self.menu_context())[page].available() {
            self.menu_page_by_screen[self.screen.index()] = page;
        }
        self.page_select_mode = false;
    }

    fn cycle_menu_page(&mut self, direction: i8) {
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
        self.playback = None;
        if let Some(e) = &self.engine {
            e.panic();
        }
        self.status = "recording playback stopped · all notes off".into();
    }
    fn stop_all(&mut self, state: &Path) {
        self.cancel_note_editor();
        self.cancel_tracker_gesture();
        self.stop_tracker_recording();
        self.sequencer.stop();
        self.loop_player.stop();
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
                    .unwrap_or(self.song.pages[self.tracker_page].program),
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
                .unwrap_or(self.song.pages[self.tracker_page].program);
            self.preview_tracker_program(program);
        }
    }
    fn adjust_note_editor(&mut self, direction: i8) {
        let page_velocity = self.song.pages[self.tracker_page].velocity;
        let page_program = self.song.pages[self.tracker_page].program;
        let song_gate = self.song.gate_percent;
        let song_tempo = self.song.tempo;
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
                        Command::Tempo(song_tempo),
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
            self.preview_tracker_program(self.song.pages[self.tracker_page].program);
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
            .unwrap_or(self.song.pages[self.tracker_page].program);
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
            self.tracker_row = (self.tracker_row + 1) % rows;
        }
    }
    fn tracker_skip(&mut self) {
        if self.tracker_mode == TrackerMode::Edit {
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
        self.status = "gesture entered · row advanced".into();
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
        self.screen = Screen::TrackerNoob;
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
        self.screen = Screen::Tracker;
        self.set_tracker_mode(TrackerMode::Noob);
        self.status = format!(
            "N00B {} {} · nearest note, ties down",
            crate::scale::note_name(self.noob_scale.root),
            self.noob_scale.kind.label()
        );
    }
    fn sync_tracker_route(&self) {
        let Some(page) = self.song.pages.get(self.tracker_page) else {
            return;
        };
        let program = self
            .note_editor
            .and_then(|editor| editor.draft.program)
            .unwrap_or(page.program);
        if let Ok(mut route) = self.tracker_route.lock() {
            route.configure(crate::engine::TrackerRouteConfig {
                enabled: self.screen == Screen::Tracker
                    && (matches!(self.tracker_mode, TrackerMode::Edit | TrackerMode::Noob)
                        || self.note_editor.is_some()
                        || self.tracker_recording.is_some()),
                target: page.target.clone(),
                channel: page.channel,
                percussion: page.percussion,
                scale: (self.tracker_mode == TrackerMode::Noob).then_some(self.noob_scale),
                selection: (program, page.bank_msb, page.bank_lsb),
                external: &self.config.external_midi,
            });
        }
    }

    fn tracker_device_profile(&self) -> Option<&DeviceProfile> {
        let page = self.song.pages.get(self.tracker_page)?;
        match &page.target {
            PageTarget::ConfiguredExternal => self
                .device_profiles
                .by_id(&self.config.external_midi.profile),
            PageTarget::Midi(port) => self.device_profiles.matching_port(port),
            PageTarget::ActiveInstrument => None,
        }
    }

    fn tracker_program_label(&self, program: u8) -> String {
        let page = &self.song.pages[self.tracker_page];
        self.tracker_device_profile()
            .and_then(|profile| profile.program_label(page.bank_msb, page.bank_lsb, program))
            .unwrap_or_else(|| format!("MIDI program {program}"))
    }

    fn preview_tracker_program(&self, program: u8) {
        let Some(page) = self.song.pages.get(self.tracker_page) else {
            return;
        };
        for message in self.tracker_program_messages(program) {
            self.tracker_live_input.send(&page.target, &message);
        }
    }

    fn tracker_program_messages(&self, program: u8) -> Vec<Vec<u8>> {
        if !self.config.external_midi.program_changes {
            return Vec::new();
        }
        let Some(page) = self.song.pages.get(self.tracker_page) else {
            return Vec::new();
        };
        let mut messages = Vec::new();
        match self.config.external_midi.bank_select {
            BankSelectMode::Off => {}
            BankSelectMode::Cc0 => {
                messages.push(vec![0xb0 | page.channel, 0, page.bank_msb]);
            }
            BankSelectMode::Cc0Cc32 => {
                messages.push(vec![0xb0 | page.channel, 0, page.bank_msb]);
                messages.push(vec![0xb0 | page.channel, 32, page.bank_lsb]);
            }
        }
        messages.push(vec![0xc0 | page.channel, program]);
        messages
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
        self.set_tracker_mode(TrackerMode::Play);
        self.page_manager_original = Some(self.song.clone());
        self.page_manager_mode = PageManagerMode::Pages;
        self.refresh_page_targets();
        self.screen = Screen::TrackerPages;
        self.reset_context_page();
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
        self.reset_context_page();
        self.status = "turn encoder for target · press to confirm · STOP cancels field".into();
    }
    fn edit_page_channel(&mut self) {
        if self.page_manager_mode != PageManagerMode::Pages {
            return;
        }
        self.page_channel_draft = self.song.pages[self.tracker_page].channel;
        self.page_manager_mode = PageManagerMode::Channel;
        self.reset_context_page();
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
        if self.stop_tracker_recording() {
            return;
        }
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
        if notes == 0 && self.song.audio_loop.is_none() {
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
        for (index, candidate) in song.pages.iter_mut().enumerate() {
            candidate.enabled = index == page;
        }
        song
    }

    fn begin_tracker_recording(&mut self) {
        if self.tracker_recording.is_some() {
            self.stop_tracker_recording();
            return;
        }
        let Some(page) = self.song.pages.get(self.tracker_page) else {
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
            self.song.pages[page_index].name
        );
    }

    fn stop_tracker_recording(&mut self) -> bool {
        let Some(recording) = self.tracker_recording.take() else {
            return false;
        };
        if let Some(page) = self.song.pages.get(recording.page) {
            self.tracker_live_input.cancel(&page.target, page.channel);
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
        let note = bytes[1];
        let note_on = bytes[0] & 0xf0 == 0x90 && bytes[2] > 0;
        if !note_on {
            if let Some(recording) = self.tracker_recording.as_mut() {
                recording.active_lanes.remove(&note);
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
                !recording.active_lanes.values().any(|active| active == lane)
                    && matches!(pattern.rows[row][first_lane + lane].note, Note::Empty)
            })
            .or_else(|| {
                (0..LANES_PER_PAGE)
                    .map(|offset| (recording.next_lane + offset) % LANES_PER_PAGE)
                    .find(|lane| !recording.active_lanes.values().any(|active| active == lane))
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
        recording.active_lanes.insert(note, lane);
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

    fn load_current_loop(&mut self) {
        let Some(settings) = self.song.audio_loop.clone() else {
            return;
        };
        let path = crate::loop_player::loops_dir().join(&settings.file);
        match crate::loop_player::DecodedLoop::open(&path)
            .and_then(|decoded| self.loop_player.load(decoded, &settings))
        {
            Ok(()) => self.status = format!("loop ready · {}", settings.file),
            Err(error) => self.status = format!("loop load: {error}"),
        }
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
                    self.song.tempo,
                    self.song.meter,
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
                match self.loop_player.load(decoded, &settings) {
                    Ok(()) => {
                        self.status = format!(
                            "imported {} · {} bar(s) · BPM {:.0}/{:.0}/{:.0}",
                            settings.file,
                            alignment.bars,
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
                    self.song.tempo,
                    self.song.meter,
                );
                if let Some(settings) = self.song.audio_loop.as_mut() {
                    settings.source_bpm_x100 = (alignment.source_bpm * 100.0).round() as u32;
                    settings.interpretation = sequencer::BpmInterpretation::Normal;
                    settings.start_beat = 0;
                    settings.length_beats = alignment.length_beats;
                    settings.offset_beats = 0;
                }
                self.load_current_loop();
                self.status = format!(
                    "auto aligned {} bar(s) at {:.2} BPM{}",
                    alignment.bars,
                    alignment.source_bpm,
                    if alignment.transient_detected {
                        ""
                    } else {
                        " (duration)"
                    }
                );
            }
            Err(error) => self.status = format!("auto align: {error}"),
        }
    }

    fn adjust_loop_offset_bars(&mut self, direction: i8) {
        let unit = i32::from(self.song.meter.clamp(1, 16));
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
        if let Some(settings) = self.song.audio_loop.as_mut() {
            settings.source_bpm_x100 = if direction < 0 {
                settings.source_bpm_x100.saturating_sub(100).max(2_000)
            } else {
                settings.source_bpm_x100.saturating_add(100).min(30_000)
            };
            self.load_current_loop();
        }
    }

    fn cycle_loop_bpm_mode(&mut self) {
        if let Some(settings) = self.song.audio_loop.as_mut() {
            settings.interpretation = match settings.interpretation {
                sequencer::BpmInterpretation::Half => sequencer::BpmInterpretation::Normal,
                sequencer::BpmInterpretation::Normal => sequencer::BpmInterpretation::Double,
                sequencer::BpmInterpretation::Double => sequencer::BpmInterpretation::Half,
            };
            self.load_current_loop();
        }
    }

    fn adjust_loop_region(&mut self, start: bool, direction: i8) {
        let unit = if self.loop_edit_bars {
            crate::loop_player::bar_to_beat(1, self.song.meter)
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
                if self.song.audio_loop.is_some() {
                    self.load_current_loop();
                } else {
                    self.status = format!("loaded {name}");
                }
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
        self.screen = Screen::TrackerFiles;
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
        self.song.meter = self.pattern_clear_beats;
        if self.pattern_setup_new {
            self.create_pattern(self.pattern_setup_rows);
            return;
        }
        let number = self.tracker_pattern_number();
        let rows = self.pattern_setup_rows;
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
        self.choose_new_pattern();
    }

    fn create_pattern(&mut self, rows: usize) {
        self.tracker_stop();
        self.song.meter = self.pattern_clear_beats;
        let number = self.song.patterns.keys().next_back().copied().unwrap_or(0) + 1;
        self.song.patterns.insert(
            number,
            sequencer::Pattern::empty(rows, self.song.total_lanes()),
        );
        self.song.order.push(number);
        self.tracker_order = self.song.order.len() - 1;
        self.tracker_row = 0;
        self.confirm_pattern_clear = false;
        self.pattern_setup_new = false;
        self.screen = Screen::Tracker;
        self.status = format!(
            "new pattern {number} · {rows} rows · order {:02}/{:02}",
            self.tracker_order + 1,
            self.song.order.len()
        );
    }
    fn clone_pattern(&mut self) {
        let old = self.tracker_pattern_number();
        let number = self.song.patterns.keys().next_back().copied().unwrap_or(0) + 1;
        let Some(pattern) = self.song.patterns.get(&old).cloned() else {
            self.status = "no pattern to clone".into();
            return;
        };
        self.song.patterns.insert(number, pattern);
        self.song.order.push(number);
        self.tracker_order = self.song.order.len() - 1;
        self.tracker_row = 0;
        self.status = format!("cloned pattern {old} as {number}");
    }
    fn clear_pattern_now(&mut self) {
        let number = self.tracker_pattern_number();
        let lanes = self.song.total_lanes();
        if let Some(pattern) = self.song.patterns.get_mut(&number) {
            *pattern = sequencer::Pattern::empty(pattern.rows.len(), lanes);
            self.tracker_row = 0;
            self.status = format!("cleared pattern {number}");
        }
    }
    fn move_order(&mut self, direction: i8) {
        self.cancel_tracker_gesture();
        self.tracker_order = if direction < 0 {
            self.tracker_order.saturating_sub(1)
        } else {
            (self.tracker_order + 1).min(self.song.order.len().saturating_sub(1))
        };
        self.tracker_row = 0;
        self.status = format!(
            "order {:02}/{:02}",
            self.tracker_order + 1,
            self.song.order.len()
        );
    }
    fn repeat_order(&mut self) {
        let number = self.tracker_pattern_number();
        self.song.order.insert(self.tracker_order + 1, number);
        self.tracker_order += 1;
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
        self.tracker_row = 0;
        self.status = "order entry removed".into();
    }
    fn tracker_note_off(&mut self) {
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
            self.status = "NOTE OFF · row advanced".into();
        }
    }
    fn change_program(&mut self, direction: i8) {
        if let Some(page) = self.song.pages.get_mut(self.tracker_page) {
            page.program = if direction < 0 {
                page.program.saturating_sub(1)
            } else {
                page.program.saturating_add(1).min(127)
            };
            self.status = format!("{} program {}", page.name, page.program);
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
            self.set_tracker_mode(TrackerMode::Play);
        }
        self.ideas = recording::list(&recording::ideas_dir()).unwrap_or_default();
        self.idea_selected = self.idea_selected.min(self.ideas.len().saturating_sub(1));
        self.confirm_delete = None;
        self.screen = Screen::Ideas;
        self.status = "ideas · select an action".into();
    }
    fn open_help(&mut self) {
        if self.screen != Screen::Help {
            self.help_previous = self.screen;
        }
        self.screen = Screen::Help;
        self.start_web_help();
        self.sync_tracker_route();
        self.reset_context_page();
        self.status = format!("HELP · {} · EXIT closes", self.web_help_status);
    }
    fn close_help(&mut self) {
        self.web_help = None;
        self.web_help_status.clear();
        self.screen = self.help_previous;
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
        let last = help::lines(38).len().saturating_sub(1);
        self.help_selected = if delta < 0 {
            self.help_selected.saturating_sub(delta.unsigned_abs())
        } else {
            (self.help_selected + delta as usize).min(last)
        };
    }
    fn activate_help(&mut self) {
        let lines = help::lines(38);
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
            MidiEvent::Value(cc, v) => {
                app.values.insert(cc, v);
            }
            MidiEvent::Raw(bytes) => {
                let now = Instant::now();
                app.held_notes.observe(&bytes);
                app.recorder.capture(now, &bytes);
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
                    app.tracker_gesture.observe(now, &bytes);
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
                if app.screen == Screen::Help {
                    let action = match action {
                        crate::pads::EncoderAction::Up => Action::Up,
                        crate::pads::EncoderAction::Down => Action::Down,
                        crate::pads::EncoderAction::Select => Action::Activate,
                    };
                    perform(action, app, state, Some(tx));
                } else if app.controller_layout == ControllerLayout::Four {
                    match action {
                        crate::pads::EncoderAction::Select => {
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
            Action::End => a.help_selected = help::lines(38).len().saturating_sub(1),
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
                a.cancel_note_editor();
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
                a.song_selected = a.song_selected.saturating_sub(1);
                a.confirm_song_delete = None;
            } else if a.screen == Screen::Tracker {
                a.cancel_tracker_gesture();
                a.tracker_row = a.tracker_row.saturating_sub(1);
            } else if a.screen == Screen::TrackerPages {
                a.turn_page_manager(-1);
            } else if a.screen == Screen::TrackerLoop {
                a.loop_selected = a.loop_selected.saturating_sub(1);
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
            } else if a.screen == Screen::TrackerLoop {
                a.loop_selected = (a.loop_selected + 1).min(a.loop_imports.len().saturating_sub(1));
            } else if a.screen == Screen::Presets {
                a.selected = (a.selected + 1).min(a.presets.len().saturating_sub(1));
            }
        }
        Action::PageUp => {
            if a.screen == Screen::Presets {
                a.selected = a.selected.saturating_sub(10);
            }
        }
        Action::PageDown => {
            if a.screen == Screen::Presets {
                a.selected = (a.selected + 10).min(a.presets.len().saturating_sub(1));
            }
        }
        Action::Home => {
            if a.screen == Screen::Ideas {
                a.idea_selected = 0;
            } else {
                a.selected = 0;
            }
        }
        Action::End => {
            if a.screen == Screen::Ideas {
                a.idea_selected = a.ideas.len().saturating_sub(1);
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
            Screen::TrackerFiles => a.load_song(),
            Screen::TrackerPages => a.confirm_page_manager(),
            Screen::TrackerTools
            | Screen::TrackerNoob
            | Screen::TrackerLoop
            | Screen::TrackerLoopAlign => {}
            Screen::AudioRecorder => a.toggle_audio_recording(),
        },
        Action::Quit => unreachable!("quit is handled before contextual dispatch"),
        Action::StopAll => unreachable!("panic is handled before contextual dispatch"),
        Action::OpenPresets => {
            a.set_tracker_edit(false);
            a.screen = Screen::Presets;
        }
        Action::OpenIdeas => a.open_ideas(),
        Action::OpenHelp => a.open_help(),
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
        Action::OpenTrackerTools => {
            a.screen = Screen::TrackerTools;
            a.reset_context_page();
            a.status = "FT2 tools · pages, files, loop, mute".into();
        }
        Action::OpenTrackerLoop => {
            a.screen = Screen::TrackerLoop;
            a.refresh_loop_imports();
            a.reset_context_page();
            a.status = format!("loop inbox · {} WAV file(s)", a.loop_imports.len());
        }
        Action::OpenTrackerLoopAlign => {
            a.screen = Screen::TrackerLoopAlign;
            a.reset_context_page();
            a.status = "loop align · AUTO or move by one bar".into();
        }
        Action::OpenAudioRecorder => {
            a.set_tracker_edit(false);
            a.screen = Screen::AudioRecorder;
            a.status = "stereo audio recorder".into();
        }
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
            a.screen = if matches!(
                a.screen,
                Screen::TrackerFiles
                    | Screen::TrackerTools
                    | Screen::TrackerNoob
                    | Screen::TrackerLoop
                    | Screen::TrackerLoopAlign
            ) {
                if a.screen == Screen::TrackerLoopAlign {
                    Screen::TrackerLoop
                } else {
                    Screen::Tracker
                }
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
            a.screen = Screen::TrackerLoop;
            a.status = "loop alignment set".into();
        }
        Action::TrackerStop => {
            a.tracker_stop();
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
        Action::TrackerPageMute => a.toggle_tracker_page_mute(),
        Action::NextTrackerPage => a.switch_tracker_page(),
        Action::PreviewSong => a.preview_song(),
        Action::DeleteSong => a.delete_song(),
        Action::NewPattern => a.new_pattern(),
        Action::ClearPattern => a.choose_pattern_clear(),
        Action::ClearPatternNow => {
            a.confirm_pattern_clear = false;
            a.clear_pattern_now();
        }
        Action::ClonePattern => a.clone_pattern(),
        Action::PreviousOrder => a.move_order(-1),
        Action::NextOrder => a.move_order(1),
        Action::RepeatOrder => a.repeat_order(),
        Action::DeleteOrder => a.delete_order(),
        Action::TrackerEdit => {
            let enabled = a.tracker_mode != TrackerMode::Edit;
            a.set_tracker_edit(enabled);
            a.status = format!("step edit {}", if enabled { "on" } else { "off" });
        }
        Action::TrackerSkip => a.tracker_skip(),
        Action::TrackerErase => a.tracker_erase(),
        Action::TrackerNoteOff => a.tracker_note_off(),
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
        Action::PreviousProgram => a.change_program(-1),
        Action::NextProgram => a.change_program(1),
        Action::TempoDown => a.set_tracker_tempo(a.song.tempo.saturating_sub(1)),
        Action::TempoUp => a.set_tracker_tempo(a.song.tempo.saturating_add(1)),
        Action::AddPage => a.add_tracker_page(),
        Action::EditPageTarget => a.edit_page_target(),
        Action::EditPageChannel => a.edit_page_channel(),
        Action::ConfirmPageManager => a.confirm_page_manager(),
        Action::SaveSong => a.save_song(),
        Action::LoadSong => a.load_song(),
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
    }
    false
}
fn key(code: KeyCode, a: &mut App, state: &Path, tx: &std::sync::mpsc::Sender<MidiEvent>) -> bool {
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
    if a.screen == Screen::TrackerLoop || a.screen == Screen::TrackerLoopAlign {
        let action = match code {
            KeyCode::Up => Some(Action::Up),
            KeyCode::Down => Some(Action::Down),
            KeyCode::Left => Some(Action::LoopOffsetDown),
            KeyCode::Right => Some(Action::LoopOffsetUp),
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
                if let Some(c) = a.tracker_cell_mut() {
                    c.note = Note::Off;
                    c.velocity = None;
                    c.program = None;
                    c.gate = None;
                    if matches!(c.command, Command::Retrigger(_)) {
                        c.command = Command::None;
                    }
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
            KeyCode::Char('r') | KeyCode::Char('R') => {
                a.begin_tracker_recording();
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

const fn pattern_sizes(beats: u8) -> [usize; 5] {
    if beats == 3 {
        [6, 12, 24, 48, 96]
    } else {
        [8, 16, 32, 64, 128]
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
            if a.screen == Screen::Ideas {
                a.idea_selected = a.idea_selected.saturating_sub(1)
            } else if a.screen == Screen::Help {
                a.move_help(-3);
            } else {
                a.selected = a.selected.saturating_sub(3)
            }
        }
        MouseEventKind::ScrollDown => {
            if a.screen == Screen::Ideas {
                a.idea_selected = (a.idea_selected + 1).min(a.ideas.len().saturating_sub(1))
            } else if a.screen == Screen::Help {
                a.move_help(3);
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
            } else if a.screen == Screen::Help && contains(a.hits.list, m.column, m.row) {
                let i = visible_index(a.hits.list, a.help_offset, m.column, m.row).unwrap();
                if i < help::lines(a.hits.list.width as usize).len() {
                    if i == a.help_selected {
                        a.activate_help();
                    } else {
                        a.help_selected = i;
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
        Screen::TrackerPages => draw_tracker_pages(f, a),
        Screen::TrackerTools => draw_tracker_child(f, "FT2 TOOLS", "Pages · Files · Loop · Mute"),
        Screen::TrackerNoob => draw_noob_setup(f, a),
        Screen::TrackerLoop => draw_tracker_loop(f, a),
        Screen::TrackerLoopAlign => draw_tracker_loop_align(f, a),
        Screen::AudioRecorder => draw_audio_recorder(f, a),
    }
    draw_pad_lock(f, a);
    draw_pad_buttons(f, a);
    if a.screen != Screen::Playback {
        draw_status_bar(f, a);
    }
}
fn draw_help<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let header = rect(z.x, z.y, z.width, 1);
    let body = rect(z.x, z.y + 1, z.width, z.height.saturating_sub(4));
    let rows = body.height as usize;
    let lines = help::lines(body.width as usize);
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

fn draw_tracker_loop<B: Backend>(f: &mut Frame<B>, a: &App) {
    let z = f.size();
    let player = a.loop_player.status();
    let selected = a
        .loop_imports
        .get(a.loop_selected)
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "(inbox empty)".into());
    let details = if let Some(settings) = &a.song.audio_loop {
        let interpreted = settings.interpreted_bpm();
        let ratio = f64::from(a.song.tempo) / interpreted.max(0.01);
        let bar_unit = i32::from(a.song.meter.clamp(1, 16));
        let offset_bars = f64::from(settings.offset_beats) / f64::from(bar_unit);
        format!(
            "FT2 WAV LOOP\n{}\n\nSource {:>6.2} BPM  {}\nTarget {:>3} BPM  ratio {:.3}\nRegion beat {} +{}\nOffset {:+.0} bar(s)\nCut {} · meter {}/4\n\n{}  {} / {}\n{} Hz · {}ch\nPitch changes with tempo",
            truncate(
                player.file.as_deref().unwrap_or(&settings.file),
                z.width.saturating_sub(2) as usize
            ),
            settings.source_bpm(),
            settings.interpretation.label(),
            a.song.tempo,
            ratio,
            settings.start_beat,
            settings.length_beats,
            offset_bars,
            if a.loop_edit_bars { "BAR" } else { "BEAT" },
            a.song.meter,
            if player.playing { "PLAY" } else { "STOP" },
            short_time(player.elapsed),
            short_time(player.duration),
            player.source_rate,
            player.source_channels,
        )
    } else {
        format!(
            "FT2 WAV LOOP\n\nInbox: {}\nSelected: {}\n\nTurn encoder to choose\nIMPORT copies to private storage\n\nWAV has no assumed BPM metadata.\nAfter import, enter source BPM.",
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
        let bar_unit = i32::from(a.song.meter.clamp(1, 16));
        let offset_bars = f64::from(settings.offset_beats) / f64::from(bar_unit);
        format!(
            "LOOP ALIGN\n{}\n\nAUTO measures pulse/length\nand snaps length to bars.\n\nBAR- / BAR+\nmove placement by 1 bar.\n\nLength: {} beat(s)\nOffset: {:+.0} bar(s)\nMeter: {}/4\n\nLeft/right also shift.",
            truncate(&settings.file, z.width.saturating_sub(2) as usize),
            settings.length_beats,
            offset_bars,
            a.song.meter,
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
    let state = if a.note_editor.is_some() {
        "CELL"
    } else if a.tracker_recording.is_some() {
        "REC"
    } else if transport.playing {
        "PLAY"
    } else if matches!(a.tracker_mode, TrackerMode::Edit | TrackerMode::Noob) {
        a.tracker_mode.label()
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
        if let Some(pattern) = a.song.patterns.get(&pattern_number) {
            for (screen_row, row_index) in
                (start..(start + rows).min(pattern.rows.len())).enumerate()
            {
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
    }
    let page = &a.song.pages[a.tracker_page];
    let lane = &page.lanes[a.tracker_track];
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
        )
    };
    f.render_widget(
        Paragraph::new(truncate(&footer, z.width as usize))
            .style(Style::default().fg(Color::DarkGray)),
        rect(z.x, z.y + z.height.saturating_sub(4), z.width, 1),
    );
}

fn draw_tracker_program_browser<B: Backend>(f: &mut Frame<B>, a: &App, area: Rect) {
    let page = &a.song.pages[a.tracker_page];
    let selected = a
        .note_editor
        .and_then(|editor| editor.draft.program)
        .unwrap_or(page.program);
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
        render(40, 20, Screen::TrackerPages);
        render(40, 20, Screen::TrackerTools);
        render(40, 20, Screen::TrackerNoob);
        render(40, 20, Screen::TrackerLoop);
        render(40, 20, Screen::TrackerLoopAlign);
        render(40, 20, Screen::AudioRecorder);
    }
    #[test]
    fn renders_smaller_and_tiny_gracefully() {
        render(38, 14, Screen::Presets);
        render(38, 14, Screen::Playback);
        render(38, 14, Screen::Ideas);
        render(38, 14, Screen::Help);
        render(38, 14, Screen::Tracker);
        render(38, 14, Screen::TrackerFiles);
        render(38, 14, Screen::TrackerPages);
        render(38, 14, Screen::TrackerTools);
        render(38, 14, Screen::TrackerNoob);
        render(38, 14, Screen::TrackerLoop);
        render(38, 14, Screen::TrackerLoopAlign);
        render(38, 14, Screen::AudioRecorder);
        render(30, 8, Screen::Presets);
        render(30, 8, Screen::Tracker)
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
        for label in ["OPS", "SOUND", "NAV", "SYS", "RESET", "FINISH", "TAP"] {
            assert!(text.contains(label), "missing {label}: {text}");
        }
        assert!(text.contains("PLY"));
        assert_eq!(a.hits.menu_pages.len(), 4);
        assert_eq!(a.hits.actions.len(), 3);
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
    fn empty_controller_items_are_silent() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Playback;
        a.select_menu_page(1);
        let before = a.status.clone();
        let (tx, rx) = mpsc::channel();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item4, true))
            .unwrap();
        drain(&rx, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.status, before);
        assert!(a.playing.is_none());
        a.screen = Screen::AudioRecorder;
        a.select_menu_page(0);
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
        a.song.pages[0].target = PageTarget::ConfiguredExternal;
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
        a.song.pages[0].bank_msb = 5;
        a.song.pages[0].bank_lsb = 9;
        assert_eq!(
            a.tracker_program_messages(7),
            vec![vec![0xb0, 0, 5], vec![0xb0, 32, 9], vec![0xc0, 7]]
        );
        a.config.external_midi.bank_select = BankSelectMode::Off;
        a.song.pages[0].bank_msb = 0;
        a.song.pages[0].bank_lsb = 0;

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
        tx.send(MidiEvent::Raw(vec![0x90, 60, 100])).unwrap();
        tx.send(MidiEvent::Raw(vec![0x80, 60, 0])).unwrap();
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
        a.song.pages[0].target = PageTarget::ActiveInstrument;
        a.begin_tracker_recording();
        assert!(a.tracker_recording.is_none());
        assert!(a.status.contains("loaded synth stays isolated"));

        a.song.pages[0].target = PageTarget::ConfiguredExternal;
        a.song
            .patterns
            .insert(1, sequencer::Pattern::empty(8, a.song.total_lanes()));
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
    fn encoder_skip_and_paged_erase_and_tempo_actions_remain_available() {
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
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Page1, true))
            .unwrap();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item2, true))
            .unwrap();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Page3, true))
            .unwrap();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item4, true))
            .unwrap();
        drain(&rx, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.song.tempo, tempo + 1);
        assert_eq!(a.tracker_row, (row + 1) % a.tracker_rows());
        assert_eq!(a.song.patterns[&0].rows[row][0].note, Note::Empty);
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
