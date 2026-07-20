//! Screen-specific four-page controller menus.
//!
//! Labels and dispatch actions deliberately live in the same table.  Physical
//! controller profiles select pages/items; they never encode screen actions.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Screen {
    Presets,
    Playback,
    Ideas,
    Help,
    Tracker,
    TrackerFiles,
    TrackerArrange,
    TrackerPages,
    TrackerTools,
    TrackerNoob,
    TrackerLoop,
    TrackerLoopAlign,
    AudioRecorder,
    FxRack,
    FxEditor,
    Meter,
}

impl Screen {
    pub const COUNT: usize = 16;
    #[cfg(test)]
    pub const ALL: [Self; 16] = [
        Self::Presets,
        Self::Playback,
        Self::Ideas,
        Self::Help,
        Self::Tracker,
        Self::TrackerFiles,
        Self::TrackerArrange,
        Self::TrackerPages,
        Self::TrackerTools,
        Self::TrackerNoob,
        Self::TrackerLoop,
        Self::TrackerLoopAlign,
        Self::AudioRecorder,
        Self::FxRack,
        Self::FxEditor,
        Self::Meter,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::Presets => 0,
            Self::Playback => 1,
            Self::Ideas => 2,
            Self::Help => 3,
            Self::Tracker => 4,
            Self::TrackerFiles => 5,
            Self::TrackerArrange => 6,
            Self::TrackerPages => 7,
            Self::TrackerTools => 8,
            Self::TrackerNoob => 9,
            Self::TrackerLoop => 10,
            Self::TrackerLoopAlign => 11,
            Self::AudioRecorder => 12,
            Self::FxRack => 13,
            Self::FxEditor => 14,
            Self::Meter => 15,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Presets => "PRESETS",
            Self::Playback => "PLAYBACK",
            Self::Ideas => "IDEAS",
            Self::Help => "HELP",
            Self::Tracker => "FT2",
            Self::TrackerFiles => "FILES",
            Self::TrackerArrange => "ARRANGE",
            Self::TrackerPages => "TRACKS",
            Self::TrackerTools => "FT2 TOOLS",
            Self::TrackerNoob => "N00B SETUP",
            Self::TrackerLoop => "FT2 LOOP",
            Self::TrackerLoopAlign => "LOOP ALIGN",
            Self::AudioRecorder => "AUDIO",
            Self::FxRack => "FX RACK",
            Self::FxEditor => "FX EDIT",
            Self::Meter => "MTR",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Action {
    Noop,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    PreviousEngine,
    NextEngine,
    Activate,
    Back,
    Quit,
    StopAll,
    OpenPresets,
    OpenIdeas,
    OpenHelp,
    OpenTracker,
    OpenTrackerFiles,
    OpenTrackerArrange,
    OpenTrackerPages,
    OpenTrackerTools,
    OpenTrackerLoop,
    OpenTrackerLoopAlign,
    OpenAudioRecorder,
    OpenFxRack,
    OpenFxEditor,
    OpenMeter,
    ResetMeter,
    BusSelectPrevious,
    BusSelectNext,
    BusLevelDecrease,
    BusLevelIncrease,
    BusMute,
    FinalRecordToggle,
    FxAdd,
    FxRemove,
    FxMoveUp,
    FxMoveDown,
    FxBypass,
    FxKindPrevious,
    FxKindNext,
    FxTargetNext,
    FxSendDecrease,
    FxSendIncrease,
    FxSendPoint,
    FxReturnCycle,
    FxParameterPrevious,
    FxParameterNext,
    FxValueDecrease,
    FxValueIncrease,
    TapTempo,
    ResetParameters,
    BeginRecord,
    StopRecord,
    FinishSaveRecord,
    SaveNew,
    InspectIdea,
    DeleteIdea,
    LoadIdea,
    PlaybackRecording,
    StopPlayback,
    TrackerEdit,
    TrackerSkip,
    TrackerErase,
    TrackerNoteOff,
    TrackerAdvance1,
    TrackerAdvance2,
    TrackerAdvance4,
    TrackerAdvance8,
    OpenNoteEditor,
    NoteField,
    GateField,
    VelocityField,
    ProgramField,
    EffectField,
    EffectParameterField,
    NoteEditorClearField,
    NoteEditorPreviousField,
    NoteEditorNextField,
    NoteEditorDecrease,
    NoteEditorIncrease,
    NoteEditorConfirm,
    NoteEditorCancel,
    TrackerPlayCursor,
    TrackerPlayStart,
    TrackerRecord,
    TrackerModePlay,
    TrackerModeEdit,
    TrackerModeNoob,
    NoobRootDown,
    NoobRootUp,
    NoobScale,
    ConfirmNoob,
    LoopImport,
    LoopRemove,
    LoopSourceDown,
    LoopSourceUp,
    LoopBpmMode,
    LoopEditUnit,
    LoopStartDown,
    LoopStartUp,
    LoopLengthDown,
    LoopLengthUp,
    LoopAutoAlign,
    LoopOffsetDown,
    LoopOffsetUp,
    LoopAlignDone,
    OpenLoopLibrary,
    DeleteLoopFile,
    TrackerStop,
    TrackerMute,
    TrackerPageMute,
    NextTrackerPage,
    PreviousTrack,
    NextTrack,
    PreviousProgram,
    NextProgram,
    BankMsbDown,
    BankMsbUp,
    BankLsbDown,
    BankLsbUp,
    SaveSong,
    SaveSongAs,
    LoadSong,
    PreviewSong,
    DeleteSong,
    RenameProject,
    NewProject,
    NewPattern,
    ClonePattern,
    PastePatternOver,
    ClearPattern,
    ClearPatternNow,
    DeleteUnusedPattern,
    OpenPatternTools,
    OpenDrumPatterns,
    CopyPattern,
    PastePatternNew,
    TransposeDownOctave,
    TransposeDownSemitone,
    TransposeUpSemitone,
    TransposeUpOctave,
    LoadDrumPattern,
    SaveDrumPattern,
    DeleteDrumPattern,
    DrumGenreDown,
    DrumGenreUp,
    DrumMeter,
    DrumSize,
    CopyLane,
    PasteLane,
    CopyPage,
    PastePage,
    ArrangementAppend,
    ArrangementInsert,
    ArrangementRemove,
    ArrangementDuplicate,
    ArrangementMoveEarlier,
    ArrangementMoveLater,
    ArrangementJumpToPattern,
    ArrangementPlayFromStep,
    PreviousOrder,
    NextOrder,
    AddPage,
    EditPageTarget,
    EditPageChannel,
    ConfirmPageManager,
    SelectThreeFour,
    SelectFourFour,
    PatternSizeDown,
    PatternSizeUp,
    ConfirmPatternClear,
    AudioRecord,
    AudioStop,
    AudioToggleArm,
    AudioArmAll,
    AudioDisarmAll,
    AudioPreviousTrack,
    AudioNextTrack,
    AudioAssignSource,
    AudioNameTrack,
    AudioRefreshSources,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotState {
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MenuSlot {
    pub label: &'static str,
    pub action: Action,
    pub state: SlotState,
}

impl MenuSlot {
    pub const fn enabled(label: &'static str, action: Action) -> Self {
        Self {
            label,
            action,
            state: SlotState::Enabled,
        }
    }
    pub const fn disabled(label: &'static str) -> Self {
        Self {
            label,
            action: Action::Noop,
            state: SlotState::Disabled,
        }
    }
    pub const fn dispatch(self) -> Option<Action> {
        match self.state {
            SlotState::Enabled => Some(self.action),
            SlotState::Disabled => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MenuPage {
    pub label: &'static str,
    pub slots: [MenuSlot; 4],
}

impl MenuPage {
    pub fn available(self) -> bool {
        self.slots.iter().any(|slot| slot.dispatch().is_some())
    }
}

const fn page(label: &'static str, slots: [MenuSlot; 4]) -> MenuPage {
    MenuPage { label, slots }
}
const fn on(label: &'static str, action: Action) -> MenuSlot {
    MenuSlot::enabled(label, action)
}
const fn off(label: &'static str) -> MenuSlot {
    MenuSlot::disabled(label)
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MenuContext {
    #[default]
    Normal,
    TrackerEdit,
    TrackerRecord,
    TrackerNoteEdit,
    PageTarget,
    PageChannel,
    PatternClear,
    LoopLibrary,
    PatternTools,
    DrumPatterns,
}

const PRESETS: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("LOAD", Action::Activate),
            on("PG UP", Action::PageUp),
            on("PG DOWN", Action::PageDown),
            on("FIRST", Action::Home),
        ],
    ),
    page(
        "ENGINE",
        [
            on("ENGINE-", Action::PreviousEngine),
            on("ENGINE+", Action::NextEngine),
            off(""),
            on("LAST", Action::End),
        ],
    ),
    page(
        "NAV",
        [
            on("MTR", Action::OpenMeter),
            on("IDEAS", Action::OpenIdeas),
            on("FT2", Action::OpenTracker),
            on("AUDIO", Action::OpenAudioRecorder),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("HELP", Action::OpenHelp),
            off(""),
            off(""),
        ],
    ),
];
const PLAYBACK: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("RECORD", Action::BeginRecord),
            on("REC END", Action::StopRecord),
            on("TAKE", Action::PlaybackRecording),
            on("SAVE", Action::SaveNew),
        ],
    ),
    page(
        "SOUND",
        [
            on("RESET", Action::ResetParameters),
            on("FINISH", Action::FinishSaveRecord),
            on("TAP", Action::TapTempo),
            on("FX", Action::OpenFxRack),
        ],
    ),
    page(
        "NAV",
        [
            on("PRESETS", Action::OpenPresets),
            on("IDEAS", Action::OpenIdeas),
            on("FT2", Action::OpenTracker),
            on("AUDIO", Action::OpenAudioRecorder),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::StopPlayback),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const IDEAS: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("INSPECT", Action::InspectIdea),
            on("LOAD", Action::LoadIdea),
            on("PLAY", Action::PlaybackRecording),
            on("DELETE", Action::DeleteIdea),
        ],
    ),
    page(
        "CAPTURE",
        [
            on("RECORD", Action::BeginRecord),
            on("REC END", Action::StopRecord),
            on("SAVE", Action::SaveNew),
            on("FIRST", Action::Home),
        ],
    ),
    page(
        "NAV",
        [
            on("PRESETS", Action::OpenPresets),
            on("HELP", Action::OpenHelp),
            on("FT2", Action::OpenTracker),
            on("AUDIO", Action::OpenAudioRecorder),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::StopPlayback),
            on("LAST", Action::End),
            on("EXIT", Action::Back),
        ],
    ),
];
const TRACKER: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("PLAY", Action::TrackerPlayCursor),
            on("START", Action::TrackerPlayStart),
            on("STEP", Action::TrackerEdit),
            on("CELL", Action::OpenNoteEditor),
        ],
    ),
    page(
        "MODE",
        [
            on("PLAY", Action::TrackerModePlay),
            on("REC", Action::TrackerRecord),
            on("EDIT", Action::TrackerModeEdit),
            on("N00B", Action::TrackerModeNoob),
        ],
    ),
    page(
        "MOVE",
        [
            on("PG-", Action::PreviousOrder),
            on("PG+", Action::NextOrder),
            on("LANE-", Action::PreviousTrack),
            on("LANE+", Action::NextTrack),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("TOOLS", Action::OpenTrackerTools),
            on("EXIT", Action::Back),
        ],
    ),
];
const TRACKER_TOOLS: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("PAGES", Action::OpenTrackerPages),
            on("FILES", Action::OpenTrackerFiles),
            on("ARR", Action::OpenTrackerArrange),
            on("MUTE", Action::TrackerMute),
        ],
    ),
    page(
        "CLIP",
        [
            on("COPY L", Action::CopyLane),
            on("PASTE L", Action::PasteLane),
            on("COPY PG", Action::CopyPage),
            on("PSTE PG", Action::PastePage),
        ],
    ),
    page(
        "LOOP",
        [
            on("LOOP", Action::OpenTrackerLoop),
            on("REMOVE", Action::LoopRemove),
            on("LIBRARY", Action::OpenLoopLibrary),
            on("MUTE PG", Action::TrackerPageMute),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const ARRANGE: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("PLAY", Action::ArrangementPlayFromStep),
            on("JUMP", Action::ArrangementJumpToPattern),
            on("APPEND", Action::ArrangementAppend),
            on("INSERT", Action::ArrangementInsert),
        ],
    ),
    page(
        "STEP",
        [
            on("UP", Action::ArrangementMoveEarlier),
            on("DOWN", Action::ArrangementMoveLater),
            on("REPEAT", Action::ArrangementDuplicate),
            on("REMOVE", Action::ArrangementRemove),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const TRACKER_NOOB: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("ROOT-", Action::NoobRootDown),
            on("ROOT+", Action::NoobRootUp),
            on("SCALE", Action::NoobScale),
            on("DONE", Action::ConfirmNoob),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const TRACKER_LOOP: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("IMPORT", Action::LoopImport),
            on("HERE", Action::TrackerPlayCursor),
            on("START", Action::TrackerPlayStart),
            on("STOP", Action::TrackerStop),
        ],
    ),
    page(
        "BPM",
        [
            on("BPM-", Action::LoopSourceDown),
            on("BPM+", Action::LoopSourceUp),
            on("BPM x", Action::LoopBpmMode),
            on("UNIT", Action::LoopEditUnit),
        ],
    ),
    page(
        "CUT",
        [
            on("START-", Action::LoopStartDown),
            on("START+", Action::LoopStartUp),
            on("LEN-", Action::LoopLengthDown),
            on("LEN+", Action::LoopLengthUp),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("ALIGN", Action::OpenTrackerLoopAlign),
            on("EXIT", Action::Back),
        ],
    ),
];
const LOOP_LIBRARY: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("DELETE", Action::DeleteLoopFile),
            on("PG UP", Action::PageUp),
            on("PG DOWN", Action::PageDown),
            off(""),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const TRACKER_LOOP_ALIGN: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("AUTO", Action::LoopAutoAlign),
            on("BAR-", Action::LoopOffsetDown),
            on("BAR+", Action::LoopOffsetUp),
            on("DONE", Action::LoopAlignDone),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const TRACKER_RECORD: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("REC END", Action::TrackerRecord),
            off(""),
            off(""),
            off(""),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const TRACKER_EDIT: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("BLANK", Action::TrackerSkip),
            on("ERASE", Action::TrackerErase),
            on("N-OFF", Action::TrackerNoteOff),
            on("DONE", Action::TrackerEdit),
        ],
    ),
    page(
        "MOVE",
        [
            on("PG-", Action::PreviousOrder),
            on("PG+", Action::NextOrder),
            on("LANE-", Action::PreviousTrack),
            on("LANE+", Action::NextTrack),
        ],
    ),
    page(
        "ADD",
        [
            on("1", Action::TrackerAdvance1),
            on("2", Action::TrackerAdvance2),
            on("4", Action::TrackerAdvance4),
            on("8", Action::TrackerAdvance8),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("PAGE", Action::NextTrackerPage),
            on("EXIT", Action::TrackerEdit),
        ],
    ),
];
const TRACKER_NOTE_EDIT: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("CONFIRM", Action::NoteEditorConfirm),
            on("STEP", Action::TrackerEdit),
            on("CLEAR", Action::NoteEditorClearField),
            on("EFFECT", Action::EffectField),
        ],
    ),
    page(
        "FIELDS",
        [
            on("NOTE", Action::NoteField),
            on("GATE", Action::GateField),
            on("VEL", Action::VelocityField),
            on("PROGRAM", Action::ProgramField),
        ],
    ),
    page(
        "ADJUST",
        [
            on("PARAM", Action::EffectParameterField),
            on("VALUE-", Action::NoteEditorDecrease),
            on("VALUE+", Action::NoteEditorIncrease),
            off(""),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            off(""),
            on("EXIT", Action::Back),
        ],
    ),
];
const FILES: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("LOAD", Action::LoadSong),
            on("SAVE", Action::SaveSong),
            on("PREVIEW", Action::PreviewSong),
            on("DELETE", Action::DeleteSong),
        ],
    ),
    page(
        "PROJECT",
        [
            on("NEW PRJ", Action::NewProject),
            on("SAVE AS", Action::SaveSongAs),
            on("NAME", Action::RenameProject),
            on("PATTERN", Action::OpenPatternTools),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const PATTERN_TOOLS: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("NEW", Action::NewPattern),
            on("CLONE", Action::ClonePattern),
            on("CLEAR", Action::ClearPattern),
            on("DRUMS", Action::OpenDrumPatterns),
        ],
    ),
    page(
        "CLIP",
        [
            on("COPY", Action::CopyPattern),
            on("NEW", Action::PastePatternNew),
            on("OVER", Action::PastePatternOver),
            on("CLEAN", Action::DeleteUnusedPattern),
        ],
    ),
    page(
        "TRANS",
        [
            on("OCT-", Action::TransposeDownOctave),
            on("NOTE-", Action::TransposeDownSemitone),
            on("NOTE+", Action::TransposeUpSemitone),
            on("OCT+", Action::TransposeUpOctave),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const DRUM_PATTERNS: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("LOAD", Action::LoadDrumPattern),
            on("SAVE", Action::SaveDrumPattern),
            on("DELETE", Action::DeleteDrumPattern),
            off(""),
        ],
    ),
    page(
        "FILTER",
        [
            on("GENRE-", Action::DrumGenreDown),
            on("GENRE+", Action::DrumGenreUp),
            on("METER", Action::DrumMeter),
            on("SIZE", Action::DrumSize),
        ],
    ),
    page(
        "MOVE",
        [
            on("PG UP", Action::PageUp),
            on("PG DOWN", Action::PageDown),
            on("FIRST", Action::Home),
            on("LAST", Action::End),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const PAGES: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("ADD", Action::AddPage),
            on("TARGET", Action::EditPageTarget),
            on("CHANNEL", Action::EditPageChannel),
            on("DONE", Action::ConfirmPageManager),
        ],
    ),
    page(
        "COLUMN",
        [
            on("COL-", Action::PreviousTrack),
            on("COL+", Action::NextTrack),
            on("PROG-", Action::PreviousProgram),
            on("PROG+", Action::NextProgram),
        ],
    ),
    page(
        "BANK",
        [
            on("MSB-", Action::BankMsbDown),
            on("MSB+", Action::BankMsbUp),
            on("LSB-", Action::BankLsbDown),
            on("LSB+", Action::BankLsbUp),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const PAGE_FIELD: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("CONFIRM", Action::ConfirmPageManager),
            off(""),
            off(""),
            off(""),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::TrackerStop),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const PATTERN_CLEAR: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("3/4", Action::SelectThreeFour),
            on("4/4", Action::SelectFourFour),
            on("SIZE-", Action::PatternSizeDown),
            on("SIZE+", Action::PatternSizeUp),
        ],
    ),
    page(
        "APPLY",
        [
            on("CONFIRM", Action::ConfirmPatternClear),
            on("KEEP", Action::ClearPatternNow),
            off(""),
            off(""),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            off(""),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const AUDIO: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("RECORD", Action::AudioRecord),
            on("ARM", Action::AudioToggleArm),
            on("ALL", Action::AudioArmAll),
            on("NONE", Action::AudioDisarmAll),
        ],
    ),
    page(
        "TRACK",
        [
            on("PREV", Action::AudioPreviousTrack),
            on("NEXT", Action::AudioNextTrack),
            on("SOURCE", Action::AudioAssignSource),
            on("NAME", Action::AudioNameTrack),
        ],
    ),
    page(
        "NAV",
        [
            on("REFRESH", Action::AudioRefreshSources),
            on("PRESETS", Action::OpenPresets),
            on("IDEAS", Action::OpenIdeas),
            on("FT2", Action::OpenTracker),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("STOP", Action::AudioStop),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];

const FX_RACK: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("EDIT", Action::OpenFxEditor),
            on("ADD", Action::FxAdd),
            on("BYPASS", Action::FxBypass),
            on("REMOVE", Action::FxRemove),
        ],
    ),
    page(
        "ORDER",
        [
            on("UP", Action::FxMoveUp),
            on("DOWN", Action::FxMoveDown),
            on("KIND-", Action::FxKindPrevious),
            on("KIND+", Action::FxKindNext),
        ],
    ),
    page(
        "ROUTE",
        [
            on("TARGET", Action::FxTargetNext),
            on("SEND-", Action::FxSendDecrease),
            on("SEND+", Action::FxSendIncrease),
            on("POINT", Action::FxSendPoint),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("RETURN", Action::FxReturnCycle),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];

const FX_EDITOR: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("PARAM-", Action::FxParameterPrevious),
            on("PARAM+", Action::FxParameterNext),
            on("VALUE-", Action::FxValueDecrease),
            on("VALUE+", Action::FxValueIncrease),
        ],
    ),
    page(
        "STATE",
        [on("BYPASS", Action::FxBypass), off(""), off(""), off("")],
    ),
    page(
        "NAV",
        [on("RACK", Action::OpenFxRack), off(""), off(""), off("")],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            off(""),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];

