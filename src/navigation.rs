//! Screen-specific four-page controller menus.
//!
//! Labels and dispatch actions deliberately live in the same table.  Physical
//! controller profiles select pages/items; they never encode screen actions.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Screen {
    Home,
    Presets,
    Playback,
    Ideas,
    Help,
    Tracker,
    TrackerFiles,
    TrackerArrange,
    TrackerPages,
    TrackerTools,
    TrackerNoteLength,
    TrackerLoop,
    TrackerLoopAlign,
    AudioRecorder,
    FxRack,
    FxEditor,
    Meter,
    Routing,
}

impl Screen {
    pub const COUNT: usize = 18;
    #[cfg(test)]
    pub const ALL: [Self; 18] = [
        Self::Home,
        Self::Presets,
        Self::Playback,
        Self::Ideas,
        Self::Help,
        Self::Tracker,
        Self::TrackerFiles,
        Self::TrackerArrange,
        Self::TrackerPages,
        Self::TrackerTools,
        Self::TrackerNoteLength,
        Self::TrackerLoop,
        Self::TrackerLoopAlign,
        Self::AudioRecorder,
        Self::FxRack,
        Self::FxEditor,
        Self::Meter,
        Self::Routing,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::Home => 0,
            Self::Presets => 1,
            Self::Playback => 2,
            Self::Ideas => 3,
            Self::Help => 4,
            Self::Tracker => 5,
            Self::TrackerFiles => 6,
            Self::TrackerArrange => 7,
            Self::TrackerPages => 8,
            Self::TrackerTools => 9,
            Self::TrackerNoteLength => 10,
            Self::TrackerLoop => 11,
            Self::TrackerLoopAlign => 12,
            Self::AudioRecorder => 13,
            Self::FxRack => 14,
            Self::FxEditor => 15,
            Self::Meter => 16,
            Self::Routing => 17,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Home => "HOME",
            Self::Presets => "PRESETS",
            Self::Playback => "PLAYBACK",
            Self::Ideas => "IDEAS",
            Self::Help => "HELP",
            Self::Tracker => "FT2",
            Self::TrackerFiles => "FILES",
            Self::TrackerArrange => "ARRANGE",
            Self::TrackerPages => "TRACKS",
            Self::TrackerTools => "FT2 TOOLS",
            Self::TrackerNoteLength => "NOTE LENGTH",
            Self::TrackerLoop => "FT2 LOOP",
            Self::TrackerLoopAlign => "LOOP ALIGN",
            Self::AudioRecorder => "AUDIO",
            Self::FxRack => "FX RACK",
            Self::FxEditor => "FX EDIT",
            Self::Meter => "MIX",
            Self::Routing => "ROUTING",
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
    OpenControllerLearn,
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
    OpenRouting,
    ResetMeter,
    BusSelectPrevious,
    BusSelectNext,
    BusLevelDecrease,
    BusLevelIncrease,
    BusMute,
    FinalRecordToggle,
    FxAdd,
    FxEditType,
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
    IdeaRecordToggle,
    SaveNew,
    InspectIdea,
    DeleteIdea,
    LoadIdea,
    IdeaPlayToggle,
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
    NoteDestinationField,
    NoteChannelField,
    DefaultProgramField,
    NoteBankMsbField,
    NoteBankLsbField,
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
    NoteEditorSave,
    NoteEditorCancel,
    TrackerPlayToggle,
    TrackerRewind,
    TrackerRecordToggle,
    TrackerNoobToggle,
    OpenPlaybackNoob,
    DisablePlaybackNoob,
    NoobRootDown,
    NoobRootUp,
    NoobScale,
    ConfirmNoob,
    CancelNoob,
    OpenNoteLength,
    ConfirmNoteLength,
    CancelNoteLength,
    ConfirmRoutingDefaults,
    CancelRoutingDefaults,
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
    TrackerMute,
    TrackerPageMute,
    NextTrackerPage,
    PreviousTrackerPage,
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
    AudioRecordToggle,
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
    NoobSetup,
    RoutingDefaults,
    TrackerRecord,
    TrackerNoteEdit,
    PageTarget,
    PageChannel,
    PatternClear,
    LoopLibrary,
    PatternTools,
    DrumPatterns,
    FxEmpty,
    FxType,
}

const PRESETS: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("LOAD", Action::Activate),
            on("FIRST", Action::Home),
            on("LAST", Action::End),
            off(""),
        ],
    ),
    page(
        "ENGINE",
        [
            on("ENGINE-", Action::PreviousEngine),
            on("ENGINE+", Action::NextEngine),
            off(""),
            off(""),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("HELP", Action::OpenHelp),
            off(""),
            on("EXIT", Action::Back),
        ],
    ),
];
const PLAYBACK: [MenuPage; 4] = [
    page(
        "PLAY",
        [
            off(""),
            on("PLAY", Action::IdeaPlayToggle),
            on("RECORD", Action::IdeaRecordToggle),
            off(""),
        ],
    ),
    page(
        "SOUND",
        [
            on("RESET", Action::ResetParameters),
            on("SAVE", Action::SaveNew),
            on("N00B", Action::OpenPlaybackNoob),
            on("NORMAL", Action::DisablePlaybackNoob),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("HELP", Action::OpenHelp),
            off(""),
            on("EXIT", Action::Back),
        ],
    ),
];
const IDEAS: [MenuPage; 4] = [
    page(
        "PLAY",
        [
            on("INSPECT", Action::InspectIdea),
            on("PLAY", Action::IdeaPlayToggle),
            on("RECORD", Action::IdeaRecordToggle),
            on("DELETE", Action::DeleteIdea),
        ],
    ),
    page(
        "FILE",
        [
            on("LOAD", Action::LoadIdea),
            on("SAVE", Action::SaveNew),
            on("FIRST", Action::Home),
            on("LAST", Action::End),
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
const TRACKER: [MenuPage; 4] = [
    page(
        "MOVE",
        [
            on("PAGE-", Action::PreviousTrackerPage),
            on("PAGE+", Action::NextTrackerPage),
            on("TRACK-", Action::PreviousTrack),
            on("TRACK+", Action::NextTrack),
        ],
    ),
    page(
        "PLAY",
        [
            on("CELL", Action::OpenNoteEditor),
            on("PLAY", Action::TrackerPlayToggle),
            on("RECORD", Action::TrackerRecordToggle),
            on("STEP", Action::TrackerEdit),
        ],
    ),
    page(
        "OPEN",
        [
            on("TRACKS", Action::OpenTrackerPages),
            on("FILES", Action::OpenTrackerFiles),
            on("TOOLS", Action::OpenTrackerTools),
            on("TAP", Action::TapTempo),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("N00B", Action::TrackerNoobToggle),
            on("HELP", Action::OpenHelp),
            on("EXIT", Action::Back),
        ],
    ),
];
const TRACKER_TOOLS: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("ARR", Action::OpenTrackerArrange),
            on("LOOP", Action::OpenTrackerLoop),
            on("N00B", Action::TrackerNoobToggle),
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
        "PAGE",
        [
            on("MUTE PG", Action::TrackerPageMute),
            off(""),
            off(""),
            off(""),
        ],
    ),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("HELP", Action::OpenHelp),
            off(""),
            on("EXIT", Action::Back),
        ],
    ),
];
const ARRANGE: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("JUMP", Action::ArrangementJumpToPattern),
            on("PLAY", Action::ArrangementPlayFromStep),
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
            on("HELP", Action::OpenHelp),
            off(""),
            on("EXIT", Action::Back),
        ],
    ),
];
const NOTE_LENGTH: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("DONE", Action::ConfirmNoteLength),
            on("CANCEL", Action::CancelNoteLength),
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
            on("HELP", Action::OpenHelp),
            off(""),
            on("EXIT", Action::CancelNoteLength),
        ],
    ),
];
const NOOB_SETUP: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("ROOT-", Action::NoobRootDown),
            on("ROOT+", Action::NoobRootUp),
            on("MAJ/MIN", Action::NoobScale),
            on("DONE", Action::ConfirmNoob),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("HELP", Action::OpenHelp),
            off(""),
            on("EXIT", Action::CancelNoob),
        ],
    ),
];
const ROUTING_DEFAULTS: [MenuPage; 4] = [
    page(
        "DEFAULT",
        [
            on("CONFIRM", Action::ConfirmRoutingDefaults),
            on("CANCEL", Action::CancelRoutingDefaults),
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
            off(""),
            off(""),
            on("EXIT", Action::CancelRoutingDefaults),
        ],
    ),
];
const TRACKER_LOOP: [MenuPage; 4] = [
    page(
        "PLAY",
        [
            on("REWIND", Action::TrackerRewind),
            on("PLAY", Action::TrackerPlayToggle),
            on("IMPORT", Action::LoopImport),
            on("REMOVE", Action::LoopRemove),
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
            on("ALIGN", Action::OpenTrackerLoopAlign),
            on("LIBRARY", Action::OpenLoopLibrary),
            on("EXIT", Action::Back),
        ],
    ),
];
const LOOP_LIBRARY: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("DELETE", Action::DeleteLoopFile),
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
            on("HELP", Action::OpenHelp),
            off(""),
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
            on("HELP", Action::OpenHelp),
            off(""),
            on("EXIT", Action::Back),
        ],
    ),
];
const TRACKER_RECORD: [MenuPage; 4] = [
    page(
        "PLAY",
        [
            on("N00B", Action::TrackerNoobToggle),
            on("PLAY", Action::TrackerPlayToggle),
            on("RECORD", Action::TrackerRecordToggle),
            off(""),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("HELP", Action::OpenHelp),
            off(""),
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
            on("N00B", Action::TrackerNoobToggle),
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
            on("LENGTH", Action::OpenNoteLength),
            on("PAGE", Action::NextTrackerPage),
            on("EXIT", Action::TrackerEdit),
        ],
    ),
];
const TRACKER_NOTE_EDIT: [MenuPage; 4] = [
    page(
        "ROUTE",
        [
            on("DEST", Action::NoteDestinationField),
            on("CHANNEL", Action::NoteChannelField),
            on("INSTR", Action::DefaultProgramField),
            off(""),
        ],
    ),
    page(
        "SOUND",
        [
            on("BANKMSB", Action::NoteBankMsbField),
            on("BANKLSB", Action::NoteBankLsbField),
            on("CELLPRG", Action::ProgramField),
            on("CLEAR", Action::NoteEditorClearField),
        ],
    ),
    page(
        "CELL",
        [
            on("NOTE", Action::NoteField),
            on("GATE", Action::GateField),
            on("VEL", Action::VelocityField),
            on("EFFECT", Action::EffectField),
        ],
    ),
    page(
        "DONE",
        [
            on("PANIC", Action::StopAll),
            on("SAVE", Action::NoteEditorSave),
            on("PARAM", Action::EffectParameterField),
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
            off(""),
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
            off(""),
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
            on("FIRST", Action::Home),
            on("LAST", Action::End),
            off(""),
            off(""),
        ],
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
            off(""),
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
            off(""),
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
        "RECORD",
        [
            off(""),
            off(""),
            on("RECORD", Action::AudioRecordToggle),
            on("ARM", Action::AudioToggleArm),
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
        "SETUP",
        [
            on("ALL", Action::AudioArmAll),
            on("NONE", Action::AudioDisarmAll),
            on("REFRESH", Action::AudioRefreshSources),
            off(""),
        ],
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

const FX_RACK: [MenuPage; 4] = [
    page(
        "OPS",
        [
            on("ADD", Action::FxAdd),
            on("DEL", Action::FxRemove),
            on("EDIT", Action::FxEditType),
            on("PARM", Action::OpenFxEditor),
        ],
    ),
    page(
        "ORDER",
        [
            on("UP", Action::FxMoveUp),
            on("DOWN", Action::FxMoveDown),
            on("BYPASS", Action::FxBypass),
            off(""),
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

const FX_RACK_EMPTY: [MenuPage; 4] = [
    page("OPS", [on("ADD", Action::FxAdd), off(""), off(""), off("")]),
    page("ORDER", [off(""), off(""), off(""), off("")]),
    FX_RACK[2],
    FX_RACK[3],
];

const FX_TYPE: [MenuPage; 4] = [
    page(
        "TYPE",
        [
            on("TYPE-", Action::FxKindPrevious),
            on("TYPE+", Action::FxKindNext),
            on("OK", Action::Activate),
            on("CANCEL", Action::Back),
        ],
    ),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
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
            off(""),
            on("RECORD", Action::FinalRecordToggle),
            on("RESET", Action::ResetMeter),
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
            on("TOP", Action::Home),
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
            off(""),
            off(""),
            on("EXIT", Action::Back),
        ],
    ),
];

const HOME: [MenuPage; 4] = [
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
];

const ROUTING: [MenuPage; 4] = [
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page("", [off(""), off(""), off(""), off("")]),
    page(
        "SYS",
        [
            on("PANIC", Action::StopAll),
            on("HELP", Action::OpenHelp),
            off(""),
            on("EXIT", Action::Back),
        ],
    ),
];

pub fn pages(screen: Screen, context: MenuContext) -> &'static [MenuPage; 4] {
    if context == MenuContext::NoobSetup {
        return &NOOB_SETUP;
    }
    if context == MenuContext::RoutingDefaults {
        return &ROUTING_DEFAULTS;
    }
    match (screen, context) {
        (Screen::Home, _) => &HOME,
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
        (Screen::TrackerNoteLength, _) => &NOTE_LENGTH,
        (Screen::TrackerLoop, MenuContext::LoopLibrary) => &LOOP_LIBRARY,
        (Screen::TrackerLoop, _) => &TRACKER_LOOP,
        (Screen::TrackerLoopAlign, _) => &TRACKER_LOOP_ALIGN,
        (Screen::AudioRecorder, _) => &AUDIO,
        (Screen::FxRack, MenuContext::FxEmpty) => &FX_RACK_EMPTY,
        (Screen::FxRack, MenuContext::FxType) => &FX_TYPE,
        (Screen::FxRack, _) => &FX_RACK,
        (Screen::FxEditor, _) => &FX_EDITOR,
        (Screen::Meter, _) => &METER,
        (Screen::Routing, _) => &ROUTING,
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
    fn noob_toggle_is_reachable_in_play_record_and_step_edit() {
        for context in [
            MenuContext::Normal,
            MenuContext::TrackerRecord,
            MenuContext::TrackerEdit,
        ] {
            assert!(pages(Screen::Tracker, context)
                .iter()
                .flat_map(|page| page.slots)
                .any(|slot| slot.dispatch() == Some(Action::TrackerNoobToggle)));
        }

        let selector = pages(Screen::TrackerNoteLength, MenuContext::Normal);
        assert_eq!(
            selector[0].slots[0].dispatch(),
            Some(Action::ConfirmNoteLength)
        );
        assert_eq!(
            selector[0].slots[1].dispatch(),
            Some(Action::CancelNoteLength)
        );
        assert_eq!(
            selector[3].slots[3].dispatch(),
            Some(Action::CancelNoteLength)
        );
    }

    #[test]
    fn empty_slots_and_pages_do_not_dispatch() {
        let empty_slot = slot(Screen::Meter, MenuContext::Normal, 1, 1).unwrap();
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
    fn every_menu_keeps_system_safety_and_exit_anchors() {
        let contexts = [
            (Screen::Routing, MenuContext::Normal),
            (Screen::Presets, MenuContext::Normal),
            (Screen::Playback, MenuContext::Normal),
            (Screen::Ideas, MenuContext::Normal),
            (Screen::Help, MenuContext::Normal),
            (Screen::Tracker, MenuContext::Normal),
            (Screen::Tracker, MenuContext::TrackerEdit),
            (Screen::Tracker, MenuContext::TrackerRecord),
            (Screen::Tracker, MenuContext::TrackerNoteEdit),
            (Screen::Tracker, MenuContext::NoobSetup),
            (Screen::TrackerFiles, MenuContext::Normal),
            (Screen::TrackerFiles, MenuContext::PatternClear),
            (Screen::TrackerFiles, MenuContext::PatternTools),
            (Screen::TrackerFiles, MenuContext::DrumPatterns),
            (Screen::TrackerFiles, MenuContext::RoutingDefaults),
            (Screen::TrackerPages, MenuContext::Normal),
            (Screen::TrackerPages, MenuContext::PageTarget),
            (Screen::TrackerTools, MenuContext::Normal),
            (Screen::TrackerNoteLength, MenuContext::Normal),
            (Screen::TrackerLoop, MenuContext::Normal),
            (Screen::TrackerLoop, MenuContext::LoopLibrary),
            (Screen::TrackerLoopAlign, MenuContext::Normal),
            (Screen::AudioRecorder, MenuContext::Normal),
            (Screen::Meter, MenuContext::Normal),
        ];
        for (screen, context) in contexts {
            let menu = pages(screen, context);
            assert_eq!(menu[3].slots[0].label, "PANIC");
            assert_eq!(menu[3].slots[3].label, "EXIT");
            assert!(menu[3].slots[3].dispatch().is_some());
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
            Action::NoteDestinationField,
            Action::NoteChannelField,
            Action::DefaultProgramField,
            Action::NoteBankMsbField,
            Action::NoteBankLsbField,
            Action::GateField,
            Action::VelocityField,
            Action::ProgramField,
            Action::EffectField,
            Action::EffectParameterField,
            Action::NoteEditorSave,
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
            Action::NoteDestinationField
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
            TRACKER[0].slots.map(|slot| slot.dispatch()),
            [
                Some(Action::PreviousTrackerPage),
                Some(Action::NextTrackerPage),
                Some(Action::PreviousTrack),
                Some(Action::NextTrack),
            ]
        );
    }

    #[test]
    fn physical_menus_have_no_page_up_or_page_down_commands() {
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
                MenuContext::FxEmpty,
                MenuContext::FxType,
            ] {
                assert!(pages(screen, context)
                    .iter()
                    .flat_map(|page| page.slots)
                    .all(|slot| !matches!(
                        slot.dispatch(),
                        Some(Action::PageUp | Action::PageDown)
                    )));
            }
        }
    }

    #[test]
    fn child_command_pages_do_not_launch_unrelated_top_level_workspaces() {
        let unrelated = [
            Action::OpenPresets,
            Action::OpenIdeas,
            Action::OpenControllerLearn,
            Action::OpenTracker,
            Action::OpenAudioRecorder,
            Action::OpenMeter,
            Action::OpenRouting,
        ];
        for screen in Screen::ALL {
            if screen == Screen::Home {
                continue;
            }
            for page in pages(screen, MenuContext::Normal) {
                for slot in page.slots {
                    assert!(
                        !slot
                            .dispatch()
                            .is_some_and(|action| unrelated.contains(&action)),
                        "{screen:?} exposes unrelated top-level action {:?}",
                        slot.action
                    );
                }
            }
        }
    }

    #[test]
    fn contextual_transports_use_conventional_soft_button_positions() {
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
                for menu_page in pages(screen, context) {
                    for (position, slot) in menu_page.slots.iter().enumerate() {
                        let expected = match slot.dispatch() {
                            Some(
                                Action::IdeaPlayToggle
                                | Action::TrackerPlayToggle
                                | Action::ArrangementPlayFromStep,
                            ) => Some(1),
                            Some(
                                Action::IdeaRecordToggle
                                | Action::TrackerRecordToggle
                                | Action::AudioRecordToggle
                                | Action::FinalRecordToggle,
                            ) => Some(2),
                            Some(Action::TapTempo) => Some(3),
                            _ => None,
                        };
                        if let Some(expected) = expected {
                            assert_eq!(
                                position, expected,
                                "{screen:?} {context:?} {} {:?}",
                                menu_page.label, slot.action
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn transport_is_contextual_and_redundant_variants_are_gone() {
        for (screen, context) in [
            (Screen::Presets, MenuContext::Normal),
            (Screen::Help, MenuContext::Normal),
            (Screen::TrackerFiles, MenuContext::Normal),
            (Screen::TrackerPages, MenuContext::Normal),
            (Screen::FxRack, MenuContext::Normal),
            (Screen::FxRack, MenuContext::FxEmpty),
            (Screen::FxRack, MenuContext::FxType),
            (Screen::FxEditor, MenuContext::Normal),
        ] {
            assert!(pages(screen, context)
                .iter()
                .flat_map(|page| page.slots)
                .all(|slot| !matches!(
                    slot.dispatch(),
                    Some(
                        Action::IdeaPlayToggle
                            | Action::TrackerPlayToggle
                            | Action::ArrangementPlayFromStep
                            | Action::IdeaRecordToggle
                            | Action::TrackerRecordToggle
                            | Action::AudioRecordToggle
                            | Action::FinalRecordToggle
                            | Action::TapTempo
                    )
                )));
        }
        let forbidden = ["STOP", "REC END", "PLAY STOP", "START", "HERE", "TAKE"];
        for screen in Screen::ALL {
            for page in pages(screen, MenuContext::Normal) {
                for slot in page.slots {
                    assert!(
                        !forbidden.contains(&slot.label),
                        "stale label {}",
                        slot.label
                    );
                }
            }
        }
    }

    #[test]
    fn effects_have_one_obvious_entry_from_the_mix_workflow() {
        let entries = Screen::ALL
            .into_iter()
            .filter(|screen| !matches!(screen, Screen::FxRack | Screen::FxEditor))
            .flat_map(|screen| pages(screen, MenuContext::Normal))
            .flat_map(|page| page.slots)
            .filter(|slot| slot.dispatch() == Some(Action::OpenFxRack))
            .count();
        assert_eq!(entries, 1);
        assert_eq!(
            slot(Screen::Meter, MenuContext::Normal, 2, 0).and_then(MenuSlot::dispatch),
            Some(Action::OpenFxRack)
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
            (Screen::Tracker, MenuContext::TrackerRecord),
            (Screen::Tracker, MenuContext::TrackerNoteEdit),
            (Screen::Tracker, MenuContext::NoobSetup),
            (Screen::TrackerFiles, MenuContext::Normal),
            (Screen::TrackerFiles, MenuContext::PatternClear),
            (Screen::TrackerFiles, MenuContext::PatternTools),
            (Screen::TrackerFiles, MenuContext::DrumPatterns),
            (Screen::TrackerFiles, MenuContext::RoutingDefaults),
            (Screen::TrackerPages, MenuContext::Normal),
            (Screen::TrackerPages, MenuContext::PageTarget),
            (Screen::TrackerPages, MenuContext::PageChannel),
            (Screen::TrackerTools, MenuContext::Normal),
            (Screen::TrackerArrange, MenuContext::Normal),
            (Screen::TrackerNoteLength, MenuContext::Normal),
            (Screen::TrackerLoop, MenuContext::Normal),
            (Screen::TrackerLoop, MenuContext::LoopLibrary),
            (Screen::TrackerLoopAlign, MenuContext::Normal),
            (Screen::AudioRecorder, MenuContext::Normal),
            (Screen::Meter, MenuContext::Normal),
            (Screen::FxRack, MenuContext::Normal),
            (Screen::FxRack, MenuContext::FxEmpty),
            (Screen::FxRack, MenuContext::FxType),
            (Screen::FxEditor, MenuContext::Normal),
        ];
        let reachable = contexts
            .into_iter()
            .flat_map(|(screen, context)| pages(screen, context))
            .flat_map(|page| page.slots)
            .filter_map(MenuSlot::dispatch)
            .collect::<HashSet<_>>();
        let inventory = [
            Action::Home,
            Action::End,
            Action::PreviousEngine,
            Action::NextEngine,
            Action::Activate,
            Action::Back,
            Action::StopAll,
            Action::OpenHelp,
            Action::OpenTrackerFiles,
            Action::OpenTrackerPages,
            Action::OpenTrackerTools,
            Action::OpenTrackerArrange,
            Action::OpenTrackerLoop,
            Action::OpenTrackerLoopAlign,
            Action::OpenFxRack,
            Action::OpenFxEditor,
            Action::ResetMeter,
            Action::BusSelectPrevious,
            Action::BusSelectNext,
            Action::BusLevelDecrease,
            Action::BusLevelIncrease,
            Action::BusMute,
            Action::FinalRecordToggle,
            Action::TapTempo,
            Action::ResetParameters,
            Action::IdeaRecordToggle,
            Action::SaveNew,
            Action::InspectIdea,
            Action::DeleteIdea,
            Action::LoadIdea,
            Action::IdeaPlayToggle,
            Action::TrackerEdit,
            Action::TrackerSkip,
            Action::TrackerErase,
            Action::TrackerNoteOff,
            Action::OpenNoteEditor,
            Action::NoteDestinationField,
            Action::NoteChannelField,
            Action::DefaultProgramField,
            Action::NoteBankMsbField,
            Action::NoteBankLsbField,
            Action::NoteField,
            Action::GateField,
            Action::VelocityField,
            Action::ProgramField,
            Action::EffectField,
            Action::EffectParameterField,
            Action::NoteEditorClearField,
            Action::NoteEditorSave,
            Action::TrackerPlayToggle,
            Action::TrackerRewind,
            Action::TrackerRecordToggle,
            Action::TrackerNoobToggle,
            Action::OpenPlaybackNoob,
            Action::DisablePlaybackNoob,
            Action::NoobRootDown,
            Action::NoobRootUp,
            Action::NoobScale,
            Action::ConfirmNoob,
            Action::CancelNoob,
            Action::OpenNoteLength,
            Action::ConfirmNoteLength,
            Action::CancelNoteLength,
            Action::ConfirmRoutingDefaults,
            Action::CancelRoutingDefaults,
            Action::TrackerMute,
            Action::TrackerPageMute,
            Action::NextTrackerPage,
            Action::PreviousTrackerPage,
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
            Action::AudioRecordToggle,
            Action::AudioToggleArm,
            Action::AudioArmAll,
            Action::AudioDisarmAll,
            Action::AudioPreviousTrack,
            Action::AudioNextTrack,
            Action::AudioAssignSource,
            Action::AudioNameTrack,
            Action::AudioRefreshSources,
            Action::FxAdd,
            Action::FxEditType,
            Action::FxRemove,
            Action::FxMoveUp,
            Action::FxMoveDown,
            Action::FxBypass,
            Action::FxKindPrevious,
            Action::FxKindNext,
            Action::FxTargetNext,
            Action::FxSendDecrease,
            Action::FxSendIncrease,
            Action::FxSendPoint,
            Action::FxReturnCycle,
            Action::FxParameterPrevious,
            Action::FxParameterNext,
            Action::FxValueDecrease,
            Action::FxValueIncrease,
        ];
        for action in inventory {
            assert!(
                reachable.contains(&action),
                "missing controller action {action:?}"
            );
        }
    }
}
