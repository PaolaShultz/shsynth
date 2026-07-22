use crate::audio_graph::{
    EffectId, EffectKind, InsertRack, ProjectAuxRouting, SendPoint, MAX_AUX_BUSES,
};
use crate::audio_recorder::{AudioRecorder, RecorderStatus, RecorderTrackStatus};
use crate::chord::HeldNotes;
use crate::config::{ExternalMidiConfig, RuntimeConfig};
use crate::control::{parameter_color, CONTROLS, VOLUME_CC};
use crate::device_profile::{DeviceProfile, Registry as DeviceProfiles};
use crate::drum_pattern::{self, DrumPattern};
use crate::engine::{self, Engine, MidiEvent};
use crate::final_bus::{
    BusSource, MASTER_GAIN_MAX_DB, MASTER_GAIN_MIN_DB, SOURCE_GAIN_MAX_DB, SOURCE_GAIN_MIN_DB,
};
use crate::geometry::{contains, rect, visible_index};
use crate::help::{self, HelpKind};
use crate::navigation::{self, Action, MenuContext, Screen, SlotState};
use crate::overlay::{
    self, CloseBehavior, OverlayDraft, OverlayKind, OverlayLauncher, OverlayState, RouteDraft,
    RouteField,
};
use crate::pads::{ControllerLayout, MenuInput, TapTempo};
use crate::performance_meter::{
    self, AudioAvailability, AudioLevel, BarCell, LedState, MeterColor, PerformanceMeter,
    VISIBLE_CPU_CORES,
};
use crate::preset::{BackendKind, Catalog, Preset};
use crate::recording::{self, Recorder, TimedEvent};
use crate::scale::{Scale, ScaleKind};
use crate::sequencer::{
    self, Cell, Command, GestureCapture, Note, PageTarget, SoftwareRoute, Song, LANES_PER_PAGE,
};
use anyhow::{bail, Context, Result};
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
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame, Terminal,
};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::{self, IsTerminal, Stdout};
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

const BUILD_BADGE: &str = if cfg!(debug_assertions) { "DEV" } else { "REL" };
const FIRST_AUX_EFFECT_INDEX: usize = 3;
const COMPRESSOR_GAIN_REDUCTION_LEDS_DB: [f32; 11] =
    [0.5, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 10.0, 12.0, 18.0, 24.0];
// U+25CF is one cell wide in the target TTY font and is the master LED glyph.
const COMPRESSOR_LED_GLYPH: &str = "●";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportIndicator {
    Play,
    Stop,
    Pause,
    Record,
}

const fn transport_glyph(state: TransportIndicator) -> (&'static str, Color) {
    match state {
        TransportIndicator::Play => (">", Color::Green),
        TransportIndicator::Stop => ("■", Color::White),
        TransportIndicator::Pause => ("‖", Color::White),
        TransportIndicator::Record => ("●", Color::Red),
    }
}

fn transport_color(state: TransportIndicator, elapsed: Duration) -> Color {
    let (_, base) = transport_glyph(state);
    if state == TransportIndicator::Record && elapsed.as_millis() / 400 % 2 == 0 {
        Color::LightRed
    } else {
        base
    }
}

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

fn fx_hardware_label(index: usize) -> String {
    format!("K{}", index + 1)
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
    finished: Arc<AtomicBool>,
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
    Engine,
    Instrument,
    MidiOutput,
    Channel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NoteEditorField {
    Destination,
    Channel,
    DefaultProgram,
    BankMsb,
    BankLsb,
    Note,
    Gate,
    Velocity,
    Program,
    Effect,
    EffectParameter,
}

impl NoteEditorField {
    const ALL: [Self; 11] = [
        Self::Destination,
        Self::Channel,
        Self::DefaultProgram,
        Self::BankMsb,
        Self::BankLsb,
        Self::Note,
        Self::Gate,
        Self::Velocity,
        Self::Program,
        Self::Effect,
        Self::EffectParameter,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Destination => "DESTINATION",
            Self::Channel => "MIDI CHANNEL",
            Self::DefaultProgram => "INSTRUMENT",
            Self::BankMsb => "BANK MSB",
            Self::BankLsb => "BANK LSB",
            Self::Note => "NOTE",
            Self::Gate => "GATE",
            Self::Velocity => "VELOCITY",
            Self::Program => "PROGRAM",
            Self::Effect => "EFFECT",
            Self::EffectParameter => "PARAM",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NoteEditor {
    pattern: u16,
    row: usize,
    lane: usize,
    original: Cell,
    original_page: sequencer::Page,
    draft: Cell,
    field: NoteEditorField,
    active: bool,
    edit_original_page: Option<sequencer::Page>,
    edit_original_draft: Option<Cell>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RecordedLane {
    lane: usize,
    start_row: usize,
}

#[derive(Debug)]
struct TrackerRecording {
    pattern: u16,
    order: usize,
    page: usize,
    return_to_play: bool,
    last_row: usize,
    next_lane: usize,
    active_lanes: HashMap<(u8, u8), Vec<RecordedLane>>,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NoteLength {
    Whole,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    SixtyFourth,
    HundredTwentyEighth,
}

impl Default for NoteLength {
    fn default() -> Self {
        Self::Sixteenth
    }
}

impl NoteLength {
    const ALL: [Self; 8] = [
        Self::Whole,
        Self::Half,
        Self::Quarter,
        Self::Eighth,
        Self::Sixteenth,
        Self::ThirtySecond,
        Self::SixtyFourth,
        Self::HundredTwentyEighth,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Whole => "1/1",
            Self::Half => "1/2",
            Self::Quarter => "1/4",
            Self::Eighth => "1/8",
            Self::Sixteenth => "1/16",
            Self::ThirtySecond => "1/32",
            Self::SixtyFourth => "1/64",
            Self::HundredTwentyEighth => "1/128",
        }
    }

    const fn denominator(self) -> usize {
        match self {
            Self::Whole => 1,
            Self::Half => 2,
            Self::Quarter => 4,
            Self::Eighth => 8,
            Self::Sixteenth => 16,
            Self::ThirtySecond => 32,
            Self::SixtyFourth => 64,
            Self::HundredTwentyEighth => 128,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EngineOwner {
    SoftwareSynth,
    Tracker(SoftwareRoute),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrackerEntryInstrument {
    ExistingProject,
    AdoptedPlayer,
    FirstSynthv1,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum IdeaMode {
    #[default]
    Play,
    Record,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum TrackerFilesMode {
    #[default]
    Projects,
    Patterns,
    Drums,
}

#[derive(Clone, Copy)]
struct HomeEntry {
    label: &'static str,
    action: Action,
}

const HOME_ENTRIES: [HomeEntry; 9] = [
    HomeEntry {
        label: "SOFTWARE SYNTHS",
        action: Action::OpenPresets,
    },
    HomeEntry {
        label: "FT2 TRACKER",
        action: Action::OpenTracker,
    },
    HomeEntry {
        label: "RECORDER",
        action: Action::OpenAudioRecorder,
    },
    HomeEntry {
        label: "PERFORMANCE",
        action: Action::OpenMeter,
    },
    HomeEntry {
        label: "MIDI LEARN",
        action: Action::OpenControllerLearn,
    },
    HomeEntry {
        label: "ROUTING",
        action: Action::OpenRouting,
    },
    HomeEntry {
        label: "EFFECTS",
        action: Action::OpenFxRack,
    },
    HomeEntry {
        label: "IDEAS",
        action: Action::OpenIdeas,
    },
    HomeEntry {
        label: "HELP",
        action: Action::OpenHelp,
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControllerLearnReason {
    Offline,
    NoReviewedProfile,
    IncompleteLearnedEncoder,
    UnusableReviewedEncoder,
}

impl ControllerLearnReason {
    const fn detail(self) -> &'static str {
        match self {
            Self::Offline => "Configured controller is offline",
            Self::NoReviewedProfile => "No reviewed profile matches controller",
            Self::IncompleteLearnedEncoder => "Learned master encoder is incomplete",
            Self::UnusableReviewedEncoder => "Reviewed profile cannot use encoder",
        }
    }
}

struct App {
    catalogs: Vec<Catalog>,
    backend_index: usize,
    presets: Vec<Preset>,
    selected: usize,
    offset: usize,
    home_selected: usize,
    home_offset: usize,
    screen: Screen,
    engine: Option<Engine>,
    engine_owner: Option<EngineOwner>,
    engine_state: PathBuf,
    #[cfg(test)]
    tracker_engine_start_override: Option<std::result::Result<(), String>>,
    playing: Option<Preset>,
    values: HashMap<u8, f32>,
    original_values: HashMap<u8, f32>,
    held_notes: HeldNotes,
    status: String,
    controller_fallback: Option<String>,
    audio_fallback: Option<String>,
    tracker_fallback: Option<String>,
    hits: Hits,
    recorder: Recorder,
    idea_mode: IdeaMode,
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
    midi_lifecycle: engine::MidiLifecycle,
    pickup: engine::SharedPickup,
    midi_backend: engine::SharedBackend,
    tracker_route: engine::SharedTrackerRoute,
    controller_profiles: crate::controller_profile::Catalog,
    device_profiles: DeviceProfiles,
    config: RuntimeConfig,
    cpu_temperature: Option<f32>,
    cpu_temperature_read_at: Option<Instant>,
    pad_locked: bool,
    song: Song,
    song_file_stem: Option<String>,
    project_name_input: Option<String>,
    audio_track_name_input: Option<String>,
    song_list: Vec<String>,
    song_selected: usize,
    tracker_order: usize,
    tracker_row: usize,
    tracker_page: usize,
    tracker_track: usize,
    tracker_advance: usize,
    tracker_mode: TrackerMode,
    tracker_noob: bool,
    tracker_recording: Option<TrackerRecording>,
    note_editor: Option<NoteEditor>,
    audition_release_revision: u64,
    tracker_octave: u8,
    note_length: NoteLength,
    noob_scale: Scale,
    playback_noob: bool,
    playback_scale: engine::SharedPlaybackScale,
    tracker_gesture: GestureCapture,
    tracker_gesture_anchor: Option<(usize, usize, usize, usize)>,
    confirm_song_save: Option<String>,
    confirm_routing_defaults: bool,
    routing_defaults: Vec<sequencer::Page>,
    routing_defaults_path: PathBuf,
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
    capture_sources: Vec<String>,
    audio_track_selected: usize,
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
    overlay: Option<OverlayState>,
    menu_page_by_screen: [usize; Screen::COUNT],
    page_select_mode: bool,
    controller_layout: ControllerLayout,
    fx_selection: FxRackSelection,
    fx_parameter: usize,
    fx_add_kind: usize,
    fx_target: usize,
    fx_value_editing: bool,
    fx_edit_original: Option<(InsertRack, ProjectAuxRouting)>,
    fx_type_edit: Option<FxTypeEdit>,
    fx_numeric_input: Option<String>,
    fx_pickup: FxPickup,
    fx_rack_parent: Screen,
    controller_config: engine::SharedControllerConfig,
    learn_mode: engine::SharedLearnMode,
    fx_control_mode: engine::SharedFxControlMode,
    controller_online: bool,
    performance_inputs: Vec<crate::engine::MidiInputState>,
    controller_learn: Option<crate::controller_learn::LearnSession>,
    bus_selected: usize,
    final_recording_last: crate::audio_recorder::FinalMixRecorderStatus,
    performance_meter: PerformanceMeter,
    loop_meter: PerformanceMeter,
    last_mapped_volume: Option<f32>,
    midi_router: Option<engine::MidiRouter>,
    routing: RoutingEditor,
    routing_inputs: Vec<String>,
    routing_outputs: Vec<String>,
    routing_audio_ports: Vec<String>,
}

struct TrackerIo {
    route: engine::SharedTrackerRoute,
    input: engine::SharedTrackerInput,
    playback_scale: engine::SharedPlaybackScale,
    lifecycle: engine::MidiLifecycle,
}

#[derive(Clone)]
struct FxTypeEdit {
    original_rack: InsertRack,
    original_aux: ProjectAuxRouting,
    effect_id: EffectId,
    provisional: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FxRackSelection {
    Effect(EffectId),
    Insert,
}

#[derive(Clone, Copy)]
struct FxCatch {
    target: f32,
    previous: Option<f32>,
    caught: bool,
}

#[derive(Default)]
struct FxPickup {
    controls: HashMap<u8, FxCatch>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RoutingRow {
    Controller,
    ControllerRole,
    Performance,
    ExternalEnabled,
    ExternalOutput,
    DeviceProfile,
    ClockEnabled,
    ClockOutput,
    AudioOutput,
}

impl RoutingRow {
    const ALL: [Self; 9] = [
        Self::Controller,
        Self::ControllerRole,
        Self::Performance,
        Self::ExternalEnabled,
        Self::ExternalOutput,
        Self::DeviceProfile,
        Self::ClockEnabled,
        Self::ClockOutput,
        Self::AudioOutput,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Controller => "CTRL",
            Self::ControllerRole => "MODE",
            Self::Performance => "PERF",
            Self::ExternalEnabled => "MIDI",
            Self::ExternalOutput => "MIDI OUT",
            Self::DeviceProfile => "DEVICE",
            Self::ClockEnabled => "CLOCK",
            Self::ClockOutput => "CLK OUT",
            Self::AudioOutput => "AUDIO",
        }
    }
}

#[derive(Clone)]
struct RoutingDraft {
    config: RuntimeConfig,
    controller: crate::pads::PadConfig,
}

#[derive(Default)]
struct RoutingEditor {
    selected: usize,
    draft: Option<RoutingDraft>,
}

impl FxPickup {
    fn arm(&mut self, targets: impl IntoIterator<Item = (u8, f32)>) {
        self.controls = targets
            .into_iter()
            .map(|(cc, target)| {
                (
                    cc,
                    FxCatch {
                        target: target.clamp(0.0, 1.0),
                        previous: None,
                        caught: false,
                    },
                )
            })
            .collect();
    }

    fn accept(&mut self, cc: u8, value: f32) -> bool {
        let Some(catch) = self.controls.get_mut(&cc) else {
            return false;
        };
        if catch.caught {
            return true;
        }
        let close = (value - catch.target).abs() <= 1.0 / 127.0 + f32::EPSILON;
        let crossed = catch
            .previous
            .is_some_and(|previous| (previous - catch.target) * (value - catch.target) <= 0.0);
        catch.previous = Some(value);
        catch.caught = close || crossed;
        catch.caught
    }
}

struct AvailablePorts {
    playback: Vec<String>,
    capture_sources: Vec<String>,
    midi_outputs: Vec<String>,
}

fn is_tracker_screen(screen: Screen) -> bool {
    matches!(
        screen,
        Screen::Tracker
            | Screen::TrackerFiles
            | Screen::TrackerArrange
            | Screen::TrackerPages
            | Screen::TrackerTools
            | Screen::TrackerLoop
            | Screen::TrackerLoopAlign
    )
}

fn is_fx_screen(screen: Screen) -> bool {
    matches!(screen, Screen::FxRack | Screen::FxEditor)
}

fn take_engine_when_owned<T>(
    engine: &mut Option<T>,
    owner: &mut Option<EngineOwner>,
    matches: impl FnOnce(&EngineOwner) -> bool,
) -> Option<T> {
    if owner.as_ref().is_some_and(matches) {
        *owner = None;
        engine.take()
    } else {
        None
    }
}

fn scheduled_software_route(
    messages: &[sequencer::ScheduledMessage],
) -> Result<Option<SoftwareRoute>> {
    let routes = messages
        .iter()
        .filter(|message| {
            matches!(message.bytes.as_slice(), [status, _, velocity, ..]
                if status & 0xf0 == 0x90 && *velocity > 0)
        })
        .filter_map(|message| match message.target.as_ref()? {
            PageTarget::Software(route) => Some(route.clone()),
            PageTarget::Synthv1(name) => Some(SoftwareRoute::synthv1(name)),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    if routes.len() > 1 {
        bail!(
            "arrangement uses multiple software instruments ({}) · play one instrument per arrangement",
            routes
                .iter()
                .map(|route| format!("{}:{}", route.engine.label(), route.instrument))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(routes.into_iter().next())
}

fn first_letter_index<I, S>(items: I, letter: char) -> Option<usize>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    items.into_iter().position(|item| {
        item.as_ref()
            .trim_start()
            .chars()
            .next()
            .is_some_and(|first| first.eq_ignore_ascii_case(&letter))
    })
}

fn wrapped_index(current: usize, len: usize, direction: i8) -> usize {
    if len == 0 {
        return 0;
    }
    let current = current.min(len - 1);
    match direction.cmp(&0) {
        std::cmp::Ordering::Less => (current + len - 1) % len,
        std::cmp::Ordering::Greater => (current + 1) % len,
        std::cmp::Ordering::Equal => current,
    }
}

fn wrapped_offset(current: usize, len: usize, amount: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let current = current.min(len - 1) as isize;
    (current + amount).rem_euclid(len as isize) as usize
}

fn cycle_text_choice(current: &str, live: &[String], include_none: bool, direction: i8) -> String {
    let mut choices = Vec::new();
    if include_none {
        choices.push(String::new());
    }
    if !current.is_empty()
        && !live
            .iter()
            .any(|choice| choice.eq_ignore_ascii_case(current))
    {
        choices.push(current.to_owned());
    }
    for choice in live {
        if !choices
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(choice))
        {
            choices.push(choice.clone());
        }
    }
    let current = choices
        .iter()
        .position(|choice| choice.eq_ignore_ascii_case(current))
        .unwrap_or(0);
    choices
        .get(wrapped_index(current, choices.len(), direction))
        .cloned()
        .unwrap_or_default()
}

fn canonicalize_routing_draft(
    draft: &mut RoutingDraft,
    inputs: &[String],
    outputs: &[String],
) -> Result<()> {
    let canonical = |names: &[String], wanted: &str, description: &str| -> Result<String> {
        let index = crate::midi_endpoint::matching_index(names, wanted, description)?;
        Ok(crate::midi_endpoint::stable_identity(&names[index]))
    };
    if let Some(input) = draft.controller.input_match.as_mut() {
        *input = canonical(inputs, input, "controller MIDI input")?;
        draft.config.midi_input_matches = vec![input.clone()];
    } else {
        draft.config.midi_input_matches.clear();
    }
    for input in &mut draft.config.midi_performance_input_matches {
        *input = canonical(inputs, input, "performance MIDI input")?;
    }
    if !draft.config.external_midi.output_match.is_empty() {
        draft.config.external_midi.output_match = canonical(
            outputs,
            &draft.config.external_midi.output_match,
            "MIDI output",
        )
        .unwrap_or_else(|_| {
            crate::midi_endpoint::stable_identity(&draft.config.external_midi.output_match)
        });
    }
    if !draft.config.controller_clock.output_match.is_empty() {
        match canonical(
            outputs,
            &draft.config.controller_clock.output_match,
            "controller clock output",
        ) {
            Ok(output) => draft.config.controller_clock.output_match = output,
            Err(error) if draft.config.controller_clock.enabled => return Err(error),
            Err(_) => {}
        }
    }
    Ok(())
}

fn validate_routing_draft(draft: &RoutingDraft, state: &Path) -> Result<()> {
    draft.controller.validate()?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "shr-routing-validate-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&directory)?;
    let runtime = directory.join("shsynth.conf");
    let controller = directory.join("controller.conf");
    let result = draft
        .config
        .save(&runtime)
        .and_then(|_| RuntimeConfig::load(&runtime).map(|_| ()))
        .and_then(|_| draft.controller.save(&controller))
        .and_then(|_| crate::pads::PadConfig::load(&controller).map(|_| ()));
    let _ = fs::remove_file(&runtime);
    let _ = fs::remove_file(&controller);
    let _ = fs::remove_dir(&directory);
    result.with_context(|| format!("validate complete candidate for {}", state.display()))
}

fn restore_config_file(path: &Path, contents: Option<&[u8]>) {
    match contents {
        Some(contents) => {
            let _ = crate::fsutil::atomic_write(path, contents);
        }
        None if path.is_file() => {
            let _ = fs::remove_file(path);
        }
        None => {}
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RoutingTransactionStage {
    Save,
    Activate,
}

fn persist_routing_transaction<F>(
    runtime_path: &Path,
    controller_path: &Path,
    draft: &RoutingDraft,
    activate: F,
) -> std::result::Result<(), (RoutingTransactionStage, anyhow::Error)>
where
    F: FnOnce() -> Result<()>,
{
    let old_runtime = fs::read(runtime_path).ok();
    let old_controller = fs::read(controller_path).ok();
    let save = crate::controller_learn::backup(runtime_path)
        .and_then(|_| crate::controller_learn::backup(controller_path))
        .and_then(|_| draft.config.save(runtime_path))
        .and_then(|_| draft.controller.save(controller_path));
    if let Err(error) = save {
        restore_config_file(runtime_path, old_runtime.as_deref());
        restore_config_file(controller_path, old_controller.as_deref());
        return Err((RoutingTransactionStage::Save, error));
    }
    if let Err(error) = activate() {
        restore_config_file(runtime_path, old_runtime.as_deref());
        restore_config_file(controller_path, old_controller.as_deref());
        return Err((RoutingTransactionStage::Activate, error));
    }
    Ok(())
}

fn drum_home_lane(note: u8) -> Option<usize> {
    match note {
        35 | 36 => Some(0), // Acoustic Bass Drum / Bass Drum 1
        38 | 40 => Some(1), // Acoustic Snare / Electric Snare
        _ => None,
    }
}

fn previous_drum_lane(
    pattern: &sequencer::Pattern,
    row_index: usize,
    page_index: usize,
    matches: impl Fn(u8) -> bool,
) -> Option<usize> {
    let page_start = page_index.checked_mul(LANES_PER_PAGE)?;
    pattern.rows.iter().take(row_index).rev().find_map(|row| {
        (0..LANES_PER_PAGE).find(|lane| {
            row.get(page_start + lane)
                .is_some_and(|cell| matches!(cell.note, Note::On(note) if matches(note)))
        })
    })
}

fn drum_entry_lanes(
    pattern: &sequencer::Pattern,
    row_index: usize,
    page_index: usize,
    notes: &[(u8, u8)],
) -> Vec<Option<usize>> {
    let Some(row) = pattern.rows.get(row_index) else {
        return vec![None; notes.len()];
    };
    let Some(page_start) = page_index.checked_mul(LANES_PER_PAGE) else {
        return vec![None; notes.len()];
    };
    let mut claimed = [false; LANES_PER_PAGE];
    let mut assignments = Vec::with_capacity(notes.len());

    for &(note, _) in notes {
        let exact_history = previous_drum_lane(pattern, row_index, page_index, |old| old == note);
        let family_history = drum_home_lane(note).and_then(|family| {
            previous_drum_lane(pattern, row_index, page_index, |old| {
                old != note && drum_home_lane(old) == Some(family)
            })
        });
        let current_match = (0..LANES_PER_PAGE).find(|lane| {
            row.get(page_start + lane)
                .is_some_and(|cell| cell.note == Note::On(note))
        });

        let mut candidates = Vec::with_capacity(LANES_PER_PAGE + 4);
        candidates.extend(current_match.map(|lane| (lane, true)));
        candidates.extend(exact_history.map(|lane| (lane, true)));
        candidates.extend(family_history.map(|lane| (lane, true)));
        candidates.extend(drum_home_lane(note).map(|lane| (lane, true)));
        // Leave the first two columns open for a new kick and snare when the
        // Pattern has not established a home for this drum yet.
        candidates.extend([2, 3, 0, 1].map(|lane| (lane, false)));

        let lane = candidates
            .into_iter()
            .find(|&(lane, may_replace_note_off)| {
                if lane >= LANES_PER_PAGE || claimed[lane] {
                    return false;
                }
                row.get(page_start + lane).is_some_and(|cell| {
                    *cell == Cell::default()
                        || cell.note == Note::On(note)
                        || (may_replace_note_off && cell.note == Note::Off)
                })
            })
            .map(|(lane, _)| lane);
        if let Some(lane) = lane {
            claimed[lane] = true;
        }
        assignments.push(lane);
    }

    assignments
}

fn write_step_note(cell: &mut Cell, note: u8, velocity: u8) {
    if cell.note == Note::On(note) {
        cell.velocity = Some(velocity);
    } else {
        *cell = Cell {
            note: Note::On(note),
            velocity: Some(velocity),
            ..Cell::default()
        };
    }
}

impl App {
    // Keep construction dependencies explicit at the one application assembly
    // boundary; hiding them in another bag type would not simplify ownership.
    #[allow(clippy::too_many_arguments)]
    fn new(
        catalogs: &[Catalog],
        midi_output: engine::SharedOutput,
        pickup: engine::SharedPickup,
        midi_backend: engine::SharedBackend,
        tracker_io: TrackerIo,
        config: RuntimeConfig,
        available_ports: AvailablePorts,
        engine_state: PathBuf,
        routing_defaults_path: PathBuf,
    ) -> Self {
        let backend_index = catalogs
            .iter()
            .position(|catalog| catalog.backend == BackendKind::Synthv1)
            .unwrap_or(0);
        let presets = catalogs
            .get(backend_index)
            .map(|catalog| catalog.presets.clone())
            .unwrap_or_default();
        let controller_profiles = crate::controller_profile::Catalog::discover();
        let device_profiles = DeviceProfiles::discover();
        let first_synthv1 = catalogs
            .iter()
            .find(|catalog| catalog.backend == BackendKind::Synthv1)
            .and_then(|catalog| catalog.presets.first())
            .map(|preset| preset.name.as_str())
            .unwrap_or("Unavailable synthv1 preset");
        let factory_routing = sequencer::factory_routing_pages(first_synthv1);
        let mut routing_defaults =
            sequencer::load_routing_defaults(&routing_defaults_path, &factory_routing)
                .unwrap_or(factory_routing);
        let mut defaults_song = Song::new_with_pages(&config.external_midi, routing_defaults);
        sequencer::upgrade_legacy_synth_routes(&mut defaults_song, first_synthv1);
        routing_defaults = defaults_song.patterns.remove(&0).unwrap().pages;
        let song = Song::new_with_pages(&config.external_midi, routing_defaults.clone());
        let transport_clock = Arc::new(crate::loop_player::TransportClock::new(
            &config.controller_clock,
            config.external_midi.default_tempo,
        ));
        let sequencer = sequencer::Sequencer::start_with_clock(
            &config.external_midi,
            Arc::clone(&midi_output),
            Arc::clone(&transport_clock),
        );
        let tracker_live_input = sequencer.live_input();
        if let Ok(mut input) = tracker_io.input.lock() {
            *input = Some(tracker_live_input.clone());
        }
        let audio_recorder = AudioRecorder::new(
            config.capture.clone(),
            available_ports.capture_sources.clone(),
        );
        let resolved_audio = config.resolve_audio_route(&available_ports.playback);
        let mut loop_config = config.loop_player.clone();
        if resolved_audio.outputs.len() == 2 {
            loop_config.outputs = resolved_audio.outputs.clone();
        }
        let loop_player = crate::loop_player::LoopPlayer::new(&loop_config, transport_clock);
        let managed_outputs = [
            config.midi_output_match.to_ascii_lowercase(),
            config
                .yoshimi
                .backend
                .midi_output_match
                .to_ascii_lowercase(),
            config
                .fluidsynth
                .backend
                .midi_output_match
                .to_ascii_lowercase(),
        ];
        let available_page_outputs = available_ports
            .midi_outputs
            .into_iter()
            .filter(|name| {
                let name = name.to_ascii_lowercase();
                !managed_outputs
                    .iter()
                    .any(|needle| !needle.is_empty() && name.contains(needle))
            })
            .collect();
        let loop_imports = crate::loop_player::list_wavs(&config.loop_player.import_directory);
        Self {
            catalogs: catalogs.to_vec(),
            backend_index,
            presets,
            selected: 0,
            offset: 0,
            home_selected: 0,
            home_offset: 0,
            screen: Screen::Home,
            engine: None,
            engine_owner: None,
            engine_state,
            #[cfg(test)]
            tracker_engine_start_override: Some(Ok(())),
            playing: None,
            values: HashMap::new(),
            original_values: HashMap::new(),
            held_notes: HeldNotes::default(),
            status: "Ready".into(),
            controller_fallback: None,
            audio_fallback: resolved_audio.notice,
            tracker_fallback: None,
            hits: Hits::default(),
            recorder: Recorder::default(),
            idea_mode: IdeaMode::Play,
            last: vec![],
            playback: None,
            tap: TapTempo::default(),
            ideas: recording::list(&recording::ideas_dir()).unwrap_or_default(),
            idea_selected: 0,
            idea_offset: 0,
            help_selected: 0,
            help_offset: 0,
            help_previous: Screen::Home,
            web_help: None,
            web_help_status: String::new(),
            web_help_enabled: true,
            confirm_delete: None,
            confirm_load: None,
            midi_output,
            midi_lifecycle: tracker_io.lifecycle,
            pickup,
            midi_backend,
            tracker_route: tracker_io.route,
            playback_scale: tracker_io.playback_scale,
            controller_profiles,
            device_profiles,
            config,
            cpu_temperature: None,
            cpu_temperature_read_at: None,
            pad_locked: false,
            song,
            song_file_stem: None,
            project_name_input: None,
            audio_track_name_input: None,
            song_list: sequencer::list(&sequencer::songs_dir()),
            song_selected: 0,
            tracker_order: 0,
            tracker_row: 0,
            tracker_page: 0,
            tracker_track: 0,
            tracker_advance: 1,
            tracker_mode: TrackerMode::Play,
            tracker_noob: false,
            tracker_recording: None,
            note_editor: None,
            audition_release_revision: 0,
            tracker_octave: 4,
            note_length: NoteLength::default(),
            noob_scale: Scale::default(),
            playback_noob: false,
            tracker_gesture: GestureCapture::default(),
            tracker_gesture_anchor: None,
            confirm_song_save: None,
            confirm_routing_defaults: false,
            routing_defaults,
            routing_defaults_path,
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
            available_page_outputs,
            page_target_selected: 0,
            page_channel_draft: 0,
            audio_recorder,
            capture_sources: available_ports.capture_sources,
            audio_track_selected: 0,
            loop_player,
            loop_imports,
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
            overlay: None,
            menu_page_by_screen: [0; Screen::COUNT],
            page_select_mode: false,
            controller_layout: ControllerLayout::Eight,
            fx_selection: FxRackSelection::Insert,
            fx_parameter: 0,
            fx_add_kind: 0,
            fx_target: 0,
            fx_value_editing: false,
            fx_edit_original: None,
            fx_type_edit: None,
            fx_numeric_input: None,
            fx_pickup: FxPickup::default(),
            fx_rack_parent: Screen::Home,
            controller_config: Arc::new(std::sync::RwLock::new(crate::pads::PadConfig::default())),
            learn_mode: Arc::new(AtomicBool::new(false)),
            fx_control_mode: Arc::new(AtomicBool::new(false)),
            controller_online: false,
            performance_inputs: Vec::new(),
            controller_learn: None,
            bus_selected: 0,
            final_recording_last: crate::audio_recorder::FinalMixRecorderStatus::default(),
            performance_meter: PerformanceMeter::default(),
            loop_meter: PerformanceMeter::default(),
            last_mapped_volume: None,
            midi_router: None,
            routing: RoutingEditor::default(),
            routing_inputs: Vec::new(),
            routing_outputs: Vec::new(),
            routing_audio_ports: available_ports.playback,
        }
    }

    fn fallback_notice(&self) -> Option<String> {
        let notices = self
            .controller_fallback
            .iter()
            .chain(self.audio_fallback.iter())
            .chain(self.tracker_fallback.iter())
            .cloned()
            .collect::<Vec<_>>();
        (!notices.is_empty()).then(|| notices.join(" · "))
    }

    fn move_home(&mut self, direction: i8) {
        self.home_selected = wrapped_index(self.home_selected, HOME_ENTRIES.len(), direction);
    }

    fn ensure_home_visible(&mut self, rows: usize) {
        self.home_offset = self
            .home_offset
            .min(HOME_ENTRIES.len().saturating_sub(rows.max(1)));
        if self.home_selected < self.home_offset {
            self.home_offset = self.home_selected;
        } else if self.home_selected >= self.home_offset.saturating_add(rows) {
            self.home_offset = self.home_selected + 1 - rows.max(1);
        }
    }

    fn controller_learn_reason(&self) -> Option<ControllerLearnReason> {
        let controller = self.controller_config.read().ok()?;
        let input = controller
            .input_match
            .as_deref()
            .filter(|input| !input.trim().is_empty())?;
        if !self.controller_online {
            return Some(ControllerLearnReason::Offline);
        }
        let usable_encoder = controller.encoder_relative_cc.is_some()
            && (controller.encoder_press_cc.is_some() || controller.encoder_press_note.is_some());
        if controller.profile.as_deref() == Some("learned") {
            return (!usable_encoder).then_some(ControllerLearnReason::IncompleteLearnedEncoder);
        }
        let reviewed = self
            .controller_profiles
            .matching(input)
            .is_some_and(|profile| controller.profile.as_deref() == Some(profile.id.as_str()));
        if !reviewed {
            return Some(ControllerLearnReason::NoReviewedProfile);
        }
        (!usable_encoder).then_some(ControllerLearnReason::UnusableReviewedEncoder)
    }

    fn recommend_controller_learn_on_home(&mut self) {
        if self.controller_learn_reason().is_some() {
            self.home_selected = HOME_ENTRIES
                .iter()
                .position(|entry| entry.action == Action::OpenControllerLearn)
                .unwrap_or(0);
            self.home_offset = 0;
        }
    }

    fn jump_to_letter(&mut self, letter: char) -> bool {
        let letter = letter.to_ascii_lowercase();
        let selected = match self.screen {
            Screen::Presets => first_letter_index(
                self.presets.iter().map(|preset| preset.name.as_str()),
                letter,
            )
            .map(|index| {
                self.selected = index;
            }),
            Screen::Ideas => first_letter_index(self.ideas.iter(), letter).map(|index| {
                self.idea_selected = index;
            }),
            Screen::TrackerFiles if self.tracker_files_mode == TrackerFilesMode::Projects => {
                first_letter_index(self.song_list.iter(), letter).map(|index| {
                    self.song_selected = index;
                })
            }
            Screen::TrackerFiles if self.tracker_files_mode == TrackerFilesMode::Drums => {
                let filtered = self.filtered_drum_indices();
                first_letter_index(
                    filtered
                        .iter()
                        .map(|index| self.drum_patterns[*index].name.as_str()),
                    letter,
                )
                .map(|position| {
                    self.drum_pattern_selected = filtered[position];
                })
            }
            Screen::TrackerLoop if self.loop_library_mode => first_letter_index(
                self.loop_library.iter().map(|entry| entry.file.as_str()),
                letter,
            )
            .map(|index| {
                self.loop_library_selected = index;
            }),
            Screen::TrackerLoop => first_letter_index(
                self.loop_imports.iter().map(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default()
                }),
                letter,
            )
            .map(|index| {
                self.loop_selected = index;
            }),
            _ => None,
        };
        selected.is_some()
    }

    fn keyboard_modal_active(&self) -> bool {
        self.audio_track_name_input.is_some()
            || self.project_name_input.is_some()
            || self.controller_learn.is_some()
            || self.note_editor.is_some()
            || self.tracker_recording.is_some()
            || self.fx_type_edit.is_some()
            || self.fx_numeric_input.is_some()
            || self.fx_value_editing
            || self.confirm_delete.is_some()
            || self.confirm_load.is_some()
            || self.confirm_song_save.is_some()
            || self.confirm_new_project
            || self.confirm_loop_remove
            || self.confirm_song_delete.is_some()
            || self.confirm_pattern_clear
            || self.confirm_pattern_paste_over.is_some()
            || self.confirm_pattern_delete.is_some()
            || self.confirm_drum_pattern_delete.is_some()
            || self.confirm_loop_delete.is_some()
            || (self.screen == Screen::TrackerPages
                && self.page_manager_mode != PageManagerMode::Pages)
    }

    fn begin_controller_learn(&mut self) {
        if !self.controller_online {
            self.status = "MIDI Learn unavailable · connect the selected controller first".into();
            return;
        }
        let input = self
            .controller_config
            .read()
            .ok()
            .and_then(|config| config.input_match.clone());
        let Some(input) = input else {
            self.status = "MIDI Learn unavailable · select a controller input in setup".into();
            return;
        };
        self.controller_learn = Some(crate::controller_learn::LearnSession::new(&input));
        self.learn_mode.store(true, Ordering::Relaxed);
        self.status = "MIDI Learn active · controller messages isolated from instruments".into();
    }

    fn refresh_live_midi_connections(&mut self) {
        if !self.config.midi_autoconnect {
            self.controller_online = false;
            self.performance_inputs.clear();
            return;
        }
        let pads = self
            .controller_config
            .read()
            .map(|pads| pads.clone())
            .unwrap_or_default();
        match crate::engine::inspect_midi_inputs(&self.config, &pads) {
            Ok(availability) => {
                self.controller_online = availability.controller_available();
                self.performance_inputs = availability.performance;
            }
            Err(_) => {
                self.controller_online = false;
                self.performance_inputs.clear();
            }
        }
    }

    fn open_routing_editor(&mut self) {
        self.routing = RoutingEditor::default();
        self.refresh_routing_discovery();
        self.set_screen(Screen::Routing);
        self.status = "Routing · turn to browse · press to edit".into();
    }

    fn refresh_routing_discovery(&mut self) {
        self.routing_inputs = crate::controller_learn::input_names().unwrap_or_default();
        self.routing_outputs =
            sequencer::available_midi_outputs(&self.config.external_midi.client_name)
                .unwrap_or_default();
        for names in [&mut self.routing_inputs, &mut self.routing_outputs] {
            for name in names.iter_mut() {
                *name = crate::midi_endpoint::stable_identity(name);
            }
            names.sort_by_key(|name| name.to_ascii_lowercase());
            names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        }
        let managed = [
            self.config.midi_output_match.to_ascii_lowercase(),
            self.config
                .yoshimi
                .backend
                .midi_output_match
                .to_ascii_lowercase(),
            self.config
                .fluidsynth
                .backend
                .midi_output_match
                .to_ascii_lowercase(),
        ];
        self.available_page_outputs = self
            .routing_outputs
            .iter()
            .filter(|name| {
                let name = name.to_ascii_lowercase();
                !managed
                    .iter()
                    .any(|needle| !needle.is_empty() && name.contains(needle))
            })
            .cloned()
            .collect();
        self.routing_audio_ports = engine::jack_ports();
        self.refresh_live_midi_connections();
        self.refresh_page_targets();
    }

    fn routing_row(&self) -> RoutingRow {
        RoutingRow::ALL[self.routing.selected.min(RoutingRow::ALL.len() - 1)]
    }

    fn move_routing(&mut self, direction: i8) {
        if self.routing.draft.is_some() {
            self.adjust_routing_draft(direction);
        } else {
            self.routing.selected =
                wrapped_index(self.routing.selected, RoutingRow::ALL.len(), direction);
            self.status = format!("{} selected · press to edit", self.routing_row().label());
        }
    }

    fn begin_routing_edit(&mut self) {
        if self.routing.draft.is_some() {
            return;
        }
        let controller = self
            .controller_config
            .read()
            .map(|controller| controller.clone())
            .unwrap_or_default();
        self.routing.draft = Some(RoutingDraft {
            config: self.config.clone(),
            controller,
        });
        self.status = format!(
            "{} EDIT · turn · press confirms",
            self.routing_row().label()
        );
    }

    fn cancel_routing_edit(&mut self) -> bool {
        if self.routing.draft.take().is_some() {
            self.status = "Routing edit cancelled · active route unchanged".into();
            true
        } else {
            false
        }
    }

    fn adjust_routing_draft(&mut self, direction: i8) {
        let row = self.routing_row();
        let input_choices = self.routing_inputs.clone();
        let output_choices = self.routing_outputs.clone();
        let audio_choices = self.routing_audio_choices();
        let profile_choices = self
            .device_profiles
            .profiles()
            .map(|profile| profile.id.clone())
            .chain(std::iter::once("raw-midi".into()))
            .collect::<Vec<_>>();
        let Some(draft) = self.routing.draft.as_mut() else {
            return;
        };
        match row {
            RoutingRow::Controller => {
                let current = draft.controller.input_match.as_deref().unwrap_or("");
                let choice = cycle_text_choice(current, &input_choices, true, direction);
                draft.controller.input_match = (!choice.is_empty()).then_some(choice.clone());
                draft.config.midi_input_matches =
                    (!choice.is_empty()).then_some(choice).into_iter().collect();
                draft.config.midi_autoconnect = draft.controller.input_match.is_some()
                    || !draft.config.midi_performance_input_matches.is_empty();
            }
            RoutingRow::ControllerRole => {
                draft.config.midi_controller_musical_input =
                    !draft.config.midi_controller_musical_input;
            }
            RoutingRow::Performance => {
                let current = draft
                    .config
                    .midi_performance_input_matches
                    .first()
                    .map(String::as_str)
                    .unwrap_or("");
                let choice = cycle_text_choice(current, &input_choices, true, direction);
                draft.config.midi_performance_input_matches =
                    (!choice.is_empty()).then_some(choice).into_iter().collect();
                draft.config.midi_autoconnect = draft.controller.input_match.is_some()
                    || !draft.config.midi_performance_input_matches.is_empty();
            }
            RoutingRow::ExternalEnabled => {
                draft.config.external_midi.enabled = !draft.config.external_midi.enabled;
            }
            RoutingRow::ExternalOutput => {
                let choice = cycle_text_choice(
                    &draft.config.external_midi.output_match,
                    &output_choices,
                    true,
                    direction,
                );
                draft.config.external_midi.output_match = choice;
            }
            RoutingRow::DeviceProfile => {
                let choice = cycle_text_choice(
                    &draft.config.external_midi.profile,
                    &profile_choices,
                    false,
                    direction,
                );
                draft.config.external_midi.profile = choice;
            }
            RoutingRow::ClockEnabled => {
                draft.config.controller_clock.enabled = !draft.config.controller_clock.enabled;
            }
            RoutingRow::ClockOutput => {
                draft.config.controller_clock.output_match = cycle_text_choice(
                    &draft.config.controller_clock.output_match,
                    &output_choices,
                    true,
                    direction,
                );
            }
            RoutingRow::AudioOutput => {
                if !audio_choices.is_empty() {
                    let current = audio_choices
                        .iter()
                        .position(|pair| pair.as_slice() == draft.config.audio_outputs.as_slice())
                        .unwrap_or(0);
                    draft.config.audio_outputs = audio_choices
                        [wrapped_index(current, audio_choices.len(), direction)]
                    .clone();
                    draft.config.loop_player.outputs = draft.config.audio_outputs.clone();
                    draft.config.audio_autoconnect = true;
                }
            }
        }
        self.status = format!("{} EDIT · draft only", row.label());
    }

    fn routing_audio_choices(&self) -> Vec<Vec<String>> {
        let mut choices = Vec::new();
        if self.config.audio_outputs.len() == 2 {
            choices.push(self.config.audio_outputs.clone());
        }
        let mut ports = self
            .routing_audio_ports
            .iter()
            .filter(|port| {
                let lower = port.to_ascii_lowercase();
                lower.contains("playback") || lower.contains("output") || lower.contains("out_")
            })
            .cloned()
            .collect::<Vec<_>>();
        ports.sort();
        for pair in ports.chunks_exact(2) {
            if pair[0].split_once(':').map(|part| part.0)
                == pair[1].split_once(':').map(|part| part.0)
                && !choices.iter().any(|choice| choice.as_slice() == pair)
            {
                choices.push(pair.to_vec());
            }
        }
        choices
    }

    fn confirm_routing_edit(&mut self, state: &Path) {
        let Some(mut draft) = self.routing.draft.take() else {
            self.begin_routing_edit();
            return;
        };
        if let Err(error) =
            canonicalize_routing_draft(&mut draft, &self.routing_inputs, &self.routing_outputs)
        {
            self.status = format!("Routing invalid · {error}");
            self.routing.draft = Some(draft);
            return;
        }
        if let Err(error) = validate_routing_draft(&draft, state) {
            self.status = format!("Routing invalid · {error:#}");
            self.routing.draft = Some(draft);
            return;
        }

        let runtime_path = state.join("shsynth.conf");
        let controller_path = state.join("controller.conf");
        let old_config = self.config.clone();
        let old_controller = self
            .controller_config
            .read()
            .map(|controller| controller.clone())
            .unwrap_or_default();
        let audio_changed = draft.config.audio_outputs != old_config.audio_outputs;
        let clock_changed = draft.config.controller_clock != old_config.controller_clock;

        let mut activated_availability = None;
        let transaction =
            persist_routing_transaction(&runtime_path, &controller_path, &draft, || {
                if let Ok(mut controller) = self.controller_config.write() {
                    *controller = draft.controller.clone();
                }
                activated_availability = self
                    .midi_router
                    .as_mut()
                    .map(|router| router.reconfigure_inputs(&draft.config))
                    .transpose()?;
                Ok(())
            });
        match transaction {
            Ok(()) => {
                if let Some(availability) = activated_availability {
                    self.controller_online = availability.controller_available();
                    self.performance_inputs = availability.performance;
                }
                self.config = draft.config;
                self.controller_layout = draft.controller.layout;
                self.refresh_routing_discovery();
                self.status = match (audio_changed, clock_changed) {
                    (true, true) => "Routing saved · AUDIO+CLOCK NEXT START".into(),
                    (true, false) => "Routing saved · AUDIO NEXT START".into(),
                    (false, true) => "Routing saved · CLOCK NEXT START".into(),
                    (false, false) => "Routing saved · live inputs active".into(),
                };
            }
            Err((stage, error)) => {
                if let Ok(mut controller) = self.controller_config.write() {
                    *controller = old_controller;
                }
                if stage == RoutingTransactionStage::Activate {
                    if let Some(router) = self.midi_router.as_mut() {
                        let _ = router.reconfigure_inputs(&old_config);
                    }
                }
                self.config = old_config;
                self.refresh_routing_discovery();
                self.status = match stage {
                    RoutingTransactionStage::Save => format!("Routing save failed · {error:#}"),
                    RoutingTransactionStage::Activate => {
                        format!("Routing activation failed · rolled back · {error:#}")
                    }
                };
            }
        }
    }

    fn cancel_controller_learn(&mut self) {
        if self
            .controller_learn
            .as_ref()
            .is_some_and(crate::controller_learn::LearnSession::save_committed)
        {
            self.finish_saved_controller_learn();
            return;
        }
        self.learn_mode.store(false, Ordering::Relaxed);
        self.controller_learn = None;
        self.status = "MIDI Learn cancelled · previous controller mapping kept".into();
    }

    fn save_controller_learn(&mut self, state: &Path, wait_for_release: bool) {
        let Some(session) = self.controller_learn.as_ref() else {
            return;
        };
        let config = match session.validated_config() {
            Ok(config) => config,
            Err(error) => {
                self.status = format!("MIDI Learn validation: {error:#}");
                if wait_for_release {
                    if let Some(session) = self.controller_learn.as_mut() {
                        session.mark_save_result(false);
                    }
                }
                return;
            }
        };
        let path = state.join("controller.conf");
        if let Err(error) = crate::controller_learn::backup(&path).and_then(|_| config.save(&path))
        {
            self.status = format!("MIDI Learn save failed: {error:#}");
            if wait_for_release {
                if let Some(session) = self.controller_learn.as_mut() {
                    session.mark_save_result(false);
                }
            }
            return;
        }
        if let Ok(mut active) = self.controller_config.write() {
            *active = config.clone();
        }
        self.controller_layout = config.layout;
        if wait_for_release {
            if let Some(session) = self.controller_learn.as_mut() {
                session.mark_save_result(true);
            }
            self.status = format!(
                "controller profile saved atomically · release encoder · {}",
                path.display()
            );
            return;
        }
        self.learn_mode.store(false, Ordering::Relaxed);
        self.controller_learn = None;
        self.status = format!("controller profile saved atomically · {}", path.display());
    }

    fn finish_saved_controller_learn(&mut self) {
        self.learn_mode.store(false, Ordering::Relaxed);
        self.controller_learn = None;
        self.status = "controller profile saved and activated".into();
    }

    fn receive_controller_learn(
        &mut self,
        received: Instant,
        message: &[u8],
    ) -> crate::controller_learn::LearnAction {
        self.controller_learn
            .as_mut()
            .map_or(crate::controller_learn::LearnAction::None, |session| {
                session.receive(message, received)
            })
    }

    fn menu_context(&self) -> MenuContext {
        if self.confirm_routing_defaults {
            MenuContext::RoutingDefaults
        } else if self.screen == Screen::FxRack && self.fx_type_edit.is_some() {
            MenuContext::FxType
        } else if self.screen == Screen::FxRack && self.selected_effect_id().is_none() {
            MenuContext::FxEmpty
        } else if self.screen == Screen::Tracker && self.note_editor.is_some() {
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
                PageManagerMode::Target
                | PageManagerMode::Engine
                | PageManagerMode::Instrument
                | PageManagerMode::MidiOutput => MenuContext::PageTarget,
                PageManagerMode::Channel => MenuContext::PageChannel,
                PageManagerMode::Pages => MenuContext::Normal,
            }
        } else {
            MenuContext::Normal
        }
    }

    fn overlay_pattern_locations(&self) -> Vec<(u16, usize)> {
        let mut locations = Vec::new();
        for (order, pattern) in self.song.order.iter().copied().enumerate() {
            if !locations.iter().any(|(candidate, _)| *candidate == pattern) {
                locations.push((pattern, order));
            }
        }
        locations
    }

    fn overlay_row_count_for(&self, overlay: &OverlayState) -> usize {
        match overlay.kind {
            // MIDI pages contribute four selectable columns. The Project-wide
            // loop player is the final musician-facing FT2 page and contributes
            // one row of its own, followed by the page manager launcher.
            OverlayKind::TrackerPage => self.current_pages().len() * LANES_PER_PAGE + 2,
            OverlayKind::TrackerPattern => self.overlay_pattern_locations().len() + 2,
            OverlayKind::TrackerSong => self.song.order.len() + 3,
            OverlayKind::TrackerRoute => RouteField::ROWS,
            OverlayKind::TrackerPatternLength => pattern_length_choices().len(),
            OverlayKind::TrackerNoteLength => NoteLength::ALL.len(),
            OverlayKind::TrackerAdvance => 33,
            OverlayKind::LoopLibrary => self.loop_imports.len() + self.loop_library.len(),
            OverlayKind::MixEffects => MAX_AUX_BUSES + 2,
        }
    }

    fn overlay_row_count(&self) -> usize {
        self.overlay
            .as_ref()
            .map_or(0, |overlay| self.overlay_row_count_for(overlay))
    }

    fn open_overlay(&mut self, action: Action) {
        let Some(kind) = OverlayKind::from_action(action) else {
            return;
        };
        if self.overlay.is_some()
            || (self.keyboard_modal_active() && kind != OverlayKind::TrackerPatternLength)
        {
            return;
        }
        let caller = self.screen;
        let allowed = match kind {
            OverlayKind::TrackerPage
            | OverlayKind::TrackerPattern
            | OverlayKind::TrackerSong
            | OverlayKind::TrackerRoute => caller == Screen::Tracker,
            OverlayKind::TrackerNoteLength | OverlayKind::TrackerAdvance => {
                caller == Screen::Tracker && self.tracker_mode == TrackerMode::Edit
            }
            OverlayKind::TrackerPatternLength => {
                caller == Screen::TrackerFiles && self.confirm_pattern_clear
            }
            OverlayKind::LoopLibrary => caller == Screen::TrackerLoop,
            OverlayKind::MixEffects => caller == Screen::Meter,
        };
        if !allowed {
            return;
        }
        if kind == OverlayKind::LoopLibrary {
            self.tracker_stop();
            self.loop_imports =
                crate::loop_player::list_wavs(&self.config.loop_player.import_directory);
            self.loop_selected = self
                .loop_selected
                .min(self.loop_imports.len().saturating_sub(1));
            self.refresh_loop_library();
        }
        let context = self.menu_context();
        let Some(launcher) = OverlayLauncher::resolve(caller, context, action) else {
            self.status = "overlay launcher is unavailable in this controller context".into();
            return;
        };
        let selection = match kind {
            OverlayKind::TrackerPage => self.tracker_page * LANES_PER_PAGE + self.tracker_track,
            OverlayKind::TrackerPattern => self
                .overlay_pattern_locations()
                .iter()
                .position(|(pattern, _)| *pattern == self.tracker_pattern_number())
                .unwrap_or(0),
            OverlayKind::TrackerSong => self.tracker_order,
            OverlayKind::TrackerRoute => 0,
            OverlayKind::TrackerPatternLength => pattern_length_choices()
                .iter()
                .position(|rows| *rows == self.pattern_setup_rows)
                .unwrap_or(0),
            OverlayKind::TrackerNoteLength => NoteLength::ALL
                .iter()
                .position(|length| *length == self.note_length)
                .unwrap_or(0),
            OverlayKind::TrackerAdvance => self.tracker_advance.min(32),
            OverlayKind::LoopLibrary if action == Action::LoopImport => self
                .loop_selected
                .min(self.loop_imports.len().saturating_sub(1)),
            OverlayKind::LoopLibrary => self
                .loop_library
                .iter()
                .position(|entry| entry.current)
                .map(|index| self.loop_imports.len() + index)
                .unwrap_or(0),
            OverlayKind::MixEffects => self.fx_target.min(MAX_AUX_BUSES + 1),
        };
        let draft = if kind == OverlayKind::TrackerRoute {
            self.refresh_overlay_target_candidates();
            let Some(page) = self.current_page().cloned() else {
                self.status = "active page routing is unavailable".into();
                return;
            };
            OverlayDraft::Route(Box::new(RouteDraft::new(
                self.tracker_pattern_number(),
                self.tracker_page,
                page,
            )))
        } else {
            OverlayDraft::None
        };
        let caller_menu_page = self.menu_page();
        let caller_page_select_mode = self.page_select_mode;
        self.page_select_mode = false;
        self.overlay = Some(OverlayState::new(
            kind,
            caller,
            launcher,
            selection,
            draft,
            caller_menu_page,
            caller_page_select_mode,
        ));
        self.status = format!("{} · turn to browse · press to select", kind.title());
    }

    fn close_overlay(&mut self, cancelled: bool) {
        let Some(overlay) = self.overlay.take() else {
            return;
        };
        match overlay.close_behavior {
            CloseBehavior::CancelDraft => {
                // The detached draft is dropped with `overlay`. Only the
                // overlay's explicit APPLY path may update the Project owner.
            }
        }
        if self.screen == overlay.caller {
            self.menu_page_by_screen[self.screen.index()] = overlay.caller_menu_page.min(3);
            self.page_select_mode = overlay.caller_page_select_mode;
        }
        self.status = if cancelled && overlay.route().is_some_and(RouteDraft::dirty) {
            "overlay closed · unconfirmed routing cancelled".into()
        } else {
            format!("returned to {}", overlay.caller.label())
        };
    }

    fn overlay_back(&mut self) {
        if self
            .overlay
            .as_mut()
            .is_some_and(OverlayState::cancel_route_field)
        {
            self.status = "route field cancelled · draft otherwise unchanged".into();
        } else {
            self.close_overlay(true);
        }
    }

    fn move_overlay(&mut self, direction: i8) {
        let active = self
            .overlay
            .as_ref()
            .and_then(|overlay| overlay.active_field);
        if let Some(field) = active {
            self.adjust_overlay_route_field(field, direction);
            return;
        }
        let rows = self.overlay_row_count();
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.move_selection(direction, rows);
        }
    }

    fn adjust_overlay_route_field(&mut self, field: RouteField, direction: i8) {
        let auto_route = self
            .overlay
            .as_ref()
            .and_then(OverlayState::route)
            .is_some_and(|route| route.page.target == PageTarget::Default);
        if auto_route
            && !matches!(
                field,
                RouteField::Target | RouteField::Engine | RouteField::MidiOutput
            )
        {
            self.status = "AUTO owns channel/bank/program · choose an explicit target first".into();
            return;
        }
        let current_page = self
            .overlay
            .as_ref()
            .and_then(OverlayState::route)
            .map(|route| route.page.clone());
        let internal = current_page
            .as_ref()
            .is_some_and(|page| matches!(page.target, PageTarget::Software(_)));
        let external = current_page.as_ref().is_some_and(|page| {
            matches!(
                page.target,
                PageTarget::ConfiguredExternal | PageTarget::Midi(_)
            )
        });
        if matches!(field, RouteField::Engine | RouteField::Instrument) && !internal {
            self.status = "choose INTERNAL target before engine/instrument".into();
            return;
        }
        if matches!(field, RouteField::MidiOutput | RouteField::DeviceProfile) && !external {
            self.status = "choose EXTERNAL MIDI target before output/profile".into();
            return;
        }
        let selected_target = current_page.as_ref().and_then(|page| match field {
            RouteField::Target => {
                let current = match page.target {
                    PageTarget::Default => 0,
                    PageTarget::ActiveInstrument
                    | PageTarget::Synthv1(_)
                    | PageTarget::Software(_) => 1,
                    PageTarget::ConfiguredExternal | PageTarget::Midi(_) => 2,
                };
                match wrapped_index(current, 3, direction) {
                    0 => Some(PageTarget::Default),
                    1 => self.first_software_route().map(PageTarget::Software),
                    _ => Some(PageTarget::ConfiguredExternal),
                }
            }
            RouteField::Engine => {
                let route = match &page.target {
                    PageTarget::Software(route) => Some(route),
                    _ => None,
                };
                let engines = self
                    .catalogs
                    .iter()
                    .filter(|catalog| !catalog.presets.is_empty())
                    .map(|catalog| catalog.backend)
                    .collect::<Vec<_>>();
                let current = route
                    .and_then(|route| engines.iter().position(|engine| *engine == route.engine))
                    .unwrap_or(0);
                let engine = *engines.get(wrapped_index(current, engines.len(), direction))?;
                self.catalogs
                    .iter()
                    .find(|catalog| catalog.backend == engine)
                    .and_then(|catalog| catalog.presets.first())
                    .map(|preset| {
                        PageTarget::Software(SoftwareRoute {
                            engine,
                            instrument: preset.route_id(),
                        })
                    })
            }
            RouteField::Instrument => {
                let PageTarget::Software(route) = &page.target else {
                    return None;
                };
                let presets = &self
                    .catalogs
                    .iter()
                    .find(|catalog| catalog.backend == route.engine)?
                    .presets;
                let current = presets
                    .iter()
                    .position(|preset| preset.route_id() == route.instrument)
                    .unwrap_or(0);
                let preset = presets.get(wrapped_index(current, presets.len(), direction))?;
                Some(PageTarget::Software(SoftwareRoute {
                    engine: route.engine,
                    instrument: preset.route_id(),
                }))
            }
            RouteField::MidiOutput => {
                let mut outputs = vec![PageTarget::ConfiguredExternal];
                outputs.extend(
                    self.available_page_outputs
                        .iter()
                        .map(|name| crate::midi_endpoint::stable_identity(name))
                        .map(PageTarget::Midi),
                );
                if matches!(
                    page.target,
                    PageTarget::ConfiguredExternal | PageTarget::Midi(_)
                ) {
                    outputs.push(page.target.clone());
                }
                outputs.sort();
                outputs.dedup();
                let current = outputs
                    .iter()
                    .position(|target| target == &page.target)
                    .unwrap_or(0);
                outputs
                    .get(wrapped_index(current, outputs.len(), direction))
                    .cloned()
            }
            _ => None,
        });
        let selected_profile = if field == RouteField::DeviceProfile {
            let current = current_page
                .as_ref()
                .and_then(|page| page.device_profile.clone());
            let mut profiles = vec![None];
            profiles.extend(
                self.device_profiles
                    .profiles()
                    .map(|profile| Some(profile.id.clone())),
            );
            if !profiles.contains(&current) {
                profiles.push(current.clone());
            }
            let index = profiles
                .iter()
                .position(|profile| profile == &current)
                .unwrap_or(0);
            profiles
                .get(wrapped_index(index, profiles.len(), direction))
                .cloned()
                .flatten()
        } else {
            None
        };
        let Some(route) = self.overlay.as_mut().and_then(OverlayState::route_mut) else {
            return;
        };
        match field {
            RouteField::Target
            | RouteField::Engine
            | RouteField::Instrument
            | RouteField::MidiOutput => {
                if let Some(target) = selected_target {
                    route.page.target = target;
                    if route.page.target == PageTarget::Default {
                        route.page.columns = [sequencer::ColumnSetup::default(); LANES_PER_PAGE];
                        route.page.setup.clear();
                    }
                }
            }
            RouteField::DeviceProfile => route.page.device_profile = selected_profile,
            RouteField::Channel(column) => {
                let value = &mut route.page.columns[column.min(LANES_PER_PAGE - 1)].channel;
                *value = if direction < 0 {
                    value.saturating_sub(1)
                } else {
                    value.saturating_add(1).min(15)
                };
            }
            RouteField::BankMsb(column) => {
                let value = &mut route.page.columns[column.min(LANES_PER_PAGE - 1)].bank_msb;
                *value = if direction < 0 {
                    value.saturating_sub(1)
                } else {
                    value.saturating_add(1).min(127)
                };
            }
            RouteField::BankLsb(column) => {
                let value = &mut route.page.columns[column.min(LANES_PER_PAGE - 1)].bank_lsb;
                *value = if direction < 0 {
                    value.saturating_sub(1)
                } else {
                    value.saturating_add(1).min(127)
                };
            }
            RouteField::Program(column) => {
                let value = &mut route.page.columns[column.min(LANES_PER_PAGE - 1)].program;
                *value = if direction < 0 {
                    value.saturating_sub(1)
                } else {
                    value.saturating_add(1).min(127)
                };
            }
        }
    }

    fn confirm_route_overlay(&mut self) {
        let Some(route) = self.overlay.as_ref().and_then(OverlayState::route).cloned() else {
            return;
        };
        let mut candidate = self.song.clone();
        let Some(page) = candidate
            .patterns
            .get_mut(&route.pattern)
            .and_then(|pattern| pattern.pages.get_mut(route.page_index))
        else {
            self.status = "routing owner changed while overlay was open".into();
            return;
        };
        *page = route.page;
        if let Err(error) = candidate.validate() {
            self.status = format!("routing conflict · {error}");
            return;
        }
        self.release_tracker_audition();
        self.song = candidate;
        self.close_overlay(false);
        self.clamp_tracker_cursor();
        self.sync_tracker_route();
        self.status = "page routing applied through the Project owner".into();
    }

    fn activate_overlay(&mut self) {
        if self
            .overlay
            .as_ref()
            .is_some_and(|overlay| overlay.active_field.is_some())
        {
            if let Some(overlay) = self.overlay.as_mut() {
                overlay.confirm_route_field();
            }
            self.status = "route field kept in draft · APPLY commits Project routing".into();
            return;
        }
        let Some((kind, selection)) = self
            .overlay
            .as_ref()
            .map(|overlay| (overlay.kind, overlay.selection))
        else {
            return;
        };
        match kind {
            OverlayKind::TrackerPage => {
                let page_rows = self.current_pages().len() * LANES_PER_PAGE;
                match selection.cmp(&page_rows) {
                    std::cmp::Ordering::Less => {
                        self.release_tracker_audition();
                        self.tracker_page = selection / LANES_PER_PAGE;
                        self.tracker_track = selection % LANES_PER_PAGE;
                        self.close_overlay(false);
                        if !self.leave_noob_on_percussion() {
                            self.sync_tracker_route();
                        }
                    }
                    std::cmp::Ordering::Equal => {
                        self.close_overlay(false);
                        self.open_tracker_loop();
                    }
                    std::cmp::Ordering::Greater => {
                        self.close_overlay(false);
                        self.open_page_manager();
                    }
                }
            }
            OverlayKind::TrackerPattern => {
                let locations = self.overlay_pattern_locations();
                if let Some((_, order)) = locations.get(selection).copied() {
                    self.release_tracker_audition();
                    self.tracker_order = order;
                    self.tracker_row = 0;
                    self.clamp_tracker_cursor();
                    self.close_overlay(false);
                    self.sync_tracker_route();
                } else if selection == locations.len() {
                    self.close_overlay(false);
                    self.set_screen(Screen::TrackerFiles);
                    self.open_pattern_tools();
                } else {
                    self.close_overlay(false);
                    self.song_list = sequencer::list(&sequencer::songs_dir());
                    self.tracker_files_mode = TrackerFilesMode::Projects;
                    self.set_screen(Screen::TrackerFiles);
                }
            }
            OverlayKind::TrackerSong => {
                if selection < self.song.order.len() {
                    self.release_tracker_audition();
                    self.tracker_order = selection;
                    self.tracker_row = 0;
                    self.clamp_tracker_cursor();
                    self.close_overlay(false);
                    self.sync_tracker_route();
                } else if selection == self.song.order.len() {
                    self.close_overlay(false);
                    self.open_arrange();
                } else if selection == self.song.order.len() + 1 {
                    self.close_overlay(false);
                    self.set_screen(Screen::TrackerTools);
                    self.reset_context_page();
                    self.status = "FT2 tools · loop, FX, clipboard, and mute".into();
                } else {
                    self.close_overlay(false);
                    if let Some(bpm) = self.tap.tap(Instant::now()) {
                        self.set_tracker_tempo(bpm.round().clamp(20.0, 300.0) as u16);
                    } else {
                        self.status = "tap again to set the Pattern tempo".into();
                    }
                }
            }
            OverlayKind::TrackerRoute => {
                if selection == RouteField::ROWS - 1 {
                    self.confirm_route_overlay();
                } else if let Some(field) = RouteField::from_row(selection) {
                    if let Some(overlay) = self.overlay.as_mut() {
                        overlay.begin_route_field(field);
                    }
                    self.status =
                        "route field active · turn changes draft · Back cancels field".into();
                }
            }
            OverlayKind::TrackerPatternLength => {
                if let Some(rows) = pattern_length_choices().get(selection).copied() {
                    self.pattern_setup_rows = rows;
                    self.close_overlay(false);
                    self.pattern_setup_status();
                }
            }
            OverlayKind::TrackerNoteLength => {
                if let Some(length) = NoteLength::ALL.get(selection).copied() {
                    self.note_length = length;
                    self.close_overlay(false);
                    self.status = format!("EDIT note length {}", length.label());
                }
            }
            OverlayKind::TrackerAdvance => {
                self.set_tracker_advance(selection.min(32));
                self.close_overlay(false);
                self.status = format!("EDIT ADD {} row(s)", self.tracker_advance);
            }
            OverlayKind::LoopLibrary => {
                self.close_overlay(false);
                if selection < self.loop_imports.len() {
                    self.loop_selected = selection;
                    self.import_selected_loop();
                } else {
                    self.select_loop_library_entry(selection - self.loop_imports.len());
                }
            }
            OverlayKind::MixEffects => {
                self.fx_target = selection.min(MAX_AUX_BUSES + 1);
                self.close_overlay(false);
                if self.selected_effect_id().is_none() {
                    self.fx_selection = FxRackSelection::Insert;
                }
                self.fx_rack_parent = Screen::Meter;
                self.set_screen(Screen::FxRack);
                self.status = format!("{} rack", fx_target_label(self.fx_target));
            }
        }
    }

    fn set_screen(&mut self, screen: Screen) {
        let previous = self.screen;
        let playback_filter_was_active =
            self.playback_noob && self.screen_keeps_playback_workspace_active(previous);
        let playback_filter_will_be_active =
            self.playback_noob && self.screen_keeps_playback_workspace_active(screen);
        if playback_filter_was_active && !playback_filter_will_be_active {
            if let Some(engine) = self.engine.as_ref() {
                engine.panic();
            }
            self.held_notes = HeldNotes::default();
        }
        let previous_tracker = self.screen_keeps_tracker_workspace_active(previous);
        let next_tracker = self.screen_keeps_tracker_workspace_active(screen);
        if self.screen != screen {
            if self.screen == Screen::TrackerFiles && self.song_previewing {
                self.stop_song_preview();
            }
            self.menu_page_by_screen[screen.index()] = 0;
            self.page_select_mode = false;
            self.prepare_confirmation_action(Action::Noop);
        }
        self.screen = screen;
        self.sync_playback_noob();
        self.fx_control_mode
            .store(screen == Screen::FxEditor, Ordering::Relaxed);
        if screen == Screen::FxEditor {
            self.arm_fx_pickup();
        }
        if previous != screen {
            if previous_tracker && !next_tracker {
                self.disable_tracker_route();
                self.unload_owned_engine(|owner| matches!(owner, EngineOwner::Tracker(_)));
            }
            if !previous_tracker && next_tracker {
                self.unload_owned_engine(|owner| *owner == EngineOwner::SoftwareSynth);
            }
        }
    }

    fn screen_keeps_playback_workspace_active(&self, screen: Screen) -> bool {
        screen == Screen::Playback
            || (is_fx_screen(screen) && self.fx_rack_parent == Screen::Playback)
            || (screen == Screen::Help
                && (self.help_previous == Screen::Playback
                    || (is_fx_screen(self.help_previous)
                        && self.fx_rack_parent == Screen::Playback)))
    }

    fn screen_keeps_tracker_workspace_active(&self, screen: Screen) -> bool {
        is_tracker_screen(screen)
            || (is_fx_screen(screen) && is_tracker_screen(self.fx_rack_parent))
            || (screen == Screen::Help
                && (is_tracker_screen(self.help_previous)
                    || (is_fx_screen(self.help_previous)
                        && is_tracker_screen(self.fx_rack_parent))))
    }

    fn tracker_workspace_active(&self) -> bool {
        self.screen_keeps_tracker_workspace_active(self.screen)
    }

    fn genuinely_new_empty_default_project(&self) -> bool {
        self.song_file_stem.is_none()
            && sequencer::matches_new_empty_default_project(
                &self.song,
                &self.config.external_midi,
                &self.routing_defaults,
            )
    }

    fn prepare_first_tracker_instrument(&mut self) -> TrackerEntryInstrument {
        if !self.genuinely_new_empty_default_project() {
            return TrackerEntryInstrument::ExistingProject;
        }
        let (route, entry) = if self.engine_owner.as_ref() == Some(&EngineOwner::SoftwareSynth) {
            let Some(preset) = self.playing.as_ref() else {
                return TrackerEntryInstrument::ExistingProject;
            };
            (
                SoftwareRoute {
                    engine: preset.backend,
                    instrument: preset.route_id(),
                },
                TrackerEntryInstrument::AdoptedPlayer,
            )
        } else {
            let Some(route) = self.first_synthv1_route() else {
                return TrackerEntryInstrument::ExistingProject;
            };
            (route, TrackerEntryInstrument::FirstSynthv1)
        };
        let Some(first_page) = self
            .current_pattern_mut()
            .and_then(|pattern| pattern.pages.first_mut())
        else {
            return TrackerEntryInstrument::ExistingProject;
        };
        first_page.target = PageTarget::Software(route.clone());
        if entry == TrackerEntryInstrument::AdoptedPlayer {
            self.engine_owner = Some(EngineOwner::Tracker(route));
        }
        entry
    }

    fn unload_owned_engine(&mut self, matches: impl FnOnce(&EngineOwner) -> bool) {
        if self.engine_owner.as_ref().is_some_and(matches) {
            if let Some(engine) = self.engine.as_ref() {
                engine.panic();
            }
            drop(take_engine_when_owned(
                &mut self.engine,
                &mut self.engine_owner,
                |_| true,
            ));
            self.performance_meter
                .set_audio_unavailable(AudioAvailability::Stopped);
            self.playing = None;
        }
    }

    fn disable_tracker_route(&self) {
        if let Ok(mut route) = self.tracker_route.lock() {
            for (target, channel) in route.destinations() {
                self.tracker_live_input.cancel(&target, channel);
            }
            let external = self.tracker_external_config();
            route.configure(crate::engine::TrackerRouteConfig {
                enabled: false,
                target: PageTarget::ConfiguredExternal,
                columns: [(0, (0, 0, 0)); LANES_PER_PAGE],
                start_column: 0,
                percussion: false,
                audition_note: None,
                scale: None,
                external: &external,
            });
        }
    }

    fn tracker_software_route(&self) -> Option<SoftwareRoute> {
        match &self.current_page()?.target {
            PageTarget::Software(route) => Some(route.clone()),
            PageTarget::Synthv1(name) => Some(SoftwareRoute::synthv1(name)),
            _ => None,
        }
    }

    fn preset_for_route(&self, route: &SoftwareRoute) -> Option<Preset> {
        self.catalogs
            .iter()
            .find(|catalog| catalog.backend == route.engine)?
            .presets
            .iter()
            .find(|preset| preset.route_id() == route.instrument)
            .cloned()
    }

    fn first_software_route(&self) -> Option<SoftwareRoute> {
        self.catalogs.iter().find_map(|catalog| {
            catalog.presets.first().map(|preset| SoftwareRoute {
                engine: catalog.backend,
                instrument: preset.route_id(),
            })
        })
    }

    fn first_synthv1_route(&self) -> Option<SoftwareRoute> {
        self.catalogs
            .iter()
            .find(|catalog| catalog.backend == BackendKind::Synthv1)?
            .presets
            .first()
            .map(|preset| SoftwareRoute::synthv1(preset.route_id()))
    }

    fn first_synthv1_name(&self) -> Option<String> {
        self.catalogs
            .iter()
            .find(|catalog| catalog.backend == BackendKind::Synthv1)?
            .presets
            .first()
            .map(|preset| preset.name.clone())
    }

    fn ensure_tracker_engine(&mut self) -> bool {
        let Some(route) = self.tracker_software_route() else {
            self.unload_owned_engine(|owner| matches!(owner, EngineOwner::Tracker(_)));
            return true;
        };
        self.ensure_tracker_engine_for(&route)
    }

    fn ensure_tracker_engine_for(&mut self, route: &SoftwareRoute) -> bool {
        let same_owner = self
            .engine_owner
            .as_ref()
            .is_some_and(|owner| owner == &EngineOwner::Tracker(route.clone()));
        if same_owner && self.engine.as_mut().is_some_and(|engine| engine.alive()) {
            return true;
        }
        let Some(preset) = self.preset_for_route(route) else {
            self.unload_owned_engine(|owner| matches!(owner, EngineOwner::Tracker(_)));
            self.status = format!(
                "FT2 instrument missing · {} · {}",
                route.engine.label(),
                route.instrument
            );
            return false;
        };
        self.release_tracker_audition();
        if let Some(engine) = self.engine.as_ref() {
            engine.panic();
        }
        self.engine.take();
        self.engine_owner = None;
        self.playing = None;
        match self.start_tracker_engine_process(&preset) {
            Ok(mut engine) => {
                engine.bind_midi_lifecycle(self.midi_lifecycle.clone());
                self.engine = Some(engine);
                self.engine_owner = Some(EngineOwner::Tracker(route.clone()));
                self.playing = Some(preset);
                if let Ok(mut backend) = self.midi_backend.lock() {
                    *backend = route.engine;
                }
                true
            }
            Err(error) => {
                self.status = format!("FT2 SYNTH START FAILED: {error:#}");
                false
            }
        }
    }

    fn start_tracker_engine_process(&self, preset: &Preset) -> Result<Engine> {
        #[cfg(test)]
        if let Some(result) = self.tracker_engine_start_override.as_ref() {
            result.clone().map_err(anyhow::Error::msg)?;
            return Engine::start_test_process(preset.backend, Arc::clone(&self.midi_output));
        }
        Engine::start_with_routing(
            preset,
            &self.engine_state,
            Arc::clone(&self.midi_output),
            &self.config,
            &self.song.insert_rack,
            &self.song.aux_routing,
        )
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
            let captured = self.recorder.events.len();
            self.recorder.stop(Instant::now());
            self.last = self.recorder.events.clone();
            if let Some(engine) = &self.engine {
                engine.panic();
            }
            self.status = format!("recorded {captured} MIDI events · Playback to review");
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
        self.set_tracker_mode(TrackerMode::Play);
        if !self.stop_song_preview() {
            self.sequencer.stop();
        }
        self.loop_player.stop();
        self.loop_meter
            .set_audio_unavailable(AudioAvailability::Stopped);
        let _ = self.audio_recorder.stop();
        self.stop_recording();
        self.stop_playback();
        if let Some(engine) = self.engine.as_mut() {
            if engine.final_recording_active() {
                let _ = engine.stop_final_recording();
            }
            if let Some(status) = engine.final_recording_status() {
                self.final_recording_last = status;
            }
        }
        self.engine.take();
        self.engine_owner = None;
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
    fn tracker_page_count(&self) -> usize {
        self.current_pages().len() + 1
    }
    fn tracker_loop_page_number(&self) -> usize {
        self.tracker_page_count()
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
        let Some(original_page) = self.current_page().cloned() else {
            self.status = "selected page is unavailable".into();
            return;
        };
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
            original_page,
            draft: original,
            field: NoteEditorField::Destination,
            active: false,
            edit_original_page: None,
            edit_original_draft: None,
        });
        self.refresh_page_targets();
        self.sync_tracker_route();
        self.reset_context_page();
        self.status = "NOTE EDIT · DESTINATION highlighted · press encoder to edit".into();
    }
    fn release_tracker_audition(&mut self) {
        let Some(page) = self.current_page().cloned() else {
            return;
        };
        self.audition_release_revision = self.audition_release_revision.wrapping_add(1);
        for channel in page
            .columns
            .iter()
            .enumerate()
            .map(|(lane, _)| page.runtime_channel(lane, &self.config.external_midi))
            .collect::<std::collections::BTreeSet<_>>()
        {
            self.tracker_live_input.cancel(&page.target, channel);
        }
    }

    fn select_note_editor_field(&mut self, field: NoteEditorField) {
        if self
            .note_editor
            .as_ref()
            .is_some_and(|editor| editor.active)
        {
            self.confirm_note_editor_field();
        }
        let page = self.current_page().cloned();
        let draft = self.note_editor.as_ref().map(|editor| editor.draft);
        let Some(editor) = self.note_editor.as_mut() else {
            return;
        };
        editor.field = field;
        editor.active = true;
        editor.edit_original_page = page;
        editor.edit_original_draft = draft;
        if matches!(
            field,
            NoteEditorField::Channel
                | NoteEditorField::DefaultProgram
                | NoteEditorField::BankMsb
                | NoteEditorField::BankLsb
        ) && self
            .current_page()
            .is_some_and(|page| page.target == PageTarget::Default)
        {
            self.release_tracker_audition();
            let target = self
                .tracker_software_route()
                .map(PageTarget::Software)
                .unwrap_or(PageTarget::ConfiguredExternal);
            if let Some(page) = self.current_page_mut() {
                page.target = target;
            }
        }
        self.sync_tracker_route();
        self.status = format!(
            "NOTE EDIT · {} ACTIVE · turn, press confirms, Back cancels",
            field.label()
        );
    }

    fn confirm_note_editor_field(&mut self) {
        let Some(editor) = self.note_editor.as_mut() else {
            return;
        };
        editor.active = false;
        editor.edit_original_page = None;
        editor.edit_original_draft = None;
        self.status = format!("NOTE EDIT · {} confirmed", editor.field.label());
    }

    fn cancel_note_editor_field(&mut self) {
        let Some((page, draft, field)) = self.note_editor.as_mut().and_then(|editor| {
            editor.active = false;
            Some((
                editor.edit_original_page.take()?,
                editor.edit_original_draft.take()?,
                editor.field,
            ))
        }) else {
            return;
        };
        self.release_tracker_audition();
        if let Some(current) = self.current_page_mut() {
            *current = page;
        }
        if let Some(editor) = self.note_editor.as_mut() {
            editor.draft = draft;
        }
        self.sync_tracker_route();
        self.status = format!("NOTE EDIT · {} change cancelled", field.label());
    }

    fn move_note_editor_field(&mut self, direction: i8) {
        if self
            .note_editor
            .as_ref()
            .is_some_and(|editor| editor.active)
        {
            return;
        }
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
        self.status = format!(
            "NOTE EDIT · {} highlighted · press encoder to edit",
            editor.field.label()
        );
    }

    fn adjust_note_editor(&mut self, direction: i8) {
        let Some(field) = self.note_editor.as_ref().map(|editor| editor.field) else {
            return;
        };
        if !self
            .note_editor
            .as_ref()
            .is_some_and(|editor| editor.active)
        {
            self.move_note_editor_field(direction);
            return;
        }
        let increase = direction >= 0;
        let page_velocity = self.current_page().map_or(96, |page| page.velocity);
        let page_program = self.current_column().map_or(0, |column| column.program);
        let song_gate = self.song.gate_percent;
        let pattern_tempo = self.current_tempo();
        let route_field = matches!(
            field,
            NoteEditorField::Destination
                | NoteEditorField::Channel
                | NoteEditorField::DefaultProgram
                | NoteEditorField::BankMsb
                | NoteEditorField::BankLsb
                | NoteEditorField::Program
        );
        if route_field {
            self.release_tracker_audition();
        }
        match field {
            NoteEditorField::Destination => {
                self.refresh_page_targets();
                let current = self.current_page().map(|page| page.target.clone());
                let index = current
                    .as_ref()
                    .and_then(|target| {
                        self.page_target_candidates
                            .iter()
                            .position(|candidate| candidate == target)
                    })
                    .unwrap_or(0);
                let len = self.page_target_candidates.len().max(1);
                let next = if increase {
                    (index + 1) % len
                } else {
                    (index + len - 1) % len
                };
                if let Some(target) = self.page_target_candidates.get(next).cloned() {
                    if let Some(page) = self.current_page_mut() {
                        page.target = target;
                        if page.target == PageTarget::Default {
                            page.columns = [sequencer::ColumnSetup::default(); LANES_PER_PAGE];
                            page.setup.clear();
                        }
                    }
                }
            }
            NoteEditorField::Channel => {
                if let Some(column) = self.current_column_mut() {
                    column.channel = if increase {
                        (column.channel + 1).min(15)
                    } else {
                        column.channel.saturating_sub(1)
                    };
                }
            }
            NoteEditorField::DefaultProgram => {
                let channel = self
                    .current_page()
                    .map(|page| {
                        page.runtime_channel(self.tracker_track, &self.config.external_midi)
                    })
                    .unwrap_or(0);
                if channel == 9 {
                    if let Some(editor) = self.note_editor.as_mut() {
                        let note = match editor.draft.note {
                            Note::On(note) if (35..=81).contains(&note) => note,
                            _ => 35,
                        };
                        editor.draft.note = Note::On(if increase {
                            note.saturating_add(1).min(81)
                        } else {
                            note.saturating_sub(1).max(35)
                        });
                    }
                } else if let Some(column) = self.current_column_mut() {
                    column.program = if increase {
                        column.program.saturating_add(1).min(127)
                    } else {
                        column.program.saturating_sub(1)
                    };
                }
            }
            NoteEditorField::BankMsb | NoteEditorField::BankLsb => {
                if let Some(column) = self.current_column_mut() {
                    let value = if field == NoteEditorField::BankMsb {
                        &mut column.bank_msb
                    } else {
                        &mut column.bank_lsb
                    };
                    *value = if increase {
                        value.saturating_add(1).min(127)
                    } else {
                        value.saturating_sub(1)
                    };
                }
            }
            NoteEditorField::Note => {
                if let Some(editor) = self.note_editor.as_mut() {
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
            }
            NoteEditorField::Gate => {
                if let Some(editor) = self.note_editor.as_mut() {
                    let value = editor.draft.gate.unwrap_or(song_gate);
                    editor.draft.gate = Some(if increase {
                        value.saturating_add(1).min(100)
                    } else {
                        value.saturating_sub(1).max(1)
                    });
                }
            }
            NoteEditorField::Velocity => {
                if let Some(editor) = self.note_editor.as_mut() {
                    let value = editor.draft.velocity.unwrap_or(page_velocity);
                    editor.draft.velocity = Some(if increase {
                        value.saturating_add(1).min(127)
                    } else {
                        value.saturating_sub(1)
                    });
                }
            }
            NoteEditorField::Program => {
                if let Some(editor) = self.note_editor.as_mut() {
                    let value = editor.draft.program.unwrap_or(page_program);
                    editor.draft.program = Some(if increase {
                        value.saturating_add(1).min(127)
                    } else {
                        value.saturating_sub(1)
                    });
                }
            }
            NoteEditorField::Effect => {
                let effects = [
                    Command::None,
                    Command::Cut(0),
                    Command::Delay(0),
                    Command::Retrigger(2),
                    Command::Tempo(pattern_tempo),
                ];
                if let Some(editor) = self.note_editor.as_mut() {
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
            }
            NoteEditorField::EffectParameter => {
                let Some(editor) = self.note_editor.as_mut() else {
                    return;
                };
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
        if route_field || field == NoteEditorField::Note {
            self.sync_tracker_route();
        }
        let detail = match field {
            NoteEditorField::Destination => {
                self.current_page().map(|page| page.target.label().into())
            }
            NoteEditorField::Channel => self.current_page().map(|page| {
                format!(
                    "MIDI channel {}",
                    sequencer::musician_channel(
                        page.runtime_channel(self.tracker_track, &self.config.external_midi)
                    )
                )
            }),
            NoteEditorField::DefaultProgram => Some(self.tracker_instrument_label()),
            NoteEditorField::BankMsb | NoteEditorField::BankLsb => self
                .current_column()
                .map(|column| format!("bank {}/{}", column.bank_msb, column.bank_lsb)),
            NoteEditorField::Program => self
                .note_editor
                .as_ref()
                .and_then(|editor| editor.draft.program)
                .map(|program| format!("cell override · {}", self.tracker_program_label(program))),
            _ => None,
        };
        self.status = detail.map_or_else(
            || format!("NOTE EDIT · {} changed", field.label()),
            |detail| format!("NOTE EDIT · {detail} · play MIDI to audition"),
        );
    }

    fn clear_note_editor_field(&mut self) {
        let Some(field) = self.note_editor.as_ref().map(|editor| editor.field) else {
            return;
        };
        if matches!(
            field,
            NoteEditorField::Destination
                | NoteEditorField::Channel
                | NoteEditorField::DefaultProgram
                | NoteEditorField::BankMsb
                | NoteEditorField::BankLsb
        ) {
            self.status = format!("{} is a route default and cannot be blank", field.label());
            return;
        }
        if field == NoteEditorField::Program {
            self.release_tracker_audition();
        }
        if let Some(editor) = self.note_editor.as_mut() {
            match field {
                NoteEditorField::Note => editor.draft.note = Note::Empty,
                NoteEditorField::Gate => editor.draft.gate = None,
                NoteEditorField::Velocity => editor.draft.velocity = None,
                NoteEditorField::Program => editor.draft.program = None,
                NoteEditorField::Effect | NoteEditorField::EffectParameter => {
                    editor.draft.command = Command::None
                }
                _ => {}
            }
        }
        self.sync_tracker_route();
        self.status = format!("NOTE EDIT · {} cleared", field.label());
    }

    fn save_note_editor(&mut self) {
        if self
            .note_editor
            .as_ref()
            .is_some_and(|editor| editor.active)
        {
            self.confirm_note_editor_field();
        }
        let Some(editor) = self.note_editor.as_ref().cloned() else {
            return;
        };
        if let Err(error) = editor.draft.validate() {
            self.status = format!("NOTE EDIT rejected · {error}");
            return;
        }
        if !matches!(editor.draft.note, Note::On(_))
            && (editor.draft.velocity.is_some()
                || editor.draft.program.is_some()
                || editor.draft.gate.is_some())
        {
            self.status =
                "NOTE EDIT rejected · velocity, program, and gate require a note-on".into();
            return;
        }
        if matches!(editor.draft.command, Command::Retrigger(_))
            && !matches!(editor.draft.note, Note::On(_))
        {
            self.status = "NOTE EDIT rejected · retrigger requires a note-on".into();
            return;
        }
        let old = self
            .song
            .patterns
            .get_mut(&editor.pattern)
            .and_then(|pattern| pattern.rows.get_mut(editor.row))
            .and_then(|row| row.get_mut(editor.lane))
            .map(|cell| {
                let old = *cell;
                *cell = editor.draft;
                old
            });
        if let Some(old) = old {
            if let Err(error) = self.song.validate() {
                if let Some(cell) = self
                    .song
                    .patterns
                    .get_mut(&editor.pattern)
                    .and_then(|pattern| pattern.rows.get_mut(editor.row))
                    .and_then(|row| row.get_mut(editor.lane))
                {
                    *cell = old;
                }
                self.status = format!("NOTE EDIT rejected · {error}");
                return;
            }
            self.note_editor = None;
            self.sync_tracker_route();
            self.reset_context_page();
            self.status = "NOTE EDIT saved · route defaults and cell are ready".into();
        } else {
            self.status = "NOTE EDIT rejected · source cell no longer exists".into();
        }
    }

    fn cancel_note_editor(&mut self) {
        let Some(editor) = self.note_editor.take() else {
            return;
        };
        self.release_tracker_audition();
        if let Some(page) = self.current_page_mut() {
            *page = editor.original_page;
        }
        if let Some(cell) = self
            .song
            .patterns
            .get_mut(&editor.pattern)
            .and_then(|pattern| pattern.rows.get_mut(editor.row))
            .and_then(|row| row.get_mut(editor.lane))
        {
            *cell = editor.original;
        }
        self.sync_tracker_route();
        self.reset_context_page();
        self.status = "NOTE EDIT cancelled · route and cell restored".into();
    }

    fn back_note_editor(&mut self) {
        if self
            .note_editor
            .as_ref()
            .is_some_and(|editor| editor.active)
        {
            self.cancel_note_editor_field();
        } else {
            self.cancel_note_editor();
        }
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
                .enumerate()
                .map(|(lane, _)| page.runtime_channel(lane, &self.config.external_midi))
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
        if !self.tracker_noob_allows(note) {
            self.status = "N00B · note outside scale ignored".into();
            return;
        }
        if self.current_page().is_some_and(|page| page.percussion) {
            let pattern_number = self.tracker_pattern_number();
            let assignment = self.song.patterns.get(&pattern_number).and_then(|pattern| {
                drum_entry_lanes(
                    pattern,
                    self.tracker_row,
                    self.tracker_page,
                    &[(note, velocity)],
                )
                .into_iter()
                .next()
                .flatten()
            });
            let Some(lane) = assignment else {
                self.status = format!(
                    "drum row {:02X} has no free column · note ignored",
                    self.tracker_row
                );
                return;
            };
            let page_start = self.tracker_page * LANES_PER_PAGE;
            if let Some(cell) = self
                .song
                .patterns
                .get_mut(&pattern_number)
                .and_then(|pattern| pattern.rows.get_mut(self.tracker_row))
                .and_then(|row| row.get_mut(page_start + lane))
            {
                write_step_note(cell, note, velocity);
                self.advance_tracker_row();
            }
            return;
        }
        self.write_edit_notes(&[(note, velocity)]);
    }
    fn commit_tracker_gesture(&mut self, now: Instant) {
        self.commit_tracker_gesture_after(now, self.config.external_midi.gesture_settle);
    }

    fn commit_released_tracker_gesture(&mut self, now: Instant) {
        self.commit_tracker_gesture_after(now, Duration::ZERO);
    }

    fn commit_tracker_gesture_after(&mut self, now: Instant, settle: Duration) {
        let Some(gesture) = self.tracker_gesture.finish(now, settle) else {
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
        let percussion = self
            .song
            .patterns
            .get(&pattern_number)
            .and_then(|pattern| pattern.pages.get(page_index))
            .is_some_and(|page| page.percussion);
        if percussion {
            let assignments = self
                .song
                .patterns
                .get(&pattern_number)
                .map(|pattern| drum_entry_lanes(pattern, row_index, page_index, &gesture.notes))
                .unwrap_or_else(|| vec![None; gesture.notes.len()]);
            let note_count = gesture.notes.len();
            let page_start = page_index * LANES_PER_PAGE;
            let mut entered = 0;
            if let Some(row) = self
                .song
                .patterns
                .get_mut(&pattern_number)
                .and_then(|pattern| pattern.rows.get_mut(row_index))
            {
                for ((note, velocity), lane) in
                    gesture.notes.into_iter().zip(assignments.into_iter())
                {
                    let Some(lane) = lane else {
                        continue;
                    };
                    if let Some(cell) = row.get_mut(page_start + lane) {
                        write_step_note(cell, note, velocity);
                        entered += 1;
                    }
                }
            }
            self.tracker_order = order;
            self.tracker_row = row_index;
            if entered == 0 {
                self.status = format!("drum row {row_index:02X} full · gesture ignored");
            } else {
                self.advance_tracker_row();
                self.status = if entered == note_count {
                    format!(
                        "drum gesture entered · advanced {} row(s)",
                        self.tracker_advance
                    )
                } else {
                    format!(
                        "drum gesture entered {entered}/{note_count} · row full · advanced {} row(s)",
                        self.tracker_advance
                    )
                };
            }
            return;
        }
        self.tracker_order = order;
        self.tracker_row = row_index;
        self.tracker_page = page_index;
        self.tracker_track = first_lane;
        self.write_edit_notes(&gesture.notes);
    }
    fn set_tracker_edit(&mut self, enabled: bool) {
        if enabled && (self.tracker_recording.is_some() || self.sequencer.status().playing) {
            self.tracker_stop();
        }
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

    fn leave_noob_on_percussion(&mut self) -> bool {
        if self.tracker_noob && self.current_page().is_some_and(|page| page.percussion) {
            self.silence_live_notes();
            self.tracker_noob = false;
            self.sync_tracker_route();
            self.status = "N00B off on Drums · current FT2 mode unchanged".into();
            true
        } else {
            false
        }
    }

    fn tracker_noob_allows(&self, note: u8) -> bool {
        !self.tracker_noob
            || self.current_page().is_some_and(|page| page.percussion)
            || self.noob_scale.contains(note)
    }

    fn sync_playback_noob(&self) {
        if let Ok(mut active) = self.playback_scale.lock() {
            *active = (self.playback_noob
                && self.screen_keeps_playback_workspace_active(self.screen))
            .then_some(self.noob_scale);
        }
    }

    fn silence_live_notes(&mut self) {
        if let Some(engine) = self.engine.as_ref() {
            engine.panic();
        }
        self.held_notes = HeldNotes::default();
        self.cancel_tracker_gesture();
        if let Some(recording) = self.tracker_recording.as_mut() {
            recording.active_lanes.clear();
        }
        self.release_tracker_audition();
    }

    fn toggle_playback_noob(&mut self) {
        self.silence_live_notes();
        self.playback_noob = !self.playback_noob;
        self.page_select_mode = false;
        self.sync_playback_noob();
        self.status = if self.playback_noob {
            format!(
                "N00B {} {} · turn the rotary to change scale",
                self.config.note_naming.pitch_name(self.noob_scale.root),
                self.noob_scale.kind.label()
            )
        } else {
            "Player N00B off · all chromatic notes enabled".into()
        };
    }

    fn adjust_playback_noob_scale(&mut self, direction: i8) {
        if !self.playback_noob || direction == 0 {
            return;
        }
        self.silence_live_notes();
        let kind = match self.noob_scale.kind {
            ScaleKind::Major => 0,
            ScaleKind::NaturalMinor => 1,
        };
        let current = usize::from(self.noob_scale.root) * 2 + kind;
        let next = wrapped_index(current, 24, direction);
        self.noob_scale = Scale {
            root: (next / 2) as u8,
            kind: if next % 2 == 0 {
                ScaleKind::Major
            } else {
                ScaleKind::NaturalMinor
            },
        };
        self.sync_playback_noob();
        self.status = format!(
            "N00B {} {} · outside notes stay silent",
            self.config.note_naming.pitch_name(self.noob_scale.root),
            self.noob_scale.kind.label()
        );
    }

    fn disable_tracker_noob(&mut self) {
        self.silence_live_notes();
        self.tracker_noob = false;
        self.sync_tracker_route();
        self.status = "FT2 N00B off · current mode unchanged · all chromatic notes enabled".into();
    }

    fn toggle_tracker_noob(&mut self) {
        if self.tracker_noob {
            self.disable_tracker_noob();
        } else if self.current_page().is_some_and(|page| page.percussion) {
            self.status = "N00B scale filter is unavailable on Drums".into();
        } else {
            self.silence_live_notes();
            self.tracker_noob = true;
            self.sync_tracker_route();
            self.status = format!(
                "FT2 N00B {} {} · current mode unchanged",
                self.config.note_naming.pitch_name(self.noob_scale.root),
                self.noob_scale.kind.label()
            );
        }
    }

    fn note_row_span_and_gate(&self) -> (usize, u8) {
        let numerator = usize::from(self.song.steps_per_beat) * 4;
        let denominator = self.note_length.denominator();
        if numerator < denominator {
            return (1, ((numerator * 100) / denominator).clamp(1, 100) as u8);
        }
        (numerator.div_ceil(denominator).max(1), 100)
    }

    fn write_edit_notes(&mut self, notes: &[(u8, u8)]) {
        let notes = notes
            .iter()
            .copied()
            .filter(|(note, _)| self.tracker_noob_allows(*note))
            .collect::<Vec<_>>();
        if notes.is_empty() {
            self.status = "N00B · notes outside scale ignored".into();
            return;
        }
        let pattern_number = self.tracker_pattern_number();
        let row_index = self.tracker_row;
        let page_index = self.tracker_page;
        let first_lane = self.tracker_track;
        let (span, gate) = self.note_row_span_and_gate();
        let Some(pattern) = self.song.patterns.get_mut(&pattern_number) else {
            return;
        };
        let page_start = page_index * LANES_PER_PAGE;
        let assignments = notes
            .iter()
            .enumerate()
            .map(|(offset, _)| Some((first_lane + offset) % LANES_PER_PAGE))
            .collect::<Vec<_>>();
        let mut entered = 0;
        for ((note, velocity), lane) in notes.iter().copied().zip(assignments) {
            let Some(lane) = lane else {
                continue;
            };
            let lane = page_start + lane;
            if let Some(cell) = pattern
                .rows
                .get_mut(row_index)
                .and_then(|row| row.get_mut(lane))
            {
                if cell.note == Note::On(note) {
                    cell.velocity = Some(velocity);
                    cell.gate = Some(gate);
                } else {
                    *cell = Cell {
                        note: Note::On(note),
                        velocity: Some(velocity),
                        gate: Some(gate),
                        ..Cell::default()
                    };
                }
                entered += 1;
            }
            let end = row_index.saturating_add(span);
            if gate == 100 && end < pattern.rows.len() {
                if let Some(cell) = pattern.rows.get_mut(end).and_then(|row| row.get_mut(lane)) {
                    if !matches!(cell.note, Note::On(_)) {
                        cell.note = Note::Off;
                        cell.velocity = None;
                        cell.program = None;
                        cell.gate = None;
                        if matches!(cell.command, Command::Retrigger(_)) {
                            cell.command = Command::None;
                        }
                    }
                }
            }
        }
        let rows = pattern.rows.len();
        if entered > 0 && rows > 0 {
            self.tracker_row = (row_index + self.tracker_advance) % rows;
        }
        self.status = if entered == 0 {
            format!("row {row_index:02X} full · note ignored")
        } else if entered == notes.len() {
            format!(
                "EDIT note entered · length {} · next row {:02X}",
                self.note_length.label(),
                self.tracker_row
            )
        } else {
            format!(
                "EDIT entered {entered}/{} · row full · next row {:02X}",
                notes.len(),
                self.tracker_row
            )
        };
    }
    fn sync_tracker_route(&mut self) -> bool {
        let engine_ready = self
            .tracker_software_route()
            .is_none_or(|route| self.ensure_tracker_engine_for(&route));
        self.configure_tracker_route(false);
        engine_ready
    }

    fn sync_tracker_route_for_navigation(&mut self) -> bool {
        let engine_ready = self
            .tracker_software_route()
            .is_none_or(|route| self.ensure_tracker_engine_for(&route));
        self.configure_tracker_route(true);
        engine_ready
    }

    fn configure_tracker_route(&mut self, preserve_notes: bool) {
        let Some(page) = self.current_page() else {
            return;
        };
        let column = *page.column(self.tracker_track);
        let external = self.tracker_external_config();
        let program = self
            .note_editor
            .as_ref()
            .and_then(|editor| editor.draft.program)
            .unwrap_or(column.program);
        let mut columns = std::array::from_fn(|index| {
            let setup = page.columns[index];
            (
                page.runtime_channel(index, &self.config.external_midi),
                (setup.program, setup.bank_msb, setup.bank_lsb),
            )
        });
        columns[self.tracker_track].1 .0 = program;
        let audition_note = self.note_editor.as_ref().and_then(|editor| {
            (columns[self.tracker_track].0 == 9)
                .then_some(editor.draft.note)
                .and_then(|note| match note {
                    Note::On(note) => Some(note),
                    Note::Empty | Note::Off => None,
                })
        });
        if let Ok(mut route) = self.tracker_route.lock() {
            if !preserve_notes {
                for (target, channel) in route.destinations() {
                    self.tracker_live_input.cancel(&target, channel);
                }
            }
            let config = crate::engine::TrackerRouteConfig {
                enabled: self.tracker_workspace_active(),
                target: page.target.clone(),
                columns,
                start_column: self.tracker_track,
                percussion: page.percussion || columns[self.tracker_track].0 == 9,
                audition_note,
                scale: (self.tracker_noob && !page.percussion).then_some(self.noob_scale),
                external: &external,
            };
            if preserve_notes {
                route.configure_navigation(config);
            } else {
                route.configure(config);
            }
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
        self.tracker_device_profile_for_page(page)
    }

    fn tracker_device_profile_for_page(&self, page: &sequencer::Page) -> Option<&DeviceProfile> {
        page.device_profile
            .as_deref()
            .and_then(|id| self.device_profiles.by_id(id))
            .or_else(|| {
                matches!(
                    page.target,
                    PageTarget::Default | PageTarget::ConfiguredExternal
                )
                .then(|| {
                    self.device_profiles
                        .by_id(&self.config.external_midi.profile)
                })
                .flatten()
            })
    }

    fn route_program_label(&self, page: &sequencer::Page, column_index: usize) -> String {
        let column = page.column(column_index);
        let channel = page.runtime_channel(column_index, &self.config.external_midi);
        if channel == 9 {
            return "GM DRUMS".into();
        }
        self.tracker_device_profile_for_page(page)
            .and_then(|profile| {
                profile.program_label(column.bank_msb, column.bank_lsb, column.program)
            })
            .unwrap_or_else(|| crate::gm::melodic_program(column.program).into())
    }

    fn tracker_program_label(&self, program: u8) -> String {
        let Some(page) = self.current_page() else {
            return crate::gm::melodic_program(program).into();
        };
        let column = page.column(self.tracker_track);
        let channel = page.runtime_channel(self.tracker_track, &self.config.external_midi);
        if channel == 9 {
            return "GM percussion · note chooses drum".into();
        }
        self.tracker_device_profile()
            .and_then(|profile| profile.program_label(column.bank_msb, column.bank_lsb, program))
            .unwrap_or_else(|| {
                format!(
                    "GM {:03} {}",
                    sequencer::musician_program(program),
                    crate::gm::melodic_program(program)
                )
            })
    }

    fn tracker_instrument_label(&self) -> String {
        let Some(page) = self.current_page() else {
            return "instrument unavailable".into();
        };
        let channel = page.runtime_channel(self.tracker_track, &self.config.external_midi);
        let column = page.column(self.tracker_track);
        let note = self
            .note_editor
            .as_ref()
            .and_then(|editor| match editor.draft.note {
                Note::On(note) => Some(note),
                Note::Empty | Note::Off => None,
            })
            .unwrap_or(36);
        if channel == 9 {
            format!(
                "GM DRUM {note} · {}",
                crate::gm::instrument(channel, column.program, note)
            )
        } else {
            self.tracker_program_label(column.program)
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
        if matches!(
            page.target,
            PageTarget::ActiveInstrument | PageTarget::Synthv1(_) | PageTarget::Software(_)
        ) {
            return Vec::new();
        }
        let column = page.column(self.tracker_track);
        let channel = page.runtime_channel(self.tracker_track, &self.config.external_midi);
        let mut messages = Vec::new();
        match external.bank_select {
            crate::config::BankSelectMode::Off => {}
            crate::config::BankSelectMode::Cc0 => {
                messages.push(vec![0xb0 | channel, 0, column.bank_msb]);
            }
            crate::config::BankSelectMode::Cc0Cc32 => {
                messages.push(vec![0xb0 | channel, 0, column.bank_msb]);
                messages.push(vec![0xb0 | channel, 32, column.bank_lsb]);
            }
        }
        messages.push(vec![0xc0 | channel, program]);
        messages
    }

    fn audition_keyboard_note(&mut self, note: u8, velocity: u8) {
        if !self.tracker_noob_allows(note) {
            return;
        }
        if let Some(route) = self.tracker_software_route() {
            if !self.ensure_tracker_engine_for(&route) {
                return;
            }
        }
        let Some(page) = self.current_page() else {
            return;
        };
        let target = page.target.clone();
        let channel = page.runtime_channel(self.tracker_track, &self.config.external_midi);
        for message in self.tracker_program_messages(page.column(self.tracker_track).program) {
            self.tracker_live_input.send(&target, &message);
        }
        self.tracker_live_input
            .send(&target, &[0x90 | channel, note, velocity]);
        let input = self.tracker_live_input.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(180));
            input.send(&target, &[0x80 | channel, note, 0]);
        });
    }
    fn move_tracker_lane(&mut self, direction: i8) {
        let current = self.tracker_page * LANES_PER_PAGE + self.tracker_track;
        let next = wrapped_index(current, self.current_total_lanes(), direction);
        let page = next / LANES_PER_PAGE;
        self.cancel_tracker_gesture();
        self.tracker_page = page;
        self.tracker_track = next % LANES_PER_PAGE;
        if !self.leave_noob_on_percussion() {
            self.sync_tracker_route();
        }
    }
    fn move_tracker_rotary_column(&mut self, direction: i8) {
        if self.tracker_mode == TrackerMode::Rec
            && self
                .tracker_recording
                .as_ref()
                .is_some_and(|recording| !recording.active_lanes.is_empty())
        {
            return;
        }
        let current = self.tracker_page * LANES_PER_PAGE + self.tracker_track;
        let next = wrapped_index(current, self.current_total_lanes(), direction);
        self.tracker_page = next / LANES_PER_PAGE;
        self.tracker_track = next % LANES_PER_PAGE;
        if let Some(recording) = self.tracker_recording.as_mut() {
            recording.page = self.tracker_page;
            recording.next_lane = self.tracker_track;
        }
        if self.tracker_noob && self.current_page().is_some_and(|page| page.percussion) {
            self.tracker_noob = false;
            self.status = "N00B off on Drums · current FT2 mode unchanged".into();
        }
        self.sync_tracker_route_for_navigation();
    }
    fn move_tracker_page(&mut self, direction: i8) {
        self.cancel_tracker_gesture();
        let pages = self.current_pages().len().max(1);
        let leaving_edge = (direction < 0 && self.tracker_page == 0)
            || (direction >= 0 && self.tracker_page + 1 >= pages);
        if leaving_edge {
            self.open_tracker_loop();
            return;
        }
        self.tracker_page = wrapped_index(self.tracker_page, pages, direction);
        if !self.leave_noob_on_percussion() && self.sync_tracker_route() {
            self.status = self
                .current_page()
                .map_or_else(|| "no page".into(), |page| format!("{} page", page.name));
        }
    }
    #[cfg(test)]
    fn switch_tracker_page(&mut self) {
        self.move_tracker_page(1);
    }
    fn refresh_page_targets(&mut self) {
        let mut targets = vec![PageTarget::Default];
        if let Some(route) = self
            .current_page()
            .and_then(|page| match &page.target {
                PageTarget::Software(route) => Some(route.clone()),
                _ => None,
            })
            .or_else(|| self.first_software_route())
        {
            targets.push(PageTarget::Software(route));
        }
        if !self.config.external_midi.output_match.is_empty() {
            targets.push(PageTarget::ConfiguredExternal);
        }
        targets.extend(
            self.available_page_outputs
                .iter()
                .map(|name| crate::midi_endpoint::stable_identity(name))
                .map(PageTarget::Midi),
        );
        if let Some(target) = self.current_page().map(|page| page.target.clone()) {
            targets.push(target);
        }
        targets.sort();
        targets.dedup();
        self.page_target_candidates = targets;
    }

    /// Build the passive overlay list from already-known ports. Unlike the
    /// full page manager refresh, opening ROUTE never creates a discovery
    /// client or touches ALSA/JACK merely to display the current state.
    fn refresh_overlay_target_candidates(&mut self) {
        let mut targets = vec![PageTarget::Default];
        if let Some(route) = self
            .current_page()
            .and_then(|page| match &page.target {
                PageTarget::Software(route) => Some(route.clone()),
                _ => None,
            })
            .or_else(|| self.first_software_route())
        {
            targets.push(PageTarget::Software(route));
        }
        if !self.config.external_midi.output_match.is_empty() {
            targets.push(PageTarget::ConfiguredExternal);
        }
        targets.extend(
            self.available_page_outputs
                .iter()
                .map(|name| crate::midi_endpoint::stable_identity(name))
                .map(PageTarget::Midi),
        );
        if let Some(target) = self.current_page().map(|page| page.target.clone()) {
            targets.push(target);
        }
        targets.sort();
        targets.dedup();
        self.page_target_candidates = targets;
    }
    fn target_online(&self, target: &PageTarget) -> bool {
        match target {
            PageTarget::Default => {
                if self.config.external_midi.enabled {
                    sequencer::matching_output_index(
                        &self.available_page_outputs,
                        &self.config.external_midi.output_match,
                        true,
                    )
                    .is_ok()
                } else {
                    self.engine.is_some()
                }
            }
            PageTarget::ActiveInstrument => false,
            PageTarget::Synthv1(name) => {
                self.engine.is_some()
                    && self.engine_owner.as_ref()
                        == Some(&EngineOwner::Tracker(SoftwareRoute::synthv1(name)))
            }
            PageTarget::Software(route) => {
                self.engine.is_some()
                    && self.engine_owner.as_ref() == Some(&EngineOwner::Tracker(route.clone()))
            }
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
    fn target_route_issue(&self, target: &PageTarget) -> Option<&'static str> {
        match target {
            PageTarget::Default => (!self.target_online(target)).then_some("OFFLINE"),
            PageTarget::ActiveInstrument => None,
            PageTarget::Synthv1(name) => self
                .preset_for_route(&SoftwareRoute::synthv1(name))
                .is_none()
                .then_some("MISSING"),
            PageTarget::Software(route) => {
                self.preset_for_route(route).is_none().then_some("MISSING")
            }
            PageTarget::ConfiguredExternal => {
                if !self.config.external_midi.enabled {
                    Some("OFFLINE")
                } else {
                    match sequencer::matching_output_index(
                        &self.available_page_outputs,
                        &self.config.external_midi.output_match,
                        true,
                    ) {
                        Ok(_) => None,
                        Err(error) if error.to_string().contains("ambiguous") => Some("AMBIG"),
                        Err(_) => Some("OFFLINE"),
                    }
                }
            }
            PageTarget::Midi(name) => {
                match sequencer::matching_output_index(&self.available_page_outputs, name, false) {
                    Ok(_) => None,
                    Err(error) if error.to_string().contains("ambiguous") => Some("AMBIG"),
                    Err(_) => Some("OFFLINE"),
                }
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
        self.release_tracker_audition();
        self.tracker_page = wrapped_index(self.tracker_page, self.current_pages().len(), direction);
        self.refresh_page_targets();
        self.sync_tracker_route();
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
                self.release_tracker_audition();
                self.tracker_page = page;
                self.tracker_track = 0;
                self.refresh_page_targets();
                self.sync_tracker_route();
                self.status = "page added · four empty lanes · choose target/channel".into();
            }
            Err(error) => self.status = format!("add page: {error}"),
        }
    }
    fn edit_page_target(&mut self) {
        if self.page_manager_mode != PageManagerMode::Pages {
            return;
        }
        let Some(current) = self.current_page().map(|page| page.target.clone()) else {
            return;
        };
        let software = match &current {
            PageTarget::Software(route) => Some(route.clone()),
            PageTarget::Synthv1(name) => Some(SoftwareRoute::synthv1(name)),
            _ => self.first_software_route(),
        };
        let external = match &current {
            PageTarget::ConfiguredExternal | PageTarget::Midi(_) => current.clone(),
            _ => PageTarget::ConfiguredExternal,
        };
        self.page_target_candidates = vec![PageTarget::Default];
        self.page_target_candidates
            .extend(software.map(PageTarget::Software));
        self.page_target_candidates.push(external);
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
        if self
            .current_page()
            .is_some_and(|page| page.target == PageTarget::Default)
        {
            self.status = "channel AUTO · choose an explicit target before assigning 1–16".into();
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
        self.release_tracker_audition();
        let mode = self.page_manager_mode;
        let selected_target = self
            .page_target_candidates
            .get(self.page_target_selected)
            .cloned();
        let channel = self.page_channel_draft;
        let track = self.tracker_track;
        let mut next_mode = PageManagerMode::Pages;
        if let Some(page) = self.current_page_mut() {
            match mode {
                PageManagerMode::Target => {
                    if let Some(target) = selected_target {
                        page.target = target;
                        if page.target == PageTarget::Default {
                            for column in &mut page.columns {
                                *column = sequencer::ColumnSetup::default();
                            }
                            page.setup.clear();
                        }
                        next_mode = match page.target {
                            PageTarget::Software(_) => PageManagerMode::Engine,
                            PageTarget::ConfiguredExternal | PageTarget::Midi(_) => {
                                PageManagerMode::MidiOutput
                            }
                            _ => PageManagerMode::Pages,
                        };
                    }
                }
                PageManagerMode::Engine => next_mode = PageManagerMode::Instrument,
                PageManagerMode::Instrument | PageManagerMode::MidiOutput => {}
                PageManagerMode::Channel => page.column_mut(track).channel = channel,
                PageManagerMode::Pages => return,
            }
        }
        self.page_manager_mode = next_mode;
        self.reset_context_page();
        if next_mode == PageManagerMode::Pages {
            self.sync_tracker_route();
            self.status = "page route updated · DONE to keep or CANCEL to restore".into();
        } else {
            self.status = match next_mode {
                PageManagerMode::Engine => "choose software ENGINE · press to confirm",
                PageManagerMode::Instrument => "choose engine INSTR · press to confirm",
                PageManagerMode::MidiOutput => "choose MIDI OUT · press to confirm",
                _ => unreachable!(),
            }
            .into();
        }
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
                self.page_target_selected = wrapped_index(
                    self.page_target_selected,
                    self.page_target_candidates.len(),
                    direction,
                );
            }
            PageManagerMode::Engine => {
                let current = self.current_page().and_then(|page| match &page.target {
                    PageTarget::Software(route) => Some(route.clone()),
                    _ => None,
                });
                let engines = self
                    .catalogs
                    .iter()
                    .filter(|catalog| !catalog.presets.is_empty())
                    .map(|catalog| catalog.backend)
                    .collect::<Vec<_>>();
                let index = current
                    .as_ref()
                    .and_then(|route| engines.iter().position(|engine| *engine == route.engine))
                    .unwrap_or(0);
                if let Some(engine) = engines.get(wrapped_index(index, engines.len(), direction)) {
                    let instrument = self
                        .catalogs
                        .iter()
                        .find(|catalog| catalog.backend == *engine)
                        .and_then(|catalog| catalog.presets.first())
                        .map(Preset::route_id);
                    if let (Some(page), Some(instrument)) = (self.current_page_mut(), instrument) {
                        page.target = PageTarget::Software(SoftwareRoute {
                            engine: *engine,
                            instrument,
                        });
                    }
                }
            }
            PageManagerMode::Instrument => {
                let Some(route) = self.current_page().and_then(|page| match &page.target {
                    PageTarget::Software(route) => Some(route.clone()),
                    _ => None,
                }) else {
                    return;
                };
                let instruments = self
                    .catalogs
                    .iter()
                    .find(|catalog| catalog.backend == route.engine)
                    .map(|catalog| {
                        catalog
                            .presets
                            .iter()
                            .map(Preset::route_id)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let index = instruments
                    .iter()
                    .position(|instrument| instrument == &route.instrument)
                    .unwrap_or(0);
                if let Some(instrument) = instruments
                    .get(wrapped_index(index, instruments.len(), direction))
                    .cloned()
                {
                    if let Some(page) = self.current_page_mut() {
                        page.target = PageTarget::Software(SoftwareRoute {
                            engine: route.engine,
                            instrument,
                        });
                    }
                }
            }
            PageManagerMode::MidiOutput => {
                let current = self.current_page().map(|page| page.target.clone());
                let mut outputs = vec![PageTarget::ConfiguredExternal];
                outputs.extend(
                    self.available_page_outputs
                        .iter()
                        .map(|name| crate::midi_endpoint::stable_identity(name))
                        .map(PageTarget::Midi),
                );
                if let Some(target @ (PageTarget::ConfiguredExternal | PageTarget::Midi(_))) =
                    current
                {
                    outputs.push(target);
                }
                outputs.sort();
                outputs.dedup();
                let index = self
                    .current_page()
                    .and_then(|page| outputs.iter().position(|target| target == &page.target))
                    .unwrap_or(0);
                if let Some(target) = outputs
                    .get(wrapped_index(index, outputs.len(), direction))
                    .cloned()
                {
                    if let Some(page) = self.current_page_mut() {
                        page.target = target;
                    }
                }
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
        "press encoder to confirm"
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
        self.loop_meter
            .set_audio_unavailable(AudioAvailability::Stopped);
        self.status = "tracker stopped".into();
    }
    fn toggle_tracker_playback(&mut self) {
        if self.tracker_recording.is_some() {
            self.stop_tracker_recording();
        } else if self.sequencer.status().playing {
            self.tracker_stop();
            return;
        }
        let mode_changed = self.tracker_mode != TrackerMode::Play;
        self.tracker_mode = TrackerMode::Play;
        if mode_changed {
            self.reset_context_page();
        }
        // Configure live input without starting the selected page's engine.
        // Playback preflight below starts only a software route that actually
        // has scheduled notes.
        self.configure_tracker_route(false);
        self.cancel_tracker_gesture();
        let (order, row) = (self.tracker_order, self.tracker_row);
        // The first pass starts at the cursor; every following pass starts at
        // row zero of this Arrangement Step, so preflight the complete loop.
        let messages = match sequencer::schedule(&self.song, &self.config.external_midi, order, 0) {
            Ok(messages) => messages,
            Err(error) => {
                self.status = format!("tracker cannot play: {error}");
                return;
            }
        };
        let notes = messages
            .iter()
            .filter(|message| {
                matches!(message.bytes.as_slice(), [status, _, velocity, ..]
                    if status & 0xf0 == 0x90 && *velocity > 0)
            })
            .count();
        let loop_status = self.loop_player.status();
        let loop_error = if self.song.audio_loop.is_some()
            && (!loop_status.loaded || loop_status.error.is_some())
            && !self.load_current_loop()
        {
            if notes == 0 {
                return;
            }
            Some(self.status.clone())
        } else {
            None
        };
        match scheduled_software_route(&messages) {
            Ok(Some(route)) if !self.ensure_tracker_engine_for(&route) => return,
            Ok(Some(_)) => {}
            // A blank software page is not a scheduled instrument. Keep an
            // already-owned FT2 engine available for live input, but do not
            // start one merely because a loop-only Project retains that page.
            Ok(None) => {}
            Err(error) => {
                self.status = format!("tracker cannot play: {error}");
                return;
            }
        }
        self.sequencer.play(&self.song, order, row);
        let offline = messages
            .iter()
            .filter(|message| {
                matches!(message.bytes.as_slice(), [status, _, velocity, ..]
                    if status & 0xf0 == 0x90 && *velocity > 0)
            })
            .filter_map(|message| message.target.as_ref())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter(|target| !self.target_online(target))
            .count();
        self.status = if let Some(error) = loop_error {
            format!("tracker playing · {notes} MIDI · {error}")
        } else if offline == 0 {
            format!(
                "tracker playing · {notes} MIDI · loop {}{}",
                if self.song.audio_loop.is_some() {
                    "on"
                } else {
                    "off"
                },
                if self.config.controller_clock.enabled {
                    " · controller clock on"
                } else {
                    ""
                }
            )
        } else {
            format!("tracker playing · {notes} events · {offline} target(s) offline")
        };
    }

    fn rewind_tracker(&mut self) {
        if self.tracker_recording.is_some() || self.sequencer.status().playing {
            self.tracker_stop();
        }
        self.tracker_order = 0;
        self.tracker_row = 0;
        self.clamp_tracker_cursor();
        self.status = "tracker rewound · press PLAY to start".into();
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

    fn toggle_tracker_recording(&mut self) {
        if self.tracker_recording.is_some() {
            self.finish_tracker_recording(false);
            return;
        }
        let transport = self.sequencer.status();
        if transport.playing {
            self.follow_tracker_transport(&transport);
        }
        let Some(target) = self.current_page().map(|page| page.target.clone()) else {
            self.status = "REC unavailable · current page is missing".into();
            return;
        };
        if let Some(route) = self.tracker_software_route() {
            if !self.ensure_tracker_engine_for(&route) {
                return;
            }
        } else if !self.target_online(&target) {
            self.status = "REC unavailable · selected page target is offline".into();
            return;
        }
        self.cancel_note_editor();
        self.set_tracker_mode(TrackerMode::Play);
        let pattern = self.tracker_pattern_number();
        let order = self.tracker_order;
        let page_index = self.tracker_page;
        self.tracker_row = 0;
        self.tracker_recording = Some(TrackerRecording {
            pattern,
            order,
            page: page_index,
            return_to_play: false,
            last_row: self.tracker_row,
            next_lane: self.tracker_track,
            active_lanes: HashMap::new(),
            notes: 0,
        });
        self.tracker_mode = TrackerMode::Rec;
        self.sync_tracker_route();
        self.reset_context_page();
        // Play replaces the current schedule and performs its own note cleanup.
        // Avoid a queued Stop between the old and recording schedules: that
        // transiently reports a stopped transport while REC is already active.
        self.sequencer
            .play(&self.tracker_record_song(pattern, page_index), 0, 0);
        self.status = format!(
            "REC pattern {pattern} · {} only · selected target",
            self.current_pages()
                .get(page_index)
                .map_or("page", |page| page.name.as_str())
        );
    }

    fn stop_tracker_recording(&mut self) -> bool {
        self.finish_tracker_recording(false)
    }

    fn finish_tracker_recording(&mut self, return_to_play: bool) -> bool {
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
                .enumerate()
                .map(|(lane, _)| page.runtime_channel(lane, &self.config.external_midi))
                .collect::<std::collections::BTreeSet<_>>()
            {
                self.tracker_live_input.cancel(&page.target, channel);
            }
        }
        if !return_to_play {
            self.sequencer.stop();
        }
        self.tracker_mode = TrackerMode::Play;
        self.sync_tracker_route();
        self.reset_context_page();
        self.status = if return_to_play {
            format!(
                "REC punch-out · {} notes · tracker playing",
                recording.notes
            )
        } else {
            format!(
                "REC stopped · {} notes in pattern {} page {}",
                recording.notes,
                recording.pattern,
                recording.page + 1
            )
        };
        true
    }

    fn record_tracker_midi(&mut self, bytes: &[u8]) {
        let row = self.sequencer.status().row;
        self.record_tracker_midi_at(row, bytes);
    }

    fn record_tracker_midi_at(&mut self, transport_row: usize, bytes: &[u8]) {
        if bytes.len() < 3 || !matches!(bytes[0] & 0xf0, 0x80 | 0x90) {
            return;
        }
        let channel = bytes[0] & 0x0f;
        let note = bytes[1];
        if !self.tracker_noob_allows(note) {
            return;
        }
        let note_on = bytes[0] & 0xf0 == 0x90 && bytes[2] > 0;
        if !note_on {
            let released = self.tracker_recording.as_mut().and_then(|recording| {
                let key = (channel, note);
                let released = recording
                    .active_lanes
                    .get_mut(&key)
                    .and_then(Vec::pop)
                    .map(|active| (recording.pattern, recording.page, active));
                let empty = recording.active_lanes.get(&key).is_some_and(Vec::is_empty);
                if empty {
                    recording.active_lanes.remove(&key);
                }
                released
            });
            if let Some((pattern_number, page, active)) = released {
                if let Some(pattern) = self.song.patterns.get_mut(&pattern_number) {
                    let rows = pattern.rows.len();
                    if rows > 0 {
                        let mut release_row = transport_row.min(rows - 1);
                        if release_row == active.start_row {
                            release_row = (release_row + 1) % rows;
                        }
                        let lane = page * LANES_PER_PAGE + active.lane;
                        if let Some(cell) = pattern
                            .rows
                            .get_mut(release_row)
                            .and_then(|row| row.get_mut(lane))
                        {
                            if !matches!(cell.note, Note::On(_)) {
                                *cell = Cell {
                                    note: Note::Off,
                                    ..Cell::default()
                                };
                            }
                        }
                    }
                }
                self.refresh_tracker_record_loop();
            }
            return;
        }
        let Some(recording) = self.tracker_recording.as_mut() else {
            return;
        };
        let Some(pattern) = self.song.patterns.get_mut(&recording.pattern) else {
            return;
        };
        let row = transport_row.min(pattern.rows.len().saturating_sub(1));
        let first_lane = recording.page * LANES_PER_PAGE;
        let lane = (0..LANES_PER_PAGE)
            .map(|offset| (recording.next_lane + offset) % LANES_PER_PAGE)
            .find(|lane| {
                !recording
                    .active_lanes
                    .values()
                    .flatten()
                    .any(|active| active.lane == *lane)
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
                            .any(|active| active.lane == *lane)
                    })
            });
        let Some(lane) = lane else {
            self.status = format!("REC row {row:02X} full · note ignored");
            return;
        };
        pattern.rows[row][first_lane + lane] = Cell {
            note: Note::On(note),
            velocity: Some(bytes[2]),
            gate: Some(100),
            ..Cell::default()
        };
        recording
            .active_lanes
            .entry((channel, note))
            .or_default()
            .push(RecordedLane {
                lane,
                start_row: row,
            });
        recording.next_lane = (lane + 1) % LANES_PER_PAGE;
        recording.notes += 1;
        self.tracker_track = lane;
        self.tracker_row = row;
        self.status = format!(
            "REC pattern {} · row {row:02X} · lane {}",
            recording.pattern,
            lane + 1
        );
        self.refresh_tracker_record_loop();
    }

    fn refresh_tracker_record_loop(&self) {
        if let Some(recording) = self.tracker_recording.as_ref() {
            self.sequencer
                .refresh_loop(&self.tracker_record_song(recording.pattern, recording.page));
        }
    }

    fn open_tracker_loop(&mut self) {
        self.loop_library_mode = false;
        self.set_screen(Screen::TrackerLoop);
        self.reset_context_page();
        self.status = if self.song.audio_loop.is_some() {
            "loop page ready".into()
        } else {
            format!(
                "loop page unloaded · {} WAV file(s) in inbox",
                self.loop_imports.len()
            )
        };
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
                self.retry_final_bus();
                true
            }
            Err(error) => {
                self.status = format!("loop load: {error}");
                false
            }
        }
    }

    fn unload_loop_player(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            if let Err(error) = engine.suspend_audio_graph() {
                self.status = format!("final bus suspend failed: {error:#}");
            }
        }
        self.loop_player.unload();
        self.loop_meter
            .set_audio_unavailable(AudioAvailability::Stopped);
    }

    fn retry_final_bus(&mut self) {
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        match engine.retry_audio_graph(&self.config, &self.song.insert_rack, &self.song.aux_routing)
        {
            Ok(true) => self.status.push_str(" · final bus active"),
            Ok(false) => {}
            Err(error) => {
                self.audio_fallback = Some(format!("final bus unavailable · {error:#}"));
                self.status.push_str(" · final bus unavailable");
            }
        }
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
        self.open_overlay(Action::OpenLoopLibrary);
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

    fn select_loop_library_entry(&mut self, index: usize) {
        let Some(entry) = self.loop_library.get(index).cloned() else {
            self.status = "private loop library is empty".into();
            return;
        };
        if self
            .song
            .audio_loop
            .as_ref()
            .is_some_and(|settings| settings.file == entry.file)
        {
            if self.load_current_loop() {
                self.status = format!("loop ready · {}", entry.file);
            }
            return;
        }
        let path = crate::loop_player::loops_dir().join(&entry.file);
        let decoded = match crate::loop_player::DecodedLoop::open(&path) {
            Ok(decoded) => decoded,
            Err(error) => {
                self.status = format!("loop browser: {error}");
                return;
            }
        };
        let alignment = crate::loop_player::analyze_alignment(
            &decoded,
            self.current_tempo(),
            self.current_meter(),
        );
        let settings = sequencer::LoopSettings {
            file: entry.file.clone(),
            source_bpm_x100: (alignment.source_bpm * 100.0).round() as u32,
            interpretation: sequencer::BpmInterpretation::Normal,
            start_beat: 0,
            length_beats: alignment.length_beats,
            offset_beats: 0,
        };
        self.song.audio_loop = Some(settings.clone());
        let tempo = self.apply_tracker_tempo(Self::loop_project_tempo(&settings));
        if self.load_loop_settings(Some(settings)) {
            self.status = format!("loop ready · {} · project {tempo} BPM", entry.file);
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
        self.ensure_tracker_engine();
        self.sync_tracker_route();
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
                self.loop_meter
                    .set_audio_unavailable(AudioAvailability::Stopped);
                if let Some(engine) = self.engine.as_mut() {
                    if let Err(error) = engine.suspend_audio_graph() {
                        self.status = format!("final bus suspend failed: {error:#}");
                        return;
                    }
                }
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
                        );
                        self.retry_final_bus();
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
        if self.confirm_routing_defaults {
            self.finish_routing_defaults_prompt(true);
            return;
        }
        let should_ask = self.should_prompt_routing_defaults();
        if should_ask {
            self.confirm_routing_defaults = true;
            self.reset_context_page();
            self.status = "Save this routing as the default for new patterns?".into();
            return;
        }
        self.save_song_file();
    }

    fn should_prompt_routing_defaults(&self) -> bool {
        self.current_pattern().is_some_and(|pattern| {
            !sequencer::pattern_has_note_events(pattern) && pattern.pages != self.routing_defaults
        })
    }

    fn resolve_routing_defaults_choice(&mut self, confirm: bool) -> Result<()> {
        self.confirm_routing_defaults = false;
        if confirm {
            let pages = self
                .current_pattern()
                .map(|pattern| pattern.pages.clone())
                .context("current Pattern is unavailable")?;
            sequencer::save_routing_defaults(&self.routing_defaults_path, &pages)?;
            self.routing_defaults = pages;
        }
        self.reset_context_page();
        Ok(())
    }

    fn finish_routing_defaults_prompt(&mut self, confirm: bool) {
        if !self.confirm_routing_defaults {
            return;
        }
        if let Err(error) = self.resolve_routing_defaults_choice(confirm) {
            self.status = format!("routing defaults: {error}");
            return;
        }
        self.save_song_file();
    }

    fn save_song_file(&mut self) {
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
        let mut song =
            Song::new_with_pages(&self.config.external_midi, self.routing_defaults.clone());
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
        self.prepare_first_tracker_instrument();
        self.set_screen(Screen::Tracker);
        self.refresh_page_targets();
        let engine_ready = self.sync_tracker_route();
        self.project_name_input = Some(name.clone());
        if engine_ready {
            self.status = format!("new project {name} · type a name or confirm the quick default");
        }
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
            Ok(mut song) => {
                if let Some(first) = self.first_synthv1_name() {
                    sequencer::upgrade_legacy_synth_routes(&mut song, &first);
                }
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
            Ok(mut song) => {
                if let Some(first) = self.first_synthv1_name() {
                    sequencer::upgrade_legacy_synth_routes(&mut song, &first);
                }
                let messages = sequencer::schedule(&song, &self.config.external_midi, 0, 0);
                match messages {
                    Ok(messages) => {
                        let notes = messages
                            .iter()
                            .filter(|message| {
                                message
                                    .bytes
                                    .first()
                                    .is_some_and(|status| status & 0xf0 == 0x90)
                            })
                            .count();
                        if notes == 0 && song.audio_loop.is_none() {
                            self.status = format!("{name} has no notes or loop to preview");
                            return;
                        }
                        match scheduled_software_route(&messages) {
                            Ok(Some(route)) if !self.ensure_tracker_engine_for(&route) => return,
                            Ok(Some(_)) => {}
                            Ok(None) => self.unload_owned_engine(|owner| {
                                matches!(owner, EngineOwner::Tracker(_))
                            }),
                            Err(error) => {
                                self.status = format!("song preview: {error}");
                                return;
                            }
                        }
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
        self.pattern_clear_beats = beats;
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
        let pattern = sequencer::Pattern::from_routing(
            &self.config.external_midi,
            rows,
            self.pattern_clear_beats,
            &self.routing_defaults,
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
        self.sync_tracker_route();
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
        let next = wrapped_offset(current, filtered.len(), amount);
        self.drum_pattern_selected = filtered[next];
        self.confirm_drum_pattern_delete = None;
    }
    fn cycle_drum_genre(&mut self, direction: isize) {
        let len = self.drum_genres().len();
        self.drum_genre_selected = wrapped_offset(self.drum_genre_selected, len, direction);
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
        self.sync_tracker_route();
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
        self.arrange_selected =
            wrapped_index(self.arrange_selected, self.song.order.len(), direction);
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
        self.sync_tracker_route();
        self.status = format!("editing pattern {}", self.tracker_pattern_number());
    }
    fn arrangement_play_from_step(&mut self) {
        self.tracker_order = self
            .arrange_selected
            .min(self.song.order.len().saturating_sub(1));
        self.tracker_row = 0;
        self.toggle_tracker_playback();
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
        if self
            .current_page()
            .is_some_and(|page| page.target == PageTarget::Default)
        {
            self.status =
                "program AUTO · choose an explicit target before assigning a program".into();
            return;
        }
        let track = self.tracker_track;
        if let Some(page) = self.current_page_mut() {
            let name = page.name.clone();
            let column = page.column_mut(track);
            column.program = if direction < 0 {
                column.program.saturating_sub(1)
            } else {
                column.program.saturating_add(1).min(127)
            };
            self.status = format!(
                "{name} column {} instrument/program {}",
                track + 1,
                sequencer::musician_program(column.program)
            );
            self.sync_tracker_route();
        }
    }
    fn change_bank(&mut self, msb: bool, direction: i8) {
        if self
            .current_page()
            .is_some_and(|page| page.target == PageTarget::Default)
        {
            self.status = "bank AUTO · choose an explicit target before assigning a bank".into();
            return;
        }
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
    fn ensure_explicit_capture_tracks(&mut self) {
        if self.config.capture.tracks.is_empty() {
            self.config.capture.tracks = self.config.capture.effective_tracks();
        }
        self.audio_track_selected = self
            .audio_track_selected
            .min(self.config.capture.tracks.len().saturating_sub(1));
    }

    fn persist_capture_tracks(&mut self, state: &Path, message: String) {
        let path = state.join("shsynth.conf");
        let result = self.config.save(&path).and_then(|()| {
            self.audio_recorder
                .update_configuration(self.config.capture.clone(), self.capture_sources.clone())
        });
        self.status = match result {
            Ok(()) => message,
            Err(error) => format!("capture configuration: {error}"),
        };
    }

    fn move_audio_track(&mut self, direction: i8) {
        let length = self.audio_recorder.status().tracks.len();
        self.audio_track_selected = wrapped_index(self.audio_track_selected, length, direction);
    }

    fn toggle_audio_track_arm(&mut self, state: &Path) {
        if self.audio_recorder.status().recording {
            self.status = "stop the take before changing track arms".into();
            return;
        }
        self.ensure_explicit_capture_tracks();
        let Some(track) = self
            .config
            .capture
            .tracks
            .get_mut(self.audio_track_selected)
        else {
            self.status = "no recording track selected".into();
            return;
        };
        track.armed = !track.armed;
        let message = format!(
            "{} · {}",
            track.label,
            if track.armed { "armed" } else { "disarmed" }
        );
        self.persist_capture_tracks(state, message);
    }

    fn set_all_audio_arms(&mut self, state: &Path, armed: bool) {
        if self.audio_recorder.status().recording {
            self.status = "stop the take before changing track arms".into();
            return;
        }
        self.ensure_explicit_capture_tracks();
        let mut armed_count = 0;
        let mut missing_count = 0;
        for track in &mut self.config.capture.tracks {
            let resolved = !track.preferred_source.is_empty()
                && self
                    .capture_sources
                    .iter()
                    .any(|source| source == &track.preferred_source);
            track.armed = armed && resolved;
            armed_count += usize::from(track.armed);
            missing_count += usize::from(armed && !resolved);
        }
        let message = if armed {
            format!("armed {armed_count} resolved tracks · {missing_count} missing left safe")
        } else {
            "all recording tracks disarmed".into()
        };
        self.persist_capture_tracks(state, message);
    }

    fn refresh_audio_sources(&mut self) {
        if self.audio_recorder.status().recording {
            self.status = "source refresh waits until the take is stopped".into();
            return;
        }
        self.capture_sources = engine::jack_capture_sources();
        let result = self
            .audio_recorder
            .update_configuration(self.config.capture.clone(), self.capture_sources.clone());
        self.status = match result {
            Ok(()) => format!(
                "found {} JACK recording sources",
                self.capture_sources.len()
            ),
            Err(error) => format!("source refresh: {error}"),
        };
    }

    fn assign_audio_source(&mut self, state: &Path) {
        if self.audio_recorder.status().recording {
            self.status = "stop the take before assigning a source".into();
            return;
        }
        self.capture_sources = engine::jack_capture_sources();
        self.ensure_explicit_capture_tracks();
        let Some(track) = self
            .config
            .capture
            .tracks
            .get_mut(self.audio_track_selected)
        else {
            self.status = "no recording track selected".into();
            return;
        };
        let next = if track.preferred_source.is_empty() {
            self.capture_sources.first().cloned()
        } else {
            self.capture_sources
                .iter()
                .position(|source| source == &track.preferred_source)
                .and_then(|index| self.capture_sources.get(index + 1).cloned())
        };
        track.preferred_source = next.unwrap_or_default();
        let message = if track.preferred_source.is_empty() {
            format!("{} source cleared · track is missing", track.label)
        } else {
            format!("{} source assigned deliberately", track.label)
        };
        self.persist_capture_tracks(state, message);
    }

    fn begin_audio_track_name(&mut self) {
        if self.audio_recorder.status().recording {
            self.status = "stop the take before naming a track".into();
            return;
        }
        self.ensure_explicit_capture_tracks();
        if let Some(track) = self.config.capture.tracks.get(self.audio_track_selected) {
            self.audio_track_name_input = Some(track.label.clone());
            self.status = "TRACK NAME · type, Enter confirms, Esc cancels".into();
        }
    }

    fn commit_audio_track_name(&mut self, state: &Path) {
        let Some(input) = self.audio_track_name_input.clone() else {
            return;
        };
        let label = input.trim();
        if label.is_empty()
            || label.chars().any(char::is_control)
            || label.contains('|')
            || label.chars().count() > 64
        {
            self.status = "track name must be 1–64 printable characters without |".into();
            return;
        }
        self.ensure_explicit_capture_tracks();
        if let Some(track) = self
            .config
            .capture
            .tracks
            .get_mut(self.audio_track_selected)
        {
            track.label = label.into();
            self.audio_track_name_input = None;
            self.persist_capture_tracks(state, format!("track named {label}"));
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
        self.engine_owner = Some(EngineOwner::SoftwareSynth);
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

    fn transport_indicator(&self) -> TransportIndicator {
        if self.tracker_recording.is_some()
            || self.audio_recorder.status().recording
            || self.recorder.is_recording()
            || self
                .engine
                .as_ref()
                .is_some_and(Engine::final_recording_active)
        {
            TransportIndicator::Record
        } else if self.playback.is_some()
            || self.sequencer.status().playing
            || self.loop_player.status().playing
            || self.song_previewing
        {
            TransportIndicator::Play
        } else if self.screen == Screen::Tracker && self.tracker_mode == TrackerMode::Play {
            TransportIndicator::Pause
        } else {
            TransportIndicator::Stop
        }
    }

    fn audio_graph_edit_blocker(&self) -> Option<&'static str> {
        if !self.config.audio_graph.enabled {
            return None;
        }
        if self.audio_recorder.status().recording
            || self.recorder.is_recording()
            || self
                .engine
                .as_ref()
                .is_some_and(Engine::final_recording_active)
        {
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

    fn move_bus_selection(&mut self, direction: i8) {
        self.bus_selected = wrapped_index(self.bus_selected, 4, direction);
        self.status = format!("final bus · {} selected", self.bus_selection_label());
    }

    fn bus_selection_label(&self) -> &'static str {
        if self.bus_selected < 3 {
            BusSource::ALL[self.bus_selected].label()
        } else {
            "MASTER"
        }
    }

    fn adjust_bus_level(&mut self, direction: i8) {
        let Some(controls) = self.engine.as_ref().and_then(Engine::bus_controls) else {
            self.status = "final bus unavailable · load loop, input, and synth first".into();
            return;
        };
        let delta = if direction < 0 { -1.0 } else { 1.0 };
        let value = if self.bus_selected < 3 {
            let source = BusSource::ALL[self.bus_selected];
            let value = (controls.source_gain_db(source) + delta)
                .clamp(SOURCE_GAIN_MIN_DB, SOURCE_GAIN_MAX_DB);
            controls.set_source_gain_db(source, value);
            value
        } else {
            let value =
                (controls.master_gain_db() + delta).clamp(MASTER_GAIN_MIN_DB, MASTER_GAIN_MAX_DB);
            controls.set_master_gain_db(value);
            value
        };
        self.status = format!("{} level {value:.0} dB", self.bus_selection_label());
    }

    fn toggle_bus_mute(&mut self) {
        if self.bus_selected >= 3 {
            self.status = "master has level only · source mutes remain explicit".into();
            return;
        }
        let Some(controls) = self.engine.as_ref().and_then(Engine::bus_controls) else {
            self.status = "final bus unavailable".into();
            return;
        };
        let source = BusSource::ALL[self.bus_selected];
        let muted = !controls.source_muted(source);
        controls.set_source_muted(source, muted);
        self.status = format!(
            "{} {}",
            source.label(),
            if muted { "muted" } else { "ready" }
        );
    }

    fn toggle_final_recording(&mut self) {
        let Some(engine) = self.engine.as_mut() else {
            self.status = "final recording unavailable · owned graph is inactive".into();
            return;
        };
        let active = engine.final_recording_active();
        let result = if active {
            engine.stop_final_recording()
        } else {
            engine.start_final_recording(None)
        };
        let final_status = engine.final_recording_status().unwrap_or_default();
        self.final_recording_last = final_status.clone();
        self.status = match result {
            Ok(()) if active => final_status.path.map_or_else(
                || "final recording stopped".into(),
                |path| format!("final recording saved · {}", path.display()),
            ),
            Ok(()) => "final recording armed · begins on next audio callback".into(),
            Err(error) => format!("final recording: {error:#}"),
        };
    }

    fn selected_effect_id(&self) -> Option<EffectId> {
        let FxRackSelection::Effect(selected) = self.fx_selection else {
            return None;
        };
        project_fx_rack(
            &self.song.insert_rack,
            &self.song.aux_routing,
            self.fx_target,
        )?
        .order
        .contains(&selected)
        .then_some(selected)
    }

    fn fx_selection_index(&self) -> usize {
        let Some(rack) = project_fx_rack(
            &self.song.insert_rack,
            &self.song.aux_routing,
            self.fx_target,
        ) else {
            return 0;
        };
        match self.fx_selection {
            FxRackSelection::Effect(id) => rack
                .order
                .iter()
                .position(|candidate| *candidate == id)
                .unwrap_or(rack.order.len()),
            FxRackSelection::Insert => rack.order.len(),
        }
    }

    fn move_fx_rack_selection(&mut self, direction: i8) {
        let Some(rack) = project_fx_rack(
            &self.song.insert_rack,
            &self.song.aux_routing,
            self.fx_target,
        ) else {
            self.fx_selection = FxRackSelection::Insert;
            return;
        };
        let index = wrapped_index(self.fx_selection_index(), rack.order.len() + 1, direction);
        self.fx_selection = rack
            .order
            .get(index)
            .copied()
            .map(FxRackSelection::Effect)
            .unwrap_or(FxRackSelection::Insert);
        self.status = match self.fx_selection {
            FxRackSelection::Effect(id) => format!("FX #{id} selected"),
            FxRackSelection::Insert => "insert effect selected".into(),
        };
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

    fn arm_fx_pickup(&mut self) {
        let targets = self
            .selected_effect()
            .map(|effect| {
                crate::effect_schema::controls(effect.kind)
                    .iter()
                    .enumerate()
                    .zip(CONTROLS.iter())
                    .filter_map(|((index, _mapping), control)| {
                        let spec = crate::effect_schema::controlled_parameter(effect.kind, index)?;
                        let value = effect
                            .parameters
                            .get(spec.name)
                            .copied()
                            .unwrap_or(spec.default);
                        let normalized = match spec.value_type {
                            crate::effect_schema::ParameterType::Choices(choices) => choices
                                .iter()
                                .position(|choice| f32::from(*choice) == value)
                                .map(|index| {
                                    index as f32 / choices.len().saturating_sub(1).max(1) as f32
                                })
                                .unwrap_or(0.0),
                            _ => (value - spec.minimum) / (spec.maximum - spec.minimum).max(1.0),
                        };
                        Some((control.cc, normalized))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.fx_pickup.arm(targets);
    }

    fn apply_fx_control(&mut self, cc: u8, value: f32) {
        let Some(control_index) = CONTROLS.iter().position(|control| control.cc == cc) else {
            return;
        };
        let Some(control) = crate::control::by_cc(cc) else {
            return;
        };
        let normalized = crate::control::normalize(control, value);
        if !self.fx_pickup.accept(cc, normalized) {
            self.status = format!("{} waiting for pickup", fx_hardware_label(control_index));
            return;
        }
        let Some(id) = self.selected_effect_id() else {
            return;
        };
        let mut rack = self.song.insert_rack.clone();
        let mut aux = self.song.aux_routing.clone();
        let effect = project_fx_rack_mut(&mut rack, &mut aux, self.fx_target)
            .and_then(|rack| rack.effect_mut(id))
            .expect("selected effect has a valid rack");
        let Some(spec) = crate::effect_schema::controlled_parameter(effect.kind, control_index)
        else {
            self.status = format!(
                "{} has no parameter on this effect",
                fx_hardware_label(control_index)
            );
            return;
        };
        if is_aux_target(self.fx_target)
            && matches!(spec.name, "dry_percent" | "wet_percent" | "mix_percent")
        {
            self.status = "aux wet/dry controls are fixed at 100% wet".into();
            return;
        }
        let mapped = match spec.value_type {
            crate::effect_schema::ParameterType::Continuous => {
                spec.minimum + normalized * (spec.maximum - spec.minimum)
            }
            crate::effect_schema::ParameterType::Integer => {
                (spec.minimum + normalized * (spec.maximum - spec.minimum)).round()
            }
            crate::effect_schema::ParameterType::Toggle => {
                if normalized >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            crate::effect_schema::ParameterType::Choices(choices) => {
                let index = (normalized * choices.len().saturating_sub(1) as f32).round() as usize;
                f32::from(choices[index.min(choices.len() - 1)])
            }
        };
        effect.parameters.insert(spec.name.into(), mapped);
        self.fx_parameter = control_index;
        self.commit_fx_routing(
            rack,
            aux,
            format!(
                "{} · {} · {mapped:.2} {}",
                fx_hardware_label(control_index),
                spec.name,
                spec.unit
            ),
        );
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
        let selected_exists = project_fx_rack(
            &self.song.insert_rack,
            &self.song.aux_routing,
            self.fx_target,
        )
        .is_some_and(|rack| match self.fx_selection {
            FxRackSelection::Effect(id) => rack.order.contains(&id),
            FxRackSelection::Insert => true,
        });
        if !selected_exists {
            self.fx_selection = FxRackSelection::Insert;
        }
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
        let original_rack = self.song.insert_rack.clone();
        let original_aux = self.song.aux_routing.clone();
        let kind = self
            .selectable_effect_kinds(None)
            .first()
            .copied()
            .unwrap_or(INSERT_EFFECTS[0]);
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
                let length = project_fx_rack(&rack, &aux, self.fx_target)
                    .map(|rack| rack.order.len())
                    .unwrap_or(0);
                let index = self.fx_selection_index().min(length.saturating_sub(1));
                if length > 1 {
                    if let Some(target) = project_fx_rack_mut(&mut rack, &mut aux, self.fx_target) {
                        if let Err(error) = target.move_to(id, index) {
                            self.status = format!("FX insert: {error}");
                            return;
                        }
                    }
                }
                if self.commit_fx_routing(
                    rack,
                    aux,
                    format!("{} inserted · turn to choose type", effect_kind_label(kind)),
                ) {
                    self.fx_selection = FxRackSelection::Effect(id);
                    self.fx_parameter = 0;
                    self.fx_type_edit = Some(FxTypeEdit {
                        original_rack,
                        original_aux,
                        effect_id: id,
                        provisional: true,
                    });
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
                self.fx_selection = FxRackSelection::Insert;
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
        let current = self.fx_selection_index();
        let destination = if direction < 0 {
            current.saturating_sub(1)
        } else {
            let length = project_fx_rack(
                &self.song.insert_rack,
                &self.song.aux_routing,
                self.fx_target,
            )
            .map(|rack| rack.order.len())
            .unwrap_or(0);
            (current + 1).min(length - 1)
        };
        if destination == current {
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
        self.commit_fx_routing(rack, aux, format!("moved FX #{id}"));
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
        if let Some(edit) = self.fx_type_edit.clone() {
            let Some(effect) = self.selected_effect() else {
                return;
            };
            let choices = self.selectable_effect_kinds(Some(edit.effect_id));
            let current = choices
                .iter()
                .position(|kind| *kind == effect.kind)
                .unwrap_or(0);
            let next = if direction < 0 {
                current.checked_sub(1).unwrap_or(choices.len() - 1)
            } else {
                (current + 1) % choices.len()
            };
            self.set_selected_effect_kind(choices[next]);
            return;
        }
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

    fn valid_effect_kinds(&self) -> Vec<EffectKind> {
        INSERT_EFFECTS
            .iter()
            .copied()
            .filter(|kind| !is_aux_target(self.fx_target) || kind.requires_wet_aux())
            .collect()
    }

    fn selectable_effect_kinds(&self, edited_id: Option<EffectId>) -> Vec<EffectKind> {
        let valid = self.valid_effect_kinds();
        let used = project_fx_rack(
            &self.song.insert_rack,
            &self.song.aux_routing,
            self.fx_target,
        )
        .into_iter()
        .flat_map(|rack| rack.effects.iter())
        .filter(|effect| Some(effect.id) != edited_id)
        .map(|effect| effect.kind)
        .collect::<Vec<_>>();
        let unused = valid
            .iter()
            .copied()
            .filter(|kind| !used.contains(kind))
            .collect::<Vec<_>>();
        if unused.is_empty() {
            valid
        } else {
            unused
        }
    }

    fn begin_effect_type_edit(&mut self) {
        let Some(id) = self.selected_effect_id() else {
            self.add_effect();
            return;
        };
        self.fx_type_edit = Some(FxTypeEdit {
            original_rack: self.song.insert_rack.clone(),
            original_aux: self.song.aux_routing.clone(),
            effect_id: id,
            provisional: false,
        });
        self.status = "TYPE ACTIVE · turn to browse · press confirms · Back cancels".into();
    }

    fn set_selected_effect_kind(&mut self, kind: EffectKind) {
        let Some(id) = self.selected_effect_id() else {
            return;
        };
        let mut rack = self.song.insert_rack.clone();
        let mut aux = self.song.aux_routing.clone();
        let effect = project_fx_rack_mut(&mut rack, &mut aux, self.fx_target)
            .and_then(|rack| rack.effect_mut(id))
            .expect("selected effect has a valid rack");
        effect.kind = kind;
        effect.parameters = crate::effect_schema::defaults(kind);
        if is_aux_target(self.fx_target) {
            for (name, value) in [
                ("dry_percent", 0.0),
                ("wet_percent", 100.0),
                ("mix_percent", 100.0),
            ] {
                if effect.parameters.contains_key(name) {
                    effect.parameters.insert(name.into(), value);
                }
            }
        }
        if self.commit_fx_routing(
            rack,
            aux,
            format!("TYPE ACTIVE · {}", effect_kind_label(kind)),
        ) {
            self.fx_parameter = 0;
        }
    }

    fn confirm_effect_type_edit(&mut self) {
        let Some(edit) = self.fx_type_edit.take() else {
            return;
        };
        self.status = format!(
            "{} type confirmed",
            if edit.provisional {
                "new effect"
            } else {
                "effect"
            }
        );
    }

    fn cancel_effect_type_edit(&mut self) {
        let Some(edit) = self.fx_type_edit.clone() else {
            return;
        };
        let message = if edit.provisional {
            "new effect cancelled and removed"
        } else {
            "effect type change cancelled"
        };
        if self.commit_fx_routing(edit.original_rack, edit.original_aux, message.into()) {
            self.fx_type_edit = None;
        }
    }

    fn cycle_fx_target(&mut self) {
        self.fx_target = (self.fx_target + 1) % (MAX_AUX_BUSES + 2);
        self.fx_selection = FxRackSelection::Insert;
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
        let controls = crate::effect_schema::controls(effect.kind);
        self.fx_parameter = self.fx_parameter.min(controls.len().saturating_sub(1));
        let spec = crate::effect_schema::controlled_parameter(effect.kind, self.fx_parameter)
            .expect("effect control layout references its persisted parameter");
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
                let index = wrapped_index(current_index, choices.len(), direction);
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

    fn move_fx_parameter(&mut self, direction: i8) {
        let len = self
            .selected_effect()
            .map(|effect| crate::effect_schema::controls(effect.kind).len())
            .unwrap_or(0);
        self.fx_parameter = wrapped_index(self.fx_parameter, len, direction);
        self.status = "FX parameter highlighted · press encoder to edit".into();
    }

    fn begin_fx_value_edit(&mut self) {
        if self.selected_effect().is_none() {
            self.status = "FX rack is empty".into();
            return;
        }
        self.fx_edit_original =
            Some((self.song.insert_rack.clone(), self.song.aux_routing.clone()));
        self.fx_value_editing = true;
        self.fx_numeric_input = None;
        self.status = "FX VALUE ACTIVE · turn, press confirms, Back cancels".into();
    }

    fn confirm_fx_value_edit(&mut self) {
        self.fx_value_editing = false;
        self.fx_edit_original = None;
        self.fx_numeric_input = None;
        self.status = "FX value confirmed".into();
    }

    fn cancel_fx_value_edit(&mut self) {
        let Some((rack, aux)) = self.fx_edit_original.clone() else {
            self.fx_value_editing = false;
            return;
        };
        if self.commit_fx_routing(rack, aux, "FX value change cancelled".into()) {
            self.fx_value_editing = false;
            self.fx_edit_original = None;
            self.fx_numeric_input = None;
        }
    }

    fn begin_fx_numeric_entry(&mut self, character: char) {
        if !self.fx_value_editing {
            self.begin_fx_value_edit();
        }
        let input = self.fx_numeric_input.get_or_insert_with(String::new);
        if input.len() < 16 {
            input.push(character);
        }
        self.status = format!("numeric value · {input}_ · Enter confirms");
    }

    fn commit_fx_numeric_entry(&mut self) {
        let Some(input) = self.fx_numeric_input.clone() else {
            self.confirm_fx_value_edit();
            return;
        };
        let Ok(value) = input.parse::<f32>() else {
            self.status = format!("invalid number · {input}");
            return;
        };
        let Some(id) = self.selected_effect_id() else {
            return;
        };
        let mut rack = self.song.insert_rack.clone();
        let mut aux = self.song.aux_routing.clone();
        let effect = project_fx_rack_mut(&mut rack, &mut aux, self.fx_target)
            .and_then(|rack| rack.effect_mut(id))
            .expect("selected effect has a valid rack");
        let spec = crate::effect_schema::controlled_parameter(effect.kind, self.fx_parameter)
            .expect("effect control layout references its persisted parameter");
        if !spec.accepts(value) {
            self.status = format!(
                "{} RANGE · {:.2}..{:.2} {}",
                crate::effect_schema::abbreviation(spec.name),
                spec.minimum,
                spec.maximum,
                spec.unit
            );
            return;
        }
        if is_aux_target(self.fx_target)
            && matches!(spec.name, "dry_percent" | "wet_percent" | "mix_percent")
        {
            self.status = "aux wet/dry controls are fixed at 100% wet".into();
            return;
        }
        effect.parameters.insert(spec.name.into(), value);
        let displayed = crate::effect_schema::format_value(effect.kind, spec, value);
        if self.commit_fx_routing(
            rack,
            aux,
            format!(
                "{} · {displayed}",
                crate::effect_schema::abbreviation(spec.name)
            ),
        ) {
            self.fx_value_editing = false;
            self.fx_edit_original = None;
            self.fx_numeric_input = None;
        }
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
        self.engine_owner = None;
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
            Ok(mut e) => {
                e.bind_midi_lifecycle(self.midi_lifecycle.clone());
                let audio_route = e.audio_route_status();
                self.audio_fallback = audio_route
                    .as_ref()
                    .filter(|route| route.contains("fallback") || route.contains("unavailable"))
                    .cloned();
                self.engine = Some(e);
                self.engine_owner = Some(EngineOwner::SoftwareSynth);
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
    fn toggle_idea_recording(&mut self) {
        self.idea_mode = IdeaMode::Record;
        if self.recorder.is_recording() {
            self.stop_recording();
            return;
        }
        if self.playback.is_some() {
            self.stop_playback();
        }
        if self.engine.is_none() {
            self.status = "load a preset before recording".into();
            return;
        }
        self.recorder.start(Instant::now());
        self.status = "● RECORDING musical MIDI".into();
    }
    fn toggle_playback(&mut self) {
        self.idea_mode = IdeaMode::Play;
        if self
            .playback
            .as_ref()
            .is_some_and(|playback| playback.finished.load(Ordering::Relaxed))
        {
            self.playback.take();
        }
        if self.playback.is_some() {
            self.stop_playback();
            return;
        }
        if self.recorder.is_recording() {
            self.stop_recording();
        }
        if self.engine.is_none() {
            self.status = "load the idea preset before playing its recording".into();
        } else if self.last.is_empty() {
            self.status = "no recording yet".into();
        } else {
            if let Some(engine) = &self.engine {
                engine.panic();
            }
            let events = self.last.clone();
            let output = Arc::clone(&self.midi_output);
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let finished = Arc::new(AtomicBool::new(false));
            let worker_finished = Arc::clone(&finished);
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
                worker_finished.store(true, Ordering::Relaxed);
            });
            self.playback = Some(Playback {
                stop,
                finished,
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
        self.set_tracker_mode(TrackerMode::Play);
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
        self.help_selected = wrapped_offset(
            self.help_selected,
            help::lines(HELP_TEXT_WIDTH).len(),
            delta,
        );
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
        if self.recorder.is_recording() {
            self.stop_recording();
        }
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
                    Ok(mut engine) => {
                        engine.bind_midi_lifecycle(self.midi_lifecycle.clone());
                        let audio_route = engine.audio_route_status();
                        self.audio_fallback = audio_route
                            .as_ref()
                            .filter(|route| {
                                route.contains("fallback") || route.contains("unavailable")
                            })
                            .cloned();
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
        if let Some(session) = self.controller_learn.as_mut() {
            session.tick(now);
        }
        self.refresh_cpu_temperature(now);
        if !self.loop_meter.is_presentation() {
            if let Some(snapshot) = self.loop_player.meter_snapshot() {
                self.loop_meter.update_loop_audio(snapshot, now);
            } else {
                self.loop_meter
                    .set_audio_unavailable(AudioAvailability::Stopped);
            }
        }
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
            if let Some(recording) = self.tracker_recording.as_mut() {
                recording.last_row = tracker.row;
            }
            if tracker.playing && !tracker.available {
                self.cancel_tracker_gesture();
                if let Some(error) = tracker.error {
                    self.status = format!("tracker target unavailable: {error}");
                }
            }
            self.tracker_fallback = tracker
                .playing
                .then(|| tracker.fallbacks.values().next().cloned())
                .flatten();
        }
        if let Some(status) = self.engine.as_mut().and_then(Engine::poll_audio_graph) {
            self.performance_meter
                .set_audio_unavailable(AudioAvailability::DirectUnavailable);
            self.status = status;
        }
        if let Some(status) = self
            .engine
            .as_mut()
            .and_then(Engine::final_recording_status)
        {
            self.final_recording_last = status;
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
            let transport_order = tracker.order.min(self.song.order.len().saturating_sub(1));
            let punch_order = self
                .tracker_recording
                .as_ref()
                .and_then(|recording| recording.return_to_play.then_some(transport_order));
            let order_changed = punch_order.is_some_and(|order| {
                self.tracker_recording
                    .as_ref()
                    .is_some_and(|recording| recording.order != order)
            });
            if let Some(order) = punch_order {
                if let Some(recording) = self.tracker_recording.as_mut() {
                    recording.order = order;
                    recording.pattern = self.song.order.get(order).copied().unwrap_or(0);
                    recording.active_lanes.clear();
                }
            }
            self.tracker_order = self
                .tracker_recording
                .as_ref()
                .map_or(transport_order, |recording| recording.order);
            self.tracker_row = tracker.row.min(self.tracker_rows().saturating_sub(1));
            if order_changed {
                self.sync_tracker_route();
            }
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
    app_loop(
        &mut terminal,
        catalogs,
        state,
        config,
        io::stdin().is_terminal(),
    )
}

fn midi_input_available(router: Option<&engine::MidiRouter>) -> bool {
    router.is_some_and(|router| {
        router.availability().controller_available()
            || router
                .availability()
                .performance
                .iter()
                .any(crate::engine::MidiInputState::available)
    })
}

fn expected_startup_midi(router: Option<&engine::MidiRouter>) -> Option<String> {
    let availability = router?.availability();
    availability
        .controller
        .as_ref()
        .filter(|input| !input.available())
        .or_else(|| {
            availability
                .performance
                .iter()
                .find(|input| !input.available())
        })
        .map(|input| input.wanted.clone())
}

fn app_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    catalogs: &[Catalog],
    state: &Path,
    config: &RuntimeConfig,
    mut terminal_keyboard: bool,
) -> Result<()> {
    let stopping = Arc::new(AtomicBool::new(false));
    for sig in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        signal_hook::flag::register(sig, Arc::clone(&stopping))?;
    }
    let splash_started = Instant::now();
    terminal.draw(|frame| {
        crate::startup_splash::draw(
            frame,
            splash_started.elapsed(),
            terminal_keyboard,
            None,
            BUILD_BADGE,
        )
    })?;
    let (tx, rx) = mpsc::channel();
    let controller_notice = auto_wire_controller(state, config)
        .err()
        .map(|error| error.to_string());
    let mut router = engine::MidiRouter::start(state, config, tx.clone());
    let mut next_input_scan = Instant::now();
    loop {
        let elapsed = splash_started.elapsed();
        let midi_available = midi_input_available(router.as_ref().ok());
        let input_available =
            crate::startup_splash::qualified_input_available(terminal_keyboard, midi_available);
        if !crate::startup_splash::waiting_for_input(elapsed, terminal_keyboard, midi_available)
            || stopping.load(Ordering::Relaxed)
        {
            break;
        }
        let expected = expected_startup_midi(router.as_ref().ok());
        terminal.draw(|frame| {
            crate::startup_splash::draw(
                frame,
                elapsed,
                input_available,
                expected.as_deref(),
                BUILD_BADGE,
            )
        })?;
        if !input_available && Instant::now() >= next_input_scan {
            router = match router {
                Ok(mut active) => active.reconfigure_inputs(config).map(|_| active),
                Err(_) => engine::MidiRouter::start(state, config, tx.clone()),
            };
            next_input_scan = Instant::now() + crate::startup_splash::INPUT_RESCAN_INTERVAL;
        }
        if event::poll(Duration::from_millis(30))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
                    _ => terminal_keyboard = true,
                },
                _ => {}
            }
        }
    }
    while rx.try_recv().is_ok() {}
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
    let playback_scale = router
        .as_ref()
        .map(engine::MidiRouter::playback_scale)
        .unwrap_or_else(|_| Arc::new(std::sync::Mutex::new(None)));
    let midi_lifecycle = router
        .as_ref()
        .map(engine::MidiRouter::lifecycle)
        .unwrap_or_default();
    let controller_config = router
        .as_ref()
        .map(engine::MidiRouter::controller_config)
        .unwrap_or_else(|_| {
            Arc::new(std::sync::RwLock::new(
                crate::pads::PadConfig::load(&state.join("controller.conf")).unwrap_or_default(),
            ))
        });
    let learn_mode = router
        .as_ref()
        .map(engine::MidiRouter::learn_mode)
        .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)));
    let fx_control_mode = router
        .as_ref()
        .map(engine::MidiRouter::fx_control_mode)
        .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)));
    let available_audio_ports = engine::jack_ports();
    let capture_sources = engine::jack_capture_sources();
    let available_midi_outputs =
        sequencer::available_midi_outputs(&config.external_midi.client_name).unwrap_or_default();
    let mut app = App::new(
        catalogs,
        output,
        pickup,
        midi_backend,
        TrackerIo {
            route: tracker_route,
            input: tracker_input,
            playback_scale,
            lifecycle: midi_lifecycle,
        },
        config.clone(),
        AvailablePorts {
            playback: available_audio_ports,
            capture_sources,
            midi_outputs: available_midi_outputs,
        },
        state.to_path_buf(),
        sequencer::routing_defaults_path(),
    );
    app.controller_layout = controller_config
        .read()
        .map(|config| config.layout)
        .unwrap_or_default();
    app.controller_config = controller_config;
    app.learn_mode = learn_mode;
    app.fx_control_mode = fx_control_mode;
    if let Ok(router) = &router {
        app.controller_online = router.availability().controller_available();
        app.performance_inputs = router.availability().performance.clone();
    }
    if let Some(notice) = controller_notice {
        app.status = format!("controller auto-wire: {notice}");
    }
    if let Err(e) = &router {
        let notice = if config.midi_autoconnect {
            format!("keyboard input · preferred MIDI input missing: {e:#}")
        } else {
            "keyboard input · MIDI controller routing disabled".into()
        };
        app.status = notice.clone();
        app.controller_fallback = Some(notice);
    } else if let Ok(router) = &router {
        let mut notices = Vec::new();
        if let Some(controller) = router
            .availability()
            .controller
            .as_ref()
            .filter(|state| !state.available())
        {
            notices.push(format!("controller {}", controller.description()));
        }
        let missing_performance = router
            .availability()
            .performance
            .iter()
            .filter(|state| !state.available())
            .count();
        if missing_performance > 0 {
            notices.push(format!(
                "{missing_performance} performance input(s) unavailable"
            ));
        }
        if !notices.is_empty() {
            let notice = format!("keyboard input · {}", notices.join(" · "));
            app.status = notice.clone();
            app.controller_fallback = Some(notice);
        }
    }
    app.recommend_controller_learn_on_home();
    app.midi_router = router.ok();
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

fn auto_wire_controller(state: &Path, config: &RuntimeConfig) -> Result<()> {
    let path = state.join("controller.conf");
    let current = crate::pads::PadConfig::load(&path)?;
    let connected = crate::controller_learn::input_names()?;
    let catalog = crate::controller_profile::Catalog::discover();
    let Some((expected, _profile_name)) = crate::controller_profile::expected_for_connected(
        &current,
        &config.midi_input_matches,
        &connected,
        &catalog,
    )?
    else {
        return Ok(());
    };
    if expected != current {
        crate::controller_learn::backup(&path)?;
        expected.save(&path)?;
    }
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
                if app.screen == Screen::FxEditor {
                    app.apply_fx_control(cc, v);
                } else {
                    app.observe_mapped_control(cc, v);
                }
            }
            MidiEvent::Value(cc, v) => {
                app.apply_control_value(cc, v);
            }
            MidiEvent::Raw { received, bytes } => {
                app.held_notes.observe(&bytes);
                app.recorder.capture(received, &bytes);
                let tracker_preview = app.tracker_workspace_active();
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
                    if app.tracker_gesture.is_released() {
                        app.commit_released_tracker_gesture(received);
                    }
                }
            }
            MidiEvent::Pad(pad, pressed) => {
                dispatch_pad(pad, pressed, app, state, tx);
            }
            MidiEvent::Encoder(action) => {
                dispatch_encoder(action, app, state, tx);
            }
            MidiEvent::PadLock(locked) => {
                app.pad_locked = locked;
                app.status = if locked {
                    "pad lock on · command pads play as notes".into()
                } else {
                    "pad lock off · command pads restored".into()
                };
            }
            MidiEvent::Learn { received, bytes } => {
                match app.receive_controller_learn(received, &bytes) {
                    crate::controller_learn::LearnAction::None => {}
                    crate::controller_learn::LearnAction::Save => {
                        app.save_controller_learn(state, true);
                    }
                    crate::controller_learn::LearnAction::FinishSaved => {
                        app.finish_saved_controller_learn();
                    }
                }
            }
            MidiEvent::Error(e) => app.status = e,
        }
    }
}

fn dispatch_pad(
    pad: crate::pads::PadAction,
    pressed: bool,
    app: &mut App,
    state: &Path,
    tx: &std::sync::mpsc::Sender<MidiEvent>,
) {
    if !pressed {
        return;
    }
    if let Some(launcher) = app.overlay.as_ref().map(|overlay| overlay.launcher.clone()) {
        if let MenuInput::ActivateItem(item) = pad.menu_input() {
            if item == launcher.item {
                perform(launcher.action, app, state, Some(tx));
            }
        }
        return;
    }
    match pad.menu_input() {
        MenuInput::SelectPage(page) => app.select_menu_page(page),
        MenuInput::CyclePage => app.cycle_menu_page(1),
        MenuInput::ActivateItem(item) => {
            let slot = navigation::slot(app.screen, app.menu_context(), app.menu_page(), item);
            if let Some(action) = slot.and_then(|slot| slot.dispatch()) {
                perform(action, app, state, Some(tx));
            }
        }
    }
}

fn dispatch_encoder(
    action: crate::pads::EncoderAction,
    app: &mut App,
    state: &Path,
    tx: &std::sync::mpsc::Sender<MidiEvent>,
) {
    dispatch_encoder_input(action, app, state, tx, true);
}

fn dispatch_encoder_input(
    action: crate::pads::EncoderAction,
    app: &mut App,
    state: &Path,
    tx: &std::sync::mpsc::Sender<MidiEvent>,
    physical: bool,
) {
    if app.overlay.is_some() {
        match action {
            crate::pads::EncoderAction::Up => app.move_overlay(-1),
            crate::pads::EncoderAction::Down => app.move_overlay(1),
            crate::pads::EncoderAction::Select => app.activate_overlay(),
        }
        return;
    }
    app.prepare_confirmation_action(Action::Noop);
    if app.screen == Screen::Home {
        match action {
            crate::pads::EncoderAction::Up => app.move_home(-1),
            crate::pads::EncoderAction::Down => app.move_home(1),
            crate::pads::EncoderAction::Select => {
                perform(Action::Activate, app, state, Some(tx));
            }
        }
        return;
    }
    if app.screen == Screen::FxRack && app.fx_type_edit.is_some() {
        match action {
            crate::pads::EncoderAction::Up => app.cycle_effect_kind(-1),
            crate::pads::EncoderAction::Down => app.cycle_effect_kind(1),
            crate::pads::EncoderAction::Select => app.confirm_effect_type_edit(),
        }
        return;
    }
    let value_editor_owns_encoder = app.note_editor.is_some()
        || (app.screen == Screen::TrackerPages && app.page_manager_mode != PageManagerMode::Pages)
        || app.screen == Screen::FxEditor
        || app.screen == Screen::Routing
        || app.confirm_routing_defaults;
    let tracker_transport_turn = physical
        && app.screen == Screen::Tracker
        && !value_editor_owns_encoder
        && !(app.controller_layout == ControllerLayout::Four && app.page_select_mode)
        && (app.sequencer.status().playing || app.tracker_recording.is_some());
    if tracker_transport_turn {
        match action {
            crate::pads::EncoderAction::Up => app.move_tracker_rotary_column(-1),
            crate::pads::EncoderAction::Down => app.move_tracker_rotary_column(1),
            crate::pads::EncoderAction::Select => {}
        }
        if action != crate::pads::EncoderAction::Select {
            return;
        }
    }
    if app.controller_layout == ControllerLayout::Four && !value_editor_owns_encoder {
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
            crate::pads::EncoderAction::Up if app.page_select_mode => app.cycle_menu_page(-1),
            crate::pads::EncoderAction::Down if app.page_select_mode => app.cycle_menu_page(1),
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

fn function_key_pad(code: KeyCode) -> Option<crate::pads::PadAction> {
    use crate::pads::PadAction;
    match code {
        KeyCode::F(5) => Some(PadAction::Page1),
        KeyCode::F(6) => Some(PadAction::Page2),
        KeyCode::F(7) => Some(PadAction::Page3),
        KeyCode::F(8) => Some(PadAction::Page4),
        KeyCode::F(9) => Some(PadAction::Item1),
        KeyCode::F(10) => Some(PadAction::Item2),
        KeyCode::F(11) => Some(PadAction::Item3),
        KeyCode::F(12) => Some(PadAction::Item4),
        _ => None,
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
    if let Some(launcher) = a.overlay.as_ref().map(|overlay| overlay.launcher.clone()) {
        if action == launcher.action {
            a.close_overlay(true);
        } else {
            match action {
                Action::Up => a.move_overlay(-1),
                Action::Down => a.move_overlay(1),
                Action::Activate => a.activate_overlay(),
                Action::Back => a.overlay_back(),
                _ => {}
            }
        }
        return false;
    }
    if OverlayKind::from_action(action).is_some() {
        a.open_overlay(action);
        return false;
    }
    if a.confirm_routing_defaults {
        match action {
            Action::ConfirmRoutingDefaults | Action::Activate | Action::SaveSong => {
                a.finish_routing_defaults_prompt(true)
            }
            Action::CancelRoutingDefaults | Action::Back => a.finish_routing_defaults_prompt(false),
            _ => {}
        }
        return false;
    }
    if a.audio_track_name_input.is_some() {
        match action {
            Action::Activate => a.commit_audio_track_name(state),
            Action::Back => {
                a.audio_track_name_input = None;
                a.status = "track naming cancelled".into();
            }
            _ => {}
        }
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
            Action::Activate | Action::NoteEditorConfirm => {
                if a.note_editor.as_ref().is_some_and(|editor| editor.active) {
                    a.confirm_note_editor_field();
                } else if let Some(field) = a.note_editor.as_ref().map(|editor| editor.field) {
                    a.select_note_editor_field(field);
                }
            }
            Action::Back | Action::NoteEditorCancel => a.back_note_editor(),
            Action::NoteEditorSave => a.save_note_editor(),
            Action::NoteDestinationField => {
                a.select_note_editor_field(NoteEditorField::Destination)
            }
            Action::NoteChannelField => a.select_note_editor_field(NoteEditorField::Channel),
            Action::DefaultProgramField => {
                a.select_note_editor_field(NoteEditorField::DefaultProgram)
            }
            Action::NoteBankMsbField => a.select_note_editor_field(NoteEditorField::BankMsb),
            Action::NoteBankLsbField => a.select_note_editor_field(NoteEditorField::BankLsb),
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
                a.status = "EDIT on".into();
            }
            Action::StopAll => unreachable!("panic is handled before contextual dispatch"),
            _ => {}
        }
        return false;
    }
    if a.tracker_recording.is_some() {
        match action {
            Action::TrackerRecordToggle => {
                a.toggle_tracker_recording();
                return false;
            }
            Action::TrackerPlayToggle => {
                a.toggle_tracker_playback();
                return false;
            }
            Action::TrackerEdit => {
                a.stop_tracker_recording();
                a.set_tracker_edit(true);
                a.status = "EDIT on".into();
                return false;
            }
            Action::Back => {
                a.stop_tracker_recording();
                a.set_tracker_mode(TrackerMode::Play);
                return false;
            }
            Action::StopAll => unreachable!("panic is handled before contextual dispatch"),
            _ => return false,
        }
    }
    if a.screen == Screen::TrackerFiles && a.confirm_pattern_clear {
        match action {
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
            if a.screen == Screen::Home {
                a.move_home(-1);
            } else if a.screen == Screen::Playback && a.playback_noob {
                a.adjust_playback_noob_scale(-1);
            } else if a.screen == Screen::Help {
                a.move_help(-1);
            } else if a.screen == Screen::TrackerLoopAlign {
                a.adjust_loop_offset_bars(-1);
            } else if a.screen == Screen::Ideas {
                a.idea_selected = wrapped_index(a.idea_selected, a.ideas.len(), -1);
            } else if a.screen == Screen::Routing {
                a.move_routing(-1);
            } else if a.screen == Screen::TrackerFiles {
                match a.tracker_files_mode {
                    TrackerFilesMode::Projects => {
                        a.song_selected = wrapped_index(a.song_selected, a.song_list.len(), -1);
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
                a.tracker_row = wrapped_index(a.tracker_row, a.tracker_rows(), -1);
            } else if a.screen == Screen::AudioRecorder {
                a.move_audio_track(-1);
            } else if a.screen == Screen::Meter {
                a.move_bus_selection(-1);
            } else if a.screen == Screen::TrackerPages {
                a.turn_page_manager(-1);
            } else if a.screen == Screen::TrackerLoop {
                if a.loop_library_mode {
                    a.loop_library_selected =
                        wrapped_index(a.loop_library_selected, a.loop_library.len(), -1);
                } else {
                    a.loop_selected = wrapped_index(a.loop_selected, a.loop_imports.len(), -1);
                }
            } else if a.screen == Screen::Presets {
                a.selected = wrapped_index(a.selected, a.presets.len(), -1);
            } else if a.screen == Screen::FxRack {
                a.move_fx_rack_selection(-1);
            } else if a.screen == Screen::FxEditor {
                if a.fx_value_editing {
                    a.adjust_effect_parameter(-1);
                } else {
                    a.move_fx_parameter(-1);
                }
            }
        }
        Action::Down => {
            if a.screen == Screen::Home {
                a.move_home(1);
            } else if a.screen == Screen::Playback && a.playback_noob {
                a.adjust_playback_noob_scale(1);
            } else if a.screen == Screen::Help {
                a.move_help(1);
            } else if a.screen == Screen::TrackerLoopAlign {
                a.adjust_loop_offset_bars(1);
            } else if a.screen == Screen::Ideas {
                a.idea_selected = wrapped_index(a.idea_selected, a.ideas.len(), 1);
            } else if a.screen == Screen::Routing {
                a.move_routing(1);
            } else if a.screen == Screen::TrackerFiles {
                match a.tracker_files_mode {
                    TrackerFilesMode::Projects => {
                        a.song_selected = wrapped_index(a.song_selected, a.song_list.len(), 1);
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
                a.tracker_row = wrapped_index(a.tracker_row, a.tracker_rows(), 1);
            } else if a.screen == Screen::AudioRecorder {
                a.move_audio_track(1);
            } else if a.screen == Screen::Meter {
                a.move_bus_selection(1);
            } else if a.screen == Screen::TrackerPages {
                a.turn_page_manager(1);
            } else if a.screen == Screen::TrackerLoop {
                if a.loop_library_mode {
                    a.loop_library_selected =
                        wrapped_index(a.loop_library_selected, a.loop_library.len(), 1);
                } else {
                    a.loop_selected = wrapped_index(a.loop_selected, a.loop_imports.len(), 1);
                }
            } else if a.screen == Screen::Presets {
                a.selected = wrapped_index(a.selected, a.presets.len(), 1);
            } else if a.screen == Screen::FxRack {
                a.move_fx_rack_selection(1);
            } else if a.screen == Screen::FxEditor {
                if a.fx_value_editing {
                    a.adjust_effect_parameter(1);
                } else {
                    a.move_fx_parameter(1);
                }
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
            if a.screen == Screen::Home {
                a.home_selected = 0;
            } else if a.screen == Screen::Ideas {
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
            if a.screen == Screen::Home {
                a.home_selected = HOME_ENTRIES.len().saturating_sub(1);
            } else if a.screen == Screen::Ideas {
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
            Screen::Home => {
                if let Some(entry) = HOME_ENTRIES.get(a.home_selected) {
                    perform(entry.action, a, state, tx);
                }
            }
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
            Screen::TrackerTools => {}
            Screen::TrackerLoop => a.open_overlay(Action::LoopImport),
            Screen::TrackerLoopAlign => {
                a.set_screen(Screen::TrackerLoop);
                a.status = "loop alignment set".into();
            }
            Screen::AudioRecorder => a.toggle_audio_track_arm(state),
            Screen::Meter => a.toggle_bus_mute(),
            Screen::FxRack => {
                if a.fx_type_edit.is_some() {
                    a.confirm_effect_type_edit();
                } else if a.selected_effect_id().is_some() {
                    a.begin_effect_type_edit();
                } else {
                    a.add_effect();
                }
            }
            Screen::FxEditor => {
                if a.fx_value_editing {
                    a.confirm_fx_value_edit();
                } else {
                    a.begin_fx_value_edit();
                }
            }
            Screen::Routing => {
                if a.routing.draft.is_some() {
                    a.confirm_routing_edit(state);
                } else {
                    a.begin_routing_edit();
                }
            }
        },
        Action::Quit => unreachable!("quit is handled before contextual dispatch"),
        Action::StopAll => unreachable!("panic is handled before contextual dispatch"),
        Action::OpenPresets => {
            a.set_tracker_edit(false);
            a.set_screen(Screen::Presets);
            a.status = "software synths · choose a sound".into();
        }
        Action::OpenIdeas => a.open_ideas(),
        Action::OpenHelp => a.open_help(),
        Action::OpenControllerLearn => a.begin_controller_learn(),
        Action::OpenTracker => {
            let entry = a.prepare_first_tracker_instrument();
            a.set_screen(Screen::Tracker);
            a.refresh_page_targets();
            let engine_ready = a.sync_tracker_route();
            let page_online = a
                .current_page()
                .is_some_and(|page| a.target_online(&page.target));
            if engine_ready {
                a.status = if entry == TrackerEntryInstrument::AdoptedPlayer {
                    "tracker ready · Player instrument assigned to page 1".into()
                } else if entry == TrackerEntryInstrument::FirstSynthv1 {
                    "tracker ready · first synthv1 instrument assigned to page 1".into()
                } else if page_online {
                    "tracker ready · EDIT toggles entry · encoder press skips".into()
                } else {
                    "tracker page target offline · PAGES to change it".into()
                };
            }
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
        Action::OpenTrackerLoop => a.open_tracker_loop(),
        Action::OpenTrackerLoopAlign => {
            a.set_screen(Screen::TrackerLoopAlign);
            a.reset_context_page();
            a.status = "loop align · AUTO or move by one bar".into();
        }
        Action::OpenPageOverlay
        | Action::OpenPatternOverlay
        | Action::OpenSongOverlay
        | Action::OpenRouteOverlay
        | Action::OpenPatternLengthOverlay
        | Action::OpenNoteLengthOverlay
        | Action::OpenTrackerAdvanceOverlay
        | Action::OpenEffectsOverlay => a.open_overlay(action),
        Action::OpenAudioRecorder => {
            a.set_tracker_edit(false);
            a.set_screen(Screen::AudioRecorder);
            a.status = "multitrack audio recorder".into();
        }
        Action::OpenFxRack => {
            if !matches!(a.screen, Screen::FxRack | Screen::FxEditor) {
                a.fx_rack_parent = a.screen;
                if a.screen == Screen::Playback || is_tracker_screen(a.screen) {
                    a.fx_target = 0;
                }
            }
            if a.selected_effect_id().is_none() {
                a.fx_selection = FxRackSelection::Insert;
            }
            a.fx_value_editing = false;
            a.fx_edit_original = None;
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
                a.fx_value_editing = false;
                a.fx_edit_original = None;
                a.set_screen(Screen::FxEditor);
                a.status = "effect editor · turn to choose parameter · press to edit".into();
            } else {
                a.status = "FX rack is empty".into();
            }
        }
        Action::OpenMeter => {
            a.set_tracker_edit(false);
            a.set_screen(Screen::Meter);
            a.reset_context_page();
            a.status = "mix, final output, and meters".into();
        }
        Action::OpenRouting => {
            a.set_tracker_edit(false);
            a.open_routing_editor();
        }
        Action::ResetMeter => {
            a.performance_meter.clear_holds();
            a.status = "meter MAX, bright peak, and clip holds cleared".into();
            if a.config.audio_graph.enabled
                && a.engine.as_ref().and_then(Engine::bus_controls).is_none()
            {
                a.retry_final_bus();
            }
        }
        Action::BusSelectPrevious => a.move_bus_selection(-1),
        Action::BusSelectNext => a.move_bus_selection(1),
        Action::BusLevelDecrease => a.adjust_bus_level(-1),
        Action::BusLevelIncrease => a.adjust_bus_level(1),
        Action::BusMute => a.toggle_bus_mute(),
        Action::FinalRecordToggle => a.toggle_final_recording(),
        Action::Back => {
            if a.screen == Screen::Routing && a.cancel_routing_edit() {
                return false;
            }
            if a.screen == Screen::FxRack && a.fx_type_edit.is_some() {
                a.cancel_effect_type_edit();
                return false;
            }
            if a.screen == Screen::FxEditor {
                if a.fx_value_editing {
                    a.cancel_fx_value_edit();
                    return false;
                }
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
            if is_tracker_screen(a.screen) {
                a.set_tracker_edit(false);
            }
            let next_screen = match a.screen {
                Screen::Home => Screen::Home,
                Screen::Presets
                | Screen::Ideas
                | Screen::Tracker
                | Screen::AudioRecorder
                | Screen::Meter
                | Screen::Routing => Screen::Home,
                Screen::Playback => Screen::Presets,
                Screen::TrackerFiles
                | Screen::TrackerArrange
                | Screen::TrackerPages
                | Screen::TrackerTools
                | Screen::TrackerLoop => Screen::Tracker,
                Screen::TrackerLoopAlign => Screen::TrackerLoop,
                Screen::FxRack => a.fx_rack_parent,
                Screen::FxEditor => Screen::FxRack,
                Screen::Help => a.help_previous,
            };
            a.set_screen(next_screen);
        }
        Action::ResetParameters => a.reset_parameters(),
        Action::IdeaRecordToggle => a.toggle_idea_recording(),
        Action::SaveNew => a.save_new(),
        Action::InspectIdea => a.inspect_idea(),
        Action::DeleteIdea => a.delete_idea(),
        Action::LoadIdea => {
            if let Some(tx) = tx {
                a.load_idea(state, tx.clone());
            }
        }
        Action::IdeaPlayToggle => a.toggle_playback(),
        Action::TrackerPlayToggle => a.toggle_tracker_playback(),
        Action::TrackerRewind => a.rewind_tracker(),
        Action::TrackerRecordToggle => a.toggle_tracker_recording(),
        Action::TrackerNoobToggle => a.toggle_tracker_noob(),
        Action::PlaybackNoobToggle => a.toggle_playback_noob(),
        Action::ConfirmRoutingDefaults => a.finish_routing_defaults_prompt(true),
        Action::CancelRoutingDefaults => a.finish_routing_defaults_prompt(false),
        Action::LoopImport => a.open_overlay(Action::LoopImport),
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
        Action::NextTrackerPage => a.move_tracker_page(1),
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
        Action::TrackerEdit => {
            let enabled = a.tracker_mode != TrackerMode::Edit;
            a.set_tracker_edit(enabled);
            a.status = format!("EDIT {}", if enabled { "on" } else { "off" });
        }
        Action::TrackerSkip => a.tracker_skip(),
        Action::TrackerErase => a.tracker_erase(),
        Action::TrackerNoteOff => a.tracker_note_off(),
        Action::OpenNoteEditor => a.open_note_editor(),
        Action::NoteDestinationField
        | Action::NoteChannelField
        | Action::DefaultProgramField
        | Action::NoteBankMsbField
        | Action::NoteBankLsbField
        | Action::NoteField
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
        | Action::NoteEditorSave
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
        Action::ConfirmPatternClear => a.apply_pattern_clear(),
        Action::AudioRecordToggle => a.toggle_audio_recording(),
        Action::AudioToggleArm => a.toggle_audio_track_arm(state),
        Action::AudioArmAll => a.set_all_audio_arms(state, true),
        Action::AudioDisarmAll => a.set_all_audio_arms(state, false),
        Action::AudioPreviousTrack => a.move_audio_track(-1),
        Action::AudioNextTrack => a.move_audio_track(1),
        Action::AudioAssignSource => a.assign_audio_source(state),
        Action::AudioNameTrack => a.begin_audio_track_name(),
        Action::AudioRefreshSources => a.refresh_audio_sources(),
        Action::FxAdd => a.add_effect(),
        Action::FxEditType => a.begin_effect_type_edit(),
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
    }
    false
}
fn key(code: KeyCode, a: &mut App, state: &Path, tx: &std::sync::mpsc::Sender<MidiEvent>) -> bool {
    if a.audio_track_name_input.is_some() {
        match code {
            KeyCode::Enter => a.commit_audio_track_name(state),
            KeyCode::Esc => {
                a.audio_track_name_input = None;
                a.status = "track naming cancelled".into();
            }
            KeyCode::Backspace => {
                if let Some(input) = a.audio_track_name_input.as_mut() {
                    input.pop();
                }
            }
            KeyCode::Char(character)
                if !character.is_control()
                    && a.audio_track_name_input
                        .as_ref()
                        .is_some_and(|input| input.chars().count() < 64) =>
            {
                if let Some(input) = a.audio_track_name_input.as_mut() {
                    input.push(character);
                }
            }
            _ => {}
        }
        return false;
    }
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
    if a.controller_learn.is_some() {
        match code {
            KeyCode::Esc | KeyCode::Char('b') | KeyCode::Char('B') => a.cancel_controller_learn(),
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if let Some(session) = a.controller_learn.as_mut() {
                    session.skip();
                }
            }
            KeyCode::Down | KeyCode::Right => {
                if let Some(session) = a.controller_learn.as_mut() {
                    session.skip();
                }
            }
            KeyCode::Up | KeyCode::Left => {
                if let Some(session) = a.controller_learn.as_mut() {
                    session.previous();
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if let Some(session) = a.controller_learn.as_mut() {
                    session.retry();
                }
            }
            KeyCode::Enter => a.save_controller_learn(state, false),
            _ => {}
        }
        return false;
    }
    if a.overlay.is_some() {
        if let Some(pad) = function_key_pad(code) {
            dispatch_pad(pad, true, a, state, tx);
            return false;
        }
        match code {
            KeyCode::Up | KeyCode::Char('k') => a.move_overlay(-1),
            KeyCode::Down | KeyCode::Char('j') => a.move_overlay(1),
            KeyCode::Enter => a.activate_overlay(),
            KeyCode::Esc | KeyCode::Char('b') | KeyCode::Char('B') => a.overlay_back(),
            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Char(' ') => a.stop_all(state),
            _ => {}
        }
        return false;
    }
    if a.screen == Screen::FxEditor {
        match code {
            KeyCode::Char(character)
                if character.is_ascii_digit()
                    || (character == '-' && a.fx_numeric_input.is_none())
                    || (character == '.'
                        && a.fx_numeric_input
                            .as_ref()
                            .is_none_or(|input| !input.contains('.'))) =>
            {
                a.begin_fx_numeric_entry(character);
                return false;
            }
            KeyCode::Backspace if a.fx_numeric_input.is_some() => {
                if let Some(input) = a.fx_numeric_input.as_mut() {
                    input.pop();
                    a.status = format!("numeric value · {input}_ · Enter confirms");
                }
                return false;
            }
            KeyCode::Left => {
                if a.fx_value_editing {
                    a.confirm_fx_value_edit();
                }
                a.move_fx_parameter(-1);
                return false;
            }
            KeyCode::Right => {
                if a.fx_value_editing {
                    a.confirm_fx_value_edit();
                }
                a.move_fx_parameter(1);
                return false;
            }
            KeyCode::Enter if a.fx_numeric_input.is_some() => {
                a.commit_fx_numeric_entry();
                return false;
            }
            _ => {}
        }
    }
    if let Some(pad) = function_key_pad(code) {
        dispatch_pad(pad, true, a, state, tx);
        return false;
    }
    if let Some(action) = match code {
        KeyCode::Up => Some(crate::pads::EncoderAction::Up),
        KeyCode::Down => Some(crate::pads::EncoderAction::Down),
        KeyCode::Enter => Some(crate::pads::EncoderAction::Select),
        _ => None,
    } {
        dispatch_encoder_input(action, a, state, tx, false);
        return false;
    }
    let letter_jump_blocked = a.keyboard_modal_active();
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
            KeyCode::Char(character) if character.is_ascii_alphabetic() && !letter_jump_blocked => {
                a.jump_to_letter(character);
                None
            }
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
    if letter_jump_blocked
        && confirmation_action == Action::Noop
        && !(a.screen == Screen::Tracker && a.tracker_recording.is_some())
        && matches!(code, KeyCode::Char(character) if character.is_ascii_alphabetic())
    {
        return false;
    }
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
                a.tracker_stop();
                None
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
            KeyCode::Char('p') => Some(Action::TrackerPlayToggle),
            KeyCode::Char('P') => Some(Action::TrackerRewind),
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
                a.tracker_stop();
                None
            }
            KeyCode::Esc | KeyCode::Char('b') => Some(Action::Back),
            KeyCode::Char(character) if character.is_ascii_alphabetic() && !letter_jump_blocked => {
                a.jump_to_letter(character);
                None
            }
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
                    a.toggle_tracker_recording();
                }
                KeyCode::Char('p') | KeyCode::Char('P') => {
                    a.toggle_tracker_playback();
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
                KeyCode::Char('w') | KeyCode::Char('W') => Some(Action::NoteEditorSave),
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
        }
        if code == KeyCode::Char('F') {
            perform(Action::OpenFxRack, a, state, Some(tx));
            return false;
        }
        if let Some(semitone) = tracker_key_note(code) {
            let note = a.tracker_keyboard_note(semitone);
            a.audition_keyboard_note(note, 96);
            if a.tracker_mode == TrackerMode::Edit {
                a.tracker_single_note(note, 96);
            }
            return false;
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
                if !a.leave_noob_on_percussion() {
                    a.sync_tracker_route();
                }
                return false;
            }
            KeyCode::PageDown => {
                a.cancel_tracker_gesture();
                a.tracker_order = (a.tracker_order + 1).min(a.song.order.len().saturating_sub(1));
                a.tracker_row = 0;
                if !a.leave_noob_on_percussion() {
                    a.sync_tracker_route();
                }
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
                a.toggle_tracker_playback();
                return false;
            }
            KeyCode::Char('P') => {
                a.rewind_tracker();
                return false;
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                a.toggle_tracker_recording();
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
    if a.screen == Screen::AudioRecorder {
        let action = match code {
            KeyCode::Char('r') => Some(Action::AudioRecordToggle),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::AudioPreviousTrack),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::AudioNextTrack),
            KeyCode::Char(' ') => Some(Action::AudioToggleArm),
            KeyCode::Char('a') => Some(Action::AudioArmAll),
            KeyCode::Char('x') => Some(Action::AudioDisarmAll),
            KeyCode::Char('s') => Some(Action::AudioAssignSource),
            KeyCode::Char('n') => Some(Action::AudioNameTrack),
            KeyCode::Char('f') => Some(Action::AudioRefreshSources),
            _ => None,
        };
        if let Some(action) = action {
            return perform(action, a, state, Some(tx));
        }
    }
    match code {
        KeyCode::Char('q') => return true,
        KeyCode::Esc => {
            if a.screen != Screen::Home {
                perform(Action::Back, a, state, Some(tx));
            } else {
                return true;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if a.screen == Screen::Ideas {
                a.idea_selected = wrapped_index(a.idea_selected, a.ideas.len(), -1)
            } else if a.screen == Screen::Presets {
                a.selected = wrapped_index(a.selected, a.presets.len(), -1)
            } else {
                perform(Action::Up, a, state, Some(tx));
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if a.screen == Screen::Ideas {
                a.idea_selected = wrapped_index(a.idea_selected, a.ideas.len(), 1)
            } else if a.screen == Screen::Presets {
                a.selected = wrapped_index(a.selected, a.presets.len(), 1)
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
        KeyCode::Char('r') if matches!(a.screen, Screen::Playback | Screen::Ideas) => {
            a.toggle_idea_recording()
        }
        KeyCode::Char('p') if matches!(a.screen, Screen::Playback | Screen::Ideas) => {
            a.toggle_playback()
        }
        KeyCode::Char('w') => a.open_ideas(),
        KeyCode::Char('m') if a.screen == Screen::Presets => {
            perform(Action::OpenMeter, a, state, Some(tx));
        }
        KeyCode::Char('t') => {
            perform(Action::OpenTracker, a, state, Some(tx));
        }
        KeyCode::Char('f') | KeyCode::Char('F') if a.screen == Screen::Playback => {
            perform(Action::OpenFxRack, a, state, Some(tx));
        }
        KeyCode::Char('a') => {
            if a.screen == Screen::Tracker {
                a.set_tracker_edit(false);
            }
            a.set_screen(Screen::AudioRecorder);
            a.status = "multitrack audio recorder".into();
        }
        KeyCode::Char('d') if a.screen == Screen::Ideas => a.delete_idea(),
        KeyCode::Char('i') if a.screen == Screen::Ideas => a.inspect_idea(),
        KeyCode::Char(character) if character.is_ascii_alphabetic() && !letter_jump_blocked => {
            a.jump_to_letter(character);
        }
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

fn pattern_length_choices() -> Vec<usize> {
    (1..=32).chain([48, 64, 96, 128, 192, 256]).collect()
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
    if a.overlay.is_some() {
        match m.kind {
            MouseEventKind::Down(MouseButton::Right) => a.overlay_back(),
            MouseEventKind::ScrollUp => a.move_overlay(-1),
            MouseEventKind::ScrollDown => a.move_overlay(1),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(action) = a.hits.action(m.column, m.row) {
                    perform(action, a, state, Some(tx));
                } else if contains(a.hits.list, m.column, m.row) {
                    let scroll = a.overlay.as_ref().map_or(0, |overlay| overlay.scroll);
                    let index = visible_index(a.hits.list, scroll, m.column, m.row).unwrap_or(0);
                    if index < a.overlay_row_count() {
                        let selected = a
                            .overlay
                            .as_ref()
                            .is_some_and(|overlay| overlay.selection == index);
                        if selected {
                            a.activate_overlay();
                        } else if let Some(overlay) = a
                            .overlay
                            .as_mut()
                            .filter(|overlay| overlay.active_field.is_none())
                        {
                            overlay.selection = index;
                        }
                    }
                }
            }
            _ => {}
        }
        return false;
    }
    if matches!(m.kind, MouseEventKind::Down(MouseButton::Right)) {
        if a.screen != Screen::Home {
            perform(Action::Back, a, state, Some(tx));
        } else {
            return true;
        }
        return false;
    }
    match m.kind {
        MouseEventKind::ScrollUp => {
            a.prepare_confirmation_action(Action::Noop);
            if a.screen == Screen::Home {
                a.move_home(-1);
            } else if a.screen == Screen::Ideas {
                a.idea_selected = wrapped_index(a.idea_selected, a.ideas.len(), -1)
            } else if a.screen == Screen::Help {
                a.move_help(-3);
            } else if a.screen == Screen::Presets {
                a.selected = wrapped_offset(a.selected, a.presets.len(), -3)
            } else {
                perform(Action::Up, a, state, Some(tx));
            }
        }
        MouseEventKind::ScrollDown => {
            a.prepare_confirmation_action(Action::Noop);
            if a.screen == Screen::Home {
                a.move_home(1);
            } else if a.screen == Screen::Ideas {
                a.idea_selected = wrapped_index(a.idea_selected, a.ideas.len(), 1)
            } else if a.screen == Screen::Help {
                a.move_help(3);
            } else if a.screen == Screen::Presets {
                a.selected = wrapped_offset(a.selected, a.presets.len(), 3)
            } else {
                perform(Action::Down, a, state, Some(tx));
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if a.screen == Screen::Home && contains(a.hits.list, m.column, m.row) {
                let index = visible_index(a.hits.list, a.home_offset, m.column, m.row).unwrap();
                if index < HOME_ENTRIES.len() {
                    if index == a.home_selected {
                        perform(Action::Activate, a, state, Some(tx));
                    } else {
                        a.home_selected = index;
                    }
                }
            } else if a.screen == Screen::Presets && contains(a.hits.list, m.column, m.row) {
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
    if a.controller_learn.is_some() {
        draw_controller_learn(f, a);
        draw_master_status(f, a);
        return;
    }
    match a.screen {
        Screen::Home => {
            draw_home(f, a);
            return;
        }
        Screen::Presets => draw_list(f, a),
        Screen::Playback => draw_playing(f, a),
        Screen::Ideas => draw_ideas(f, a),
        Screen::Help => draw_help(f, a),
        Screen::Tracker => draw_tracker(f, a),
        Screen::TrackerFiles => draw_tracker_files(f, a),
        Screen::TrackerArrange => draw_tracker_arrange(f, a),
        Screen::TrackerPages => draw_tracker_pages(f, a),
        Screen::TrackerTools => {
            draw_tracker_child(f, "FT2 TOOLS", "Arrange · Loop · FX · Clipboard · Mute")
        }
        Screen::TrackerLoop => draw_tracker_loop(f, a),
        Screen::TrackerLoopAlign => draw_tracker_loop_align(f, a),
        Screen::AudioRecorder => draw_audio_recorder(f, a),
        Screen::FxRack => draw_fx_rack(f, a),
        Screen::FxEditor => draw_fx_editor(f, a),
        Screen::Meter => draw_performance_meter(f, a),
        Screen::Routing => draw_routing(f, a),
    }
    if a.overlay.is_some() {
        draw_overlay(f, a);
        draw_overlay_launcher(f, a);
        draw_master_status(f, a);
        return;
    }
    draw_pad_lock(f, a);
    draw_fallback_badge(f, a);
    draw_pad_buttons(f, a);
    if a.confirm_routing_defaults {
        let z = f.size();
        let area = rect(z.x + 2, z.y + 4, z.width.saturating_sub(4), 7);
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new(
                "Save this routing as the default\nfor new patterns?\n\nCONFIRM saves the new default\nCANCEL keeps the previous default",
            )
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL)),
            area,
        );
    }
    if let Some(input) = a.project_name_input.as_deref() {
        let z = f.size();
        let area = rect(z.x + 2, z.y + 4, z.width.saturating_sub(4), 5);
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new(format!(
                "PROJECT NAME\n{}\nEnter confirm · Esc cancel",
                crate::ui_text::fit_line(
                    &format!("{input}_"),
                    usize::from(area.width.saturating_sub(2))
                )
            ))
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL)),
            area,
        );
    }
    if let Some(input) = a.audio_track_name_input.as_deref() {
        let z = f.size();
        let area = rect(z.x + 2, z.y + 4, z.width.saturating_sub(4), 5);
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new(format!(
                "TRACK NAME\n{}\nEnter confirm · Esc cancel",
                crate::ui_text::fit_line(
                    &format!("{input}_"),
                    usize::from(area.width.saturating_sub(2))
                )
            ))
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL)),
            area,
        );
    }
    draw_master_status(f, a);
}

fn draw_home<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    f.render_widget(Block::default().style(Style::default().bg(Color::Black)), z);
    let recommendation = a.controller_learn_reason();
    let bottom_rows = if recommendation.is_some() { 3 } else { 1 };
    let available = rect(
        z.x + 2,
        z.y,
        z.width.saturating_sub(4),
        z.height.saturating_sub(bottom_rows),
    );
    let rows = usize::from(available.height).min(HOME_ENTRIES.len());
    a.ensure_home_visible(rows);
    let visible = HOME_ENTRIES.len().saturating_sub(a.home_offset).min(rows);
    let list = rect(
        available.x,
        available.y + available.height.saturating_sub(visible as u16) / 2,
        available.width,
        visible as u16,
    );
    a.hits.list = list;
    let lines = HOME_ENTRIES
        .iter()
        .enumerate()
        .skip(a.home_offset)
        .take(rows)
        .map(|(index, entry)| {
            let selected = index == a.home_selected;
            let label = centered_text(entry.label, usize::from(list.width));
            Spans::from(Span::styled(
                label,
                if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray).bg(Color::Black)
                },
            ))
        })
        .collect::<Vec<_>>();
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(Color::Black)),
        list,
    );
    if let Some(reason) = recommendation {
        f.render_widget(
            Paragraph::new(vec![
                Spans::from(Span::styled(
                    "CONTROLLER NEEDS SETUP · MIDI LEARN",
                    Style::default()
                        .fg(Color::Yellow)
                        .bg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )),
                Spans::from(Span::styled(
                    reason.detail(),
                    Style::default().fg(Color::DarkGray).bg(Color::Black),
                )),
            ])
            .alignment(Alignment::Center),
            rect(z.x, z.y + z.height.saturating_sub(3), z.width, 2),
        );
    }
    let status = if a.status == "Ready" {
        "↑↓ / rotary browse · Enter / click open · Esc quit"
    } else {
        &a.status
    };
    f.render_widget(
        Paragraph::new(truncate(status, z.width as usize))
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray).bg(Color::Black)),
        rect(z.x, z.y + z.height.saturating_sub(1), z.width, 1),
    );
}

fn centered_text(value: &str, width: usize) -> String {
    let value = truncate(value, width);
    let padding = width.saturating_sub(crate::ui_text::width(&value));
    let left = padding / 2;
    format!(
        "{}{}{}",
        " ".repeat(left),
        value,
        " ".repeat(padding - left)
    )
}

fn draw_routing<B: Backend>(f: &mut Frame<B>, a: &App) {
    let z = f.size();
    let body = rect(z.x, z.y, z.width, z.height.saturating_sub(3));
    let width = usize::from(body.width.saturating_sub(2));
    let owned_controller = a.controller_config.read().ok();
    let (config, controller) = a.routing.draft.as_ref().map_or_else(
        || (&a.config, owned_controller.as_deref()),
        |draft| (&draft.config, Some(&draft.controller)),
    );
    let controller_name = controller
        .and_then(|controller| controller.input_match.as_deref())
        .unwrap_or("");
    let performance_name = config
        .midi_performance_input_matches
        .first()
        .map(String::as_str)
        .unwrap_or("");
    const ROUTING_LABEL_CELLS: usize = 9;
    let value_width = width.saturating_sub(ROUTING_LABEL_CELLS);
    let endpoint = |name: &str, names: &[String]| {
        if name.is_empty() {
            "NONE".into()
        } else {
            let state = match crate::midi_endpoint::matching_index(names, name, "endpoint") {
                Ok(_) => "ONLINE",
                Err(error) if error.to_string().contains("ambiguous") => "AMBIG",
                Err(_) => "OFFLINE",
            };
            crate::ui_text::label_value(
                &crate::ui_text::endpoint_label(name, value_width.saturating_sub(10)),
                state,
                value_width,
            )
        }
    };
    let profile = a.device_profiles.by_id(&config.external_midi.profile);
    let device = crate::ui_text::label_value(
        profile.map_or("RAW MIDI", |profile| profile.model.as_str()),
        "UNVERIFIED",
        value_width,
    );
    let audio_online = config.audio_outputs.len() == 2
        && config
            .audio_outputs
            .iter()
            .all(|port| a.routing_audio_ports.iter().any(|live| live == port));
    let audio_name = config
        .audio_outputs
        .first()
        .and_then(|port| port.split_once(':').map(|parts| parts.0))
        .unwrap_or("NONE");
    let values = [
        endpoint(controller_name, &a.routing_inputs),
        if config.midi_controller_musical_input {
            "COMBINED".into()
        } else {
            "CONTROL".into()
        },
        endpoint(performance_name, &a.routing_inputs),
        if config.external_midi.enabled {
            "ON"
        } else {
            "OFF"
        }
        .into(),
        endpoint(&config.external_midi.output_match, &a.routing_outputs),
        device,
        if config.controller_clock.enabled {
            "ON"
        } else {
            "OFF"
        }
        .into(),
        endpoint(&config.controller_clock.output_match, &a.routing_outputs),
        crate::ui_text::label_value(
            audio_name,
            if audio_online { "ONLINE" } else { "OFFLINE" },
            value_width,
        ),
    ];
    let editing = a.routing.draft.is_some();
    let mut lines = vec![Spans::from(Span::styled(
        crate::ui_text::label_value("ROUTING", if editing { "EDIT" } else { "BROWSE" }, width),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ))];
    for (index, (row, value)) in RoutingRow::ALL.iter().zip(values).enumerate() {
        let selected = index == a.routing.selected;
        let text = crate::ui_text::fixed_label_value(
            &format!("{}{}", if selected { ">" } else { " " }, row.label()),
            ROUTING_LABEL_CELLS,
            &value,
            width,
        );
        let style = if selected && editing {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else if selected {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Spans::from(Span::styled(text, style)));
    }
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        ),
        body,
    );
}

fn draw_controller_learn<B: Backend>(f: &mut Frame<B>, a: &App) {
    let full = f.size();
    let area = rect(full.x, full.y, full.width, full.height.saturating_sub(1));
    let session = a.controller_learn.as_ref().expect("learn modal is active");
    let (step, total) = session.progress();
    let role = session.role();
    let draft = session.draft();
    let feedback_lower = session.feedback().to_ascii_lowercase();
    let mut lines = vec![
        Spans::from(Span::styled(
            format!("MIDI LEARN · {step}/{total}"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Spans::from("Controller isolated · synth protected"),
        Spans::from(""),
        Spans::from(Span::styled(
            role.label(),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Spans::from(if session.can_finish() {
            "Move once · auto-next after settle/release"
        } else {
            "Finish left, finish right, click/release"
        }),
        Spans::from(""),
        Spans::from(Span::styled(
            crate::ui_text::fit_line(
                session.feedback(),
                usize::from(area.width.saturating_sub(2)),
            ),
            Style::default().fg(
                if feedback_lower.contains("conflict") || feedback_lower.contains("expected") {
                    Color::Red
                } else {
                    Color::Green
                },
            ),
        )),
        Spans::from(""),
        Spans::from(format!(
            "Mapped: {} controls · {} buttons",
            draft.controls.len(),
            draft.pads.len() + draft.cc_buttons.len()
        )),
        Spans::from(format!(
            "Encoder: turn {} · click {}",
            if draft.encoder_relative_cc.is_some() {
                "OK"
            } else {
                "--"
            },
            if draft.encoder_press_cc.is_some() || draft.encoder_press_note.is_some() {
                "OK"
            } else {
                "--"
            }
        )),
        Spans::from(""),
    ];
    if role == crate::controller_learn::LearnRole::Confirm {
        lines.push(Spans::from(Span::styled(
            "Rotary click / Enter SAVE + EXIT",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Spans::from("Save makes a backup and activates now"));
    } else if session.can_finish() {
        lines.push(Spans::from("←/→ browse · S skip · R retry"));
        lines.push(Spans::from("Rotary gestures latch · click saves"));
        lines.push(Spans::from("Esc cancel keeps the previous file"));
    } else {
        lines.push(Spans::from("Master rotary required · Esc cancel"));
    }
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" CONTROLLER SETUP ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        ),
        area,
    );
}

fn draw_fx_rack<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let body = rect(z.x, z.y, z.width, z.height.saturating_sub(3));
    a.hits.list = body;
    let rack = project_fx_rack(&a.song.insert_rack, &a.song.aux_routing, a.fx_target);
    let rack_length = rack.map(|rack| rack.order.len()).unwrap_or(0);
    let inner_width = usize::from(body.width.saturating_sub(2));
    let mut lines = vec![Spans::from(Span::styled(
        crate::ui_text::label_value(
            &format!("FX {}", fx_target_label(a.fx_target)),
            &format!("CHAIN {rack_length}/8"),
            inner_width,
        ),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ))];
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
        lines.push(Spans::from(crate::ui_text::fit_line(
            &format!(
                "SEND {}  {}  RETURN {return_gain:.0} dB",
                send.map(|send| format!("{:.0} dB", send.level_db))
                    .unwrap_or_else(|| "OFF".into()),
                send.map(|send| send_point_label(send.point))
                    .unwrap_or("POST")
            ),
            inner_width,
        )));
        if let Some(meter) = a
            .engine
            .as_ref()
            .and_then(|engine| engine.aux_meter(aux_id))
        {
            let peak = meter.output.peak.left.max(meter.output.peak.right);
            let rms = meter.output.rms.left.max(meter.output.rms.right);
            lines.push(Spans::from(crate::ui_text::fit_line(
                &format!(
                    "RETURN pk {:>5.1} rms {:>5.1} dBFS",
                    meter_db(peak),
                    meter_db(rms)
                ),
                inner_width,
            )));
        }
    } else if a.fx_target > MAX_AUX_BUSES {
        if let Some(meter) = a.engine.as_ref().and_then(Engine::master_meter) {
            let peak = meter.output.peak.left.max(meter.output.peak.right);
            let rms = meter.output.rms.left.max(meter.output.rms.right);
            lines.push(Spans::from(crate::ui_text::fit_line(
                &format!(
                    "MASTER pk {:>5.1} rms {:>5.1} dBFS",
                    meter_db(peak),
                    meter_db(rms)
                ),
                inner_width,
            )));
        }
    }
    if let Some(rack) = rack {
        for (index, id) in rack.order.iter().copied().enumerate() {
            let effect = rack.effect(id).expect("validated rack order");
            let selected = a.fx_selection == FxRackSelection::Effect(id);
            let marker = if selected { ">" } else { " " };
            let state = if effect.bypass { "BYP" } else { "ON " };
            let type_active = a
                .fx_type_edit
                .as_ref()
                .is_some_and(|edit| edit.effect_id == id);
            let style = if selected && type_active {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else if selected {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else if effect.bypass {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Spans::from(Span::styled(
                crate::ui_text::fit_line(
                    &format!(
                        "{marker} {:>2}. {:<12} #{id:<3} {state}",
                        index + 1,
                        effect_kind_label(effect.kind)
                    ),
                    inner_width,
                ),
                style,
            )));
        }
    }
    let insert_selected = a.fx_selection == FxRackSelection::Insert;
    lines.push(Spans::from(Span::styled(
        crate::ui_text::fit_line(
            &format!(
                "{} + INSERT EFFECT",
                if insert_selected { ">" } else { " " }
            ),
            inner_width,
        ),
        if insert_selected {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        },
    )));
    lines.push(Spans::from(""));
    lines.push(Spans::from("Stop transport for structural edits"));
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

fn compressor_gain_reduction_meter(gain_reduction_db: f32, width: usize) -> Spans<'static> {
    let gain_reduction_db = if gain_reduction_db.is_finite() {
        gain_reduction_db.max(0.0)
    } else {
        0.0
    };
    let label = "GR .5 ";
    let scale_end = " 24dB";
    let meter_width =
        label.len() + COMPRESSOR_GAIN_REDUCTION_LEDS_DB.len() * 2 - 1 + scale_end.len();
    let mut spans = vec![Span::raw(" ".repeat(width.saturating_sub(meter_width) / 2))];
    spans.push(Span::styled(label, Style::default().fg(Color::Gray)));
    for (index, threshold_db) in COMPRESSOR_GAIN_REDUCTION_LEDS_DB.iter().enumerate() {
        spans.push(Span::styled(
            COMPRESSOR_LED_GLYPH,
            Style::default().fg(if gain_reduction_db >= *threshold_db {
                Color::LightRed
            } else {
                Color::Red
            }),
        ));
        if index + 1 < COMPRESSOR_GAIN_REDUCTION_LEDS_DB.len() {
            spans.push(Span::raw(" "));
        }
    }
    spans.push(Span::styled(scale_end, Style::default().fg(Color::Gray)));
    Spans::from(spans)
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
            let color = match (cell.state, cell.color) {
                (LedState::Off, _) => Color::DarkGray,
                (LedState::Level, color) => performance_color(color),
                (LedState::Peak, MeterColor::Green) => Color::LightGreen,
                (LedState::Peak, MeterColor::Yellow) => Color::LightYellow,
                (LedState::Peak, MeterColor::Red) => Color::LightRed,
            };
            Span::styled(
                "●",
                Style::default()
                    .fg(color)
                    .add_modifier(if cell.state == LedState::Off {
                        Modifier::empty()
                    } else {
                        Modifier::BOLD
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
        AudioAvailability::LoopActive => "Separate WAV loop callback",
        AudioAvailability::DirectUnavailable => "Direct · meter unavailable",
        AudioAvailability::Stopped => "Engine stopped · meter unavailable",
        AudioAvailability::Presentation => "Presentation · no live audio",
    }
}

fn draw_performance_meter<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    if a.config.audio_graph.enabled {
        draw_final_performance_bus(f, a);
        return;
    }
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
        AudioAvailability::GraphActive
            | AudioAvailability::LoopActive
            | AudioAvailability::Presentation
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
            AudioAvailability::LoopActive => Color::Green,
            AudioAvailability::Presentation => Color::LightYellow,
            AudioAvailability::Stopped | AudioAvailability::DirectUnavailable => Color::DarkGray,
        }),
    )));
    if detailed {
        lines.push(Spans::from(vec![
            Span::styled("● RMS smoothed  ", Style::default().fg(Color::Green)),
            Span::styled("● bright peak  ", Style::default().fg(Color::LightYellow)),
            Span::styled("● unlit", Style::default().fg(Color::DarkGray)),
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
                .title(" MIX · PERFORMANCE ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        ),
        body,
    );
}

fn draw_final_performance_bus<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let body = rect(z.x, z.y, z.width, z.height.saturating_sub(3));
    let width = usize::from(body.width.saturating_sub(2));
    let controls = a.engine.as_ref().and_then(Engine::bus_controls);
    let meter = a.engine.as_ref().and_then(Engine::final_bus_meter);
    let sample_rate = a.engine.as_ref().and_then(Engine::audio_graph_sample_rate);
    if let Some(recording) = a.engine.as_mut().and_then(Engine::final_recording_status) {
        a.final_recording_last = recording;
    }
    let recording = a.final_recording_last.clone();
    let active = controls.is_some() && meter.is_some();
    let input = a
        .config
        .audio_graph
        .input
        .as_ref()
        .or_else(|| a.config.capture.inputs.first());
    let input_ready = input.is_some_and(|input| {
        a.capture_sources
            .iter()
            .any(|source| source == &input.left_port)
            && a.capture_sources
                .iter()
                .any(|source| source == &input.right_port)
    });
    let mut lines = Vec::new();
    lines.push(Spans::from(Span::styled(
        truncate(
            &format!(
                "THREE-SOURCE SUM · {}",
                if active { "ACTIVE" } else { "UNAVAILABLE" }
            ),
            width,
        ),
        Style::default()
            .fg(if active { Color::Green } else { Color::Red })
            .add_modifier(Modifier::BOLD),
    )));
    for (index, source) in BusSource::ALL.iter().copied().enumerate() {
        let ready = active
            && match source {
                BusSource::Synth => a.engine.is_some(),
                BusSource::Loop => a.loop_player.status().loaded,
                BusSource::Input => input_ready,
            };
        let (gain, muted) = controls.as_ref().map_or((0.0, false), |controls| {
            (
                controls.source_gain_db(source),
                controls.source_muted(source),
            )
        });
        let selected = a.bus_selected == index;
        lines.push(Spans::from(Span::styled(
            truncate(
                &format!(
                    "{} {:<5} {:>4.0}dB {:<4} {}",
                    if selected { ">" } else { " " },
                    source.label(),
                    gain,
                    if muted { "MUTE" } else { "ON" },
                    if ready { "READY" } else { "OFFLINE" }
                ),
                width,
            ),
            if selected {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else if ready {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        )));
    }
    let master_gain = controls
        .as_ref()
        .map(|controls| controls.master_gain_db())
        .unwrap_or(0.0);
    lines.push(Spans::from(Span::styled(
        truncate(
            &format!(
                "{} MASTER {:>4.0}dB",
                if a.bus_selected == 3 { ">" } else { " " },
                master_gain
            ),
            width,
        ),
        if a.bus_selected == 3 {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        },
    )));
    let meter = meter.unwrap_or_default();
    lines.push(Spans::from(truncate(
        &format!(
            "FINAL L {:>5.1}  R {:>5.1} dBFS",
            meter_db(meter.output.peak.left),
            meter_db(meter.output.peak.right)
        ),
        width,
    )));
    lines.push(Spans::from(Span::styled(
        truncate(
            &format!(
                "PRE CLIP {} · FINAL CLIP {} · GR {:.1}dB",
                meter.limiter_input.clips, meter.output.clips, meter.limiter_gain_reduction_db
            ),
            width,
        ),
        if meter.limiter_input.clips > 0 || meter.output.clips > 0 {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::Green)
        },
    )));
    let latency = sample_rate.map_or_else(
        || "2.5ms lookahead".into(),
        |rate| {
            let samples =
                (rate as f32 * crate::final_bus::LIMITER_LOOKAHEAD_SECONDS).round() as u32;
            format!(
                "{samples} samples/{:.3}ms",
                samples as f64 * 1000.0 / f64::from(rate)
            )
        },
    );
    lines.push(Spans::from(truncate(
        &format!("LIMIT -1.0dBFS · knee 3dB · {latency}"),
        width,
    )));
    lines.push(Spans::from(truncate(
        &format!(
            "REC {}  {:02}:{:02}  {}",
            if recording.recording {
                "ACTIVE"
            } else if recording.error.is_some() {
                "ERROR "
            } else {
                "STOPPED"
            },
            recording.elapsed.as_secs() / 60,
            recording.elapsed.as_secs() % 60,
            format_bytes(recording.bytes)
        ),
        width,
    )));
    lines.push(Spans::from(truncate(
        &format!(
            "DROP {} · OVF {} · HIGH {}f",
            recording.dropped_frames, recording.overflow_events, recording.writer_high_water_frames
        ),
        width,
    )));
    if let Some(error) = recording.error {
        lines.push(Spans::from(Span::styled(
            truncate(&error, width),
            Style::default().fg(Color::Red),
        )));
    } else if let Some(path) = recording.path {
        lines.push(Spans::from(truncate(
            &format!("FILE {}", path.display()),
            width,
        )));
    }
    lines.push(Spans::from(Span::styled(
        truncate(
            if a.config.audio_graph.input_direct_monitoring {
                if a.config.audio_graph.confirm_doubled_monitoring {
                    "MONITOR software + confirmed direct"
                } else {
                    "MONITOR REFUSED · doubled path"
                }
            } else {
                "MONITOR software · direct off"
            },
            width,
        ),
        Style::default().fg(if a.config.audio_graph.input_direct_monitoring {
            Color::LightYellow
        } else {
            Color::Green
        }),
    )));
    lines.truncate(usize::from(body.height.saturating_sub(2)));
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" MIX · PERFORMANCE BUS ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if active { Color::Green } else { Color::Red })),
        ),
        body,
    );
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1}MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1}KiB", bytes as f64 / 1024.0)
    }
}

fn draw_fx_editor<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let body = rect(z.x, z.y, z.width, z.height.saturating_sub(3));
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
    let controls = crate::effect_schema::controls(effect.kind);
    a.fx_parameter = a.fx_parameter.min(controls.len().saturating_sub(1));
    let inner_width = usize::from(body.width.saturating_sub(2));
    let state = if a.fx_value_editing {
        if a.fx_numeric_input.is_some() {
            "NUM"
        } else {
            "EDIT"
        }
    } else if effect.bypass {
        "BYP"
    } else {
        "ON"
    };
    let title = crate::ui_text::label_value(
        &format!(
            "{} · {} #{id}",
            fx_target_label(a.fx_target),
            effect_kind_label(effect.kind)
        ),
        state,
        inner_width,
    );
    let mut lines = vec![Spans::from(Span::styled(
        title,
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ))];
    lines.push(Spans::from(""));
    let base_width = inner_width / 4;
    let remainder = inner_width % 4;
    let widths = [
        base_width + usize::from(remainder > 0),
        base_width + usize::from(remainder > 1),
        base_width + usize::from(remainder > 2),
        base_width,
    ];
    let centered = |text: &str, width: usize| {
        let text = truncate(text, width);
        let left = width.saturating_sub(text.chars().count()) / 2;
        format!(
            "{}{}{}",
            " ".repeat(left),
            text,
            " ".repeat(width.saturating_sub(left + text.chars().count()))
        )
    };
    for control_row in 0..2 {
        let mut headings = Vec::with_capacity(4);
        let mut values = Vec::with_capacity(4);
        for (column, width) in widths.iter().copied().enumerate() {
            let index = control_row * 4 + column;
            let selected = index == a.fx_parameter;
            let style = if selected && a.fx_value_editing {
                Style::default().fg(Color::Black).bg(Color::Green)
            } else if selected {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };
            let Some(control) = controls.get(index) else {
                headings.push(Span::raw(" ".repeat(width)));
                values.push(Span::raw(" ".repeat(width)));
                continue;
            };
            let spec = crate::effect_schema::controlled_parameter(effect.kind, index)
                .expect("effect control layout references its persisted parameter");
            let value = effect
                .parameters
                .get(spec.name)
                .copied()
                .unwrap_or(spec.default);
            let shown_value = if selected {
                a.fx_numeric_input
                    .as_ref()
                    .map(|input| format!("{input}_"))
                    .unwrap_or_else(|| crate::effect_schema::format_value(effect.kind, spec, value))
            } else {
                crate::effect_schema::format_value(effect.kind, spec, value)
            };
            headings.push(Span::styled(centered(control.label, width), style));
            values.push(Span::styled(centered(&shown_value, width), style));
        }
        lines.push(Spans::from(headings));
        lines.push(Spans::from(values));
        if control_row == 0 {
            lines.push(Spans::from(""));
            lines.push(Spans::from(""));
        }
    }
    lines.push(Spans::from(""));
    let meter = a.engine.as_ref().and_then(|engine| engine.effect_meter(id));
    let meter_line = if effect.kind == EffectKind::Compressor {
        let gain_reduction_db = if effect.bypass {
            0.0
        } else {
            meter
                .and_then(|snapshot| snapshot.gain_reduction_db)
                .unwrap_or(0.0)
        };
        compressor_gain_reduction_meter(gain_reduction_db, inner_width)
    } else if let Some(meter) = meter {
        let input_peak = meter.input.peak.left.max(meter.input.peak.right);
        let output_peak = meter.output.peak.left.max(meter.output.peak.right);
        Spans::from(crate::ui_text::fit_line(
            &format!(
                "IN {:.0}  OUT {:.0}",
                meter_db(input_peak),
                meter_db(output_peak),
            ),
            inner_width,
        ))
    } else {
        Spans::from(crate::ui_text::fit_line("METER --", inner_width))
    };
    if lines.len() < usize::from(body.height.saturating_sub(2)) {
        lines.push(meter_line);
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
        rect(z.x, z.y + 1, z.width, z.height.saturating_sub(4)),
    );
}

fn draw_tracker_loop<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    if a.loop_library_mode {
        let body = rect(z.x, z.y, z.width, z.height.saturating_sub(3));
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
        let state = if let Some(error) = player.error.as_deref() {
            format!(
                "OUTPUT FAULT · {}",
                truncate(error, z.width.saturating_sub(15) as usize)
            )
        } else if player.loaded {
            "READY".into()
        } else {
            "NOT READY".into()
        };
        format!(
            "P{:02}/{:02} · FT2 WAV LOOP\n{}\n{}\nSource {:>6.2} BPM  {}\nProject tempo {:>3} BPM\nRegion beat {} +{}\nOffset {:+.0} bar(s)\nCut {} · meter {}/4\n\n{}  {} / {}\n{} Hz · {}ch\nNative pitch playback",
            a.tracker_loop_page_number(),
            a.tracker_page_count(),
            truncate(
                player.file.as_deref().unwrap_or(&settings.file),
                z.width.saturating_sub(2) as usize
            ),
            state,
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
            "P{:02}/{:02} · FT2 WAV LOOP\nUNLOADED\n\nInbox: {}\nSelected: {}\n\nTurn encoder to choose\nIMPORT copies to private storage\n\nAUTO estimates beat length.\nProject tempo follows WAV.",
            a.tracker_loop_page_number(),
            a.tracker_page_count(),
            truncate(
                &a.config.loop_player.import_directory.display().to_string(),
                z.width.saturating_sub(2) as usize
            ),
            truncate(&selected, z.width.saturating_sub(2) as usize)
        )
    };
    let mut lines = details.lines().map(Spans::from).collect::<Vec<_>>();
    if a.song.audio_loop.is_some() && !player.duration.is_zero() {
        // Keep the loop playhead sample-relative: playback and tracker REC
        // share the transport clock which advances `player.elapsed`.
        lines.insert(3.min(lines.len()), loop_position_bar(&player, z.width));
    }
    let body = rect(z.x, z.y, z.width, z.height.saturating_sub(3));
    if body.height >= 15 {
        let availability = a.loop_meter.audio_availability();
        let clipping = a.loop_meter.clipping(Instant::now());
        let fault = a.loop_meter.non_finite() > 0;
        let state = match availability {
            AudioAvailability::LoopActive => "",
            AudioAvailability::Presentation => " · PREVIEW",
            AudioAvailability::Stopped | AudioAvailability::DirectUnavailable => " · STOP",
            AudioAvailability::GraphActive => "",
        };
        let mut heading = vec![Span::styled(
            format!("STEREO VU · LOOP OUT · dBFS{state}"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )];
        if clipping {
            heading.push(Span::styled(
                " CLIP!",
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED),
            ));
        } else if fault {
            heading.push(Span::styled(
                " FAULT!",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
        }
        lines.push(Spans::from(heading));
        let levels = if matches!(
            availability,
            AudioAvailability::LoopActive | AudioAvailability::Presentation
        ) {
            a.loop_meter.audio_levels()
        } else {
            [AudioLevel::default(); 2]
        };
        let numeric_peaks = a.loop_meter.numeric_peak_dbfs();
        lines.push(audio_meter_line(
            'L',
            levels[0],
            numeric_peaks[0],
            usize::from(z.width),
        ));
        lines.push(audio_meter_line(
            'R',
            levels[1],
            numeric_peaks[1],
            usize::from(z.width),
        ));
    }
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(Style::default().fg(if player.error.is_some() {
                Color::Yellow
            } else {
                Color::Green
            })),
        body,
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
        rect(z.x, z.y, z.width, z.height.saturating_sub(3)),
    );
}

fn short_time(duration: Duration) -> String {
    format!(
        "{:02}:{:02}",
        duration.as_secs() / 60,
        duration.as_secs() % 60
    )
}

fn loop_position_bar(
    player: &crate::loop_player::LoopStatus,
    available_width: u16,
) -> Spans<'static> {
    let width = usize::from(if available_width >= 40 {
        40
    } else {
        available_width.min(38)
    });
    if width == 0 {
        return Spans::default();
    }
    let phase = (player.elapsed.as_secs_f64() / player.duration.as_secs_f64()).clamp(0.0, 1.0);
    let playhead = ((phase * width as f64).floor() as usize).min(width - 1);
    let track = Style::default().fg(Color::Black).bg(Color::White);
    let marker = Style::default().fg(Color::Black).bg(Color::Green);
    Spans::from(vec![
        Span::styled(" ".repeat(playhead), track),
        Span::styled(" ", marker),
        Span::styled(" ".repeat(width - playhead - 1), track),
    ])
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
fn draw_fallback_badge<B: Backend>(f: &mut Frame<B>, a: &App) {
    if a.fallback_notice().is_none() || a.screen != Screen::Playback {
        return;
    }
    let z = f.size();
    f.render_widget(
        Paragraph::new("FLT").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        rect(z.x, z.y, z.width.min(3), 1),
    );
}
fn overlay_rows(a: &App, overlay: &OverlayState) -> Vec<String> {
    match overlay.kind {
        OverlayKind::TrackerPage => {
            let mut rows = Vec::new();
            for (page_index, page) in a.current_pages().iter().enumerate() {
                for column_index in 0..LANES_PER_PAGE {
                    let column = page.column(column_index);
                    let channel = if page.target == PageTarget::Default {
                        "AU".into()
                    } else {
                        format!("{:02}", sequencer::musician_channel(column.channel))
                    };
                    rows.push(format!(
                        "P{:02} C{} {:<9} ch{} p{:03}",
                        page_index + 1,
                        column_index + 1,
                        truncate(&page.name, 9),
                        channel,
                        sequencer::musician_program(column.program)
                    ));
                }
            }
            rows.push(if let Some(settings) = &a.song.audio_loop {
                format!(
                    "P{:02} LOOP PLAYER · {}",
                    a.tracker_loop_page_number(),
                    truncate(&settings.file, 18)
                )
            } else {
                format!(
                    "P{:02} LOOP PLAYER · UNLOADED",
                    a.tracker_loop_page_number()
                )
            });
            rows.push("MANAGE PAGES / TRACKS…".into());
            rows
        }
        OverlayKind::TrackerPattern => {
            let mut rows = a
                .overlay_pattern_locations()
                .into_iter()
                .map(|(number, order)| {
                    a.song.patterns.get(&number).map_or_else(
                        || format!("PAT {number:02} · missing"),
                        |pattern| {
                            format!(
                                "PAT {number:02} · {} rows · {} BPM · ord {:02}",
                                pattern.rows.len(),
                                pattern.tempo,
                                order + 1
                            )
                        },
                    )
                })
                .collect::<Vec<_>>();
            rows.push("OPEN PATTERN TOOLS…".into());
            rows.push("OPEN PROJECT FILES…".into());
            rows
        }
        OverlayKind::TrackerSong => {
            let mut rows = a
                .song
                .order
                .iter()
                .enumerate()
                .map(|(order, pattern)| {
                    let detail = a.song.patterns.get(pattern).map_or_else(
                        || "missing".into(),
                        |pattern| {
                            format!(
                                "{}r {} BPM {}/4",
                                pattern.rows.len(),
                                pattern.tempo,
                                pattern.meter
                            )
                        },
                    );
                    format!("STEP {:02} · PAT {pattern:02} · {detail}", order + 1)
                })
                .collect::<Vec<_>>();
            rows.push("EDIT ARRANGEMENT…".into());
            rows.push("OPEN LOOP / PAGE TOOLS…".into());
            rows.push("TAP PATTERN TEMPO".into());
            rows
        }
        OverlayKind::TrackerRoute => {
            let Some(route) = overlay.route() else {
                return vec!["routing draft unavailable".into()];
            };
            let page = &route.page;
            let target_kind = match page.target {
                PageTarget::Default => "AUTO",
                PageTarget::ActiveInstrument | PageTarget::Synthv1(_) | PageTarget::Software(_) => {
                    "INTERNAL"
                }
                PageTarget::Midi(_) | PageTarget::ConfiguredExternal => "EXTERNAL MIDI",
            };
            let software = match &page.target {
                PageTarget::Software(route) => Some(route),
                _ => None,
            };
            let midi_output = match &page.target {
                PageTarget::ConfiguredExternal => {
                    Some(a.config.external_midi.output_match.as_str())
                }
                PageTarget::Midi(output) => Some(output.as_str()),
                _ => None,
            };
            let target = a.target_route_issue(&page.target).map_or_else(
                || format!("TARGET · {target_kind}"),
                |issue| format!("TARGET · {target_kind} · {issue}"),
            );
            let mut rows = vec![
                target,
                format!(
                    "ENGINE · {}",
                    software.map_or("—", |route| route.engine.label())
                ),
                format!(
                    "INSTR · {}",
                    software.map_or("—", |route| route.instrument.as_str())
                ),
                format!("MIDI OUT · {}", midi_output.unwrap_or("—")),
                format!(
                    "PROFILE · {}",
                    page.device_profile.as_deref().unwrap_or("RAW MIDI")
                ),
            ];
            for column_index in 0..LANES_PER_PAGE {
                let column = page.column(column_index);
                let auto = page.target == PageTarget::Default;
                rows.push(format!(
                    "C{} CHANNEL · {}",
                    column_index + 1,
                    if auto {
                        "AUTO".into()
                    } else {
                        sequencer::musician_channel(column.channel).to_string()
                    }
                ));
                rows.push(format!(
                    "C{} BANK MSB · {}",
                    column_index + 1,
                    if auto {
                        "AUTO".into()
                    } else {
                        column.bank_msb.to_string()
                    }
                ));
                rows.push(format!(
                    "C{} BANK LSB · {}",
                    column_index + 1,
                    if auto {
                        "AUTO".into()
                    } else {
                        column.bank_lsb.to_string()
                    }
                ));
                rows.push(format!(
                    "C{} PROGRAM · {} · {}",
                    column_index + 1,
                    if auto {
                        "AUTO".into()
                    } else {
                        sequencer::musician_program(column.program).to_string()
                    },
                    if auto {
                        "machine default".into()
                    } else {
                        a.route_program_label(page, column_index)
                    }
                ));
            }
            rows.push(format!(
                "APPLY ROUTING{}",
                if route.dirty() { " · CHANGED" } else { "" }
            ));
            rows
        }
        OverlayKind::TrackerPatternLength => pattern_length_choices()
            .into_iter()
            .map(|rows| format!("{rows:>3} ROWS"))
            .collect(),
        OverlayKind::TrackerNoteLength => NoteLength::ALL
            .iter()
            .map(|length| format!("NOTE {}", length.label()))
            .collect(),
        OverlayKind::TrackerAdvance => (0..=32)
            .map(|rows| {
                if rows == 0 {
                    "0 ROWS · STAY ON CURRENT ROW".into()
                } else {
                    format!("{rows} ROW(S)")
                }
            })
            .collect(),
        OverlayKind::LoopLibrary => a
            .loop_imports
            .iter()
            .map(|path| {
                let file = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                crate::ui_text::label_value(&file, "INBOX", 35)
            })
            .chain(a.loop_library.iter().map(|entry| {
                let state = if entry.current {
                    "CURRENT".into()
                } else if entry.saved_references == 0 {
                    "PRIVATE".into()
                } else {
                    format!("{} SAVED", entry.saved_references)
                };
                crate::ui_text::label_value(&entry.file, &state, 35)
            }))
            .collect(),
        OverlayKind::MixEffects => (0..=MAX_AUX_BUSES + 1)
            .map(|target| {
                let effects = project_fx_rack(&a.song.insert_rack, &a.song.aux_routing, target)
                    .map_or(0, |rack| rack.order.len());
                format!("{} · {effects} effect(s)", fx_target_label(target))
            })
            .collect(),
    }
}

fn draw_overlay<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let geometry = overlay::geometry(f.size());
    if geometry.outer.width == 0 || geometry.outer.height == 0 {
        return;
    }
    let Some(snapshot) = a.overlay.as_ref().cloned() else {
        return;
    };
    let rows = overlay_rows(a, &snapshot);
    let visible_rows = usize::from(geometry.inner.height);
    if let Some(overlay) = a.overlay.as_mut() {
        overlay.keep_selection_visible(visible_rows, rows.len());
    }
    let Some(overlay) = a.overlay.as_ref() else {
        return;
    };
    let scroll = overlay.scroll;
    let selection = overlay.selection;
    let active_field = overlay.active_field;
    f.render_widget(Clear, geometry.outer);
    f.render_widget(
        Block::default()
            .title(Spans::from(Span::styled(
                format!(" {} ", overlay.title),
                Style::default()
                    .fg(Color::LightYellow)
                    .bg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )))
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Cyan).bg(Color::Black))
            .style(Style::default().bg(Color::Black)),
        geometry.outer,
    );
    f.render_widget(Clear, geometry.inner);
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Black)),
        geometry.inner,
    );
    for (screen_row, (index, line)) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_rows)
        .enumerate()
    {
        let selected = index == selection;
        let active = selected && active_field.is_some();
        let marker = if active {
            "*"
        } else if selected {
            ">"
        } else {
            " "
        };
        let style = if active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else if selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray).bg(Color::Black)
        };
        f.render_widget(
            Paragraph::new(truncate(
                &format!("{marker}{line}"),
                usize::from(geometry.inner.width),
            ))
            .style(style),
            rect(
                geometry.inner.x,
                geometry.inner.y + screen_row as u16,
                geometry.inner.width,
                1,
            ),
        );
    }
    a.hits.actions.clear();
    a.hits.menu_pages.clear();
    a.hits.list = geometry.inner;
}

fn draw_overlay_launcher<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let Some(launcher) = a.overlay.as_ref().map(|overlay| overlay.launcher.clone()) else {
        return;
    };
    let z = f.size();
    let geometry = overlay::geometry(z);
    if geometry.outer.height == 0 {
        return;
    }
    let row = rect(
        geometry.outer.x,
        geometry.outer.y + geometry.outer.height - 1,
        geometry.outer.width,
        1,
    );
    let menu_width = geometry.inner.width;
    let menu_x = geometry.inner.x;
    let width = menu_width / 4;
    let button = rect(menu_x + launcher.item as u16 * width, row.y, width, 1);
    f.render_widget(
        Paragraph::new(format!(
            "[{}]",
            truncate(launcher.label, usize::from(button.width.saturating_sub(2)))
        ))
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        ),
        button,
    );
    a.hits.actions.push((button, launcher.action));
}

fn draw_master_status<B: Backend>(f: &mut Frame<B>, a: &App) {
    let z = f.size();
    if z.height == 0 || z.width == 0 {
        return;
    }
    let row = rect(z.x, z.y + z.height - 1, z.width, 1);
    f.render_widget(Clear, row);
    let state = a.transport_indicator();
    let (glyph, _) = transport_glyph(state);
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let indicator_color = transport_color(state, elapsed);
    let message = if a.status == "Ready" {
        ""
    } else {
        a.status.as_str()
    };
    f.render_widget(
        Paragraph::new(Spans::from(vec![
            Span::styled(
                glyph,
                Style::default()
                    .fg(indicator_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                truncate(message, usize::from(z.width.saturating_sub(2))),
                Style::default().fg(Color::Gray),
            ),
        ]))
        .style(Style::default().bg(Color::Black)),
        row,
    );
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
    let footer_rows = 3;
    f.render_widget(
        Clear,
        rect(
            menu_x,
            z.y + z.height - footer_rows,
            menu_width,
            footer_rows - 1,
        ),
    );
    if a.screen != Screen::Playback {
        f.render_widget(
            Paragraph::new(BUILD_BADGE).style(Style::default().fg(if cfg!(debug_assertions) {
                Color::Yellow
            } else {
                Color::Green
            })),
            rect(menu_x, z.y + z.height - footer_rows, 3, 1),
        );
    }
    for (i, page) in pages.iter().enumerate() {
        let col = i as u16;
        let cell_width = menu_width / 4;
        let badge_width = if i == 0 && a.screen != Screen::Playback {
            3
        } else {
            0
        };
        let width = cell_width.saturating_sub(badge_width);
        let x0 = menu_x + col * cell_width + badge_width;
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
fn draw_list<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let head = rect(z.x, z.y, z.width, 2);
    let list = rect(z.x, z.y + 2, z.width, z.height.saturating_sub(5));
    let rows = list.height.saturating_sub(2) as usize;
    a.ensure_visible(rows);
    let inner = rect(list.x + 1, list.y + 1, list.width - 2, list.height - 2);
    a.hits.list = inner;
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
}
fn draw_playing<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let header = rect(z.x, z.y, z.width, 1);
    let actions = rect(z.x, z.y + z.height - 3, z.width, 2);
    let params = rect(z.x, z.y + 1, z.width, z.height.saturating_sub(4));
    let name = a
        .playing
        .as_ref()
        .map(|p| format!("{} · {}", p.backend.label(), p.name))
        .unwrap_or_else(|| "none".into());
    f.render_widget(
        Paragraph::new(BUILD_BADGE).style(
            Style::default()
                .fg(if cfg!(debug_assertions) {
                    Color::Yellow
                } else {
                    Color::Green
                })
                .add_modifier(Modifier::BOLD),
        ),
        rect(header.x, header.y, header.width.min(3), 1),
    );
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
    } else if a.idea_mode == IdeaMode::Record {
        ("R-M", Color::Yellow)
    } else if a.playback_noob {
        ("N0B", Color::Yellow)
    } else {
        ("", Color::White)
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
    let scale_rows = if a.playback_noob { 1 } else { 0 };
    if a.playback_noob {
        f.render_widget(
            Paragraph::new(Spans::from(vec![
                Span::styled("SCALE ", Style::default().fg(Color::DarkGray)),
                Span::styled("◉ ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!(
                        "{} {}",
                        a.config.note_naming.pitch_name(a.noob_scale.root),
                        a.noob_scale.kind.label()
                    ),
                    Style::default()
                        .fg(Color::LightYellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]))
            .alignment(Alignment::Center),
            rect(z.x, params.y + 6, z.width, 1),
        );
    }
    let chord_area = rect(
        z.x,
        params.y + 6 + scale_rows,
        z.width,
        actions.y.saturating_sub(params.y + 6 + scale_rows),
    );
    let content_height = 5.min(chord_area.height);
    let top = chord_area.y + chord_area.height.saturating_sub(content_height) / 2;
    if let Some(display) = a.held_notes.display(a.config.note_naming) {
        f.render_widget(
            Paragraph::new(display.chord)
                .style(
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
                .alignment(Alignment::Center),
            rect(chord_area.x, top, chord_area.width, 1),
        );
        if chord_area.height >= 2 {
            let (notes, velocities) = held_note_rows(&display.notes, chord_area.width);
            f.render_widget(
                Paragraph::new(notes)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::DarkGray)),
                rect(chord_area.x, top + 1, chord_area.width, 1),
            );
            if chord_area.height >= 3 {
                f.render_widget(
                    Paragraph::new(velocities)
                        .alignment(Alignment::Center)
                        .style(Style::default().fg(Color::LightYellow)),
                    rect(chord_area.x, top + 2, chord_area.width, 1),
                );
            }
        }
    }
    if chord_area.height >= 5 {
        draw_playback_keyboard(f, a, rect(chord_area.x, top + 3, chord_area.width, 2));
    }
}

fn held_note_rows(notes: &[crate::chord::HeldNoteDisplay], width: u16) -> (String, String) {
    let visible = notes
        .iter()
        .take((usize::from(width) + 1) / 4)
        .collect::<Vec<_>>();
    let names = visible
        .iter()
        .map(|note| format!("{:^3}", note.name))
        .collect::<Vec<_>>()
        .join(" ");
    let velocities = visible
        .iter()
        .map(|note| format!("{:^3}", note.velocity))
        .collect::<Vec<_>>()
        .join(" ");
    (names, velocities)
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
    let list = rect(z.x, z.y + 2, z.width, z.height.saturating_sub(5));
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
                truncate(
                    &format!(
                        "{} {}",
                        if i == a.idea_selected { "▶" } else { " " },
                        a.ideas[i]
                    ),
                    usize::from(inner.width),
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
        format!("EDIT +{} {}", a.tracker_advance, a.note_length.label())
    } else if a.tracker_mode == TrackerMode::Rec {
        "REC READY".into()
    } else {
        "PAUSE".into()
    };
    let state = if a.tracker_noob && !page.percussion {
        format!(
            "{state} N0B {} {}",
            a.config.note_naming.pitch_name(a.noob_scale.root),
            a.noob_scale.kind.label()
        )
    } else {
        state
    };
    f.render_widget(
        Paragraph::new(truncate(
            &format!(
                "{} · O{:02}/{:02} P{:02} · {state}",
                a.song.name,
                a.tracker_order + 1,
                a.song.order.len(),
                pattern_number
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
    let grid = rect(z.x, z.y + 1, z.width, z.height.saturating_sub(5));
    let program_browser = a.note_editor.as_ref().is_some_and(|editor| {
        editor.active
            && matches!(
                editor.field,
                NoteEditorField::DefaultProgram | NoteEditorField::Program
            )
            && page.runtime_channel(a.tracker_track, &a.config.external_midi) != 9
    });
    if program_browser {
        draw_tracker_program_browser(f, a, grid);
    } else if a.note_editor.is_some() {
        draw_tracker_note_editor(f, a, grid);
    } else {
        let visible_tracks = LANES_PER_PAGE;
        let first_track = a.tracker_page * LANES_PER_PAGE;
        let row_width = 3u16;
        let column_width = grid.width.saturating_sub(row_width) / visible_tracks.max(1) as u16;
        let rows = grid.height.saturating_sub(1) as usize;
        let start = a.tracker_row.saturating_sub(rows / 2);
        for index in 0..visible_tracks {
            f.render_widget(
                Block::default().style(Style::default().bg(if index == a.tracker_track {
                    Color::Indexed(234)
                } else {
                    Color::Black
                })),
                rect(
                    grid.x + row_width + index as u16 * column_width,
                    grid.y,
                    column_width,
                    grid.height,
                ),
            );
        }
        let mut header = vec![Span::styled(
            "ROW",
            Style::default().fg(Color::Yellow).bg(Color::Black),
        )];
        for (index, lane) in page.lanes.iter().enumerate() {
            let setup = page.column(index);
            let channel = if page.target == PageTarget::Default {
                "AU".to_owned()
            } else {
                format!("{:02}", sequencer::musician_channel(setup.channel))
            };
            let compact = format!(
                "{}:{}/{:03}",
                index + 1,
                channel,
                sequencer::musician_program(setup.program)
            );
            header.push(Span::styled(
                format!(
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
                ),
                Style::default()
                    .fg(Color::Yellow)
                    .bg(if index == a.tracker_track {
                        Color::Indexed(234)
                    } else {
                        Color::Black
                    }),
            ));
        }
        f.render_widget(
            Paragraph::new(Spans::from(header)),
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
                    .as_ref()
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
                let column_background = if track_index == first_track + a.tracker_track {
                    Color::Indexed(234)
                } else {
                    Color::Black
                };
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
                        Style::default().fg(Color::Yellow).bg(column_background)
                    } else {
                        Style::default().bg(column_background)
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
    let route_issue = a.target_route_issue(&page.target);
    let footer = if let Some(editor) = a.note_editor.as_ref() {
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
            "{}{} · {} v{} g{} · {} · {command}",
            editor.field.label(),
            if editor.active { " ACTIVE" } else { "" },
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
            "P{}/{} {} L{} ch{} {} {}{}",
            a.tracker_page + 1,
            a.tracker_page_count(),
            page.name,
            a.tracker_track + 1,
            sequencer::musician_channel(column.channel),
            truncate(page.target.label(), 10),
            if !page.enabled {
                "PAGE MUTE"
            } else if !lane.enabled {
                "MUTE"
            } else if page.percussion {
                "DRUM"
            } else {
                "ON"
            },
            route_issue.map_or_else(String::new, |issue| format!(" · {issue}")),
        )
    };
    f.render_widget(
        Paragraph::new(truncate(&footer, z.width as usize)).style(Style::default().fg(
            if route_issue.is_some() {
                Color::Yellow
            } else {
                Color::DarkGray
            },
        )),
        rect(z.x, z.y + z.height.saturating_sub(4), z.width, 1),
    );
}

fn draw_tracker_note_editor<B: Backend>(f: &mut Frame<B>, a: &App, area: Rect) {
    let (Some(editor), Some(page)) = (a.note_editor.as_ref(), a.current_page()) else {
        return;
    };
    let column = page.column(a.tracker_track);
    let channel = page.runtime_channel(a.tracker_track, &a.config.external_midi);
    let command = match editor.draft.command {
        Command::None => ("none".into(), "—".into()),
        Command::Cut(value) => ("cut".into(), format!("{value}/15")),
        Command::Delay(value) => ("delay".into(), format!("{value}/15")),
        Command::Retrigger(value) => ("retrigger".into(), format!("{value}/8")),
        Command::Tempo(value) => ("tempo".into(), value.to_string()),
    };
    let note = match editor.draft.note {
        Note::On(note) if channel == 9 => format!(
            "{note} {}",
            crate::gm::percussion_note(note).unwrap_or("GM drum")
        ),
        note => sequencer::note_name(note),
    };
    let values = [
        (
            NoteEditorField::Destination,
            page.target.label().to_owned(),
            "audition + page",
        ),
        (
            NoteEditorField::Channel,
            format!(
                "{} · {}",
                sequencer::musician_channel(channel),
                if channel == 9 {
                    "GM DRUMS"
                } else {
                    "GM MELODIC"
                }
            ),
            "audition + new notes",
        ),
        (
            NoteEditorField::DefaultProgram,
            a.tracker_instrument_label(),
            if channel == 9 {
                "audition + cell note"
            } else {
                "audition + new notes"
            },
        ),
        (
            NoteEditorField::BankMsb,
            column.bank_msb.to_string(),
            "new notes",
        ),
        (
            NoteEditorField::BankLsb,
            column.bank_lsb.to_string(),
            "new notes",
        ),
        (NoteEditorField::Note, note, "this cell"),
        (
            NoteEditorField::Gate,
            editor
                .draft
                .gate
                .map_or_else(|| "inherit".into(), |value| format!("{value}%")),
            "this cell",
        ),
        (
            NoteEditorField::Velocity,
            editor
                .draft
                .velocity
                .map_or_else(|| "inherit".into(), |value| value.to_string()),
            "this cell",
        ),
        (
            NoteEditorField::Program,
            editor.draft.program.map_or_else(
                || "inherit default".into(),
                |value| a.tracker_program_label(value),
            ),
            "this cell only",
        ),
        (NoteEditorField::Effect, command.0, "this cell"),
        (NoteEditorField::EffectParameter, command.1, "this cell"),
    ];
    for (index, (field, value, scope)) in values.into_iter().enumerate() {
        if index >= usize::from(area.height) {
            break;
        }
        let selected = editor.field == field;
        let marker = if selected && editor.active {
            "*"
        } else if selected {
            ">"
        } else {
            " "
        };
        let line = format!(
            "{marker}{:<8} {:<17} {}",
            truncate(field.label(), 8),
            truncate(&value, 17),
            scope
        );
        let style = if selected && editor.active {
            Style::default().fg(Color::Black).bg(Color::Green)
        } else if selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        f.render_widget(
            Paragraph::new(truncate(&line, area.width as usize)).style(style),
            rect(area.x, area.y + index as u16, area.width, 1),
        );
    }
}

fn draw_tracker_program_browser<B: Backend>(f: &mut Frame<B>, a: &App, area: Rect) {
    let Some(page) = a.current_page() else {
        return;
    };
    let selected = a
        .note_editor
        .as_ref()
        .and_then(|editor| {
            if editor.field == NoteEditorField::DefaultProgram {
                Some(page.column(a.tracker_track).program)
            } else {
                editor.draft.program
            }
        })
        .unwrap_or(page.column(a.tracker_track).program);
    let title = a
        .tracker_device_profile()
        .map(DeviceProfile::label)
        .unwrap_or_else(|| "General MIDI".into());
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
        let text = format!(
            "{:03}  {}",
            sequencer::musician_program(program),
            a.tracker_program_label(program)
        );
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
            let channel = if a
                .current_page()
                .is_some_and(|page| page.target == PageTarget::Default)
            {
                "AUTO".to_owned()
            } else {
                sequencer::musician_channel(column.channel).to_string()
            };
            format!(
                "C{} ch{} b{}/{} {}",
                a.tracker_track + 1,
                channel,
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
    let body_height = z.height.saturating_sub(4);
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
                    let visible_width = usize::from(z.width.saturating_sub(2));
                    let issue = a.target_route_issue(&page.target);
                    let channel = if page.target == PageTarget::Default {
                        "AU".to_owned()
                    } else {
                        format!(
                            "{:02}",
                            sequencer::musician_channel(page.column(a.tracker_track).channel)
                        )
                    };
                    let base = format!(
                        "{}{:02} {:<8} C{} ch{} p{:03} {}",
                        if index == a.tracker_page { "▶" } else { " " },
                        index + 1,
                        truncate(&page.name, 8),
                        a.tracker_track + 1,
                        channel,
                        sequencer::musician_program(page.column(a.tracker_track).program),
                        truncate(page.target.label(), 7),
                    );
                    let suffix = issue.map_or_else(String::new, |issue| format!(" · {issue}"));
                    let text = format!(
                        "{}{}",
                        truncate(&base, visible_width.saturating_sub(suffix.chars().count())),
                        suffix
                    );
                    Spans::from(Span::styled(
                        truncate(&text, visible_width),
                        if index == a.tracker_page {
                            Style::default().fg(Color::Black).bg(Color::Yellow)
                        } else if issue.is_some() {
                            Style::default().fg(Color::Yellow)
                        } else {
                            Style::default()
                        },
                    ))
                })
                .collect::<Vec<_>>();
            f.render_widget(
                Paragraph::new(lines).block(
                    Block::default()
                        .title(format!(" {} MIDI pages ", a.current_pages().len()))
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
                .map(|target| match target {
                    PageTarget::Default => "AUTO",
                    PageTarget::Software(_) | PageTarget::Synthv1(_) => "INTERNAL SOFTWARE",
                    PageTarget::ConfiguredExternal | PageTarget::Midi(_) => "EXTERNAL MIDI",
                    PageTarget::ActiveInstrument => "LEGACY INTERNAL",
                })
                .unwrap_or("no MIDI outputs");
            let issue = a
                .page_target_candidates
                .get(a.page_target_selected)
                .and_then(|target| a.target_route_issue(target));
            let mut lines = vec![
                Spans::from("TARGET DEVICE"),
                Spans::from(""),
                Spans::from(Span::styled(
                    format!(
                        "▶ {}",
                        truncate(target, usize::from(z.width.saturating_sub(6)))
                    ),
                    Style::default().fg(Color::Black).bg(Color::Green),
                )),
            ];
            if let Some(issue) = issue {
                lines.push(Spans::from(format!("{issue} · data is kept")));
            }
            lines.push(Spans::from(format!(
                "turn encoder · {}",
                a.page_field_confirm_hint()
            )));
            f.render_widget(
                Paragraph::new(lines).block(Block::default().borders(Borders::ALL)),
                rect(z.x, z.y + 1, z.width, body_height),
            );
        }
        PageManagerMode::Engine | PageManagerMode::Instrument | PageManagerMode::MidiOutput => {
            let (title, value, issue) = match a.page_manager_mode {
                PageManagerMode::Engine => (
                    "SOFTWARE ENGINE",
                    a.current_page()
                        .and_then(|page| match &page.target {
                            PageTarget::Software(route) => Some(route.engine.label()),
                            _ => None,
                        })
                        .unwrap_or("NONE"),
                    None,
                ),
                PageManagerMode::Instrument => (
                    "ENGINE INSTRUMENT",
                    a.current_page()
                        .and_then(|page| match &page.target {
                            PageTarget::Software(route) => Some(route.instrument.as_str()),
                            _ => None,
                        })
                        .unwrap_or("NONE"),
                    None,
                ),
                PageManagerMode::MidiOutput => {
                    let target = a.current_page().map(|page| &page.target);
                    (
                        "MIDI OUTPUT",
                        target.map(PageTarget::label).unwrap_or("NONE"),
                        target.and_then(|target| a.target_route_issue(target)),
                    )
                }
                _ => unreachable!(),
            };
            let mut lines = vec![
                Spans::from(title),
                Spans::from(""),
                Spans::from(Span::styled(
                    format!(
                        "▶ {}",
                        truncate(value, usize::from(z.width.saturating_sub(6)))
                    ),
                    Style::default().fg(Color::Black).bg(Color::Green),
                )),
            ];
            if let Some(issue) = issue {
                lines.push(Spans::from(format!("{issue} · data is kept")));
            }
            lines.push(Spans::from(format!(
                "turn encoder · {}",
                a.page_field_confirm_hint()
            )));
            f.render_widget(
                Paragraph::new(lines).block(Block::default().borders(Borders::ALL)),
                rect(z.x, z.y + 1, z.width, body_height),
            );
        }
        PageManagerMode::Channel => {
            f.render_widget(
                Paragraph::new(vec![
                    Spans::from("MIDI CHANNEL · ACTIVE"),
                    Spans::from(""),
                    Spans::from(Span::styled(
                        format!("▶ {:02}", a.page_channel_draft + 1),
                        Style::default().fg(Color::Black).bg(Color::Green),
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
    let list = rect(z.x, z.y + 1, z.width, z.height.saturating_sub(4));
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
            Spans::from("LNGTH: rows · buttons: meter/confirm"),
            Spans::from("EXIT cancels"),
        ];
        f.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL)),
            rect(z.x, z.y + 1, z.width, z.height.saturating_sub(4)),
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
            rect(z.x, z.y + 1, z.width, z.height.saturating_sub(4)),
        );
        return;
    }
    let list = rect(z.x, z.y + 1, z.width, z.height.saturating_sub(4));
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
                truncate(
                    &format!("{} {name}", if selected { "▶" } else { " " }),
                    usize::from(inner.width),
                ),
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
}

fn draw_audio_recorder<B: Backend>(f: &mut Frame<B>, a: &mut App) {
    let z = f.size();
    let s = a.audio_recorder.status();
    let state = if s.recording {
        "● SYNCHRONIZED TAKE"
    } else {
        "MULTITRACK RECORDER"
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
        .unwrap_or_else(|| "no saved take yet".into());
    a.audio_track_selected = a.audio_track_selected.min(s.tracks.len().saturating_sub(1));
    let selected = s.tracks.get(a.audio_track_selected);
    let list_rows = 6usize.min(s.tracks.len());
    let offset = a
        .audio_track_selected
        .saturating_add(1)
        .saturating_sub(list_rows)
        .min(s.tracks.len().saturating_sub(list_rows));
    let line_width = z.width.saturating_sub(1) as usize;
    let mut lines = vec![Spans::from(truncate(
        &format!(
            "{elapsed} · {}/{} armed · {} Hz",
            s.active_tracks,
            s.tracks.len(),
            s.sample_rate
        ),
        line_width,
    ))];
    if s.tracks.is_empty() {
        lines.push(Spans::from("No capture tracks configured"));
    } else {
        for (index, track) in s.tracks.iter().enumerate().skip(offset).take(list_rows) {
            let marker = if index == a.audio_track_selected {
                ">"
            } else {
                " "
            };
            let arm = if track.armed { "●" } else { "○" };
            let source = if track.resolved { "ready" } else { "missing" };
            lines.push(Spans::from(Span::styled(
                truncate(
                    &format!("{marker}{arm} {:02} {} · {source}", index + 1, track.label),
                    line_width,
                ),
                Style::default().fg(if index == a.audio_track_selected {
                    if track.resolved {
                        Color::Green
                    } else {
                        Color::Yellow
                    }
                } else if track.armed {
                    Color::Gray
                } else {
                    Color::DarkGray
                }),
            )));
        }
    }
    if let Some(track) = selected {
        let source = if track.preferred_source.is_empty() {
            "unassigned".into()
        } else {
            track.preferred_source.clone()
        };
        let peak = track
            .peak_dbfs
            .map(|db| format!("{db:>5.1} dBFS"))
            .unwrap_or_else(|| "no level".into());
        lines.push(Spans::from(truncate(
            &format!("Source {source}"),
            line_width,
        )));
        lines.push(Spans::from(truncate(
            &format!("Selected meter {peak}"),
            line_width,
        )));
    }
    lines.push(Spans::from(truncate(
        &format!(
            "24-bit mono stems · {:.1} MiB",
            s.bytes as f64 / 1_048_576.0
        ),
        line_width,
    )));
    lines.push(Spans::from(truncate(
        &format!(
            "Drop {} · ovf {} · xrun {} · high {}f",
            s.dropped_frames, s.overflow_events, s.xruns, s.writer_high_water_frames
        ),
        line_width,
    )));
    lines.push(Spans::from(truncate(
        s.error.as_deref().unwrap_or(&path),
        line_width,
    )));
    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(
            if s.error.is_some() || s.dropped_frames > 0 || s.incomplete {
                Color::Yellow
            } else {
                Color::Gray
            },
        )),
        rect(z.x, z.y + 1, z.width, z.height.saturating_sub(4)),
    );
}
fn truncate(s: &str, n: usize) -> String {
    crate::ui_text::fit_line(s, n)
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

const SCREENSHOT_COLS: u16 = 40;
const SCREENSHOT_ROWS: u16 = 13;

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
    for scenario in ScreenshotSpecialScenario::ALL {
        let mut app = screenshot_app(config.clone());
        configure_special_screenshot_scenario(&mut app, scenario);
        frames.push(render_screenshot_frame(
            &mut app,
            format!("menu/{}.png", scenario.slug()),
        )?);
    }
    Ok(serde_json::to_string_pretty(&ScreenshotSet {
        cols: SCREENSHOT_COLS,
        rows: SCREENSHOT_ROWS,
        screens: frames,
    })?)
}

fn render_screenshot_frame(app: &mut App, name: String) -> Result<ScreenshotFrame> {
    let backend = TestBackend::new(SCREENSHOT_COLS, SCREENSHOT_ROWS);
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
    PlaybackNoob,
    RoutingDefaults,
    TrackerLoop,
    TrackerLoopAlign,
    AudioRecorder,
    FxRack,
    FxRackEmpty,
    FxType,
    FxEditor,
    Meter,
    Routing,
}

impl ScreenshotScenario {
    const ALL: [Self; 28] = [
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
        Self::PlaybackNoob,
        Self::RoutingDefaults,
        Self::TrackerLoop,
        Self::TrackerLoopAlign,
        Self::AudioRecorder,
        Self::FxRack,
        Self::FxRackEmpty,
        Self::FxType,
        Self::FxEditor,
        Self::Meter,
        Self::Routing,
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
            Self::PlaybackNoob => Screen::Playback,
            Self::RoutingDefaults => Screen::TrackerFiles,
            Self::TrackerLoop => Screen::TrackerLoop,
            Self::TrackerLoopAlign => Screen::TrackerLoopAlign,
            Self::AudioRecorder => Screen::AudioRecorder,
            Self::FxRack | Self::FxRackEmpty | Self::FxType => Screen::FxRack,
            Self::FxEditor => Screen::FxEditor,
            Self::Meter => Screen::Meter,
            Self::Routing => Screen::Routing,
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
            Self::PlaybackNoob => "playback-noob",
            Self::RoutingDefaults => "routing-defaults",
            Self::TrackerLoop => "ft2-loop",
            Self::TrackerLoopAlign => "loop-align",
            Self::AudioRecorder => "audio-recorder",
            Self::FxRack => "fx-rack",
            Self::FxRackEmpty => "fx-rack-empty",
            Self::FxType => "fx-type",
            Self::FxEditor => "fx-editor",
            Self::Meter => "performance-meter",
            Self::Routing => "routing",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ScreenshotSpecialScenario {
    Home,
    MidiLearn,
    TrackerPageOverlay,
    TrackerPatternOverlay,
    TrackerSongOverlay,
    TrackerRouteOverlay,
    PatternLengthOverlay,
    NoteLengthOverlay,
    EditAddOverlay,
    LoopLibraryOverlay,
    MixEffectsOverlay,
}

impl ScreenshotSpecialScenario {
    const ALL: [Self; 11] = [
        Self::Home,
        Self::MidiLearn,
        Self::TrackerPageOverlay,
        Self::TrackerPatternOverlay,
        Self::TrackerSongOverlay,
        Self::TrackerRouteOverlay,
        Self::PatternLengthOverlay,
        Self::NoteLengthOverlay,
        Self::EditAddOverlay,
        Self::LoopLibraryOverlay,
        Self::MixEffectsOverlay,
    ];

    const fn slug(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::MidiLearn => "midi-learn",
            Self::TrackerPageOverlay => "overlay-ft2-page",
            Self::TrackerPatternOverlay => "overlay-ft2-pattern",
            Self::TrackerSongOverlay => "overlay-ft2-song",
            Self::TrackerRouteOverlay => "overlay-ft2-route",
            Self::PatternLengthOverlay => "overlay-pattern-length",
            Self::NoteLengthOverlay => "overlay-note-length",
            Self::EditAddOverlay => "overlay-edit-add",
            Self::LoopLibraryOverlay => "overlay-loop-library",
            Self::MixEffectsOverlay => "overlay-performance-fx",
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
    config.capture.tracks = (0..18)
        .map(|index| crate::config::CaptureTrackConfig {
            id: format!("input-{}", index + 1),
            label: if index == 4 {
                "Input 5 · Bass mic".into()
            } else {
                format!("Input {}", index + 1)
            },
            group: if index >= 16 {
                "line-stereo".into()
            } else {
                String::new()
            },
            role: match index {
                16 => crate::config::CaptureTrackRole::StereoLeft,
                17 => crate::config::CaptureTrackRole::StereoRight,
                _ => crate::config::CaptureTrackRole::Mono,
            },
            armed: index < 8,
            preferred_source: format!("interface:capture_{}", index + 1),
        })
        .collect();
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
    let available_audio_ports = config.audio_outputs.clone();
    let capture_sources = config
        .capture
        .effective_tracks()
        .into_iter()
        .map(|track| track.preferred_source)
        .filter(|source| !source.is_empty())
        .collect();
    let mut app = App::new(
        &catalogs,
        Arc::new(std::sync::Mutex::new(None)),
        Arc::new(std::sync::Mutex::new(crate::midi::Pickup::default())),
        Arc::new(std::sync::Mutex::new(BackendKind::Synthv1)),
        TrackerIo {
            route: Arc::new(std::sync::Mutex::new(engine::TrackerRoute::default())),
            input: Arc::new(std::sync::Mutex::new(None)),
            playback_scale: Arc::new(std::sync::Mutex::new(None)),
            lifecycle: engine::MidiLifecycle::default(),
        },
        config,
        AvailablePorts {
            playback: available_audio_ports,
            capture_sources,
            midi_outputs: Vec::new(),
        },
        PathBuf::from("/none"),
        PathBuf::from("/none"),
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
            for (note, velocity) in [(62, 100), (66, 92), (69, 104)] {
                app.held_notes.observe(&[0x90, note, velocity]);
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
            app.status = "EDIT on".into();
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
            app.loop_meter.seed_audio_presentation(
                [
                    AudioLevel {
                        rms_dbfs: -18.2,
                        peak_dbfs: -7.1,
                    },
                    AudioLevel {
                        rms_dbfs: -13.4,
                        peak_dbfs: -3.8,
                    },
                ],
                [-5.2, -2.4],
                Instant::now(),
            );
        }
        Screen::AudioRecorder => {
            app.audio_track_selected = 4;
            app.audio_recorder.set_preview_status(RecorderStatus {
                recording: false,
                elapsed: Duration::from_secs(134),
                bytes: 154_200_000,
                sample_rate: 48_000,
                total_frames: 6_432_000,
                dropped_frames: 0,
                active_tracks: 8,
                writer_high_water_frames: 384,
                path: Some(PathBuf::from("recordings/dusk-project-001.take")),
                tracks: (0..18)
                    .map(|index| RecorderTrackStatus {
                        label: if index == 4 {
                            "Input 5 · Bass mic".into()
                        } else {
                            format!("Input {}", index + 1)
                        },
                        armed: index < 8,
                        preferred_source: format!("interface:capture_{}", index + 1),
                        resolved: index != 10,
                        peak_dbfs: (index == 4).then_some(-9.4),
                    })
                    .collect(),
                error: None,
                ..RecorderStatus::default()
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
            app.status = "PLAY paused · encoder moves rows".into();
        }
        ScreenshotScenario::TrackerEdit => {
            fill_demo_song(app);
            app.tracker_mode = TrackerMode::Edit;
            app.tracker_row = 4;
            app.tracker_track = 1;
            app.status = "EDIT · ADD 2 rows after entry".into();
        }
        ScreenshotScenario::TrackerRecord => {
            fill_demo_song(app);
            app.tracker_mode = TrackerMode::Rec;
            app.tracker_row = 7;
            app.tracker_recording = Some(TrackerRecording {
                pattern: 0,
                order: 0,
                page: 0,
                return_to_play: false,
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
        ScreenshotScenario::PlaybackNoob => {
            app.playback_noob = true;
            app.noob_scale = Scale {
                root: 4,
                kind: ScaleKind::NaturalMinor,
            };
            app.sync_playback_noob();
            app.status = "N00B E MINOR · turn rotary to change scale".into();
        }
        ScreenshotScenario::RoutingDefaults => {
            fill_demo_song(app);
            app.confirm_routing_defaults = true;
            app.status = "new-pattern routing changed".into();
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
        ScreenshotScenario::FxRackEmpty => {
            app.screen = Screen::FxRack;
            app.status = "SOURCE rack is empty · ADD inserts an effect".into();
        }
        ScreenshotScenario::FxType => {
            configure_demo_fx(app);
            app.screen = Screen::FxRack;
            app.begin_effect_type_edit();
        }
        ScreenshotScenario::FxEditor => {
            configure_demo_fx(app);
            app.screen = Screen::FxEditor;
            app.fx_selection = app
                .song
                .insert_rack
                .order
                .get(1)
                .copied()
                .map(FxRackSelection::Effect)
                .unwrap_or(FxRackSelection::Insert);
            app.fx_parameter = 2;
            app.status = "COMPRESSOR · ratio selected · graph inactive".into();
        }
        ScreenshotScenario::Routing => {
            app.routing_inputs = vec!["Arturia MiniLab 3".into(), "KeyStep 37".into()];
            app.routing_outputs = vec!["USB MIDI Interface".into(), "Roland D-50".into()];
            app.routing_audio_ports = vec!["system:playback_1".into(), "system:playback_2".into()];
            app.config.midi_performance_input_matches = vec!["KeyStep 37".into()];
            app.config.external_midi.enabled = true;
            app.config.external_midi.output_match = "Roland D-50".into();
            app.config.audio_outputs = vec!["system:playback_1".into(), "system:playback_2".into()];
            if let Ok(mut controller) = app.controller_config.write() {
                controller.input_match = Some("Arturia MiniLab 3".into());
            }
            app.routing.selected = 4;
            app.status = "Routing · browse rows · click edits one draft".into();
        }
    }
    app.select_menu_page(0);
}

fn configure_special_screenshot_scenario(app: &mut App, scenario: ScreenshotSpecialScenario) {
    match scenario {
        ScreenshotSpecialScenario::Home => {
            configure_screenshot(app, Screen::Home);
            app.home_selected = 1;
            app.status = "Master rotary browses · press opens".into();
        }
        ScreenshotSpecialScenario::MidiLearn => {
            configure_screenshot(app, Screen::Home);
            let now = Instant::now();
            let mut session = crate::controller_learn::LearnSession::new_at(
                "Arturia MiniLab 3",
                now - Duration::from_secs(1),
            );
            session.tick(now);
            app.controller_learn = Some(session);
            app.status = "MIDI Learn active · instrument routing isolated".into();
        }
        ScreenshotSpecialScenario::TrackerPageOverlay
        | ScreenshotSpecialScenario::TrackerPatternOverlay
        | ScreenshotSpecialScenario::TrackerSongOverlay
        | ScreenshotSpecialScenario::TrackerRouteOverlay => {
            configure_screenshot_scenario(app, ScreenshotScenario::TrackerPlay);
            let action = match scenario {
                ScreenshotSpecialScenario::TrackerPageOverlay => Action::OpenPageOverlay,
                ScreenshotSpecialScenario::TrackerPatternOverlay => Action::OpenPatternOverlay,
                ScreenshotSpecialScenario::TrackerSongOverlay => Action::OpenSongOverlay,
                ScreenshotSpecialScenario::TrackerRouteOverlay => Action::OpenRouteOverlay,
                _ => unreachable!(),
            };
            app.open_overlay(action);
            if let Some(overlay) = app.overlay.as_mut() {
                overlay.selection = match scenario {
                    ScreenshotSpecialScenario::TrackerPageOverlay => 4,
                    ScreenshotSpecialScenario::TrackerPatternOverlay => 1,
                    ScreenshotSpecialScenario::TrackerSongOverlay => 2,
                    ScreenshotSpecialScenario::TrackerRouteOverlay => 5,
                    _ => 0,
                };
            }
        }
        ScreenshotSpecialScenario::PatternLengthOverlay => {
            configure_screenshot_scenario(app, ScreenshotScenario::PatternSetup);
            app.open_overlay(Action::OpenPatternLengthOverlay);
            if let Some(overlay) = app.overlay.as_mut() {
                overlay.selection = pattern_length_choices()
                    .iter()
                    .position(|rows| *rows == 48)
                    .unwrap_or(0);
            }
        }
        ScreenshotSpecialScenario::NoteLengthOverlay
        | ScreenshotSpecialScenario::EditAddOverlay => {
            configure_screenshot_scenario(app, ScreenshotScenario::TrackerEdit);
            let action = match scenario {
                ScreenshotSpecialScenario::NoteLengthOverlay => Action::OpenNoteLengthOverlay,
                ScreenshotSpecialScenario::EditAddOverlay => Action::OpenTrackerAdvanceOverlay,
                _ => unreachable!(),
            };
            app.open_overlay(action);
            if let Some(overlay) = app.overlay.as_mut() {
                overlay.selection = match scenario {
                    ScreenshotSpecialScenario::NoteLengthOverlay => 3,
                    ScreenshotSpecialScenario::EditAddOverlay => 12,
                    _ => 0,
                };
            }
        }
        ScreenshotSpecialScenario::LoopLibraryOverlay => {
            configure_screenshot_scenario(app, ScreenshotScenario::TrackerLoop);
            app.loop_imports = vec![
                PathBuf::from("/demo/inbox/new-break-104.wav"),
                PathBuf::from("/demo/inbox/tape-drums-92.wav"),
            ];
            app.loop_library = vec![
                crate::loop_player::LibraryEntry {
                    file: "breakbeat-96.wav".into(),
                    current: true,
                    saved_references: 2,
                },
                crate::loop_player::LibraryEntry {
                    file: "room-pulse-120.wav".into(),
                    current: false,
                    saved_references: 0,
                },
                crate::loop_player::LibraryEntry {
                    file: "odd-percussion-135.wav".into(),
                    current: false,
                    saved_references: 1,
                },
            ];
            let launcher = OverlayLauncher::resolve(
                Screen::TrackerLoop,
                MenuContext::Normal,
                Action::OpenLoopLibrary,
            )
            .expect("loop overlay launcher");
            app.overlay = Some(OverlayState::new(
                OverlayKind::LoopLibrary,
                Screen::TrackerLoop,
                launcher,
                2,
                OverlayDraft::None,
                3,
                false,
            ));
        }
        ScreenshotSpecialScenario::MixEffectsOverlay => {
            configure_screenshot_scenario(app, ScreenshotScenario::Meter);
            configure_demo_fx(app);
            app.screen = Screen::Meter;
            app.open_overlay(Action::OpenEffectsOverlay);
            if let Some(overlay) = app.overlay.as_mut() {
                overlay.selection = 3;
            }
        }
    }
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
        app.confirm_effect_type_edit();
    }
    app.fx_selection = app
        .song
        .insert_rack
        .order
        .get(1)
        .copied()
        .map(FxRackSelection::Effect)
        .unwrap_or(FxRackSelection::Insert);
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
    use crate::config::BankSelectMode;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::{Buffer, Cell as BufferCell};
    fn presets() -> Vec<Preset> {
        (0..39)
            .map(|i| Preset::synthv1(format!("Preset {i:02}"), format!("x{i}").into()))
            .collect()
    }
    fn app(presets: &[Preset]) -> App {
        app_with_routing_defaults(presets, PathBuf::from("/none"))
    }
    fn app_with_routing_defaults(presets: &[Preset], defaults: PathBuf) -> App {
        let catalogs = [Catalog {
            backend: BackendKind::Synthv1,
            presets: presets.to_vec(),
            unavailable: None,
        }];
        let config = RuntimeConfig::default();
        let available_midi_outputs = (!config.external_midi.output_match.is_empty())
            .then(|| config.external_midi.output_match.clone())
            .into_iter()
            .collect();
        let available_audio_ports = config.audio_outputs.clone();
        let capture_sources = config
            .capture
            .effective_tracks()
            .into_iter()
            .map(|track| track.preferred_source)
            .filter(|source| !source.is_empty())
            .collect();
        let mut app = App::new(
            &catalogs,
            Arc::new(std::sync::Mutex::new(None)),
            Arc::new(std::sync::Mutex::new(crate::midi::Pickup::default())),
            Arc::new(std::sync::Mutex::new(BackendKind::Synthv1)),
            TrackerIo {
                route: Arc::new(std::sync::Mutex::new(engine::TrackerRoute::default())),
                input: Arc::new(std::sync::Mutex::new(None)),
                playback_scale: Arc::new(std::sync::Mutex::new(None)),
                lifecycle: engine::MidiLifecycle::default(),
            },
            config,
            AvailablePorts {
                playback: available_audio_ports,
                capture_sources,
                midi_outputs: available_midi_outputs,
            },
            PathBuf::from("/none"),
            defaults,
        );
        app.web_help_enabled = false;
        app
    }
    fn learn_send(
        a: &mut App,
        now: &mut Instant,
        message: &[u8],
    ) -> crate::controller_learn::LearnAction {
        *now += Duration::from_millis(1);
        a.receive_controller_learn(*now, message)
    }
    fn learn_settle(a: &mut App, now: &mut Instant) {
        *now += Duration::from_millis(200);
        a.controller_learn.as_mut().unwrap().tick(*now);
    }
    fn learn_master(a: &mut App, rotary: u8, click: u8) -> Instant {
        a.begin_controller_learn();
        let mut now = Instant::now() + Duration::from_secs(1);
        a.controller_learn.as_mut().unwrap().tick(now);
        learn_send(a, &mut now, &[0xb0, rotary, 63]);
        learn_settle(a, &mut now);
        learn_send(a, &mut now, &[0xb0, rotary, 65]);
        learn_settle(a, &mut now);
        learn_send(a, &mut now, &[0xb0, click, 127]);
        learn_send(a, &mut now, &[0xb0, click, 0]);
        now
    }
    fn connect_test_midi_hardware(app: &mut App) {
        app.config.external_midi.enabled = true;
        app.config.external_midi.output_match = "Test MIDI output".into();
        app.available_page_outputs = vec!["Test MIDI output".into()];
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
    fn render_app(app: &mut App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }
    fn buffer_cell(buffer: &Buffer, x: u16, y: u16) -> &BufferCell {
        &buffer.content[usize::from(y * buffer.area.width + x)]
    }
    fn buffer_text(buffer: &Buffer) -> String {
        buffer
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect()
    }
    fn row_text(buffer: &Buffer, row: u16) -> String {
        (0..buffer.area.width)
            .map(|column| buffer_cell(buffer, column, row).symbol.as_str())
            .collect()
    }

    fn inner_text(buffer: &Buffer, area: Rect) -> String {
        let mut text = String::new();
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                text.push_str(buffer_cell(buffer, x, y).symbol.as_str());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn overlay_geometry_border_title_clear_and_selection_render_exactly_at_40x20() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.menu_page_by_screen[Screen::Tracker.index()] = 2;
        a.open_overlay(Action::OpenPageOverlay);
        let buffer = render_app(&mut a, 40, 20);
        let geometry = overlay::geometry(Rect::new(0, 0, 40, 20));
        assert_eq!(geometry.outer, Rect::new(1, 1, 38, 18));
        assert_eq!(geometry.inner, Rect::new(2, 2, 36, 16));
        assert_eq!(buffer_cell(&buffer, 1, 1).symbol, "╔");
        assert_eq!(buffer_cell(&buffer, 38, 1).symbol, "╗");
        assert_eq!(buffer_cell(&buffer, 1, 18).symbol, "╚");
        assert_eq!(buffer_cell(&buffer, 38, 18).symbol, "╝");
        assert!(row_text(&buffer, 1).contains(" PAGE NAVIGATION "));
        assert!(!inner_text(&buffer, geometry.inner).contains("ROW"));
        assert_eq!(buffer_cell(&buffer, 2, 2).bg, Color::Yellow);
        assert_ne!(buffer_cell(&buffer, 0, 1).symbol, "║");
        assert_ne!(buffer_cell(&buffer, 39, 1).symbol, "║");
    }

    #[test]
    fn route_overlay_scrolls_and_truncates_without_damaging_its_border() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        for page in &mut a.song.patterns.get_mut(&0).unwrap().pages {
            page.target = PageTarget::ConfiguredExternal;
        }
        a.open_overlay(Action::OpenRouteOverlay);
        if let Some(route) = a.overlay.as_mut().and_then(OverlayState::route_mut) {
            route.page.target = PageTarget::Midi("LONG CONNECTION NAME ".repeat(8));
        }
        a.overlay.as_mut().unwrap().selection = RouteField::ROWS - 1;
        let buffer = render_app(&mut a, 40, 20);
        let overlay = a.overlay.as_ref().unwrap();
        assert!(overlay.scroll > 0);
        assert!(inner_text(&buffer, Rect::new(2, 2, 36, 16)).contains("APPLY ROUTING"));
        for y in 2..18 {
            assert_eq!(buffer_cell(&buffer, 1, y).symbol, "║");
            assert_eq!(buffer_cell(&buffer, 38, y).symbol, "║");
        }
    }

    #[test]
    fn overlay_launcher_stays_inside_border_and_leaves_last_row_for_status() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.menu_page_by_screen[Screen::Tracker.index()] = 2;
        a.open_overlay(Action::OpenRouteOverlay);
        let buffer = render_app(&mut a, 40, 20);
        let row = row_text(&buffer, 18);
        assert_eq!(row.matches('[').count(), 1);
        assert!(row.contains("ROUTE"));
        assert_eq!(buffer_cell(&buffer, 29, 18).bg, Color::LightYellow);
        assert!(row_text(&buffer, 19).starts_with('‖'));
        assert_eq!(a.hits.actions.len(), 1);
        assert_eq!(a.hits.actions[0].1, Action::OpenRouteOverlay);
    }

    #[test]
    fn forty_by_thirteen_overlay_preserves_all_four_one_cell_reveals() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.open_overlay(Action::OpenRouteOverlay);
        let buffer = render_app(&mut a, 40, 13);
        let geometry = overlay::geometry(Rect::new(0, 0, 40, 13));

        assert_eq!(geometry.outer, Rect::new(1, 1, 38, 11));
        assert_eq!(buffer_cell(&buffer, 1, 1).symbol, "╔");
        assert_eq!(buffer_cell(&buffer, 38, 1).symbol, "╗");
        assert_eq!(buffer_cell(&buffer, 1, 11).symbol, "╚");
        assert_eq!(buffer_cell(&buffer, 38, 11).symbol, "╝");
        assert!(row_text(&buffer, 11).contains("ROUTE"));
        assert!(row_text(&buffer, 12).starts_with('‖'));
    }

    #[test]
    fn master_transport_glyphs_and_colours_are_exact() {
        assert_eq!(
            transport_glyph(TransportIndicator::Play),
            (">", Color::Green)
        );
        assert_eq!(
            transport_glyph(TransportIndicator::Stop),
            ("■", Color::White)
        );
        assert_eq!(
            transport_glyph(TransportIndicator::Pause),
            ("‖", Color::White)
        );
        assert_eq!(
            transport_glyph(TransportIndicator::Record),
            ("●", Color::Red)
        );
        for state in [
            TransportIndicator::Play,
            TransportIndicator::Stop,
            TransportIndicator::Pause,
            TransportIndicator::Record,
        ] {
            assert_eq!(crate::ui_text::width(transport_glyph(state).0), 1);
        }
        assert_eq!(
            transport_color(TransportIndicator::Record, Duration::ZERO),
            Color::LightRed
        );
        assert_eq!(
            transport_color(TransportIndicator::Record, Duration::from_millis(400)),
            Color::Red
        );
        assert_eq!(
            transport_color(TransportIndicator::Record, Duration::from_millis(800)),
            Color::LightRed
        );
    }

    #[test]
    fn overlay_launcher_toggle_is_exclusive_and_restores_controller_state() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.menu_page_by_screen[Screen::Tracker.index()] = 1;
        a.page_select_mode = true;
        a.open_overlay(Action::OpenRouteOverlay);
        let (tx, _rx) = mpsc::channel();
        dispatch_pad(
            crate::pads::PadAction::Item1,
            true,
            &mut a,
            Path::new("/none"),
            &tx,
        );
        assert!(a.overlay.is_some(), "hidden item must be silent");
        dispatch_pad(
            crate::pads::PadAction::Item4,
            true,
            &mut a,
            Path::new("/none"),
            &tx,
        );
        assert!(a.overlay.is_none());
        assert_eq!(a.menu_page(), 1);
        assert!(a.page_select_mode);
    }

    #[test]
    fn overlay_open_close_preserves_caller_owners_and_project_state() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.engine_owner = Some(EngineOwner::Tracker(SoftwareRoute::synthv1("Preset 00")));
        a.tracker_mode = TrackerMode::Play;
        a.tracker_row = 6;
        a.tracker_advance = 8;
        let song = a.song.clone();
        let owner = a.engine_owner.clone();
        let transport = a.sequencer.status();
        let recorder = a.audio_recorder.status();
        a.open_overlay(Action::OpenPatternOverlay);
        a.move_overlay(1);
        a.close_overlay(true);
        assert_eq!(a.song, song);
        assert_eq!(a.engine_owner, owner);
        assert_eq!(a.tracker_row, 6);
        assert_eq!(a.tracker_advance, 8);
        assert_eq!(a.sequencer.status().playing, transport.playing);
        assert_eq!(a.audio_recorder.status().recording, recorder.recording);
    }

    #[test]
    fn overlay_keyboard_and_rotary_match_and_block_the_caller() {
        let p = presets();
        let (tx, _rx) = mpsc::channel();
        let mut rotary = app(&p);
        rotary.screen = Screen::Tracker;
        rotary.tracker_row = 7;
        rotary.open_overlay(Action::OpenPageOverlay);
        dispatch_encoder(
            crate::pads::EncoderAction::Down,
            &mut rotary,
            Path::new("/none"),
            &tx,
        );
        let mut keyboard = app(&p);
        keyboard.screen = Screen::Tracker;
        keyboard.tracker_row = 7;
        keyboard.open_overlay(Action::OpenPageOverlay);
        assert!(!key(KeyCode::Down, &mut keyboard, Path::new("/none"), &tx));
        assert_eq!(
            rotary.overlay.as_ref().unwrap().selection,
            keyboard.overlay.as_ref().unwrap().selection
        );
        assert_eq!(keyboard.tracker_row, 7, "covered caller must not move");
        key(KeyCode::Enter, &mut keyboard, Path::new("/none"), &tx);
        assert!(keyboard.overlay.is_none());
        assert_eq!(keyboard.tracker_track, 1);
    }

    #[test]
    fn overlay_mouse_and_back_are_confined_before_caller_navigation() {
        let p = presets();
        let (tx, _rx) = mpsc::channel();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_row = 9;
        a.tracker_advance = 4;
        a.open_overlay(Action::OpenPageOverlay);
        render_app(&mut a, 40, 20);
        let left = |column, row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        mouse(left(0, 8), &mut a, Path::new("/none"), &tx);
        assert_eq!(a.overlay.as_ref().unwrap().selection, 0);
        assert_eq!(a.tracker_row, 9);
        mouse(left(3, 3), &mut a, Path::new("/none"), &tx);
        assert_eq!(a.overlay.as_ref().unwrap().selection, 1);
        assert_eq!(a.screen, Screen::Tracker);
        perform(Action::Back, &mut a, Path::new("/none"), None);
        assert!(a.overlay.is_none());
        assert_eq!(a.screen, Screen::Tracker);
        assert_eq!(a.tracker_mode, TrackerMode::Play);
        assert_eq!(a.tracker_advance, 4);
        assert_eq!(a.tracker_row, 9);
        perform(Action::Back, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Home);
    }

    #[test]
    fn route_back_cancels_a_field_before_it_cancels_the_overlay() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        for page in &mut a.song.patterns.get_mut(&0).unwrap().pages {
            page.target = PageTarget::ConfiguredExternal;
        }
        a.open_overlay(Action::OpenRouteOverlay);
        a.overlay.as_mut().unwrap().selection = 5;
        a.activate_overlay();
        a.move_overlay(1);
        assert_eq!(
            a.overlay.as_ref().unwrap().route().unwrap().page.columns[0].channel,
            1
        );
        a.overlay_back();
        assert!(a.overlay.is_some());
        assert_eq!(a.overlay.as_ref().unwrap().active_field, None);
        assert_eq!(
            a.overlay.as_ref().unwrap().route().unwrap().page.columns[0].channel,
            0
        );
        a.overlay_back();
        assert!(a.overlay.is_none());
    }

    #[test]
    fn every_controller_layout_keeps_master_navigation_and_launcher_close() {
        let p = presets();
        let (tx, _rx) = mpsc::channel();
        for layout in [
            ControllerLayout::Eight,
            ControllerLayout::Five,
            ControllerLayout::Four,
        ] {
            let mut a = app(&p);
            a.screen = Screen::Tracker;
            a.controller_layout = layout;
            a.menu_page_by_screen[Screen::Tracker.index()] = 2;
            a.open_overlay(Action::OpenPageOverlay);
            dispatch_encoder(
                crate::pads::EncoderAction::Down,
                &mut a,
                Path::new("/none"),
                &tx,
            );
            assert_eq!(a.overlay.as_ref().unwrap().selection, 1, "{layout:?}");
            dispatch_pad(
                crate::pads::PadAction::Item1,
                true,
                &mut a,
                Path::new("/none"),
                &tx,
            );
            assert!(a.overlay.is_none(), "{layout:?}");
            assert_eq!(a.menu_page(), 2, "{layout:?}");
            assert!(!a.page_select_mode, "{layout:?}");
        }
    }

    #[test]
    fn framework_supports_distinct_ft2_overlays_and_a_second_caller() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        for (action, kind) in [
            (Action::OpenPageOverlay, OverlayKind::TrackerPage),
            (Action::OpenPatternOverlay, OverlayKind::TrackerPattern),
            (Action::OpenSongOverlay, OverlayKind::TrackerSong),
            (Action::OpenRouteOverlay, OverlayKind::TrackerRoute),
        ] {
            a.open_overlay(action);
            assert_eq!(a.overlay.as_ref().unwrap().kind, kind);
            assert_eq!(a.overlay.as_ref().unwrap().caller, Screen::Tracker);
            a.close_overlay(true);
        }
        a.open_overlay(Action::OpenPageOverlay);
        perform(Action::OpenPatternOverlay, &mut a, Path::new("/none"), None);
        assert_eq!(
            a.overlay.as_ref().unwrap().kind,
            OverlayKind::TrackerPage,
            "another launcher stays silent until the first overlay closes"
        );
        a.close_overlay(true);
        a.screen = Screen::Meter;
        a.menu_page_by_screen[Screen::Meter.index()] = 2;
        a.open_overlay(Action::OpenEffectsOverlay);
        assert_eq!(a.overlay.as_ref().unwrap().kind, OverlayKind::MixEffects);
        assert_eq!(a.overlay.as_ref().unwrap().caller, Screen::Meter);
        a.overlay.as_mut().unwrap().selection = 1;
        a.activate_overlay();
        assert_eq!(a.screen, Screen::FxRack);
        assert_eq!(a.fx_target, 1);
        assert_eq!(a.fx_rack_parent, Screen::Meter);
    }

    #[test]
    fn ft2_selector_buttons_open_their_exact_rotary_overlays() {
        use crate::pads::PadAction;

        let p = presets();
        let (tx, _rx) = mpsc::channel();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);

        dispatch_pad(PadAction::Page3, true, &mut a, Path::new("/none"), &tx);
        dispatch_pad(PadAction::Item3, true, &mut a, Path::new("/none"), &tx);
        assert_eq!(
            a.overlay.as_ref().map(|overlay| overlay.kind),
            Some(OverlayKind::TrackerNoteLength)
        );
        assert_eq!(a.screen, Screen::Tracker, "LENGTH must remain over FT2");
        a.close_overlay(true);

        dispatch_pad(PadAction::Page2, true, &mut a, Path::new("/none"), &tx);
        dispatch_pad(PadAction::Item4, true, &mut a, Path::new("/none"), &tx);
        assert_eq!(
            a.overlay.as_ref().map(|overlay| overlay.kind),
            Some(OverlayKind::TrackerAdvance)
        );
        a.overlay.as_mut().unwrap().selection = 0;
        a.activate_overlay();
        assert_eq!(a.tracker_advance, 0);

        a.screen = Screen::TrackerFiles;
        a.choose_pattern_clear();
        dispatch_pad(PadAction::Item3, true, &mut a, Path::new("/none"), &tx);
        assert_eq!(
            a.overlay.as_ref().map(|overlay| overlay.kind),
            Some(OverlayKind::TrackerPatternLength)
        );
    }

    #[test]
    fn ft2_page_overlay_exposes_the_unloaded_loop_after_the_midi_pages() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;

        a.open_overlay(Action::OpenPageOverlay);
        let overlay = a.overlay.as_ref().unwrap();
        let rows = overlay_rows(&a, overlay);
        assert_eq!(rows.len(), 3 * LANES_PER_PAGE + 2);
        assert_eq!(rows[3 * LANES_PER_PAGE], "P04 LOOP PLAYER · UNLOADED");

        a.overlay.as_mut().unwrap().selection = 3 * LANES_PER_PAGE;
        a.activate_overlay();
        assert!(a.overlay.is_none());
        assert_eq!(a.screen, Screen::TrackerLoop);
        assert!(a.song.audio_loop.is_none());
        assert!(a.status.contains("loop page unloaded"));
    }

    #[test]
    fn ft2_page_pattern_and_song_overlays_select_through_existing_owners() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        fill_demo_song(&mut a);

        a.open_overlay(Action::OpenPageOverlay);
        a.overlay.as_mut().unwrap().selection = LANES_PER_PAGE + 1;
        a.activate_overlay();
        assert_eq!((a.tracker_page, a.tracker_track), (1, 1));
        assert_eq!(a.song.order, vec![0, 1, 0, 2]);

        a.open_overlay(Action::OpenPatternOverlay);
        a.overlay.as_mut().unwrap().selection = 1;
        a.activate_overlay();
        assert_eq!(a.tracker_pattern_number(), 1);
        assert_eq!(a.tracker_order, 1);

        a.open_overlay(Action::OpenSongOverlay);
        a.overlay.as_mut().unwrap().selection = 3;
        a.activate_overlay();
        assert_eq!(a.tracker_order, 3);
        assert_eq!(a.tracker_pattern_number(), 2);
        assert_eq!(a.tracker_page, 0, "existing cursor clamp owns the return");
    }

    #[test]
    fn route_overlay_is_passive_transactional_and_uses_the_existing_owner() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        for page in &mut a.song.patterns.get_mut(&0).unwrap().pages {
            page.target = PageTarget::ConfiguredExternal;
        }
        let original = a.song.clone();
        let release_revision = a.audition_release_revision;
        let destinations = a.tracker_route.lock().unwrap().destinations();
        a.open_overlay(Action::OpenRouteOverlay);
        assert_eq!(a.song, original);
        assert_eq!(a.audition_release_revision, release_revision);
        assert_eq!(a.tracker_route.lock().unwrap().destinations(), destinations);
        assert!(a.engine.is_none());
        a.overlay.as_mut().unwrap().selection = 5;
        a.activate_overlay();
        a.move_overlay(1);
        a.activate_overlay();
        assert_eq!(a.song, original, "confirmed field is still only a draft");
        a.overlay.as_mut().unwrap().selection = RouteField::ROWS - 1;
        a.activate_overlay();
        assert!(a.overlay.is_none());
        assert_eq!(a.current_page().unwrap().columns[0].channel, 1);
        assert_eq!(a.audition_release_revision, release_revision + 1);
        assert!(a.engine.is_none());
    }

    #[test]
    fn route_launcher_cancel_preserves_unavailable_preference_exactly() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.available_page_outputs.clear();
        let preferred = PageTarget::Midi("Borrowed hardware currently absent".into());
        a.song.patterns.get_mut(&0).unwrap().pages[0].target = preferred.clone();
        let original = a.song.clone();
        a.open_overlay(Action::OpenRouteOverlay);
        perform(Action::OpenRouteOverlay, &mut a, Path::new("/none"), None);
        assert_eq!(a.song, original);
        assert_eq!(a.current_page().unwrap().target, preferred);
    }

    #[test]
    fn raw_device_profile_sentinel_remains_reachable() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.open_overlay(Action::OpenRouteOverlay);
        a.overlay.as_mut().unwrap().selection = 4;
        a.activate_overlay();
        a.move_overlay(-1);
        assert!(a
            .overlay
            .as_ref()
            .unwrap()
            .route()
            .unwrap()
            .page
            .device_profile
            .is_some());
        a.move_overlay(1);
        assert_eq!(
            a.overlay
                .as_ref()
                .unwrap()
                .route()
                .unwrap()
                .page
                .device_profile,
            None
        );
    }

    #[test]
    fn route_overlay_selects_engine_before_instrument_and_ignores_current_engine() {
        let p = presets();
        let mut a = app(&p);
        a.catalogs.push(Catalog {
            backend: BackendKind::Yoshimi,
            presets: vec![
                Preset {
                    backend: BackendKind::Yoshimi,
                    name: "Soft Pad".into(),
                    category: Some("Pads".into()),
                    id: crate::preset::PresetId::Yoshimi {
                        path: PathBuf::from("/catalog/pads/soft-pad.xiz"),
                    },
                },
                Preset {
                    backend: BackendKind::Yoshimi,
                    name: "Wide Pad".into(),
                    category: Some("Pads".into()),
                    id: crate::preset::PresetId::Yoshimi {
                        path: PathBuf::from("/catalog/pads/wide-pad.xiz"),
                    },
                },
            ],
            unavailable: None,
        });
        a.backend_index = 0;
        a.selected = 17;
        a.screen = Screen::Tracker;
        a.open_overlay(Action::OpenRouteOverlay);

        a.overlay.as_mut().unwrap().selection = 1;
        a.activate_overlay();
        a.move_overlay(1);
        a.activate_overlay();
        let route = &a.overlay.as_ref().unwrap().route().unwrap().page.target;
        assert_eq!(
            route,
            &PageTarget::Software(SoftwareRoute {
                engine: BackendKind::Yoshimi,
                instrument: "Pads/Soft Pad".into(),
            })
        );

        a.overlay.as_mut().unwrap().selection = 2;
        a.activate_overlay();
        a.move_overlay(1);
        a.activate_overlay();
        let route = &a.overlay.as_ref().unwrap().route().unwrap().page.target;
        assert_eq!(
            route,
            &PageTarget::Software(SoftwareRoute {
                engine: BackendKind::Yoshimi,
                instrument: "Pads/Wide Pad".into(),
            })
        );
        assert_eq!(a.selected_backend(), BackendKind::Synthv1);
        assert_eq!(a.selected, 17);
    }

    #[test]
    fn route_target_wraps_and_channel_keyboard_matches_master_rotary() {
        let p = presets();
        let (tx, _rx) = mpsc::channel();
        let mut rotary = app(&p);
        rotary.screen = Screen::Tracker;
        rotary.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        rotary.open_overlay(Action::OpenRouteOverlay);
        rotary.overlay.as_mut().unwrap().selection = 0;
        rotary.activate_overlay();
        rotary.move_overlay(1);
        assert_eq!(
            rotary
                .overlay
                .as_ref()
                .unwrap()
                .route()
                .unwrap()
                .page
                .target,
            PageTarget::Default,
            "last target wraps to first"
        );
        rotary.overlay_back();
        rotary.overlay.as_mut().unwrap().selection = 5;
        rotary.activate_overlay();
        dispatch_encoder(
            crate::pads::EncoderAction::Down,
            &mut rotary,
            Path::new("/none"),
            &tx,
        );

        let mut keyboard = app(&p);
        keyboard.screen = Screen::Tracker;
        keyboard.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        keyboard.open_overlay(Action::OpenRouteOverlay);
        keyboard.overlay.as_mut().unwrap().selection = 5;
        keyboard.activate_overlay();
        assert!(!key(KeyCode::Down, &mut keyboard, Path::new("/none"), &tx));
        assert_eq!(
            rotary
                .overlay
                .as_ref()
                .unwrap()
                .route()
                .unwrap()
                .page
                .columns[0]
                .channel,
            keyboard
                .overlay
                .as_ref()
                .unwrap()
                .route()
                .unwrap()
                .page
                .columns[0]
                .channel
        );
        assert_eq!(
            keyboard
                .overlay
                .as_ref()
                .unwrap()
                .route()
                .unwrap()
                .page
                .columns[0]
                .channel,
            1
        );

        keyboard
            .overlay
            .as_mut()
            .unwrap()
            .route_mut()
            .unwrap()
            .page
            .columns[0]
            .channel = 15;
        keyboard.move_overlay(1);
        assert_eq!(
            keyboard
                .overlay
                .as_ref()
                .unwrap()
                .route()
                .unwrap()
                .page
                .columns[0]
                .channel,
            15,
            "numeric editing keeps its MIDI limit instead of list wrapping"
        );
    }

    #[test]
    fn staged_route_choosers_fit_the_40x20_tracker_contract() {
        let p = presets();
        let mut app = app(&p);
        app.screen = Screen::Tracker;
        app.open_page_manager();
        app.edit_page_target();
        app.confirm_page_field();
        assert_eq!(app.page_manager_mode, PageManagerMode::Engine);
        let engine = buffer_text(&render_app(&mut app, 40, 20));
        assert!(engine.contains("SOFTWARE ENGINE"));

        app.confirm_page_field();
        assert_eq!(app.page_manager_mode, PageManagerMode::Instrument);
        let instrument = buffer_text(&render_app(&mut app, 40, 20));
        assert!(instrument.contains("ENGINE INSTRUMENT"));
        app.confirm_page_field();

        app.edit_page_target();
        app.page_target_selected = app.page_target_candidates.len() - 1;
        app.confirm_page_field();
        assert_eq!(app.page_manager_mode, PageManagerMode::MidiOutput);
        let output = buffer_text(&render_app(&mut app, 40, 20));
        assert!(output.contains("MIDI OUTPUT"));
    }
    #[test]
    fn renders_40x13_all_screens() {
        render(40, 13, Screen::Home);
        render(40, 13, Screen::Presets);
        render(40, 13, Screen::Playback);
        render(40, 13, Screen::Ideas);
        render(40, 13, Screen::Help);
        render(40, 13, Screen::Tracker);
        render(40, 13, Screen::TrackerFiles);
        render(40, 13, Screen::TrackerArrange);
        render(40, 13, Screen::TrackerPages);
        render(40, 13, Screen::TrackerTools);
        render(40, 13, Screen::TrackerLoop);
        render(40, 13, Screen::TrackerLoopAlign);
        render(40, 13, Screen::AudioRecorder);
        render(40, 13, Screen::FxRack);
        render(40, 13, Screen::FxEditor);
        render(40, 13, Screen::Meter);
        render(40, 13, Screen::Routing);
    }

    #[test]
    fn every_working_screen_owns_the_final_status_row() {
        let p = presets();
        for screen in Screen::ALL
            .into_iter()
            .filter(|screen| *screen != Screen::Home)
        {
            let mut a = app(&p);
            a.screen = screen;
            let buffer = render_app(&mut a, 40, 13);
            let expected = if screen == Screen::Tracker {
                '‖'
            } else {
                '■'
            };
            assert!(
                row_text(&buffer, 12).starts_with(expected),
                "{} did not preserve the shared status row",
                screen.label()
            );
        }
    }

    #[test]
    fn home_is_initial_minimal_and_uses_an_inverted_selection() {
        let p = presets();
        let mut a = app(&p);
        assert_eq!(a.screen, Screen::Home);

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        assert!(text.contains("SOFTWARE SYNTHS"));
        assert!(text.contains("FT2 TRACKER"));
        assert!(text.contains("RECORDER"));
        assert!(text.contains("ROUTING"));
        assert!(text.contains("MIDI LEARN"));
        assert!(!text.contains("SHR-DAW · HOME"));
        assert!(!text.contains("controller menu below"));
        assert!(row_text(buffer, 19).contains("rotary browse"));
        assert!(!row_text(buffer, 18).contains("rotary browse"));
        assert!(buffer.content.iter().any(|cell| {
            cell.fg == Color::Black
                && cell.bg == Color::White
                && cell.modifier.contains(Modifier::BOLD)
        }));
    }

    #[test]
    fn home_rows_are_equal_width_and_centered_at_40x20() {
        let p = presets();
        let mut app = app(&p);
        let buffer = render_app(&mut app, 40, 20);

        assert_eq!(app.hits.list, Rect::new(2, 5, 36, 9));
        for (index, entry) in HOME_ENTRIES.iter().enumerate() {
            let row = app.hits.list.y + index as u16;
            let text = row_text(&buffer, row);
            let label_x = 2 + (36usize - entry.label.len()) / 2;
            assert_eq!(
                &text[label_x..label_x + entry.label.len()],
                entry.label,
                "{} is not centered",
                entry.label
            );
            for column in 2..38 {
                let cell = buffer_cell(&buffer, column, row);
                if index == app.home_selected {
                    assert_eq!((cell.fg, cell.bg), (Color::Black, Color::White));
                    assert!(cell.modifier.contains(Modifier::BOLD));
                } else {
                    assert_eq!((cell.fg, cell.bg), (Color::Gray, Color::Black));
                    assert!(!cell.modifier.contains(Modifier::BOLD));
                }
            }
            assert_ne!(buffer_cell(&buffer, 1, row).bg, Color::White);
            assert_ne!(buffer_cell(&buffer, 38, row).bg, Color::White);
        }
    }

    #[test]
    fn home_selection_colors_are_stable_when_focus_moves() {
        let p = presets();
        let mut app = app(&p);
        let first = render_app(&mut app, 40, 20);
        let first_row = app.hits.list.y;
        app.move_home(1);
        let second = render_app(&mut app, 40, 20);
        let second_row = app.hits.list.y + 1;

        for column in 2..38 {
            assert_eq!(buffer_cell(&first, column, first_row).bg, Color::White);
            assert_eq!(buffer_cell(&second, column, first_row).fg, Color::Gray);
            assert_eq!(buffer_cell(&second, column, first_row).bg, Color::Black);
            assert_eq!(buffer_cell(&second, column, second_row).fg, Color::Black);
            assert_eq!(buffer_cell(&second, column, second_row).bg, Color::White);
        }
    }

    #[test]
    fn home_fits_without_truncation_or_overflow_at_compact_size() {
        let p = presets();
        let mut app = app(&p);
        for (selected, entry) in HOME_ENTRIES.iter().enumerate() {
            app.home_selected = selected;
            let buffer = render_app(&mut app, 38, 10);
            assert_eq!((app.hits.list.x, app.hits.list.width), (2, 34));
            assert_eq!(app.hits.list.height, HOME_ENTRIES.len() as u16);
            assert_eq!(app.home_offset, 0);
            let selected_row = (app.hits.list.y..app.hits.list.y + app.hits.list.height)
                .find(|row| buffer_cell(&buffer, 2, *row).bg == Color::White)
                .expect("selected Home row remains visible");
            assert!(row_text(&buffer, selected_row).contains(entry.label));
            for column in 2..36 {
                assert_eq!(buffer_cell(&buffer, column, selected_row).bg, Color::White);
            }
            assert_ne!(buffer_cell(&buffer, 1, selected_row).bg, Color::White);
            assert_ne!(buffer_cell(&buffer, 36, selected_row).bg, Color::White);
        }
    }

    #[test]
    fn home_setup_destinations_are_distinct_direct_actions() {
        let actions = HOME_ENTRIES
            .iter()
            .map(|entry| (entry.label, entry.action))
            .collect::<HashMap<_, _>>();
        assert_eq!(
            actions.get("MIDI LEARN"),
            Some(&Action::OpenControllerLearn)
        );
        assert_eq!(actions.get("ROUTING"), Some(&Action::OpenRouting));
        assert_eq!(actions.get("EFFECTS"), Some(&Action::OpenFxRack));

        let p = presets();
        let mut app = app(&p);
        perform(Action::OpenRouting, &mut app, Path::new("/none"), None);
        assert_eq!(app.screen, Screen::Routing);
        perform(Action::Back, &mut app, Path::new("/none"), None);
        perform(Action::OpenFxRack, &mut app, Path::new("/none"), None);
        assert_eq!(app.screen, Screen::FxRack);
        assert_eq!(app.fx_rack_parent, Screen::Home);
    }

    #[test]
    fn home_recommends_midi_learn_only_for_controller_failures() {
        let p = presets();
        let learn_index = HOME_ENTRIES
            .iter()
            .position(|entry| entry.action == Action::OpenControllerLearn)
            .unwrap();
        let mut app = app(&p);
        *app.controller_config.write().unwrap() =
            crate::pads::PadConfig::unmapped("Unknown Controller MIDI");

        app.controller_online = false;
        assert_eq!(
            app.controller_learn_reason(),
            Some(ControllerLearnReason::Offline)
        );
        app.recommend_controller_learn_on_home();
        assert_eq!(app.home_selected, learn_index);
        let offline = render_app(&mut app, 40, 20);
        assert!(buffer_text(&offline).contains("Configured controller is offline"));

        app.controller_online = true;
        assert_eq!(
            app.controller_learn_reason(),
            Some(ControllerLearnReason::NoReviewedProfile)
        );

        let mut learned = crate::pads::PadConfig::unmapped("Unknown Controller MIDI");
        learned.profile = Some("learned".into());
        learned.encoder_relative_cc = Some(28);
        *app.controller_config.write().unwrap() = learned.clone();
        assert_eq!(
            app.controller_learn_reason(),
            Some(ControllerLearnReason::IncompleteLearnedEncoder)
        );

        learned.encoder_press_cc = Some(118);
        *app.controller_config.write().unwrap() = learned;
        assert_eq!(app.controller_learn_reason(), None);
        assert!(app.controller_config.read().unwrap().pads.is_empty());
        assert!(app.controller_config.read().unwrap().cc_buttons.is_empty());

        let profile = app
            .controller_profiles
            .matching("Arturia MiniLab3 MIDI")
            .unwrap();
        let mut reviewed = crate::pads::PadConfig::unmapped("Arturia MiniLab3 MIDI");
        profile
            .apply(&mut reviewed, "Arturia MiniLab3 MIDI")
            .unwrap();
        *app.controller_config.write().unwrap() = reviewed;
        assert_eq!(app.controller_learn_reason(), None);
    }

    #[test]
    fn keyboard_navigation_remains_available_with_controller_offline() {
        let p = presets();
        let mut app = app(&p);
        let (tx, _rx) = mpsc::channel();
        *app.controller_config.write().unwrap() =
            crate::pads::PadConfig::unmapped("Offline Controller");
        app.controller_online = false;
        app.recommend_controller_learn_on_home();
        let start = app.home_selected;

        key(KeyCode::Down, &mut app, Path::new("/none"), &tx);
        assert_eq!(app.home_selected, start + 1);
        key(KeyCode::Up, &mut app, Path::new("/none"), &tx);
        assert_eq!(app.home_selected, start);
        app.home_selected = HOME_ENTRIES
            .iter()
            .position(|entry| entry.action == Action::OpenRouting)
            .unwrap();
        key(KeyCode::Enter, &mut app, Path::new("/none"), &tx);
        assert_eq!(app.screen, Screen::Routing);
    }

    #[test]
    fn home_keyboard_and_rotary_select_and_open_workspaces() {
        let p = presets();
        let (tx, _rx) = mpsc::channel();
        let mut keyboard = app(&p);
        key(KeyCode::Down, &mut keyboard, Path::new("/none"), &tx);
        assert_eq!(keyboard.home_selected, 1);
        key(KeyCode::Enter, &mut keyboard, Path::new("/none"), &tx);
        assert_eq!(keyboard.screen, Screen::Tracker);

        let mut rotary = app(&p);
        rotary.controller_layout = ControllerLayout::Four;
        dispatch_encoder(
            crate::pads::EncoderAction::Down,
            &mut rotary,
            Path::new("/none"),
            &tx,
        );
        dispatch_encoder(
            crate::pads::EncoderAction::Down,
            &mut rotary,
            Path::new("/none"),
            &tx,
        );
        assert_eq!(rotary.home_selected, 2);
        dispatch_encoder(
            crate::pads::EncoderAction::Select,
            &mut rotary,
            Path::new("/none"),
            &tx,
        );
        assert_eq!(rotary.screen, Screen::AudioRecorder);
        assert!(!rotary.page_select_mode);
    }

    #[test]
    fn home_opens_encoder_first_midi_learn_directly() {
        let p = presets();
        let mut a = app(&p);
        let (tx, _rx) = mpsc::channel();
        a.controller_online = true;
        *a.controller_config.write().unwrap() =
            crate::pads::PadConfig::unmapped("Test Controller MIDI");
        a.home_selected = HOME_ENTRIES
            .iter()
            .position(|entry| entry.action == Action::OpenControllerLearn)
            .unwrap();

        key(KeyCode::Enter, &mut a, Path::new("/none"), &tx);

        assert!(a.controller_learn.is_some());
        assert!(a.learn_mode.load(Ordering::Relaxed));
        assert_eq!(
            a.controller_learn.as_ref().unwrap().role(),
            crate::controller_learn::LearnRole::EncoderCounterClockwise
        );
    }

    #[test]
    fn top_level_exit_returns_home_and_nested_exit_returns_one_level() {
        let p = presets();
        for screen in [
            Screen::Presets,
            Screen::Ideas,
            Screen::Tracker,
            Screen::AudioRecorder,
            Screen::Meter,
            Screen::Routing,
            Screen::FxRack,
        ] {
            let mut a = app(&p);
            a.screen = screen;
            a.fx_rack_parent = Screen::Home;
            assert!(!perform(Action::Back, &mut a, Path::new("/none"), None));
            assert_eq!(a.screen, Screen::Home, "{screen:?}");
        }

        let mut a = app(&p);
        a.screen = Screen::Playback;
        perform(Action::Back, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Presets);
        perform(Action::Back, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Home);

        a.screen = Screen::TrackerFiles;
        perform(Action::Back, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Tracker);
        a.screen = Screen::TrackerLoopAlign;
        perform(Action::Back, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::TrackerLoop);
    }

    #[test]
    fn home_back_paths_keep_standalone_sound_and_release_other_workspace_state() {
        let p = presets();

        let mut synth = app(&p);
        synth.screen = Screen::Presets;
        synth.engine_owner = Some(EngineOwner::SoftwareSynth);
        perform(Action::Back, &mut synth, Path::new("/none"), None);
        assert_eq!(synth.screen, Screen::Home);
        assert_eq!(synth.engine_owner, Some(EngineOwner::SoftwareSynth));

        let mut tracker = app(&p);
        tracker.screen = Screen::Tracker;
        tracker.engine_owner = Some(EngineOwner::Tracker(SoftwareRoute::synthv1(
            "Pattern Sound",
        )));
        perform(Action::Back, &mut tracker, Path::new("/none"), None);
        assert_eq!(tracker.screen, Screen::Home);
        assert_eq!(tracker.engine_owner, None);
        assert!(!tracker.tracker_route.lock().unwrap().preview_state().0);

        let mut effects = app(&p);
        perform(Action::OpenFxRack, &mut effects, Path::new("/none"), None);
        effects.add_effect();
        effects.confirm_effect_type_edit();
        perform(Action::OpenFxEditor, &mut effects, Path::new("/none"), None);
        effects.begin_fx_value_edit();
        assert!(effects.fx_edit_original.is_some());
        perform(Action::Back, &mut effects, Path::new("/none"), None);
        assert_eq!(effects.screen, Screen::FxEditor);
        assert!(effects.fx_edit_original.is_none());
        perform(Action::Back, &mut effects, Path::new("/none"), None);
        assert_eq!(effects.screen, Screen::FxRack);
        perform(Action::Back, &mut effects, Path::new("/none"), None);
        assert_eq!(effects.screen, Screen::Home);
    }

    #[test]
    fn keyboard_page_keys_remain_available_without_pad_commands() {
        let p = presets();
        let mut a = app(&p);
        let (tx, _rx) = mpsc::channel();
        a.screen = Screen::Presets;
        a.selected = 20;
        key(KeyCode::PageUp, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.selected, 10);
        key(KeyCode::PageDown, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.selected, 20);

        fill_demo_song(&mut a);
        a.screen = Screen::Tracker;
        a.tracker_order = 2;
        key(KeyCode::PageUp, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.tracker_order, 1);
        key(KeyCode::PageDown, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.tracker_order, 2);
    }

    #[test]
    fn tracker_page_navigation_preserves_column_and_row() {
        let p = presets();
        let mut a = app(&p);
        fill_demo_song(&mut a);
        a.screen = Screen::Tracker;
        a.tracker_track = 3;
        a.tracker_row = 7;

        perform(Action::NextTrackerPage, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_page, 1);
        assert_eq!(a.tracker_track, 3);
        assert_eq!(a.tracker_row, 7);

        a.open_overlay(Action::OpenPageOverlay);
        a.overlay.as_mut().unwrap().selection = 3;
        a.activate_overlay();
        assert_eq!(a.tracker_page, 0);
        assert_eq!(a.tracker_track, 3);
        assert_eq!(a.tracker_row, 7);
    }

    #[test]
    fn letter_jump_is_case_insensitive_for_sound_and_file_lists() {
        let p = vec![
            Preset::synthv1("Amber", "amber".into()),
            Preset::synthv1("Cobalt", "cobalt".into()),
            Preset::synthv1("Cedar", "cedar".into()),
        ];
        assert_eq!(first_letter_index(["amber", "Cobalt"], 'c'), Some(1));
        let mut a = app(&p);
        let (tx, _rx) = mpsc::channel();
        a.screen = Screen::Presets;
        key(KeyCode::Char('C'), &mut a, Path::new("/none"), &tx);
        assert_eq!(a.selected, 1);

        a.screen = Screen::TrackerFiles;
        a.song_list = vec!["alpha".into(), "Cedar".into(), "delta".into()];
        a.song_selected = 0;
        key(KeyCode::Char('D'), &mut a, Path::new("/none"), &tx);
        assert_eq!(a.song_selected, 2);

        a.screen = Screen::TrackerLoop;
        a.loop_library_mode = true;
        a.loop_library = vec![
            crate::loop_player::LibraryEntry {
                file: "break.wav".into(),
                current: false,
                saved_references: 0,
            },
            crate::loop_player::LibraryEntry {
                file: "Room.wav".into(),
                current: false,
                saved_references: 0,
            },
        ];
        key(KeyCode::Char('r'), &mut a, Path::new("/none"), &tx);
        assert_eq!(a.loop_library_selected, 1);
    }

    #[test]
    fn letter_jump_preserves_shortcuts_and_text_input_ownership() {
        let p = vec![
            Preset::synthv1("Amber", "amber".into()),
            Preset::synthv1("Mellow", "mellow".into()),
        ];
        let mut a = app(&p);
        let (tx, _rx) = mpsc::channel();
        a.screen = Screen::Presets;
        key(KeyCode::Char('m'), &mut a, Path::new("/none"), &tx);
        assert_eq!(a.screen, Screen::Meter, "explicit shortcut wins");

        a.screen = Screen::TrackerFiles;
        a.song_list = vec!["alpha".into(), "cobalt".into()];
        a.song_selected = 0;
        a.project_name_input = Some("Proje".into());
        key(KeyCode::Char('c'), &mut a, Path::new("/none"), &tx);
        assert_eq!(a.project_name_input.as_deref(), Some("Projec"));
        assert_eq!(a.song_selected, 0, "text editor owns the letter");

        a.project_name_input = None;
        a.confirm_pattern_clear = true;
        key(KeyCode::Char('c'), &mut a, Path::new("/none"), &tx);
        assert_eq!(a.song_selected, 0, "modal blocks the list behind it");
        assert!(a.confirm_pattern_clear);
    }

    #[test]
    fn debug_build_badge_is_visible_on_normal_and_playback_screens() {
        let p = presets();
        for screen in [Screen::Presets, Screen::Playback] {
            let mut a = app(&p);
            a.screen = screen;
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
            assert!(text.contains(BUILD_BADGE), "missing {BUILD_BADGE}: {text}");
        }
    }

    #[test]
    fn every_populated_context_and_controller_page_renders_at_40x13() {
        let config = RuntimeConfig::default();
        for scenario in ScreenshotScenario::ALL {
            let mut a = screenshot_app(config.clone());
            configure_screenshot_scenario(&mut a, scenario);
            let context = a.menu_context();
            for page in 0..4 {
                if !navigation::pages(a.screen, context)[page].available() {
                    continue;
                }
                a.select_menu_page(page);
                let frame = render_screenshot_frame(&mut a, format!("{scenario:?}-{page}"))
                    .expect("40x13 context must render");
                assert_eq!(frame.cells.len(), 40 * 13);
            }
        }
    }

    #[test]
    fn every_master_overlay_and_contextual_frame_renders_at_40x13() {
        let config = RuntimeConfig::default();
        for scenario in ScreenshotSpecialScenario::ALL {
            let mut app = screenshot_app(config.clone());
            configure_special_screenshot_scenario(&mut app, scenario);
            let frame = render_screenshot_frame(&mut app, format!("{scenario:?}"))
                .expect("40x13 special scenario must render");
            assert_eq!(frame.cells.len(), 40 * 13);
        }
    }

    #[test]
    fn note_edit_at_40x20_names_route_default_and_cell_scopes() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.open_note_editor();
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
        for expected in ["DESTINAT", "CHANNEL", "INSTRUM", "audition +", "this cell"] {
            assert!(text.contains(expected), "missing {expected:?}: {text}");
        }
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
            "MIX · PERFORMANCE",
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
    fn final_bus_renders_complete_offline_state_at_forty_by_twenty() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Meter;
        a.config.audio_graph.enabled = true;
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
        for expected in [
            "MIX · PERFORMANCE BUS",
            "THREE-SOURCE SUM · UNAVAILABLE",
            "SYNTH",
            "LOOP",
            "INPUT",
            "MASTER",
            "OFFLINE",
            "LIMIT -1.0dBFS",
            "REC STOPPED",
            "MONITOR software",
        ] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
        let reachable = navigation::pages(Screen::Meter, MenuContext::Normal)
            .iter()
            .flat_map(|page| page.slots)
            .filter_map(|slot| slot.dispatch())
            .collect::<std::collections::HashSet<_>>();
        for action in [
            Action::BusSelectPrevious,
            Action::BusSelectNext,
            Action::BusLevelDecrease,
            Action::BusLevelIncrease,
            Action::BusMute,
            Action::FinalRecordToggle,
        ] {
            assert!(reachable.contains(&action));
        }
    }

    #[test]
    fn meter_and_effects_follow_home_and_contextual_parentage() {
        let p = presets();
        let mut a = app(&p);
        assert!(!perform(
            Action::OpenMeter,
            &mut a,
            Path::new("/none"),
            None
        ));
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
        assert_eq!(a.screen, Screen::Home);

        perform(Action::OpenMeter, &mut a, Path::new("/none"), None);
        perform(Action::OpenFxRack, &mut a, Path::new("/none"), None);
        assert_eq!(a.fx_rack_parent, Screen::Meter);
        perform(Action::Back, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Meter);
    }

    #[test]
    fn playback_and_ft2_effects_return_to_their_callers_without_dropping_ownership() {
        let p = presets();

        let mut playback = app(&p);
        playback.screen = Screen::Playback;
        playback.engine_owner = Some(EngineOwner::SoftwareSynth);
        perform(Action::OpenFxRack, &mut playback, Path::new("/none"), None);
        assert_eq!(playback.screen, Screen::FxRack);
        assert_eq!(playback.fx_rack_parent, Screen::Playback);
        assert_eq!(playback.fx_target, 0);
        assert_eq!(playback.engine_owner, Some(EngineOwner::SoftwareSynth));
        perform(Action::Back, &mut playback, Path::new("/none"), None);
        assert_eq!(playback.screen, Screen::Playback);
        assert_eq!(playback.engine_owner, Some(EngineOwner::SoftwareSynth));

        let route = SoftwareRoute::synthv1("Preset 00");
        let mut tracker = app(&p);
        tracker.screen = Screen::Tracker;
        tracker.engine_owner = Some(EngineOwner::Tracker(route.clone()));
        tracker.sync_tracker_route();
        perform(Action::OpenFxRack, &mut tracker, Path::new("/none"), None);
        assert_eq!(tracker.screen, Screen::FxRack);
        assert_eq!(tracker.fx_rack_parent, Screen::Tracker);
        assert_eq!(
            tracker.engine_owner,
            Some(EngineOwner::Tracker(route.clone()))
        );
        assert!(tracker.tracker_route.lock().unwrap().preview_state().0);
        perform(Action::Back, &mut tracker, Path::new("/none"), None);
        assert_eq!(tracker.screen, Screen::Tracker);
        assert_eq!(tracker.engine_owner, Some(EngineOwner::Tracker(route)));
        assert!(tracker.tracker_route.lock().unwrap().preview_state().0);
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
            AudioAvailability::LoopActive,
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
        a.recorder.stop(Instant::now());
        a.song_previewing = true;
        assert_eq!(
            a.audio_graph_edit_blocker(),
            Some("stop transport before changing the insert rack")
        );
    }
    #[test]
    fn renders_smaller_and_tiny_gracefully() {
        render(38, 14, Screen::Home);
        render(38, 14, Screen::Presets);
        render(38, 14, Screen::Playback);
        render(38, 14, Screen::Ideas);
        render(38, 14, Screen::Help);
        render(38, 14, Screen::Tracker);
        render(38, 14, Screen::TrackerFiles);
        render(38, 14, Screen::TrackerArrange);
        render(38, 14, Screen::TrackerPages);
        render(38, 14, Screen::TrackerTools);
        render(38, 14, Screen::TrackerLoop);
        render(38, 14, Screen::TrackerLoopAlign);
        render(38, 14, Screen::AudioRecorder);
        render(38, 14, Screen::FxRack);
        render(38, 14, Screen::FxEditor);
        render(38, 14, Screen::Meter);
        render(38, 14, Screen::Routing);
        render(30, 8, Screen::Presets);
        render(30, 8, Screen::Tracker)
    }

    #[test]
    fn routing_keeps_all_operational_rows_visible_at_40x20() {
        let p = presets();
        let mut app = app(&p);
        app.screen = Screen::Routing;
        let text = buffer_text(&render_app(&mut app, 40, 20));

        for expected in [
            "CTRL", "MODE", "PERF", "MIDI OUT", "DEVICE", "CLK OUT", "AUDIO",
        ] {
            assert!(
                text.contains(expected),
                "missing compact Routing text {expected}"
            );
        }
    }

    #[test]
    fn routing_distinguishes_offline_interface_from_unverified_device_profile() {
        let p = presets();
        let mut app = app(&p);
        app.screen = Screen::Routing;
        app.controller_online = false;
        app.config.external_midi.enabled = true;
        app.config.external_midi.output_match = "AudioBox USB 96:AudioBox USB 96 MIDI 1".into();
        app.config.external_midi.profile = "roland-d-50".into();
        app.routing_outputs.clear();
        app.performance_inputs = vec![crate::engine::MidiInputState {
            wanted: "Casiotone keyboard".into(),
            resolved: None,
            error: Some("not found".into()),
        }];
        *app.controller_config.write().unwrap() =
            crate::pads::PadConfig::unmapped("Casiotone controller");

        let text = buffer_text(&render_app(&mut app, 40, 20));
        assert!(text.contains("OFFLINE"));
        assert!(text.contains("D-50"));
        assert!(text.contains("UNVERIFIED"));
        assert!(!text.to_ascii_lowercase().contains("connected"));
        assert!(!text.to_ascii_lowercase().contains("detected"));
    }

    #[test]
    fn duplicate_stable_midi_output_is_reported_ambiguous_without_route_mutation() {
        let p = presets();
        let mut app = app(&p);
        app.screen = Screen::Tracker;
        let target = PageTarget::Midi("Shared DIN output".into());
        app.current_page_mut().unwrap().target = target.clone();
        app.available_page_outputs = vec![
            "Shared DIN output 20:0".into(),
            "Shared DIN output 21:0".into(),
        ];
        assert_eq!(app.target_route_issue(&target), Some("AMBIG"));
        app.open_overlay(Action::OpenRouteOverlay);
        let text = buffer_text(&render_app(&mut app, 40, 20));
        assert!(text.contains("AMBIG"));
        assert_eq!(
            app.overlay.as_ref().unwrap().route().unwrap().page.target,
            target
        );
    }

    #[test]
    fn routing_editor_wraps_drafts_and_cancels_without_writing() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Routing;
        a.routing_inputs = vec!["Keys:Keys MIDI".into()];
        a.routing_outputs = vec!["AudioBox USB 96:AudioBox USB 96 MIDI 1".into()];
        a.routing.selected = 0;
        a.move_routing(-1);
        assert_eq!(a.routing_row(), RoutingRow::AudioOutput);
        a.move_routing(1);
        assert_eq!(a.routing_row(), RoutingRow::Controller);

        a.routing.selected = 1;
        let original = a.config.midi_controller_musical_input;
        a.begin_routing_edit();
        a.move_routing(1);
        assert_eq!(a.config.midi_controller_musical_input, original);
        assert_eq!(
            a.routing
                .draft
                .as_ref()
                .unwrap()
                .config
                .midi_controller_musical_input,
            !original
        );
        assert!(a.cancel_routing_edit());
        assert_eq!(a.config.midi_controller_musical_input, original);
    }

    #[test]
    fn routing_confirm_is_atomic_and_migrates_legacy_audiobox_identity() {
        let base = std::env::temp_dir().join(format!(
            "shr-routing-editor-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        let p = presets();
        let mut a = app(&p);
        a.config.external_midi.enabled = true;
        a.config.external_midi.output_match = "AudioBox USB 96 AudioBox USB 96 MIDI 1".into();
        a.config.save(&base.join("shsynth.conf")).unwrap();
        let controller = a.controller_config.read().unwrap().clone();
        controller.save(&base.join("controller.conf")).unwrap();
        a.routing_outputs = vec!["AudioBox USB 96:AudioBox USB 96 MIDI 1 32:0".into()];
        a.routing.selected = 1;
        a.begin_routing_edit();
        a.adjust_routing_draft(1);
        let routing = &mut a.routing.draft.as_mut().unwrap().config.external_midi;
        routing.profile = "roland-d-50".into();
        a.confirm_routing_edit(&base);

        let saved = RuntimeConfig::load(&base.join("shsynth.conf")).unwrap();
        assert_eq!(
            saved.external_midi.output_match,
            "AudioBox USB 96:AudioBox USB 96 MIDI 1"
        );
        assert!(!saved.external_midi.output_match.ends_with("32:0"));
        assert_eq!(saved.external_midi.profile, "roland-d-50");
        assert_ne!(
            saved.midi_controller_musical_input,
            RuntimeConfig::default().midi_controller_musical_input
        );
        assert!(fs::read_dir(&base)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".bak-")));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn enabled_offline_output_is_valid_saveable_configuration() {
        let base = std::env::temp_dir().join(format!(
            "shr-offline-routing-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        let mut draft = RoutingDraft {
            config: RuntimeConfig::default(),
            controller: crate::pads::PadConfig::default(),
        };
        draft.config.external_midi.enabled = true;
        draft.config.external_midi.output_match = "Offline interface:DIN OUT 88:1".into();
        draft.config.external_midi.profile = "raw-midi".into();

        canonicalize_routing_draft(&mut draft, &[], &[]).unwrap();
        assert_eq!(
            draft.config.external_midi.output_match,
            "Offline interface:DIN OUT"
        );
        validate_routing_draft(&draft, &base).unwrap();
        draft.config.save(&base.join("shsynth.conf")).unwrap();
        let loaded = RuntimeConfig::load(&base.join("shsynth.conf")).unwrap();
        assert!(loaded.external_midi.enabled);
        assert_eq!(
            loaded.external_midi.output_match,
            "Offline interface:DIN OUT"
        );
        assert_eq!(loaded.external_midi.profile, "raw-midi");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn routing_transaction_restores_files_after_activation_failure() {
        let base = std::env::temp_dir().join(format!(
            "shr-routing-rollback-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        let runtime_path = base.join("shsynth.conf");
        let controller_path = base.join("controller.conf");
        let old_runtime = RuntimeConfig::default();
        old_runtime.save(&runtime_path).unwrap();
        let old_controller = crate::pads::PadConfig::default();
        old_controller.save(&controller_path).unwrap();
        let runtime_bytes = fs::read(&runtime_path).unwrap();
        let controller_bytes = fs::read(&controller_path).unwrap();
        let mut candidate = RoutingDraft {
            config: old_runtime,
            controller: old_controller,
        };
        candidate.config.midi_controller_musical_input = false;

        let error =
            persist_routing_transaction(&runtime_path, &controller_path, &candidate, || {
                anyhow::bail!("injected activation failure")
            })
            .unwrap_err();
        assert_eq!(error.0, RoutingTransactionStage::Activate);
        assert_eq!(fs::read(&runtime_path).unwrap(), runtime_bytes);
        assert_eq!(fs::read(&controller_path).unwrap(), controller_bytes);

        let absent_runtime = base.join("absent-runtime.conf");
        let absent_controller = base.join("absent-controller.conf");
        assert!(persist_routing_transaction(
            &absent_runtime,
            &absent_controller,
            &candidate,
            || anyhow::bail!("injected activation failure"),
        )
        .is_err());
        assert!(!absent_runtime.exists());
        assert!(!absent_controller.exists());

        let save_runtime = base.join("save-runtime.conf");
        candidate.config.save(&save_runtime).unwrap();
        let save_bytes = fs::read(&save_runtime).unwrap();
        let bad_controller = base.join("controller-is-directory");
        fs::create_dir(&bad_controller).unwrap();
        let error =
            persist_routing_transaction(&save_runtime, &bad_controller, &candidate, || Ok(()))
                .unwrap_err();
        assert_eq!(error.0, RoutingTransactionStage::Save);
        assert_eq!(fs::read(&save_runtime).unwrap(), save_bytes);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn every_effect_performance_control_is_visible_together_at_40x20() {
        let kinds = [
            EffectKind::Utility,
            EffectKind::Eq,
            EffectKind::Compressor,
            EffectKind::Distortion,
            EffectKind::Delay,
            EffectKind::Reverb,
            EffectKind::Chorus,
            EffectKind::Flanger,
            EffectKind::Phaser,
            EffectKind::TremoloPan,
            EffectKind::Filter,
            EffectKind::Gate,
            EffectKind::Crusher,
        ];
        let p = presets();
        for kind in kinds {
            let mut a = app(&p);
            a.song.insert_rack.add_with_id(kind, 1).unwrap();
            a.fx_selection = FxRackSelection::Effect(1);
            a.fx_parameter = crate::effect_schema::controls(kind).len().saturating_sub(1);
            a.screen = Screen::FxEditor;
            let text = buffer_text(&render_app(&mut a, 40, 20));
            for control in crate::effect_schema::controls(kind) {
                assert!(
                    text.contains(control.label),
                    "{kind:?} omitted {}: {text}",
                    control.label
                );
            }
            assert!(!text.contains("Meters unavailable"));
        }
    }

    #[test]
    fn compressor_gain_reduction_leds_use_the_declared_scale_and_red_states() {
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(COMPRESSOR_LED_GLYPH),
            1
        );
        let colors = |gain_reduction_db| {
            compressor_gain_reduction_meter(gain_reduction_db, 38)
                .0
                .into_iter()
                .filter(|span| span.content == COMPRESSOR_LED_GLYPH)
                .map(|span| span.style.fg)
                .collect::<Vec<_>>()
        };
        assert_eq!(colors(0.0), vec![Some(Color::Red); 11]);
        assert_eq!(colors(f32::NAN), vec![Some(Color::Red); 11]);

        let six_db = colors(6.0);
        assert_eq!(six_db[..6], [Some(Color::LightRed); 6]);
        assert_eq!(six_db[6..], [Some(Color::Red); 5]);
        assert_eq!(colors(24.0), vec![Some(Color::LightRed); 11]);
    }

    #[test]
    fn compressor_editor_renders_visible_dim_round_leds_at_40x20() {
        let p = presets();
        let mut a = app(&p);
        a.song
            .insert_rack
            .add_with_id(EffectKind::Compressor, 1)
            .unwrap();
        a.fx_selection = FxRackSelection::Effect(1);
        a.screen = Screen::FxEditor;
        let frame = render_app(&mut a, 40, 20);
        let meter_row = (0..20)
            .find(|row| row_text(&frame, *row).contains("GR .5"))
            .expect("compressor LED scale row");
        let leds = (0..40)
            .map(|column| buffer_cell(&frame, column, meter_row))
            .filter(|cell| cell.symbol == COMPRESSOR_LED_GLYPH)
            .collect::<Vec<_>>();
        assert_eq!(leds.len(), COMPRESSOR_GAIN_REDUCTION_LEDS_DB.len());
        assert!(leds.iter().all(|cell| cell.fg == Color::Red));
        assert!(row_text(&frame, meter_row).contains("24dB"));
    }

    #[test]
    fn universal_list_wrapping_contract_covers_workspaces_and_overlays() {
        assert_eq!(wrapped_index(9, 0, -1), 0);
        assert_eq!(wrapped_index(9, 1, -1), 0);
        assert_eq!(wrapped_index(9, 1, 1), 0);

        let p = presets();
        let mut a = app(&p);

        a.home_selected = 0;
        a.move_home(-1);
        assert_eq!(a.home_selected, HOME_ENTRIES.len() - 1);
        a.move_home(1);
        assert_eq!(a.home_selected, 0);

        a.screen = Screen::Presets;
        a.selected = 0;
        perform(Action::Up, &mut a, Path::new("/none"), None);
        assert_eq!(a.selected, p.len() - 1);
        render_app(&mut a, 40, 20);
        assert!(a.offset > 0, "wrapped preset must be scrolled into view");
        perform(Action::Down, &mut a, Path::new("/none"), None);
        assert_eq!(a.selected, 0);

        a.ideas = vec!["one".into(), "two".into()];
        a.screen = Screen::Ideas;
        a.idea_selected = 0;
        perform(Action::Up, &mut a, Path::new("/none"), None);
        assert_eq!(a.idea_selected, 1);

        a.help_selected = 0;
        a.move_help(-1);
        assert_eq!(a.help_selected, help::lines(HELP_TEXT_WIDTH).len() - 1);

        a.song_list = vec!["one".into(), "two".into()];
        a.screen = Screen::TrackerFiles;
        a.tracker_files_mode = TrackerFilesMode::Projects;
        a.song_selected = 0;
        perform(Action::Up, &mut a, Path::new("/none"), None);
        assert_eq!(a.song_selected, 1);

        let filtered = a.filtered_drum_indices();
        assert!(!filtered.is_empty());
        a.tracker_files_mode = TrackerFilesMode::Drums;
        a.drum_pattern_selected = filtered[0];
        a.move_drum_selection(-1);
        assert_eq!(a.drum_pattern_selected, *filtered.last().unwrap());

        a.screen = Screen::TrackerLoop;
        a.loop_library_mode = false;
        a.loop_imports = vec![PathBuf::from("one.wav"), PathBuf::from("two.wav")];
        a.loop_selected = 0;
        perform(Action::Up, &mut a, Path::new("/none"), None);
        assert_eq!(a.loop_selected, 1);
        a.loop_library_mode = true;
        a.loop_library = vec![
            crate::loop_player::LibraryEntry {
                file: "one.wav".into(),
                current: false,
                saved_references: 0,
            },
            crate::loop_player::LibraryEntry {
                file: "two.wav".into(),
                current: false,
                saved_references: 0,
            },
        ];
        a.loop_library_selected = 0;
        perform(Action::Up, &mut a, Path::new("/none"), None);
        assert_eq!(a.loop_library_selected, 1);

        a.screen = Screen::TrackerArrange;
        a.arrange_selected = 0;
        perform(Action::Up, &mut a, Path::new("/none"), None);
        assert_eq!(a.arrange_selected, a.song.order.len() - 1);

        a.screen = Screen::Tracker;
        a.tracker_row = 0;
        perform(Action::Up, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_row, a.tracker_rows() - 1);
        a.tracker_track = 0;
        a.tracker_page = 0;
        a.move_tracker_lane(-1);
        assert_eq!(
            a.tracker_page * LANES_PER_PAGE + a.tracker_track,
            a.current_total_lanes() - 1
        );

        a.screen = Screen::AudioRecorder;
        a.audio_track_selected = 0;
        a.move_audio_track(-1);
        assert_eq!(
            a.audio_track_selected,
            a.audio_recorder.status().tracks.len() - 1
        );
        a.screen = Screen::Meter;
        a.bus_selected = 0;
        a.move_bus_selection(-1);
        assert_eq!(a.bus_selected, 3);

        a.screen = Screen::Routing;
        a.routing.selected = 0;
        a.move_routing(-1);
        assert_eq!(a.routing.selected, RoutingRow::ALL.len() - 1);

        assert_eq!(NoteLength::ALL.first(), Some(&NoteLength::Whole));
        assert_eq!(
            NoteLength::ALL.last(),
            Some(&NoteLength::HundredTwentyEighth)
        );

        a.screen = Screen::Tracker;
        a.open_overlay(Action::OpenRouteOverlay);
        let rows = a.overlay_row_count();
        a.overlay.as_mut().unwrap().selection = 0;
        a.move_overlay(-1);
        assert_eq!(a.overlay.as_ref().unwrap().selection, rows - 1);
        render_app(&mut a, 40, 20);
        assert!(a.overlay.as_ref().unwrap().scroll > 0);
        a.move_overlay(1);
        assert_eq!(a.overlay.as_ref().unwrap().selection, 0);
    }

    #[test]
    fn fx_insert_is_a_typed_reachable_row_and_parameter_motion_is_single_step() {
        let p = presets();
        let mut a = app(&p);
        a.set_screen(Screen::FxRack);
        assert_eq!(a.fx_selection, FxRackSelection::Insert);
        a.add_effect();
        a.confirm_effect_type_edit();
        let first = a.selected_effect_id().unwrap();

        a.fx_selection = FxRackSelection::Insert;
        a.move_fx_rack_selection(1);
        assert_eq!(a.fx_selection, FxRackSelection::Effect(first));
        a.move_fx_rack_selection(-1);
        assert_eq!(a.fx_selection, FxRackSelection::Insert);
        let frame = render_app(&mut a, 40, 20);
        assert_eq!(buffer_text(&frame).matches("+ INSERT EFFECT").count(), 1);

        let before = a.song.insert_rack.order.len();
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert_eq!(a.song.insert_rack.order.len(), before + 1);
        a.confirm_effect_type_edit();
        a.set_screen(Screen::FxEditor);
        let count = crate::effect_schema::controls(a.selected_effect().unwrap().kind).len();
        a.fx_parameter = 0;
        perform(Action::Up, &mut a, Path::new("/none"), None);
        assert_eq!(a.fx_parameter, count - 1);
        perform(Action::Down, &mut a, Path::new("/none"), None);
        assert_eq!(a.fx_parameter, 0);
        perform(Action::Down, &mut a, Path::new("/none"), None);
        assert_eq!(a.fx_parameter, 1, "one gesture must move one parameter");
    }

    #[test]
    fn fx_grid_maps_two_rows_of_four_and_keeps_selection_while_editing() {
        let p = presets();
        for kind in [EffectKind::Eq, EffectKind::Delay, EffectKind::Compressor] {
            assert!(crate::effect_schema::controls(kind).len() <= 8);
        }

        let mut a = app(&p);
        a.song.insert_rack.add_with_id(EffectKind::Eq, 1).unwrap();
        a.fx_selection = FxRackSelection::Effect(1);
        a.fx_parameter = 7;
        a.screen = Screen::FxEditor;
        let browse = render_app(&mut a, 40, 20);
        assert_eq!(buffer_cell(&browse, 30, 7).bg, Color::Yellow);
        a.begin_fx_value_edit();
        a.begin_fx_numeric_entry('1');
        let editing = render_app(&mut a, 40, 20);
        assert_eq!(buffer_cell(&editing, 30, 7).bg, Color::Green);
        assert!(row_text(&editing, 8).contains("1_"));
        let text = buffer_text(&editing);
        for control in crate::effect_schema::controls(EffectKind::Eq) {
            assert!(text.contains(control.label), "missing {}", control.label);
        }
    }

    #[test]
    fn routing_keyboard_and_encoder_dispatch_identically() {
        let p = presets();
        let (tx, _rx) = mpsc::channel();
        let mut encoder = app(&p);
        let mut keyboard = app(&p);
        encoder.screen = Screen::Routing;
        keyboard.screen = Screen::Routing;
        dispatch_encoder(
            crate::pads::EncoderAction::Down,
            &mut encoder,
            Path::new("/none"),
            &tx,
        );
        key(KeyCode::Down, &mut keyboard, Path::new("/none"), &tx);
        assert_eq!(encoder.routing.selected, keyboard.routing.selected);
        dispatch_encoder(
            crate::pads::EncoderAction::Select,
            &mut encoder,
            Path::new("/none"),
            &tx,
        );
        key(KeyCode::Enter, &mut keyboard, Path::new("/none"), &tx);
        assert!(encoder.routing.draft.is_some());
        assert!(keyboard.routing.draft.is_some());
        dispatch_encoder(
            crate::pads::EncoderAction::Down,
            &mut encoder,
            Path::new("/none"),
            &tx,
        );
        key(KeyCode::Down, &mut keyboard, Path::new("/none"), &tx);
        assert_eq!(
            encoder
                .routing
                .draft
                .as_ref()
                .unwrap()
                .config
                .midi_controller_musical_input,
            keyboard
                .routing
                .draft
                .as_ref()
                .unwrap()
                .config
                .midi_controller_musical_input
        );
    }

    #[test]
    fn fx_rack_actions_preserve_ids_order_bypass_and_strict_parameters() {
        let p = presets();
        let mut a = app(&p);
        a.fx_add_kind = 0;
        a.add_effect();
        let eq = a.selected_effect_id().unwrap();
        a.confirm_effect_type_edit();
        a.fx_add_kind = 1;
        a.add_effect();
        let compressor = a.selected_effect_id().unwrap();
        assert_eq!(a.song.insert_rack.order, [compressor, eq]);
        a.confirm_effect_type_edit();
        a.move_effect(1);
        assert_eq!(a.song.insert_rack.order, [eq, compressor]);
        a.toggle_effect_bypass();
        assert!(a.song.insert_rack.effect(compressor).unwrap().bypass);
        a.fx_parameter = 0;
        a.adjust_effect_parameter(-1);
        let effect = a.song.insert_rack.effect(compressor).unwrap();
        assert!(effect.parameters["threshold_db"].is_finite());
        a.song.insert_rack.validate().unwrap();
    }

    #[test]
    fn function_keys_are_exact_physical_pad_equivalents() {
        use crate::pads::PadAction;
        let expected = [
            PadAction::Page1,
            PadAction::Page2,
            PadAction::Page3,
            PadAction::Page4,
            PadAction::Item1,
            PadAction::Item2,
            PadAction::Item3,
            PadAction::Item4,
        ];
        for (offset, action) in expected.into_iter().enumerate() {
            assert_eq!(function_key_pad(KeyCode::F(5 + offset as u8)), Some(action));
        }
        assert_eq!(function_key_pad(KeyCode::F(4)), None);
    }

    #[test]
    fn keyboard_and_physical_rotary_use_the_same_dispatch_path() {
        let p = presets();
        let (tx, _rx) = mpsc::channel();
        let mut physical = app(&p);
        let mut keyboard = app(&p);
        physical.set_screen(Screen::FxRack);
        keyboard.set_screen(Screen::FxRack);

        dispatch_encoder(
            crate::pads::EncoderAction::Select,
            &mut physical,
            Path::new("/none"),
            &tx,
        );
        key(KeyCode::Enter, &mut keyboard, Path::new("/none"), &tx);
        assert_eq!(physical.song.insert_rack, keyboard.song.insert_rack);
        assert_eq!(physical.fx_selection, keyboard.fx_selection);
        assert!(physical.fx_type_edit.is_some());
        assert!(keyboard.fx_type_edit.is_some());

        dispatch_encoder(
            crate::pads::EncoderAction::Down,
            &mut physical,
            Path::new("/none"),
            &tx,
        );
        key(KeyCode::Down, &mut keyboard, Path::new("/none"), &tx);
        assert_eq!(
            physical.selected_effect().unwrap().kind,
            keyboard.selected_effect().unwrap().kind
        );
    }

    #[test]
    fn rack_inserts_first_unused_before_cursor_and_type_cancel_restores() {
        let p = presets();
        let mut a = app(&p);
        a.set_screen(Screen::FxRack);
        a.add_effect();
        let first = a.selected_effect_id().unwrap();
        assert_eq!(a.selected_effect().unwrap().kind, EffectKind::Eq);
        assert!(a.fx_type_edit.as_ref().unwrap().provisional);
        a.cancel_effect_type_edit();
        assert!(a.song.insert_rack.order.is_empty());

        a.add_effect();
        assert_eq!(a.selected_effect_id(), Some(first));
        a.confirm_effect_type_edit();
        let before = a.song.insert_rack.clone();
        a.add_effect();
        let inserted = a.selected_effect_id().unwrap();
        assert_ne!(inserted, first);
        assert_eq!(a.selected_effect().unwrap().kind, EffectKind::Compressor);
        assert_eq!(a.song.insert_rack.order, [inserted, first]);
        a.cycle_effect_kind(1);
        assert_eq!(a.selected_effect().unwrap().kind, EffectKind::Distortion);
        a.cancel_effect_type_edit();
        assert_eq!(a.song.insert_rack, before);
    }

    #[test]
    fn rack_active_type_row_is_inverted_and_adaptive_actions_are_reachable() {
        let p = presets();
        let mut a = app(&p);
        a.set_screen(Screen::FxRack);
        assert_eq!(a.menu_context(), MenuContext::FxEmpty);
        assert_eq!(
            navigation::slot(a.screen, a.menu_context(), 0, 0).and_then(|slot| slot.dispatch()),
            Some(Action::FxAdd)
        );
        a.add_effect();
        assert_eq!(a.menu_context(), MenuContext::FxType);
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
        assert!(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|cell| cell.modifier.contains(Modifier::REVERSED)));
        a.confirm_effect_type_edit();
        assert_eq!(a.menu_context(), MenuContext::Normal);
        let actions = navigation::pages(a.screen, a.menu_context())[0]
            .slots
            .into_iter()
            .filter_map(|slot| slot.dispatch())
            .collect::<Vec<_>>();
        assert_eq!(
            actions,
            [
                Action::FxAdd,
                Action::FxRemove,
                Action::FxEditType,
                Action::OpenFxEditor
            ]
        );
    }

    #[test]
    fn effect_parameter_keyboard_numeric_validation_cancel_and_knob_pickup() {
        let p = presets();
        let mut a = app(&p);
        a.add_effect();
        a.confirm_effect_type_edit();
        a.set_screen(Screen::FxEditor);
        let id = a.selected_effect_id().unwrap();
        let (tx, _rx) = mpsc::channel();

        key(KeyCode::Right, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.fx_parameter, 1);
        for character in ['1', '2', '3'] {
            key(KeyCode::Char(character), &mut a, Path::new("/none"), &tx);
        }
        key(KeyCode::Enter, &mut a, Path::new("/none"), &tx);
        assert_eq!(
            a.song.insert_rack.effect(id).unwrap().parameters["low_mid_hz"],
            123.0
        );

        for character in ['9', '9', '9', '9', '9'] {
            key(KeyCode::Char(character), &mut a, Path::new("/none"), &tx);
        }
        key(KeyCode::Enter, &mut a, Path::new("/none"), &tx);
        assert!(a.status.contains("RANGE"));
        key(KeyCode::Esc, &mut a, Path::new("/none"), &tx);
        assert_eq!(
            a.song.insert_rack.effect(id).unwrap().parameters["low_mid_hz"],
            123.0
        );

        a.fx_parameter = 0;
        a.arm_fx_pickup();
        a.apply_fx_control(CONTROLS[0].cc, 1.0);
        assert!(a.status.contains("waiting for pickup"));
        a.apply_fx_control(CONTROLS[0].cc, 0.0);
        a.apply_fx_control(CONTROLS[0].cc, 1.0);
        assert_eq!(
            a.song.insert_rack.effect(id).unwrap().parameters["low_shelf_hz"],
            800.0
        );

        a.arm_fx_pickup();
        a.apply_fx_control(CONTROLS[4].cc, 0.5);
        a.apply_fx_control(CONTROLS[4].cc, 1.0);
        assert_eq!(
            a.song.insert_rack.effect(id).unwrap().parameters["low_shelf_db"],
            18.0
        );
    }

    #[test]
    fn every_effect_and_performance_control_remains_reachable() {
        assert_eq!(INSERT_EFFECTS.len(), 13);
        for kind in INSERT_EFFECTS {
            assert!(!crate::effect_schema::schema(kind).is_empty(), "{kind:?}");
            assert!(!crate::effect_schema::controls(kind).is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn in_app_learn_cancel_keeps_state_and_rotary_click_backs_up_saves_and_activates() {
        let p = presets();
        let mut a = app(&p);
        a.controller_online = true;
        *a.controller_config.write().unwrap() =
            crate::pads::PadConfig::unmapped("Test Controller MIDI");
        let original = a.controller_config.read().unwrap().clone();
        a.begin_controller_learn();
        assert!(a.learn_mode.load(Ordering::Relaxed));
        a.cancel_controller_learn();
        assert_eq!(*a.controller_config.read().unwrap(), original);

        let mut now = learn_master(&mut a, 28, 118);
        for cc in 10..=21 {
            learn_send(&mut a, &mut now, &[0xb0, cc, 64]);
            learn_settle(&mut a, &mut now);
        }
        for note in 36..=43 {
            learn_send(&mut a, &mut now, &[0x99, note, 100]);
            learn_send(&mut a, &mut now, &[0x89, note, 0]);
        }
        let state = std::env::temp_dir().join(format!(
            "shr-in-app-learn-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&state).unwrap();
        original.save(&state.join("controller.conf")).unwrap();
        let (tx, rx) = mpsc::channel();
        now += Duration::from_millis(1);
        tx.send(MidiEvent::Learn {
            received: now,
            bytes: vec![0xb0, 118, 127],
        })
        .unwrap();
        drain(&rx, &mut a, &state, &tx);
        assert!(a.controller_learn.is_some());
        assert!(a.learn_mode.load(Ordering::Relaxed));
        let saved = crate::pads::PadConfig::load(&state.join("controller.conf")).unwrap();
        assert_eq!(saved.controls.len(), 12);
        assert_eq!(saved.pads.len(), 8);
        assert_eq!(*a.controller_config.read().unwrap(), saved);
        assert!(fs::read_dir(&state)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".bak-")));

        for value in [127, 127] {
            now += Duration::from_millis(1);
            tx.send(MidiEvent::Learn {
                received: now,
                bytes: vec![0xb0, 118, value],
            })
            .unwrap();
        }
        now += Duration::from_millis(1);
        tx.send(MidiEvent::Learn {
            received: now,
            bytes: vec![0xb0, 118, 0],
        })
        .unwrap();
        drain(&rx, &mut a, &state, &tx);
        assert!(a.controller_learn.is_none());
        assert!(!a.learn_mode.load(Ordering::Relaxed));

        a.begin_controller_learn();
        let reentry = a.controller_learn.as_mut().unwrap();
        reentry.receive(&[0xb0, 118, 0], now + Duration::from_millis(1));
        assert_eq!(
            reentry.role(),
            crate::controller_learn::LearnRole::EncoderCounterClockwise
        );
        assert_eq!(reentry.draft().encoder_relative_cc, None);
        a.cancel_controller_learn();
        fs::remove_dir_all(state).unwrap();
    }

    #[test]
    fn in_app_learn_rotary_browses_and_second_click_saves_encoder_only() {
        let p = presets();
        let mut a = app(&p);
        a.controller_online = true;
        *a.controller_config.write().unwrap() =
            crate::pads::PadConfig::unmapped("Test Controller MIDI");
        let mut now = learn_master(&mut a, 28, 118);
        assert_eq!(
            a.controller_learn.as_ref().unwrap().role(),
            crate::controller_learn::LearnRole::AbsoluteControl(0)
        );

        let state = std::env::temp_dir().join(format!(
            "shr-in-app-learn-nav-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&state).unwrap();
        let (tx, rx) = mpsc::channel();

        now += Duration::from_millis(1);
        tx.send(MidiEvent::Learn {
            received: now,
            bytes: vec![0xb0, 28, 65],
        })
        .unwrap();
        drain(&rx, &mut a, &state, &tx);
        assert_eq!(
            a.controller_learn.as_ref().unwrap().role(),
            crate::controller_learn::LearnRole::AbsoluteControl(1)
        );
        now += Duration::from_millis(200);
        a.controller_learn.as_mut().unwrap().tick(now);
        now += Duration::from_millis(1);
        tx.send(MidiEvent::Learn {
            received: now,
            bytes: vec![0xb0, 28, 63],
        })
        .unwrap();
        drain(&rx, &mut a, &state, &tx);
        assert_eq!(
            a.controller_learn.as_ref().unwrap().role(),
            crate::controller_learn::LearnRole::AbsoluteControl(0)
        );

        now += Duration::from_millis(200);
        a.controller_learn.as_mut().unwrap().tick(now);
        now += Duration::from_millis(1);
        tx.send(MidiEvent::Learn {
            received: now,
            bytes: vec![0xb0, 118, 127],
        })
        .unwrap();
        now += Duration::from_millis(1);
        tx.send(MidiEvent::Learn {
            received: now,
            bytes: vec![0xb0, 118, 0],
        })
        .unwrap();
        drain(&rx, &mut a, &state, &tx);
        assert!(a.controller_learn.is_none());
        let saved = crate::pads::PadConfig::load(&state.join("controller.conf")).unwrap();
        assert_eq!(saved.encoder_relative_cc, Some(28));
        assert_eq!(saved.encoder_press_cc, Some(118));
        assert!(saved.controls.is_empty());
        assert!(saved.pads.is_empty());
        assert_eq!(saved.layout, crate::pads::ControllerLayout::Four);
        fs::remove_dir_all(state).unwrap();
    }

    #[test]
    fn fx_rotary_browses_then_confirms_or_cancels_an_active_value() {
        let p = presets();
        let mut a = app(&p);
        a.fx_add_kind = 0;
        a.add_effect();
        a.set_screen(Screen::FxEditor);
        let id = a.selected_effect_id().unwrap();

        perform(Action::Down, &mut a, Path::new("/none"), None);
        assert_eq!(a.fx_parameter, 1, "inactive rotation browses parameters");
        let before = a.song.insert_rack.effect(id).unwrap().clone();
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert!(a.fx_value_editing);
        perform(Action::Down, &mut a, Path::new("/none"), None);
        assert_ne!(a.song.insert_rack.effect(id).unwrap(), &before);
        perform(Action::Back, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::FxEditor);
        assert!(!a.fx_value_editing);
        assert_eq!(a.song.insert_rack.effect(id).unwrap(), &before);

        perform(Action::Activate, &mut a, Path::new("/none"), None);
        perform(Action::Down, &mut a, Path::new("/none"), None);
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert!(!a.fx_value_editing);
        assert_ne!(a.song.insert_rack.effect(id).unwrap(), &before);
    }

    #[test]
    fn list_rotary_activation_never_starts_audio_or_final_recording() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::AudioRecorder;
        let tracks = a.audio_recorder.status().tracks.len();
        assert!(tracks > 1);

        perform(Action::Down, &mut a, Path::new("/none"), None);
        assert_eq!(a.audio_track_selected, 1);
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert!(!a.audio_recorder.status().recording);

        a.screen = Screen::Meter;
        perform(Action::Down, &mut a, Path::new("/none"), None);
        assert_eq!(a.bus_selected, 1);
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert!(a
            .engine
            .as_ref()
            .is_none_or(|engine| !engine.final_recording_active()));
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
    fn forty_by_thirteen_hides_empty_child_navigation_pages_and_items() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Playback;
        a.select_menu_page(1);
        let backend = TestBackend::new(40, 13);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        for label in ["PLAY", "SOUND", "SYS", "RESET", "SAVE", "N00B"] {
            assert!(text.contains(label), "missing {label}: {text}");
        }
        for removed in ["NAV", "IDEAS", "PRESETS", "FT2", "AUDIO"] {
            assert!(!text.contains(removed), "stale {removed}: {text}");
        }
        assert!(!text.contains("PLY"));
        assert!(row_text(terminal.backend().buffer(), 12).starts_with('■'));
        assert_eq!(a.hits.menu_pages.len(), 3);
        assert_eq!(a.hits.actions.len(), 3);
    }

    #[test]
    fn forty_by_thirteen_tracker_places_menu_above_shared_status() {
        let p = presets();
        let mut a = app(&p);
        fill_demo_song(&mut a);
        a.screen = Screen::Tracker;
        a.current_page_mut().unwrap().target =
            PageTarget::Software(SoftwareRoute::synthv1(p[0].route_id()));

        let buffer = render_app(&mut a, 40, 13);
        let text = buffer_text(&buffer);
        for noise in ["AVAILABLE", "ONLINE", "CONNECTED", "IDLE"] {
            assert!(!text.contains(noise), "stale {noise}: {text}");
        }
        assert!(row_text(&buffer, 10).contains("1:"));
        assert!(row_text(&buffer, 11).contains('['));
        assert!(row_text(&buffer, 12).starts_with('‖'));
    }

    #[test]
    fn eight_five_and_four_control_layouts_select_pages_without_losing_encoder_navigation() {
        let p = presets();
        let (tx, rx) = mpsc::channel();
        let mut eight = app(&p);
        eight.screen = Screen::Tracker;
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Page2, true))
            .unwrap();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item1, true))
            .unwrap();
        drain(&rx, &mut eight, Path::new("/none"), &tx);
        assert_eq!(eight.menu_page(), 1);
        assert_eq!(
            eight.overlay.as_ref().map(|overlay| overlay.kind),
            Some(OverlayKind::TrackerPage)
        );

        let mut five = app(&p);
        five.screen = Screen::Tracker;
        five.controller_layout = ControllerLayout::Five;
        five.select_menu_page(1);
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item1, true))
            .unwrap();
        drain(&rx, &mut five, Path::new("/none"), &tx);
        assert_eq!(
            five.overlay.as_ref().map(|overlay| overlay.kind),
            Some(OverlayKind::TrackerPage)
        );

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
        assert_eq!(four.tracker_track, 0);
        assert_eq!(
            four.tracker_row, 1,
            "a stopped FT2 transport leaves normal row navigation available"
        );
    }

    #[test]
    fn ft2_rotary_selects_columns_only_during_transport_but_keyboard_and_edit_keep_rows() {
        let p = presets();
        let (tx, _rx) = mpsc::channel();
        let mut a = app(&p);
        fill_demo_song(&mut a);
        a.screen = Screen::Tracker;
        a.tracker_mode = TrackerMode::Play;
        a.tracker_order = 2;
        a.tracker_row = 9;
        a.tracker_page = 0;
        a.tracker_track = LANES_PER_PAGE - 1;

        // Remembered Play mode is still paused, so the rotary moves rows.
        dispatch_encoder(
            crate::pads::EncoderAction::Down,
            &mut a,
            Path::new("/none"),
            &tx,
        );
        assert_eq!((a.tracker_page, a.tracker_track), (0, LANES_PER_PAGE - 1));
        assert_eq!((a.tracker_order, a.tracker_row), (2, 10));

        a.sequencer.play(&a.song, a.tracker_order, a.tracker_row);
        let transport = a.sequencer.status();
        dispatch_encoder(
            crate::pads::EncoderAction::Down,
            &mut a,
            Path::new("/none"),
            &tx,
        );
        assert_eq!((a.tracker_page, a.tracker_track), (1, 0));
        assert_eq!((a.tracker_order, a.tracker_row), (2, 10));
        assert_eq!(a.sequencer.status().playing, transport.playing);
        assert_eq!(a.sequencer.status().order, transport.order);
        assert_eq!(a.sequencer.status().row, transport.row);

        key(KeyCode::Down, &mut a, Path::new("/none"), &tx);
        assert_eq!((a.tracker_page, a.tracker_track), (1, 0));
        assert_eq!(a.tracker_row, 11);

        a.sequencer.stop();
        a.tracker_mode = TrackerMode::Edit;
        dispatch_encoder(
            crate::pads::EncoderAction::Down,
            &mut a,
            Path::new("/none"),
            &tx,
        );
        assert_eq!((a.tracker_page, a.tracker_track), (1, 0));
        assert_eq!(a.tracker_row, 12);
    }

    #[test]
    fn ft2_record_rotary_ignores_turns_until_every_recorded_note_is_off() {
        let p = presets();
        let (tx, _rx) = mpsc::channel();
        let mut a = app(&p);
        fill_demo_song(&mut a);
        a.screen = Screen::Tracker;
        a.tracker_mode = TrackerMode::Rec;
        a.tracker_row = 7;
        a.tracker_page = 0;
        a.tracker_track = LANES_PER_PAGE - 1;
        a.tracker_recording = Some(TrackerRecording {
            pattern: 0,
            order: 0,
            page: 0,
            return_to_play: false,
            last_row: 7,
            next_lane: LANES_PER_PAGE - 1,
            active_lanes: HashMap::from([(
                (0, 60),
                vec![RecordedLane {
                    lane: LANES_PER_PAGE - 1,
                    start_row: 7,
                }],
            )]),
            notes: 1,
        });

        for _ in 0..2 {
            dispatch_encoder(
                crate::pads::EncoderAction::Down,
                &mut a,
                Path::new("/none"),
                &tx,
            );
        }
        assert_eq!((a.tracker_page, a.tracker_track), (0, LANES_PER_PAGE - 1));
        assert_eq!(a.tracker_row, 7);

        a.record_tracker_midi(&[0x80, 60, 0]);
        dispatch_encoder(
            crate::pads::EncoderAction::Down,
            &mut a,
            Path::new("/none"),
            &tx,
        );
        assert_eq!((a.tracker_page, a.tracker_track), (1, 0));
        let recording = a.tracker_recording.as_ref().unwrap();
        assert_eq!(
            (recording.pattern, recording.order, recording.page),
            (0, 0, 1)
        );
        assert_eq!(recording.next_lane, 0);
        assert_eq!(recording.notes, 1);
        assert_eq!(a.tracker_row, 7);
        assert_eq!(a.tracker_mode, TrackerMode::Rec);
    }

    #[test]
    fn all_item_buttons_use_the_selected_screen_menu_and_screen_entry_starts_on_page_one() {
        let p = presets();
        let mut a = app(&p);
        let (tx, rx) = mpsc::channel();
        a.screen = Screen::Tracker;
        a.select_menu_page(0);
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item4, true))
            .unwrap();
        drain(&rx, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.tracker_mode, TrackerMode::Edit);
        assert_eq!(a.menu_page(), 0, "entering EDIT starts on its MODE page");
        a.select_menu_page(1);
        perform(Action::OpenIdeas, &mut a, Path::new("/none"), None);
        assert_eq!(a.menu_page(), 0);
        a.select_menu_page(1);
        perform(Action::OpenTracker, &mut a, Path::new("/none"), None);
        assert_eq!(a.menu_page(), 0, "FT2 entry starts on PLAY");
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
        connect_test_midi_hardware(&mut a);
        a.set_screen(Screen::Tracker);
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.toggle_tracker_recording();
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
        connect_test_midi_hardware(&mut a);
        a.set_screen(Screen::Tracker);
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.toggle_tracker_recording();

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
        let draft = a.note_editor.as_ref().unwrap().draft;

        a.tracker_stop();

        assert_eq!(a.note_editor.as_ref().unwrap().draft, draft);
        assert_eq!(a.screen, Screen::Tracker);
    }

    #[test]
    fn empty_controller_items_are_silent() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Meter;
        a.select_menu_page(1);
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
    fn tracker_renders_the_three_factory_page_names_and_compact_pause_label() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.song.patterns.get_mut(&0).unwrap().tempo = 137;
        let now = Instant::now();
        a.tap.tap(now);
        a.tap.tap(now + Duration::from_millis(500));
        let b = TestBackend::new(40, 13);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let text = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol.as_str())
            .collect::<String>();
        assert!(text.contains("Software Synth"));
        assert!(text.contains("PAUSE"));
        assert!(!text.contains("STOP/BACK"));
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
        assert!(text.contains("MIDI"));
        a.switch_tracker_page();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let text = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol.as_str())
            .collect::<String>();
        assert!(text.contains("Drums"));
    }

    #[test]
    fn next_page_after_drums_opens_the_unloaded_fourth_loop_page() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_page = 2;

        a.switch_tracker_page();
        assert_eq!(a.screen, Screen::TrackerLoop);
        assert!(a.song.audio_loop.is_none());
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
        assert!(text.contains("P04/04 · FT2 WAV LOOP"));
        assert!(text.contains("UNLOADED"));
    }
    #[test]
    fn page_management_is_fully_reachable_with_pads_and_encoder_actions() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.open_page_manager();
        assert_eq!(a.screen, Screen::TrackerPages);
        perform(Action::AddPage, &mut a, Path::new("/none"), None);
        assert_eq!(a.current_pages().len(), 4);
        assert_eq!(a.current_pages()[3].lanes.len(), 4);

        perform(Action::EditPageTarget, &mut a, Path::new("/none"), None);
        assert_eq!(a.page_manager_mode, PageManagerMode::Target);
        perform(Action::Down, &mut a, Path::new("/none"), None);
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert_eq!(a.page_manager_mode, PageManagerMode::MidiOutput);
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert_eq!(a.page_manager_mode, PageManagerMode::Pages);

        perform(Action::EditPageChannel, &mut a, Path::new("/none"), None);
        perform(Action::Down, &mut a, Path::new("/none"), None);
        perform(Action::Activate, &mut a, Path::new("/none"), None);
        assert_eq!(a.current_pages()[3].column(0).channel, 1);

        perform(Action::PreviousTrack, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_page, 3);
        assert_eq!(a.tracker_track, 0);
        perform(Action::NextTrack, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_page, 3);
        assert_eq!(a.tracker_track, 1);
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
            let cell = b.get(x, 2);
            assert_eq!(cell.fg, Color::Black, "unexpected foreground at x={x}");
            assert_eq!(cell.bg, Color::Yellow, "unexpected background at x={x}");
            assert!(!cell.modifier.contains(Modifier::BOLD));
        }
    }
    #[test]
    fn scrolled_hit_test_maps_visible_row() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Presets;
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
    fn wheel_and_page_pad_button_clicks_work_at_40x13() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Presets;
        a.selected = 5;
        let b = TestBackend::new(40, 13);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        assert_eq!(a.hits.actions.len(), 3);
        assert_eq!(a.hits.menu_pages.len(), 3);
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
                row: 10,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            &mut a,
            Path::new("/nonexistent"),
            &tx,
        );
        assert_eq!(a.menu_page(), 1);
        assert_eq!(a.selected, 8, "page selection preserves the list cursor");
    }

    #[test]
    fn controller_strip_stays_compact_on_wide_terminals() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Presets;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();

        assert_eq!(a.hits.menu_pages.len(), 3);
        assert!(a
            .hits
            .menu_pages
            .iter()
            .all(|(area, _)| area.width >= 7 && area.width <= 10 && area.x >= 20 && area.x < 60));
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
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
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
        assert!(title.starts_with(BUILD_BADGE));
        assert!(title.contains("synthv1 · Preset 00"));
        assert!(!title.contains("PLY"));
        let left = title
            .chars()
            .position(|character| character == 's')
            .unwrap();
        let right = 40 - left - "synthv1 · Preset 00".chars().count();
        assert!(left.abs_diff(right) <= 1);
        assert!((37..40).all(|x| b.get(x, 0).symbol == " "));
        let buttons = (17..19)
            .flat_map(|y| (0..40).map(move |x| b.get(x, y).symbol.as_str()))
            .collect::<String>();
        assert!(buttons.contains('['));
        assert!(!buttons.contains(&a.status));

        a.recorder.start(Instant::now());
        t.draw(|f| draw(f, &mut a)).unwrap();
        let b = t.backend().buffer();
        assert_eq!(b.get(0, 19).symbol, "●");
        assert!(matches!(b.get(0, 19).fg, Color::Red | Color::LightRed));
        assert!((37..40).all(|x| b.get(x, 0).symbol == " "));
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
        let config = RuntimeConfig::default();
        let available_audio_ports = config.audio_outputs.clone();
        let capture_sources = config
            .capture
            .effective_tracks()
            .into_iter()
            .map(|track| track.preferred_source)
            .filter(|source| !source.is_empty())
            .collect();
        let mut a = App::new(
            &catalogs,
            Arc::new(std::sync::Mutex::new(None)),
            Arc::new(std::sync::Mutex::new(crate::midi::Pickup::default())),
            Arc::new(std::sync::Mutex::new(BackendKind::Synthv1)),
            TrackerIo {
                route: Arc::new(std::sync::Mutex::new(engine::TrackerRoute::default())),
                input: Arc::new(std::sync::Mutex::new(None)),
                playback_scale: Arc::new(std::sync::Mutex::new(None)),
                lifecycle: engine::MidiLifecycle::default(),
            },
            config,
            AvailablePorts {
                playback: available_audio_ports,
                capture_sources,
                midi_outputs: Vec::new(),
            },
            PathBuf::from("/none"),
            PathBuf::from("/none"),
        );
        a.screen = Screen::Presets;
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
    fn playback_aligns_each_held_note_with_its_decimal_velocity_at_40x13() {
        let p = presets();
        let mut a = app(&p);
        configure_screenshot(&mut a, Screen::Playback);
        let b = TestBackend::new(40, 13);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let b = t.backend().buffer();
        let row = |y| {
            (0..40)
                .map(|x| b.get(x, y).symbol.as_str())
                .collect::<String>()
        };

        assert_eq!(
            held_note_rows(
                &a.held_notes.display(a.config.note_naming).unwrap().notes,
                40
            ),
            (" D  F#   A ".into(), "100 92  104".into(),)
        );
        assert!(row(8).contains(" D  F#   A "));
        assert!(row(9).contains("100 92  104"));
        assert!(row(12).starts_with('■'));
    }

    #[test]
    fn playback_velocity_rows_compact_without_overlapping_controls_or_footer() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Playback;
        a.playing = Some(p[0].clone());
        for (note, velocity) in [(48, 41), (60, 87), (64, 103), (67, 119)] {
            a.held_notes.observe(&[0x90, note, velocity]);
        }
        let b = TestBackend::new(38, 14);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let b = t.backend().buffer();
        let text = b
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        for expected in ["C maj", "41", "87", "103", "119", "PLAY"] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
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
        let b = TestBackend::new(40, 2);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| {
            let area = f.size();
            draw_playback_keyboard(f, &a, area);
        })
        .unwrap();
        let b = t.backend().buffer();

        // Every column is a white-key column; octave boundaries have no gaps.
        assert!((0..40).all(|x| b.get(x, 1).symbol == "█"));
        assert_eq!(b.get(6, 0).symbol, "█"); // B2
        assert_eq!(b.get(7, 0).symbol, "└"); // C3 immediately follows

        // C4: the white natural region and lower block are red, not its └ stroke.
        assert_eq!(b.get(14, 0).symbol, "└");
        assert_eq!(b.get(14, 0).fg, Color::Black);
        assert_eq!(b.get(14, 0).bg, Color::Red);
        assert_eq!(b.get(14, 1).fg, Color::Red);

        // E4 has no sharp above it, so both complete blocks are red.
        assert_eq!(b.get(16, 0).symbol, "█");
        assert_eq!(b.get(16, 0).fg, Color::Red);
        assert_eq!(b.get(16, 1).fg, Color::Red);

        // F#4 colours only the └ foreground; the unplayed F stays white.
        assert_eq!(b.get(17, 0).symbol, "└");
        assert_eq!(b.get(17, 0).fg, Color::Red);
        assert_eq!(b.get(17, 0).bg, Color::White);
        assert_eq!(b.get(17, 1).fg, Color::White);
    }

    #[test]
    fn tracker_play_runs_an_empty_pattern_and_pad_actions_move_tracks() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        connect_test_midi_hardware(&mut a);
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.toggle_tracker_playback();
        assert!(a.sequencer.status().playing);
        assert!(a.status.contains("tracker playing · 0 MIDI"));
        perform(Action::NextTrack, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_track, 1);
        perform(Action::PreviousTrack, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_track, 0);
        a.tracker_stop();
    }

    #[test]
    fn tracker_keyboard_uses_drum_range_on_percussion_track() {
        let p = presets();
        let mut a = app(&p);
        assert_eq!(a.tracker_keyboard_note(0), 60);
        a.tracker_page = a.percussion_page_index().unwrap();
        assert!(a.current_pages()[a.tracker_page].percussion);
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
    fn idea_play_and_record_controls_toggle_modes_and_finalize_safely() {
        let p = presets();
        let mut a = app(&p);
        let now = Instant::now();
        a.recorder.start(now);
        a.recorder
            .capture(now + Duration::from_millis(10), &[0x90, 60, 100]);

        a.toggle_playback();
        assert_eq!(a.idea_mode, IdeaMode::Play);
        assert!(!a.recorder.is_recording());
        assert_eq!(a.last.len(), 4, "Record-to-Play finalizes before playback");
        assert_eq!(
            a.last[1..]
                .iter()
                .map(|event| event.bytes.as_slice())
                .collect::<Vec<_>>(),
            vec![&[0xb0, 64, 0][..], &[0xb0, 123, 0][..], &[0xb0, 120, 0][..]]
        );

        let stop = Arc::new(AtomicBool::new(false));
        a.playback = Some(Playback {
            stop,
            finished: Arc::new(AtomicBool::new(false)),
            worker: None,
        });
        a.toggle_idea_recording();
        assert_eq!(a.idea_mode, IdeaMode::Record);
        assert!(a.playback.is_none(), "Play-to-Record stops playback first");

        a.recorder.start(Instant::now());
        a.toggle_idea_recording();
        assert!(!a.recorder.is_recording());
        assert_eq!(a.idea_mode, IdeaMode::Record);
    }

    #[test]
    fn idea_transport_keyboard_shortcuts_are_not_forced_onto_unrelated_screens() {
        let p = presets();
        let mut a = app(&p);
        let (tx, _rx) = mpsc::channel();
        a.screen = Screen::Meter;

        key(KeyCode::Char('r'), &mut a, Path::new("/none"), &tx);
        key(KeyCode::Char('p'), &mut a, Path::new("/none"), &tx);

        assert_eq!(a.idea_mode, IdeaMode::Play);
        assert!(!a.recorder.is_recording());
        assert!(a.playback.is_none());

        a.screen = Screen::Playback;
        key(KeyCode::Char('r'), &mut a, Path::new("/none"), &tx);
        assert_eq!(a.idea_mode, IdeaMode::Record);
    }

    #[test]
    fn tracker_play_record_and_edit_modes_are_mutually_exclusive() {
        let p = presets();
        let mut a = app(&p);
        connect_test_midi_hardware(&mut a);
        a.screen = Screen::Tracker;
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        let (tx, _rx) = mpsc::channel();

        a.toggle_tracker_playback();
        assert!(a.sequencer.status().playing);
        assert_eq!(a.tracker_mode, TrackerMode::Play);
        a.toggle_tracker_playback();
        assert!(!a.sequencer.status().playing);

        a.toggle_tracker_playback();
        perform(
            Action::TrackerRecordToggle,
            &mut a,
            Path::new("/none"),
            None,
        );
        assert!(a.tracker_recording.is_some());
        assert!(a
            .tracker_recording
            .as_ref()
            .is_some_and(|recording| !recording.return_to_play));
        assert_eq!(a.tracker_mode, TrackerMode::Rec);
        assert!(a.sequencer.status().playing);

        perform(
            Action::TrackerRecordToggle,
            &mut a,
            Path::new("/none"),
            None,
        );
        assert!(a.tracker_recording.is_none());
        assert_eq!(a.tracker_mode, TrackerMode::Play);
        assert!(!a.sequencer.status().playing);
        assert!(a.status.contains("REC stopped"));

        key(KeyCode::Char('r'), &mut a, Path::new("/none"), &tx);
        assert!(a.tracker_recording.is_some());
        assert!(a
            .tracker_recording
            .as_ref()
            .is_some_and(|recording| !recording.return_to_play));
        assert!(a.sequencer.status().playing);
        key(KeyCode::Char('r'), &mut a, Path::new("/none"), &tx);
        assert!(a.tracker_recording.is_none());
        assert_eq!(a.tracker_mode, TrackerMode::Play);
        assert!(!a.sequencer.status().playing);
        assert!(a.status.contains("REC stopped"));

        a.toggle_tracker_playback();
        assert!(a.sequencer.status().playing);
        perform(Action::TrackerEdit, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_mode, TrackerMode::Edit);
        assert!(!a.sequencer.status().playing);

        a.toggle_tracker_recording();
        assert!(a.tracker_recording.is_some());
        perform(Action::TrackerEdit, &mut a, Path::new("/none"), None);
        assert!(a.tracker_recording.is_none());
        assert_eq!(a.tracker_mode, TrackerMode::Edit);
        assert!(!a.sequencer.status().playing);
    }

    #[test]
    fn inactive_tracker_record_mode_is_visible_at_forty_by_twenty() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_mode = TrackerMode::Rec;
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
        assert!(text.contains("REC READY"));
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
            finished: Arc::new(AtomicBool::new(false)),
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
    fn loop_screen_renders_populated_loop_only_meter_and_preserves_details() {
        let p = presets();
        let mut a = app(&p);
        configure_screenshot(&mut a, Screen::TrackerLoop);
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
        for expected in [
            "breakbeat-96.wav",
            "Project tempo  96 BPM",
            "Region beat 0 +16",
            "Offset +0 bar(s)",
            "48000 Hz · 2ch",
            "Native pitch playback",
            "LOOP OUT",
            "dBFS",
            "L [",
            "R [",
            "MAX  -5.2",
            "MAX  -2.4",
            "PREVIEW",
        ] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
        assert!(!text.contains("FINAL OUT"));
    }

    #[test]
    fn loop_screen_maps_sample_position_across_40_or_38_cell_bar() {
        let p = presets();
        let mut a = app(&p);
        configure_screenshot(&mut a, Screen::TrackerLoop);

        let wide = render_app(&mut a, 40, 20);
        for x in 0..40 {
            let expected = if x == 15 { Color::Green } else { Color::White };
            assert_eq!(wide.get(x, 3).bg, expected, "40-cell bar at x={x}");
        }

        let compact = render_app(&mut a, 38, 14);
        for x in 0..38 {
            let expected = if x == 14 { Color::Green } else { Color::White };
            assert_eq!(compact.get(x, 3).bg, expected, "38-cell bar at x={x}");
        }
    }

    #[test]
    fn loop_meter_presentation_resets_on_unload_and_compact_layout_is_safe() {
        let p = presets();
        let mut a = app(&p);
        configure_screenshot(&mut a, Screen::TrackerLoop);
        assert!(a.loop_meter.numeric_peak_dbfs()[0] > -6.0);
        seed_numeric_meter_peaks(&mut a);
        let final_output_peaks = a.performance_meter.numeric_peak_dbfs();
        a.unload_loop_player();
        assert_eq!(
            a.loop_meter.numeric_peak_dbfs(),
            [
                performance_meter::AUDIO_FLOOR_DBFS,
                performance_meter::AUDIO_FLOOR_DBFS,
            ]
        );
        assert_eq!(a.performance_meter.numeric_peak_dbfs(), final_output_peaks);

        configure_screenshot(&mut a, Screen::TrackerLoop);
        let b = TestBackend::new(38, 14);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let text = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        assert!(text.contains("FT2 WAV LOOP"));
        assert!(text.contains("PLAY"));
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
        perform(Action::NoteEditorSave, &mut a, Path::new("/none"), None);
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
        a.save_note_editor();
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
    fn note_edit_reaches_channel_gm_instrument_and_cancelable_audition_route() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.open_note_editor();
        assert_eq!(a.tracker_program_label(0), "GM 001 Acoustic Grand Piano");
        assert_eq!(a.tracker_program_label(127), "GM 128 Gunshot");

        a.select_note_editor_field(NoteEditorField::Channel);
        assert!(a.note_editor.as_ref().unwrap().active);
        let release_before = a.audition_release_revision;
        for _ in 0..9 {
            a.adjust_note_editor(1);
        }
        assert_eq!(a.current_column().unwrap().channel, 9);
        assert!(a.audition_release_revision > release_before);
        a.confirm_note_editor_field();

        a.select_note_editor_field(NoteEditorField::DefaultProgram);
        a.adjust_note_editor(1);
        assert_eq!(a.note_editor.as_ref().unwrap().draft.note, Note::On(36));
        assert!(a.tracker_instrument_label().contains("Bass Drum 1"));
        a.cancel_note_editor_field();
        assert_eq!(a.note_editor.as_ref().unwrap().draft.note, Note::Empty);

        a.select_note_editor_field(NoteEditorField::Channel);
        a.adjust_note_editor(-1);
        a.back_note_editor();
        assert_eq!(a.current_column().unwrap().channel, 9);
        assert!(
            a.note_editor.is_some(),
            "Back cancels only the active field"
        );
        a.back_note_editor();
        assert!(a.note_editor.is_none());
        assert_eq!(a.current_column().unwrap().channel, 0);
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
        assert_eq!(
            a.tracker_program_messages(7),
            vec![vec![0xb0, 0, 5], vec![0xb0, 32, 9], vec![0xc0, 7]]
        );

        a.current_page_mut().unwrap().target = PageTarget::ActiveInstrument;
        assert!(a.tracker_program_messages(7).is_empty());
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
        assert!(a.tracker_route.lock().unwrap().preview_state().0);
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
        a.save_note_editor();
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
        perform(Action::NoteField, &mut a, Path::new("/none"), None);
        perform(Action::NoteEditorDecrease, &mut a, Path::new("/none"), None);
        perform(Action::NoteEditorConfirm, &mut a, Path::new("/none"), None);
        perform(Action::NoteEditorSave, &mut a, Path::new("/none"), None);
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::Off);
    }

    #[test]
    fn all_four_controller_item_buttons_dispatch_in_note_edit_mode() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.open_note_editor();
        a.select_menu_page(2);
        let (tx, rx) = mpsc::channel();
        for (pad, field) in [
            (crate::pads::PadAction::Item1, NoteEditorField::Note),
            (crate::pads::PadAction::Item2, NoteEditorField::Gate),
            (crate::pads::PadAction::Item3, NoteEditorField::Velocity),
            (crate::pads::PadAction::Item4, NoteEditorField::Effect),
        ] {
            tx.send(MidiEvent::Pad(pad, true)).unwrap();
            drain(&rx, &mut a, Path::new("/none"), &tx);
            assert_eq!(a.note_editor.as_ref().unwrap().field, field);
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
        assert!(!four.page_select_mode);
        assert!(four.note_editor.as_ref().unwrap().active);
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Select))
            .unwrap();
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Down))
            .unwrap();
        drain(&rx, &mut four, Path::new("/none"), &tx);
        assert!(!four.page_select_mode);
        assert_eq!(
            four.note_editor.as_ref().unwrap().field,
            NoteEditorField::Channel
        );
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
            assert_eq!(buffer.get(x, 2).symbol, marker);
        }
        assert_eq!(buffer.get(9, 2).fg, Color::Black);
        assert_eq!(buffer.get(9, 2).bg, Color::Yellow);
        assert_eq!(buffer.get(18, 2).bg, Color::DarkGray);

        let mut empty = app(&p);
        empty.screen = Screen::Tracker;
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut empty)).unwrap();
        assert_eq!(terminal.backend().buffer().get(9, 2).symbol, " ");
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
    fn released_melody_notes_stay_in_the_selected_column() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        a.tracker_track = 2;
        let (tx, rx) = mpsc::channel();
        let start = Instant::now();
        for (offset, note) in [60, 62, 64].into_iter().enumerate() {
            tx.send(MidiEvent::Raw {
                received: start + Duration::from_millis(offset as u64 * 10),
                bytes: vec![0x90, note, 90],
            })
            .unwrap();
            tx.send(MidiEvent::Raw {
                received: start + Duration::from_millis(offset as u64 * 10 + 5),
                bytes: vec![0x80, note, 0],
            })
            .unwrap();
        }

        drain(&rx, &mut a, Path::new("/none"), &tx);

        let rows = &a.song.patterns[&0].rows;
        for (row, note) in [60, 62, 64].into_iter().enumerate() {
            assert_eq!(rows[row][2].note, Note::On(note));
            assert!(rows[row][..2]
                .iter()
                .chain(rows[row][3..4].iter())
                .all(|cell| cell.note == Note::Empty));
        }
    }

    #[test]
    fn drum_gesture_reuses_each_voice_column_from_earlier_rows() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        a.tracker_page = a.percussion_page_index().unwrap();
        a.tracker_row = 4;
        let page_start = a.tracker_page * LANES_PER_PAGE;
        let pattern = a.song.patterns.get_mut(&0).unwrap();
        pattern.rows[0][page_start].note = Note::On(36);
        pattern.rows[2][page_start + 3].note = Note::On(57);

        let start = Instant::now();
        for (note, velocity) in [(57, 71), (36, 109)] {
            a.tracker_gesture.observe(start, &[0x90, note, velocity]);
            a.tracker_gesture
                .observe(start + Duration::from_millis(2), &[0x80, note, 0]);
        }
        a.commit_tracker_gesture(start + Duration::from_millis(60));

        let row = &a.song.patterns[&0].rows[4][page_start..page_start + LANES_PER_PAGE];
        assert_eq!(row[0].note, Note::On(36));
        assert_eq!(row[0].velocity, Some(109));
        assert_eq!(row[3].note, Note::On(57));
        assert_eq!(row[3].velocity, Some(71));
        assert_eq!(row[1], Cell::default());
        assert_eq!(row[2], Cell::default());
    }

    #[test]
    fn new_kicks_and_snares_get_home_columns_and_family_continuity() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        a.tracker_page = a.percussion_page_index().unwrap();
        let page_start = a.tracker_page * LANES_PER_PAGE;
        a.tracker_single_note(35, 100);
        assert_eq!(a.song.patterns[&0].rows[0][page_start].note, Note::On(35));

        a.tracker_single_note(36, 101);
        assert_eq!(a.song.patterns[&0].rows[1][page_start].note, Note::On(36));

        a.tracker_single_note(38, 102);
        assert_eq!(
            a.song.patterns[&0].rows[2][page_start + 1].note,
            Note::On(38)
        );

        a.tracker_single_note(40, 103);
        assert_eq!(
            a.song.patterns[&0].rows[3][page_start + 1].note,
            Note::On(40)
        );
    }

    #[test]
    fn drum_gesture_preserves_occupied_cells_and_falls_to_a_free_column() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.set_tracker_edit(true);
        a.tracker_page = a.percussion_page_index().unwrap();
        a.tracker_row = 3;
        let page_start = a.tracker_page * LANES_PER_PAGE;
        let pattern = a.song.patterns.get_mut(&0).unwrap();
        pattern.rows[0][page_start + 2].note = Note::On(42);
        pattern.rows[1][page_start + 2].note = Note::On(57);
        pattern.rows[3][page_start + 3] = Cell {
            note: Note::On(51),
            velocity: Some(88),
            command: Command::Retrigger(2),
            ..Cell::default()
        };

        let start = Instant::now();
        for note in [42, 57] {
            a.tracker_gesture.observe(start, &[0x90, note, 99]);
            a.tracker_gesture
                .observe(start + Duration::from_millis(2), &[0x80, note, 0]);
        }
        a.commit_tracker_gesture(start + Duration::from_millis(60));

        let row = &a.song.patterns[&0].rows[3][page_start..page_start + LANES_PER_PAGE];
        assert_eq!(row[2].note, Note::On(42));
        assert_eq!(row[3].note, Note::On(51));
        assert_eq!(row[3].command, Command::Retrigger(2));
        assert_eq!(row[0].note, Note::On(57));
    }

    #[test]
    fn unseen_drum_does_not_consume_an_unrelated_note_off_as_fallback_space() {
        let p = presets();
        let mut a = app(&p);
        a.tracker_page = a.percussion_page_index().unwrap();
        let page_start = a.tracker_page * LANES_PER_PAGE;
        let pattern = a.song.patterns.get_mut(&0).unwrap();
        pattern.rows[0][page_start].note = Note::On(42);
        pattern.rows[0][page_start + 1].note = Note::Off;
        pattern.rows[0][page_start + 2].note = Note::On(46);
        pattern.rows[0][page_start + 3].note = Note::On(51);

        assert_eq!(
            drum_entry_lanes(pattern, 0, a.tracker_page, &[(57, 99)]),
            [None]
        );
        assert_eq!(pattern.rows[0][page_start + 1].note, Note::Off);
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
        a.open_overlay(Action::OpenPatternLengthOverlay);
        a.overlay.as_mut().unwrap().selection = pattern_length_choices()
            .iter()
            .position(|rows| *rows == 24)
            .unwrap();
        a.activate_overlay();
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
        a.open_overlay(Action::OpenPatternLengthOverlay);
        a.overlay.as_mut().unwrap().selection = pattern_length_choices()
            .iter()
            .position(|rows| *rows == 128)
            .unwrap();
        a.activate_overlay();
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
        let percussion = a.percussion_page_index().unwrap();
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
    fn pattern_setup_offers_every_short_length_and_extended_choices() {
        assert_eq!(pattern_sizes(4), [8, 16, 32, 64, 128]);
        assert_eq!(pattern_sizes(3), [6, 12, 24, 48, 96]);
        assert_eq!(
            &pattern_length_choices()[..32],
            (1..=32).collect::<Vec<_>>()
        );
        for rows in [48, 64, 96, 128, 192, 256] {
            assert!(pattern_length_choices().contains(&rows));
        }

        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::TrackerFiles;
        a.new_pattern();
        a.select_pattern_meter(3);
        a.open_overlay(Action::OpenPatternLengthOverlay);
        a.overlay.as_mut().unwrap().selection = pattern_length_choices()
            .iter()
            .position(|rows| *rows == 24)
            .unwrap();
        a.activate_overlay();
        assert_eq!(a.pattern_setup_rows, 24);
        a.apply_pattern_clear();
        assert_eq!(a.song.order, vec![0, 1]);
        assert_eq!(a.song.patterns[&1].rows.len(), 24);
    }

    #[test]
    fn tracker_record_starts_on_an_empty_online_page_and_writes_only_that_page() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        connect_test_midi_hardware(&mut a);
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        let setup = a.current_pattern().unwrap().clone();
        a.song
            .patterns
            .insert(1, sequencer::Pattern::empty_like_setup(8, &setup));
        a.song.order.push(1);
        a.tracker_order = 1;
        a.toggle_tracker_recording();
        assert_eq!(a.menu_context(), MenuContext::TrackerRecord);
        assert!(a.tracker_route.lock().unwrap().preview_state().0);
        a.tracker_noob = true;
        a.sync_tracker_route();

        a.record_tracker_midi(&[0x90, 61, 110]);
        assert_eq!(a.song.patterns[&1].rows[0][0].note, Note::Empty);
        assert_eq!(a.tracker_mode, TrackerMode::Rec);

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
        assert!(!a
            .tracker_recording
            .as_ref()
            .unwrap()
            .active_lanes
            .is_empty());

        a.toggle_tracker_noob();
        assert_eq!(a.tracker_mode, TrackerMode::Rec);
        assert!(a.tracker_recording.is_some());
        assert!(a
            .tracker_recording
            .as_ref()
            .unwrap()
            .active_lanes
            .is_empty());

        a.stop_tracker_recording();
        assert!(a.tracker_route.lock().unwrap().preview_state().0);
    }

    #[test]
    fn tracker_record_keeps_overlapping_same_notes_owned_by_channel_and_instance() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        connect_test_midi_hardware(&mut a);
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.toggle_tracker_recording();

        a.record_tracker_midi(&[0x90, 60, 100]);
        a.record_tracker_midi(&[0x90, 60, 101]);
        a.record_tracker_midi(&[0x91, 60, 102]);
        let recording = a.tracker_recording.as_ref().unwrap();
        assert_eq!(
            recording.active_lanes[&(0, 60)]
                .iter()
                .map(|active| active.lane)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(recording.active_lanes[&(1, 60)][0].lane, 2);

        a.record_tracker_midi(&[0x80, 60, 0]);
        assert_eq!(
            a.tracker_recording.as_ref().unwrap().active_lanes[&(0, 60)][0].lane,
            0
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
    fn tracker_record_writes_release_based_note_off_instead_of_the_edit_length() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        connect_test_midi_hardware(&mut a);
        a.current_page_mut().unwrap().target = PageTarget::ConfiguredExternal;
        a.note_length = NoteLength::Whole;
        a.toggle_tracker_recording();

        a.record_tracker_midi_at(1, &[0x90, 60, 100]);
        a.record_tracker_midi_at(4, &[0x80, 60, 0]);

        let pattern = &a.song.patterns[&0];
        assert_eq!(pattern.rows[1][0].note, Note::On(60));
        assert_eq!(pattern.rows[1][0].gate, Some(100));
        assert_eq!(pattern.rows[4][0].note, Note::Off);
        assert!(a
            .tracker_recording
            .as_ref()
            .unwrap()
            .active_lanes
            .is_empty());
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
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Page2, true))
            .unwrap();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item2, true))
            .unwrap();
        tx.send(MidiEvent::Pad(crate::pads::PadAction::Item4, true))
            .unwrap();
        for _ in 0..7 {
            tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Down))
                .unwrap();
        }
        tx.send(MidiEvent::Encoder(crate::pads::EncoderAction::Select))
            .unwrap();
        drain(&rx, &mut a, Path::new("/none"), &tx);
        assert_eq!(a.tracker_advance, 8);
        assert_eq!(a.tracker_row, (row + 1) % a.tracker_rows());
        assert_eq!(a.song.patterns[&0].rows[row][0].note, Note::Empty);
        let note_row = a.tracker_row;
        a.tracker_single_note(60, 96);
        assert_eq!(a.tracker_row, (note_row + 8) % a.tracker_rows());
        a.select_menu_page(1);
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
        a.tracker_stop();
        assert_eq!(a.screen, Screen::Tracker);
        assert_eq!(a.tracker_mode, TrackerMode::Edit);
    }

    #[test]
    fn noob_toggle_and_edit_note_length_are_independent_controls() {
        let p = presets();
        let mut a = app(&p);
        let (tx, _rx) = mpsc::channel();
        a.screen = Screen::Tracker;
        perform(Action::TrackerEdit, &mut a, Path::new("/none"), None);
        assert_eq!(a.tracker_mode, TrackerMode::Edit);
        assert!(a.tracker_recording.is_none());
        perform(Action::TrackerNoobToggle, &mut a, Path::new("/none"), None);
        assert_eq!(a.screen, Screen::Tracker);
        assert!(a.tracker_noob);
        assert!(a.overlay.is_none());
        assert_eq!(a.menu_context(), MenuContext::TrackerEdit);
        assert_eq!(a.tracker_mode, TrackerMode::Edit);
        a.noob_scale = Scale {
            root: 1,
            kind: ScaleKind::NaturalMinor,
        };
        a.sync_tracker_route();
        a.set_tracker_mode(TrackerMode::Play);
        assert!(a.tracker_noob);
        assert_eq!(a.tracker_mode, TrackerMode::Play);
        a.set_tracker_mode(TrackerMode::Edit);
        assert!(a.tracker_noob);
        perform(Action::TrackerNoobToggle, &mut a, Path::new("/none"), None);
        assert!(!a.tracker_noob);
        assert_eq!(a.tracker_mode, TrackerMode::Edit);
        perform(
            Action::OpenNoteLengthOverlay,
            &mut a,
            Path::new("/none"),
            None,
        );
        assert_eq!(
            a.overlay.as_ref().map(|overlay| overlay.kind),
            Some(OverlayKind::TrackerNoteLength)
        );
        a.controller_layout = ControllerLayout::Four;
        dispatch_encoder(
            crate::pads::EncoderAction::Up,
            &mut a,
            Path::new("/none"),
            &tx,
        );
        assert_eq!(a.overlay.as_ref().map(|overlay| overlay.selection), Some(3));
        dispatch_encoder(
            crate::pads::EncoderAction::Select,
            &mut a,
            Path::new("/none"),
            &tx,
        );
        assert_eq!(a.note_length, NoteLength::Eighth);
        a.set_tracker_mode(TrackerMode::Play);
        assert_eq!(a.tracker_mode, TrackerMode::Play);
        assert!(a.tracker_recording.is_none());
    }

    #[test]
    fn standalone_engine_exit_takes_only_the_standalone_owner() {
        let mut engine = Some("owned engine");
        let mut owner = Some(EngineOwner::Tracker(SoftwareRoute::synthv1(
            "Pattern Sound",
        )));
        assert!(take_engine_when_owned(&mut engine, &mut owner, |owner| {
            *owner == EngineOwner::SoftwareSynth
        })
        .is_none());
        assert_eq!(engine, Some("owned engine"));
        assert_eq!(
            owner,
            Some(EngineOwner::Tracker(SoftwareRoute::synthv1(
                "Pattern Sound"
            )))
        );

        owner = Some(EngineOwner::SoftwareSynth);
        assert_eq!(
            take_engine_when_owned(&mut engine, &mut owner, |owner| {
                *owner == EngineOwner::SoftwareSynth
            }),
            Some("owned engine")
        );
        assert!(engine.is_none());
        assert!(owner.is_none());
    }

    #[test]
    fn leaving_top_level_software_synth_keeps_its_owned_engine_available() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Presets;
        a.engine_owner = Some(EngineOwner::SoftwareSynth);
        a.set_screen(Screen::Home);
        assert_eq!(a.engine_owner, Some(EngineOwner::SoftwareSynth));

        a.screen = Screen::Presets;
        a.engine_owner = Some(EngineOwner::Tracker(SoftwareRoute::synthv1(
            "Pattern Sound",
        )));
        a.set_screen(Screen::Home);
        assert_eq!(
            a.engine_owner,
            Some(EngineOwner::Tracker(SoftwareRoute::synthv1(
                "Pattern Sound"
            )))
        );
    }

    #[test]
    fn fresh_ft2_adopts_the_player_instrument_without_restarting_its_engine() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Playback;
        let engine =
            Engine::start_test_process(BackendKind::Synthv1, Arc::clone(&a.midi_output)).unwrap();
        let process_id = engine.process_id();
        a.engine = Some(engine);
        a.playing = Some(p[38].clone());
        a.engine_owner = Some(EngineOwner::SoftwareSynth);

        perform(Action::OpenTracker, &mut a, Path::new("/none"), None);

        let route = SoftwareRoute::synthv1("Preset 38");
        assert_eq!(a.screen, Screen::Tracker);
        assert_eq!(
            a.current_pages()[0].target,
            PageTarget::Software(route.clone())
        );
        assert_eq!(a.engine_owner, Some(EngineOwner::Tracker(route)));
        assert_eq!(a.engine.as_ref().unwrap().process_id(), process_id);
        assert_eq!(
            a.playing.as_ref().map(Preset::route_id).as_deref(),
            Some("Preset 38")
        );
        assert!(a.status.contains("Player instrument assigned"));
        assert_eq!(
            a.tracker_route.lock().unwrap().destinations(),
            vec![(PageTarget::Software(SoftwareRoute::synthv1("Preset 38")), 0)]
        );
    }

    #[test]
    fn fresh_ft2_without_a_player_loads_the_first_synthv1_for_tracker() {
        let p = presets();
        let mut a = app(&p);
        a.tracker_engine_start_override = Some(Ok(()));

        perform(Action::OpenTracker, &mut a, Path::new("/none"), None);

        let route = SoftwareRoute::synthv1("Preset 00");
        assert_eq!(a.screen, Screen::Tracker);
        assert_eq!(
            a.current_pages()[0].target,
            PageTarget::Software(route.clone())
        );
        assert_eq!(a.engine_owner, Some(EngineOwner::Tracker(route)));
        assert!(a.engine.as_mut().unwrap().alive());
        assert_eq!(
            a.playing.as_ref().map(Preset::route_id).as_deref(),
            Some("Preset 00")
        );
        assert!(a.status.contains("first synthv1 instrument assigned"));
        assert!(a.tracker_route.lock().unwrap().preview_state().0);
    }

    #[test]
    fn saved_or_nondefault_empty_project_never_adopts_the_player_route() {
        let p = presets();
        let explicit = SoftwareRoute::synthv1("Preset 17");
        let mut saved = app(&p);
        saved.current_pattern_mut().unwrap().pages[0].target =
            PageTarget::Software(explicit.clone());
        saved.song_file_stem = Some("saved-empty".into());
        saved.tracker_page = 1;
        saved.tracker_track = 3;
        saved.tracker_row = 11;
        saved.tracker_order = 0;
        saved.screen = Screen::Playback;
        saved.engine = Some(
            Engine::start_test_process(BackendKind::Synthv1, Arc::clone(&saved.midi_output))
                .unwrap(),
        );
        saved.playing = Some(p[38].clone());
        saved.engine_owner = Some(EngineOwner::SoftwareSynth);

        perform(Action::OpenTracker, &mut saved, Path::new("/none"), None);

        assert_eq!(
            saved.song.patterns[&0].pages[0].target,
            PageTarget::Software(explicit.clone())
        );
        assert_eq!(
            (saved.tracker_page, saved.tracker_track, saved.tracker_row),
            (1, 3, 11)
        );
        assert!(!sequencer::pattern_has_note_events(
            &saved.song.patterns[&0]
        ));

        let mut unsaved = app(&p);
        unsaved.current_pattern_mut().unwrap().pages[0].target =
            PageTarget::Software(explicit.clone());
        unsaved.screen = Screen::Playback;
        unsaved.engine = Some(
            Engine::start_test_process(BackendKind::Synthv1, Arc::clone(&unsaved.midi_output))
                .unwrap(),
        );
        unsaved.playing = Some(p[38].clone());
        unsaved.engine_owner = Some(EngineOwner::SoftwareSynth);
        unsaved.tracker_engine_start_override = Some(Ok(()));

        perform(Action::OpenTracker, &mut unsaved, Path::new("/none"), None);

        assert_eq!(
            unsaved.song.patterns[&0].pages[0].target,
            PageTarget::Software(explicit)
        );
        assert_eq!(
            unsaved.engine_owner,
            Some(EngineOwner::Tracker(SoftwareRoute::synthv1("Preset 17")))
        );
    }

    #[test]
    fn arrangement_refuses_to_misroute_two_pattern_owned_synth_presets() {
        let p = presets();
        let mut a = app(&p);
        let empty = sequencer::schedule(&a.song, &a.config.external_midi, 0, 0).unwrap();
        assert_eq!(scheduled_software_route(&empty).unwrap(), None);

        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        let one_route = sequencer::schedule(&a.song, &a.config.external_midi, 0, 0).unwrap();
        assert_eq!(
            scheduled_software_route(&one_route).unwrap(),
            Some(SoftwareRoute::synthv1("Preset 00"))
        );
        let mut second = a.song.patterns[&0].pages[0].clone();
        second.name = "SECOND SYNTH".into();
        second.target = PageTarget::Software(SoftwareRoute::synthv1("Preset 01"));
        let pattern = a.song.patterns.get_mut(&0).unwrap();
        let second_page_lane = pattern.pages.len() * LANES_PER_PAGE;
        pattern.pages.push(second);
        for row in &mut pattern.rows {
            row.extend([Cell::default(); LANES_PER_PAGE]);
        }
        pattern.rows[0][second_page_lane].note = Note::On(64);
        let two_routes = sequencer::schedule(&a.song, &a.config.external_midi, 0, 0).unwrap();
        assert!(scheduled_software_route(&two_routes)
            .unwrap_err()
            .to_string()
            .contains("multiple software instruments"));
    }

    #[test]
    fn fresh_ft2_engine_start_failure_is_actionable_and_never_uses_external_midi() {
        let p = presets();
        let mut a = app(&p);
        a.tracker_engine_start_override = Some(Err("synthetic start failure".into()));
        a.screen = Screen::Tracker;
        a.tracker_page = 1;
        a.sync_tracker_route();
        assert_eq!(
            a.tracker_route.lock().unwrap().destinations(),
            vec![(PageTarget::ConfiguredExternal, 0)]
        );
        a.screen = Screen::Home;
        a.tracker_page = 0;

        perform(Action::OpenTracker, &mut a, Path::new("/none"), None);

        assert!(a.engine.is_none());
        assert!(a.engine_owner.is_none());
        assert!(a.playing.is_none());
        assert!(a.status.contains("FT2 SYNTH START FAILED"));
        assert!(a.status.contains("synthetic start failure"));
        assert_eq!(
            a.tracker_route.lock().unwrap().destinations(),
            vec![(PageTarget::Software(SoftwareRoute::synthv1("Preset 00")), 0)]
        );
        assert!(a.midi_output.lock().unwrap().is_none());
        assert_eq!(a.song.patterns[&0].rows[0][0].note, Note::Empty);
    }

    #[test]
    fn loop_only_transport_does_not_wait_for_an_empty_software_page() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::TrackerLoop;
        a.song.audio_loop = Some(sequencer::LoopSettings {
            file: "ready.wav".into(),
            source_bpm_x100: 12_000,
            interpretation: sequencer::BpmInterpretation::Normal,
            start_beat: 0,
            length_beats: 4,
            offset_beats: 0,
        });
        a.loop_player
            .set_preview_status(crate::loop_player::LoopStatus {
                loaded: true,
                file: Some("ready.wav".into()),
                source_rate: 48_000,
                source_channels: 2,
                duration: Duration::from_secs(2),
                ..crate::loop_player::LoopStatus::default()
            });

        a.toggle_tracker_playback();

        let deadline = Instant::now() + Duration::from_millis(250);
        while !a.sequencer.status().available && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let transport = a.sequencer.status();
        assert!(transport.playing);
        assert!(transport.available);
        assert!(transport.targets.is_empty());
        assert!(a.engine.is_none());
        assert!(a.engine_owner.is_none());
    }

    #[test]
    fn ft2_live_route_follows_the_selected_synth_midi_and_drum_page() {
        let p = presets();
        let mut a = app(&p);
        a.config.external_midi.program_changes = true;
        a.config.external_midi.bank_select = BankSelectMode::Off;
        a.screen = Screen::Tracker;
        a.sync_tracker_route();
        let software_owner = a.engine_owner.clone();
        assert_eq!(
            software_owner,
            Some(EngineOwner::Tracker(SoftwareRoute::synthv1("Preset 00")))
        );
        assert!(matches!(
            a.current_pages()[0].target,
            PageTarget::Software(_)
        ));
        assert!(!a.current_pages()[0].percussion);
        assert_eq!(
            a.tracker_route.lock().unwrap().destinations(),
            vec![(PageTarget::Software(SoftwareRoute::synthv1("Preset 00")), 0)]
        );
        assert!(a.tracker_program_messages(0).is_empty());

        a.tracker_page = 1;
        a.sync_tracker_route();
        assert_eq!(a.engine_owner, software_owner);
        assert_eq!(a.current_pages()[1].target, PageTarget::ConfiguredExternal);
        assert!(!a.current_pages()[1].percussion);
        assert_eq!(
            a.tracker_route.lock().unwrap().destinations(),
            vec![(PageTarget::ConfiguredExternal, 0)]
        );
        assert_eq!(a.tracker_program_messages(0), vec![vec![0xc0, 0]]);

        a.tracker_page = 2;
        a.sync_tracker_route();
        assert_eq!(a.engine_owner, software_owner);
        assert_eq!(a.current_pages()[2].target, PageTarget::ConfiguredExternal);
        assert!(a.current_pages()[2].percussion);
        assert_eq!(
            a.tracker_route.lock().unwrap().destinations(),
            vec![(PageTarget::ConfiguredExternal, 9)]
        );
        assert_eq!(a.tracker_program_messages(0), vec![vec![0xc9, 0]]);
    }

    #[test]
    fn tracker_grid_shows_musician_facing_channel_and_program_one() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_page = 1;
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
        assert!(text.contains("1:01/001"), "{text}");
    }

    #[test]
    fn empty_pattern_routing_default_prompt_confirms_or_cancels_explicitly() {
        let p = presets();
        let mut a = app(&p);
        let base =
            std::env::temp_dir().join(format!("shr-ui-routing-defaults-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        a.routing_defaults_path = base.join("defaults.shsong");
        a.screen = Screen::TrackerFiles;
        a.song.patterns.get_mut(&0).unwrap().pages[1]
            .column_mut(0)
            .channel = 6;
        assert!(a.should_prompt_routing_defaults());
        let original = a.routing_defaults.clone();
        a.save_song();
        assert!(a.confirm_routing_defaults);
        assert_eq!(
            a.status,
            "Save this routing as the default for new patterns?"
        );
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
        assert!(text.contains("Save this routing as the default"), "{text}");
        assert!(text.contains("for new patterns?"), "{text}");
        a.resolve_routing_defaults_choice(false).unwrap();
        assert_eq!(a.routing_defaults, original);
        assert!(!a.routing_defaults_path.exists());

        a.confirm_routing_defaults = true;
        a.resolve_routing_defaults_choice(true).unwrap();
        assert_eq!(a.routing_defaults[1].column(0).channel, 6);
        assert_eq!(
            sequencer::load_routing_defaults(&a.routing_defaults_path, &original).unwrap(),
            a.routing_defaults
        );
        a.song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        a.song.patterns.get_mut(&0).unwrap().pages[1]
            .column_mut(0)
            .channel = 7;
        assert!(!a.should_prompt_routing_defaults());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn later_app_sessions_and_patterns_use_the_saved_routing_defaults() {
        let p = presets();
        let base = std::env::temp_dir().join(format!(
            "shr-ui-later-routing-defaults-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let path = base.join("defaults.shsong");
        let mut defaults = sequencer::factory_routing_pages("Preset 00");
        defaults[1].column_mut(0).channel = 6;
        defaults[1].column_mut(0).program = 41;
        sequencer::save_routing_defaults(&path, &defaults).unwrap();

        let mut a = app_with_routing_defaults(&p, path);
        assert_eq!(a.song.patterns[&0].pages, defaults);
        a.create_pattern(16);
        assert_eq!(a.song.patterns[&1].pages, defaults);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn step_edit_enters_every_supported_note_length_without_linking_row_advance() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_mode = TrackerMode::Edit;
        a.tracker_advance = 2;
        let cases = [
            (NoteLength::Whole, 16, 100),
            (NoteLength::Half, 8, 100),
            (NoteLength::Quarter, 4, 100),
            (NoteLength::Eighth, 2, 100),
            (NoteLength::Sixteenth, 1, 100),
            (NoteLength::ThirtySecond, 1, 50),
        ];
        for (length, span, gate) in cases {
            let pattern = a.song.patterns.get_mut(&0).unwrap();
            for row in &mut pattern.rows {
                row.fill(Cell::default());
            }
            a.tracker_row = 0;
            a.note_length = length;
            a.write_edit_notes(&[(60, 99)]);
            let pattern = &a.song.patterns[&0];
            assert_eq!(pattern.rows[0][0].note, Note::On(60));
            assert_eq!(pattern.rows[0][0].velocity, Some(99));
            assert_eq!(pattern.rows[0][0].gate, Some(gate));
            if gate == 100 {
                assert_eq!(pattern.rows[span][0].note, Note::Off);
            } else {
                assert_eq!(pattern.rows[span][0].note, Note::Empty);
            }
            assert_eq!(a.tracker_row, 2);
            let snapshot = pattern.rows.clone();
            a.set_tracker_mode(TrackerMode::Play);
            a.set_tracker_mode(TrackerMode::Edit);
            assert_eq!(a.song.patterns[&0].rows, snapshot);
        }
    }

    #[test]
    fn noob_filter_is_unavailable_on_a_percussion_page() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_page = a.percussion_page_index().unwrap();
        let before = a.song.patterns[&0].rows.clone();

        a.toggle_tracker_noob();

        assert!(!a.tracker_noob);
        assert_eq!(a.tracker_mode, TrackerMode::Play);
        assert!(a.status.contains("unavailable on Drums"));
        assert_eq!(a.song.patterns[&0].rows, before);
    }

    #[test]
    fn moving_from_noob_to_a_percussion_page_disables_filter_but_keeps_mode() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_page = a.percussion_page_index().unwrap() - 1;
        a.tracker_mode = TrackerMode::Edit;
        a.tracker_noob = true;

        a.move_tracker_page(1);

        assert!(a.current_page().unwrap().percussion);
        assert!(!a.tracker_noob);
        assert_eq!(a.tracker_mode, TrackerMode::Edit);
        assert!(a.status.contains("current FT2 mode unchanged"));
    }

    #[test]
    fn step_edit_with_noob_writes_allowed_notes_and_suppresses_outsiders() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_mode = TrackerMode::Edit;
        a.tracker_noob = true;
        a.tracker_page = 0;
        a.tracker_row = 4;
        let before = a.song.patterns[&0].rows.clone();

        a.tracker_single_note(61, 91);

        assert_eq!(a.tracker_row, 4);
        assert_eq!(a.song.patterns[&0].rows, before);

        a.tracker_single_note(60, 92);

        assert_eq!(a.song.patterns[&0].rows[4][0].note, Note::On(60));
        assert_eq!(a.song.patterns[&0].rows[4][0].velocity, Some(92));
        assert_eq!(a.tracker_row, 5);
    }

    #[test]
    fn playback_noob_toggles_in_place_and_rotary_changes_the_shared_scale() {
        let p = presets();
        let mut a = app(&p);
        let (tx, _rx) = mpsc::channel();
        a.screen = Screen::Playback;
        a.playing = Some(p[0].clone());
        let playing = a.playing.clone();
        let values = a.values.clone();
        a.toggle_playback_noob();
        assert!(a.playback_noob);
        assert_eq!(a.screen, Screen::Playback);
        assert!(a.overlay.is_none());
        assert_eq!(a.playing, playing);
        assert_eq!(a.values, values);
        assert_eq!(*a.playback_scale.lock().unwrap(), Some(a.noob_scale));
        let original = a.noob_scale;

        dispatch_encoder(
            crate::pads::EncoderAction::Down,
            &mut a,
            Path::new("/none"),
            &tx,
        );
        assert_ne!(a.noob_scale, original);
        assert_eq!(*a.playback_scale.lock().unwrap(), Some(a.noob_scale));

        let buffer = render_app(&mut a, 40, 20);
        let text = buffer_text(&buffer);
        assert!(text.contains("SCALE"));
        assert!(text.contains("Flt cut"));

        a.toggle_playback_noob();
        assert!(!a.playback_noob);
        assert_eq!(*a.playback_scale.lock().unwrap(), None);
        assert!(!buffer_text(&render_app(&mut a, 40, 20)).contains("SCALE"));
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
        assert_eq!(b.get(0, 2).fg, Color::Yellow);
        assert_eq!(b.get(0, 10).fg, Color::Yellow);
        assert_eq!(b.get(0, 9).fg, Color::DarkGray);
    }

    #[test]
    fn tracker_selected_column_uses_subtle_background_below_stronger_cursor_styles() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::Tracker;
        a.tracker_track = 1;
        a.tracker_row = 4;
        let b = TestBackend::new(40, 20);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let b = t.backend().buffer();

        assert_eq!(b.get(12, 1).bg, Color::Indexed(234));
        assert_eq!(b.get(12, 2).bg, Color::Indexed(234));
        assert_eq!(b.get(12, 15).bg, Color::Indexed(234));
        assert_eq!(b.get(3, 1).bg, Color::Black);
        assert_eq!(b.get(3, 2).bg, Color::Black);
        assert_eq!(b.get(3, 15).bg, Color::Black);
        assert_eq!(b.get(12, 6).bg, Color::Yellow);
        assert_eq!(b.get(3, 6).bg, Color::DarkGray);
    }

    #[test]
    fn three_four_pattern_has_24_rows_and_marks_one_seven_thirteen() {
        let p = presets();
        let mut a = app(&p);
        a.screen = Screen::TrackerFiles;
        a.choose_pattern_clear();
        a.select_pattern_meter(3);
        a.pattern_setup_rows = 24;
        a.apply_pattern_clear();
        assert_eq!(a.song.patterns[&0].rows.len(), 24);

        a.screen = Screen::Tracker;
        a.tracker_row = 4;
        let b = TestBackend::new(40, 20);
        let mut t = Terminal::new(b).unwrap();
        t.draw(|f| draw(f, &mut a)).unwrap();
        let b = t.backend().buffer();
        assert_eq!(b.get(0, 2).fg, Color::Yellow);
        assert_eq!(b.get(0, 8).fg, Color::Yellow);
        assert_eq!(b.get(0, 14).fg, Color::Yellow);
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
        a.pattern_setup_rows = 32;
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