const METER: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("SOURCE-", Action::BusSelectPrevious),
            on("SOURCE+", Action::BusSelectNext),
            on("LEVEL-", Action::BusLevelDecrease),
            on("LEVEL+", Action::BusLevelIncrease),
        ],
    ),
    page(
        "MIX",
        [
            on("MUTE", Action::BusMute),
            on("REC", Action::FinalRecordToggle),
            on("RESET", Action::ResetMeter),
            off(""),
        ],
    ),
    page(
        "NAV",
        [on("FX", Action::OpenFxRack), off(""), off(""), off("")],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            off(""),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];

const HELP: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("OPEN", Action::Activate),
            on("PG UP", Action::PageUp),
            on("PG DOWN", Action::PageDown),
            on("TOP", Action::Home),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            off(""),
            off(""),
            on("EXIT", Action::Back),
        ],
    ),
];

pub fn pages(screen: Screen, context: MenuContext) -> &'static [MenuPage; 4] {
    match (screen, context) {
        (Screen::Presets, _) => &PRESETS,
        (Screen::Playback, _) => &PLAYBACK,
        (Screen::Ideas, _) => &IDEAS,
        (Screen::Help, _) => &HELP,
        (Screen::Tracker, MenuContext::TrackerNoteEdit) => &TRACKER_NOTE_EDIT,
        (Screen::Tracker, MenuContext::TrackerRecord) => &TRACKER_RECORD,
        (Screen::Tracker, MenuContext::TrackerEdit) => &TRACKER_EDIT,
        (Screen::Tracker, _) => &TRACKER,
        (Screen::TrackerFiles, MenuContext::PatternClear) => &PATTERN_CLEAR,
        (Screen::TrackerFiles, MenuContext::PatternTools) => &PATTERN_TOOLS,
        (Screen::TrackerFiles, MenuContext::DrumPatterns) => &DRUM_PATTERNS,
        (Screen::TrackerFiles, _) => &FILES,
        (Screen::TrackerArrange, _) => &ARRANGE,
        (Screen::TrackerPages, MenuContext::PageTarget | MenuContext::PageChannel) => &PAGE_FIELD,
        (Screen::TrackerPages, _) => &PAGES,
        (Screen::TrackerTools, _) => &TRACKER_TOOLS,
        (Screen::TrackerNoob, _) => &TRACKER_NOOB,
        (Screen::TrackerLoop, MenuContext::LoopLibrary) => &LOOP_LIBRARY,
        (Screen::TrackerLoop, _) => &TRACKER_LOOP,
        (Screen::TrackerLoopAlign, _) => &TRACKER_LOOP_ALIGN,
        (Screen::AudioRecorder, _) => &AUDIO,
        (Screen::FxRack, _) => &FX_RACK,
        (Screen::FxEditor, _) => &FX_EDITOR,
        (Screen::Meter, _) => &METER,
    }
}

pub fn slot(screen: Screen, context: MenuContext, page: usize, item: usize) -> Option<MenuSlot> {
    pages(screen, context).get(page)?.slots.get(item).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_screen_and_context_has_exactly_four_pages_of_four_slots() {
        for screen in Screen::ALL {
            for context in [
                MenuContext::Normal,
                MenuContext::TrackerEdit,
                MenuContext::TrackerRecord,
                MenuContext::TrackerNoteEdit,
                MenuContext::PageTarget,
                MenuContext::PageChannel,
                MenuContext::PatternClear,
                MenuContext::LoopLibrary,
                MenuContext::PatternTools,
                MenuContext::DrumPatterns,
            ] {
                let menu = pages(screen, context);
                assert_eq!(menu.len(), 4);
                assert!(menu.iter().all(|page| page.slots.len() == 4));
                assert!(menu
                    .iter()
                    .flat_map(|page| page.slots)
                    .all(|slot| slot.state == SlotState::Disabled || !slot.label.is_empty()));
            }
        }
    }

    #[test]
    fn empty_slots_and_pages_do_not_dispatch() {
        let empty_slot = slot(Screen::Meter, MenuContext::Normal, 1, 3).unwrap();
        let empty_page = pages(Screen::Help, MenuContext::Normal)[1];
        assert_eq!((empty_slot.label, empty_slot.dispatch()), ("", None));
        assert!(!empty_page.available());
    }

    #[test]
    fn forty_column_controller_labels_fit_without_truncation() {
        const MAX_BUTTON_TEXT: usize = 7;
        for screen in Screen::ALL {
            for context in [
                MenuContext::Normal,
                MenuContext::TrackerEdit,
                MenuContext::TrackerRecord,
                MenuContext::TrackerNoteEdit,
                MenuContext::PageTarget,
                MenuContext::PageChannel,
                MenuContext::PatternClear,
            ] {
                for page in pages(screen, context) {
                    assert!(
                        page.label.len() <= MAX_BUTTON_TEXT,
                        "{screen:?} {context:?} page label {:?} is too wide",
                        page.label
                    );
                    for slot in page.slots {
                        assert!(
                            slot.label.len() <= MAX_BUTTON_TEXT,
                            "{screen:?} {context:?} slot label {:?} is too wide",
                            slot.label
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_menu_uses_the_same_ops_and_system_anchors() {
        let contexts = [
            (Screen::Presets, MenuContext::Normal),
            (Screen::Playback, MenuContext::Normal),
            (Screen::Ideas, MenuContext::Normal),
            (Screen::Help, MenuContext::Normal),
            (Screen::Tracker, MenuContext::Normal),
            (Screen::Tracker, MenuContext::TrackerEdit),
            (Screen::Tracker, MenuContext::TrackerRecord),
            (Screen::Tracker, MenuContext::TrackerNoteEdit),
            (Screen::TrackerFiles, MenuContext::Normal),
            (Screen::TrackerFiles, MenuContext::PatternClear),
            (Screen::TrackerFiles, MenuContext::PatternTools),
            (Screen::TrackerFiles, MenuContext::DrumPatterns),
            (Screen::TrackerPages, MenuContext::Normal),
            (Screen::TrackerPages, MenuContext::PageTarget),
            (Screen::TrackerTools, MenuContext::Normal),
            (Screen::TrackerNoob, MenuContext::Normal),
            (Screen::TrackerLoop, MenuContext::Normal),
            (Screen::TrackerLoop, MenuContext::LoopLibrary),
            (Screen::TrackerLoopAlign, MenuContext::Normal),
            (Screen::AudioRecorder, MenuContext::Normal),
            (Screen::Meter, MenuContext::Normal),
        ];
        for (screen, context) in contexts {
            let menu = pages(screen, context);
            assert_eq!(menu[0].label, "OPS", "{screen:?} {context:?}");
            assert_eq!(menu[3].label, "SYS", "{screen:?} {context:?}");
            assert_eq!(menu[3].slots[0].label, "PANIC");
            if screen == Screen::Presets {
                assert_eq!(menu[3].slots[3].dispatch(), None);
            } else {
                assert_eq!(menu[3].slots[3].label, "EXIT");
                assert!(menu[3].slots[3].dispatch().is_some());
            }
            assert!(menu
                .iter()
                .flat_map(|page| page.slots)
                .all(|slot| slot.dispatch() != Some(Action::Quit)));
        }
    }

    #[test]
    fn note_editor_has_direct_access_to_every_field_and_core_operation() {
        let menu = pages(Screen::Tracker, MenuContext::TrackerNoteEdit);
        assert_eq!(menu.len(), 4);
        let actions = menu
            .iter()
            .flat_map(|page| page.slots)
            .filter_map(MenuSlot::dispatch)
            .collect::<HashSet<_>>();
        for action in [
            Action::NoteField,
            Action::GateField,
            Action::VelocityField,
            Action::ProgramField,
            Action::EffectField,
            Action::EffectParameterField,
            Action::NoteEditorDecrease,
            Action::NoteEditorIncrease,
            Action::NoteEditorConfirm,
            Action::TrackerEdit,
            Action::TrackerStop,
            Action::StopAll,
            Action::Back,
        ] {
            assert!(actions.contains(&action), "missing {action:?}");
        }
    }

    #[test]
    fn contextual_menus_replace_ambiguous_actions() {
        assert_eq!(
            slot(Screen::Tracker, MenuContext::TrackerNoteEdit, 0, 0)
                .unwrap()
                .action,
            Action::NoteEditorConfirm
        );
        assert_eq!(
            slot(Screen::Tracker, MenuContext::TrackerEdit, 0, 1)
                .unwrap()
                .action,
            Action::TrackerErase
        );
        assert_eq!(
            slot(Screen::TrackerPages, MenuContext::PageTarget, 0, 0)
                .unwrap()
                .action,
            Action::ConfirmPageManager
        );
        assert_eq!(
            slot(Screen::TrackerFiles, MenuContext::PatternClear, 3, 3)
                .unwrap()
                .action,
            Action::Back
        );
    }

    #[test]
    fn master_rotary_navigation_is_not_duplicated_on_menu_slots() {
        for (screen, context) in [
            (Screen::Presets, MenuContext::Normal),
            (Screen::Ideas, MenuContext::Normal),
            (Screen::TrackerFiles, MenuContext::Normal),
            (Screen::TrackerPages, MenuContext::Normal),
            (Screen::TrackerPages, MenuContext::PageTarget),
            (Screen::TrackerPages, MenuContext::PageChannel),
        ] {
            assert!(pages(screen, context)
                .iter()
                .flat_map(|page| page.slots)
                .all(|slot| !matches!(slot.dispatch(), Some(Action::Up | Action::Down))));
        }
        assert_eq!(
            TRACKER[2].slots.map(|slot| slot.dispatch()),
            [
                Some(Action::PreviousOrder),
                Some(Action::NextOrder),
                Some(Action::PreviousTrack),
                Some(Action::NextTrack),
            ]
        );
    }

    #[test]
    fn inventoried_controller_workflow_actions_are_all_reachable() {
        let contexts = [
            (Screen::Presets, MenuContext::Normal),
            (Screen::Playback, MenuContext::Normal),
            (Screen::Ideas, MenuContext::Normal),
            (Screen::Help, MenuContext::Normal),
            (Screen::Tracker, MenuContext::Normal),
            (Screen::Tracker, MenuContext::TrackerEdit),
            (Screen::Tracker, MenuContext::TrackerNoteEdit),
            (Screen::TrackerFiles, MenuContext::Normal),
            (Screen::TrackerFiles, MenuContext::PatternClear),
            (Screen::TrackerFiles, MenuContext::PatternTools),
            (Screen::TrackerFiles, MenuContext::DrumPatterns),
            (Screen::TrackerPages, MenuContext::Normal),
            (Screen::TrackerPages, MenuContext::PageTarget),
            (Screen::TrackerPages, MenuContext::PageChannel),
            (Screen::TrackerTools, MenuContext::Normal),
            (Screen::TrackerNoob, MenuContext::Normal),
            (Screen::TrackerLoop, MenuContext::Normal),
            (Screen::TrackerLoop, MenuContext::LoopLibrary),
            (Screen::TrackerLoopAlign, MenuContext::Normal),
            (Screen::AudioRecorder, MenuContext::Normal),
            (Screen::Meter, MenuContext::Normal),
        ];
        let reachable = contexts
            .into_iter()
            .flat_map(|(screen, context)| pages(screen, context))
            .flat_map(|page| page.slots)
            .filter_map(MenuSlot::dispatch)
            .collect::<HashSet<_>>();
        let inventory = [
            Action::PageUp,
            Action::PageDown,
            Action::Home,
            Action::End,
            Action::PreviousEngine,
            Action::NextEngine,
            Action::Activate,
            Action::Back,
            Action::StopAll,
            Action::OpenPresets,
            Action::OpenIdeas,
            Action::OpenHelp,
            Action::OpenTracker,
            Action::OpenTrackerFiles,
            Action::OpenTrackerPages,
            Action::OpenTrackerTools,
            Action::OpenTrackerLoop,
            Action::OpenTrackerLoopAlign,
            Action::OpenAudioRecorder,
            Action::OpenMeter,
            Action::ResetMeter,
            Action::BusSelectPrevious,
            Action::BusSelectNext,
            Action::BusLevelDecrease,
            Action::BusLevelIncrease,
            Action::BusMute,
            Action::FinalRecordToggle,
            Action::TapTempo,
            Action::ResetParameters,
            Action::BeginRecord,
            Action::StopRecord,
            Action::FinishSaveRecord,
            Action::SaveNew,
            Action::InspectIdea,
            Action::DeleteIdea,
            Action::LoadIdea,
            Action::PlaybackRecording,
            Action::StopPlayback,
            Action::TrackerEdit,
            Action::TrackerSkip,
            Action::TrackerErase,
            Action::TrackerNoteOff,
            Action::OpenNoteEditor,
            Action::NoteField,
            Action::GateField,
            Action::VelocityField,
            Action::ProgramField,
            Action::EffectField,
            Action::EffectParameterField,
            Action::NoteEditorClearField,
            Action::NoteEditorDecrease,
            Action::NoteEditorIncrease,
            Action::NoteEditorConfirm,
            Action::TrackerPlayCursor,
            Action::TrackerPlayStart,
            Action::TrackerRecord,
            Action::TrackerModePlay,
            Action::TrackerModeEdit,
            Action::TrackerModeNoob,
            Action::NoobRootDown,
            Action::NoobRootUp,
            Action::NoobScale,
            Action::ConfirmNoob,
            Action::TrackerStop,
            Action::TrackerMute,
            Action::TrackerPageMute,
            Action::NextTrackerPage,
            Action::PreviousTrack,
            Action::NextTrack,
            Action::PreviousProgram,
            Action::NextProgram,
            Action::BankMsbDown,
            Action::BankMsbUp,
            Action::BankLsbDown,
            Action::BankLsbUp,
            Action::SaveSong,
            Action::SaveSongAs,
            Action::LoadSong,
            Action::PreviewSong,
            Action::DeleteSong,
            Action::RenameProject,
            Action::NewProject,
            Action::NewPattern,
            Action::ClonePattern,
            Action::ClearPattern,
            Action::ClearPatternNow,
            Action::DeleteUnusedPattern,
            Action::PreviousOrder,
            Action::NextOrder,
            Action::AddPage,
            Action::EditPageTarget,
            Action::EditPageChannel,
            Action::ConfirmPageManager,
            Action::SelectThreeFour,
            Action::SelectFourFour,
            Action::PatternSizeDown,
            Action::PatternSizeUp,
            Action::ConfirmPatternClear,
            Action::LoopImport,
            Action::LoopRemove,
            Action::LoopSourceDown,
            Action::LoopSourceUp,
            Action::LoopBpmMode,
            Action::LoopEditUnit,
            Action::LoopStartDown,
            Action::LoopStartUp,
            Action::LoopLengthDown,
            Action::LoopLengthUp,
            Action::LoopAutoAlign,
            Action::LoopOffsetDown,
            Action::LoopOffsetUp,
            Action::LoopAlignDone,
            Action::OpenLoopLibrary,
            Action::DeleteLoopFile,
            Action::AudioRecord,
            Action::AudioStop,
            Action::AudioToggleArm,
            Action::AudioArmAll,
            Action::AudioDisarmAll,
            Action::AudioPreviousTrack,
            Action::AudioNextTrack,
            Action::AudioAssignSource,
            Action::AudioNameTrack,
            Action::AudioRefreshSources,
        ];
        for action in inventory {
            assert!(
                reachable.contains(&action),
                "missing controller action {action:?}"
            );
        }
    }
}
