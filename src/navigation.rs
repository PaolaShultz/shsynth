use crate::pads::PadAction;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Presets,
    Playback,
    Ideas,
    Tracker,
    TrackerFiles,
    TrackerPages,
    AudioRecorder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Noop,
    Arp,
    Up,
    Down,
    PreviousEngine,
    NextEngine,
    Activate,
    Cancel,
    OpenIdeas,
    OpenTracker,
    OpenTrackerFiles,
    OpenTrackerPages,
    Back,
    TapTempo,
    SaveNew,
    SaveRecord,
    InspectIdea,
    DeleteIdea,
    PlaybackRecording,
    ToggleTracker,
    TrackerStop,
    TrackerMute,
    PreviewSong,
    DeleteSong,
    NewPattern,
    ClearPattern,
    TrackerEdit,
    PreviousTrack,
    NextTrack,
    SaveSong,
    AddPage,
    EditPageTarget,
    EditPageChannel,
    ConfirmPageManager,
    AudioRecord,
    AudioStop,
}

#[derive(Clone, Copy)]
pub struct PadAssignment {
    pub pad: PadAction,
    pub action: Action,
    pub label: &'static str,
}

const PRESETS: &[PadAssignment] = &[
    p(PadAction::Arp, Action::Arp, "ARP"),
    p(PadAction::Pad, Action::OpenTracker, "TRACKER"),
    p(PadAction::Prog, Action::PreviousEngine, "ENGINE−"),
    p(PadAction::Loop, Action::NextEngine, "ENGINE+"),
    p(PadAction::Stop, Action::Cancel, "STOP"),
    p(PadAction::Play, Action::Activate, "PLAY"),
    p(PadAction::Rec, Action::OpenIdeas, "SAVE"),
    p(PadAction::TapTempo, Action::TapTempo, "TAP"),
];
const PLAYBACK: &[PadAssignment] = &[
    p(PadAction::Arp, Action::OpenIdeas, "FILE"),
    p(PadAction::Pad, Action::OpenTracker, "TRACKER"),
    p(PadAction::Prog, Action::Up, "PROG"),
    p(PadAction::Loop, Action::Down, "LOOP"),
    p(PadAction::Stop, Action::Cancel, "STOP"),
    p(PadAction::Play, Action::PlaybackRecording, "PLAYBACK"),
    p(PadAction::Rec, Action::SaveRecord, "SAVE"),
    p(PadAction::TapTempo, Action::TapTempo, "TAP"),
];
const IDEAS: &[PadAssignment] = &[
    p(PadAction::Arp, Action::InspectIdea, "PREVIEW"),
    p(PadAction::Pad, Action::Noop, ""),
    p(PadAction::Prog, Action::Noop, ""),
    p(PadAction::Loop, Action::Noop, ""),
    p(PadAction::Stop, Action::Back, "BACK"),
    p(PadAction::Play, Action::PlaybackRecording, "PLAY"),
    p(PadAction::Rec, Action::SaveNew, "SAVE"),
    p(PadAction::TapTempo, Action::DeleteIdea, "DELETE"),
];
const TRACKER: &[PadAssignment] = &[
    p(PadAction::Arp, Action::OpenTrackerPages, "PAGES"),
    p(PadAction::Pad, Action::TrackerEdit, "EDIT"),
    p(PadAction::Prog, Action::PreviousTrack, "LANE−"),
    p(PadAction::Loop, Action::NextTrack, "LANE+"),
    p(PadAction::Stop, Action::TrackerStop, "STOP"),
    p(PadAction::Play, Action::ToggleTracker, "PLAY"),
    p(PadAction::Rec, Action::SaveSong, "SAVE"),
    p(PadAction::TapTempo, Action::TapTempo, "TAP"),
];
const TRACKER_PAGES: &[PadAssignment] = &[
    p(PadAction::Arp, Action::OpenTrackerFiles, "FILE"),
    p(PadAction::Pad, Action::AddPage, "ADD"),
    p(PadAction::Prog, Action::PreviousTrack, "PAGE−"),
    p(PadAction::Loop, Action::NextTrack, "PAGE+"),
    p(PadAction::Stop, Action::Back, "CANCEL"),
    p(PadAction::Play, Action::EditPageTarget, "TARGET"),
    p(PadAction::Rec, Action::EditPageChannel, "CHANNEL"),
    p(PadAction::TapTempo, Action::ConfirmPageManager, "DONE"),
];
const TRACKER_FILES: &[PadAssignment] = &[
    p(PadAction::Arp, Action::Noop, ""),
    p(PadAction::Pad, Action::NewPattern, "NEW PAT"),
    p(PadAction::Prog, Action::Noop, ""),
    p(PadAction::Loop, Action::ClearPattern, "PAT CLR"),
    p(PadAction::Stop, Action::Back, "BACK"),
    p(PadAction::Play, Action::PreviewSong, "PLAY"),
    p(PadAction::Rec, Action::SaveSong, "SAVE"),
    p(PadAction::TapTempo, Action::DeleteSong, "DELETE"),
];
const AUDIO_RECORDER: &[PadAssignment] = &[
    p(PadAction::Stop, Action::AudioStop, "STOP REC"),
    p(PadAction::Rec, Action::AudioRecord, "RECORD"),
];

const fn p(pad: PadAction, action: Action, label: &'static str) -> PadAssignment {
    PadAssignment { pad, action, label }
}
pub fn assignments(screen: Screen) -> &'static [PadAssignment] {
    match screen {
        Screen::Presets => PRESETS,
        Screen::Playback => PLAYBACK,
        Screen::Ideas => IDEAS,
        Screen::Tracker => TRACKER,
        Screen::TrackerFiles => TRACKER_FILES,
        Screen::TrackerPages => TRACKER_PAGES,
        Screen::AudioRecorder => AUDIO_RECORDER,
    }
}
pub fn pad_action(screen: Screen, pad: PadAction) -> Option<Action> {
    assignments(screen)
        .iter()
        .find(|x| x.pad == pad)
        .map(|x| x.action)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pads_are_screen_specific_and_fixed() {
        assert_eq!(assignments(Screen::Presets).len(), 8);
        let play_label = |screen| {
            assignments(screen)
                .iter()
                .find(|assignment| assignment.pad == PadAction::Play)
                .unwrap()
                .label
        };
        assert_eq!(play_label(Screen::Presets), "PLAY");
        assert_eq!(play_label(Screen::Playback), "PLAYBACK");
        assert_eq!(
            pad_action(Screen::Playback, PadAction::Play),
            Some(Action::PlaybackRecording)
        );
        assert_eq!(
            pad_action(Screen::Playback, PadAction::Rec),
            Some(Action::SaveRecord)
        );
        assert_eq!(
            pad_action(Screen::Ideas, PadAction::Rec),
            Some(Action::SaveNew)
        );
        assert_eq!(
            pad_action(Screen::Presets, PadAction::TapTempo),
            Some(Action::TapTempo)
        );
        assert_eq!(
            pad_action(Screen::Presets, PadAction::Prog),
            Some(Action::PreviousEngine)
        );
        assert_eq!(
            pad_action(Screen::Playback, PadAction::Prog),
            Some(Action::Up)
        );
        assert_eq!(
            pad_action(Screen::Tracker, PadAction::Stop),
            Some(Action::TrackerStop)
        );
        assert_eq!(
            pad_action(Screen::Tracker, PadAction::Arp),
            Some(Action::OpenTrackerPages)
        );
        assert_eq!(assignments(Screen::Tracker)[0].label, "PAGES");
        assert_eq!(assignments(Screen::TrackerPages).len(), 8);
        assert_eq!(
            pad_action(Screen::TrackerPages, PadAction::Pad),
            Some(Action::AddPage)
        );
        assert_eq!(
            pad_action(Screen::TrackerPages, PadAction::Play),
            Some(Action::EditPageTarget)
        );
        assert_eq!(
            pad_action(Screen::TrackerPages, PadAction::Rec),
            Some(Action::EditPageChannel)
        );
        assert_eq!(assignments(Screen::TrackerFiles).len(), 8);
        assert_eq!(assignments(Screen::TrackerFiles)[0].label, "");
        assert_eq!(
            pad_action(Screen::TrackerFiles, PadAction::Play),
            Some(Action::PreviewSong)
        );
        assert_eq!(
            pad_action(Screen::TrackerFiles, PadAction::Pad),
            Some(Action::NewPattern)
        );
        assert_eq!(
            pad_action(Screen::TrackerFiles, PadAction::Loop),
            Some(Action::ClearPattern)
        );
        assert_eq!(
            assignments(Screen::Tracker)
                .iter()
                .find(|assignment| assignment.pad == PadAction::Stop)
                .unwrap()
                .label,
            "STOP"
        );
        assert_eq!(
            pad_action(Screen::Tracker, PadAction::Prog),
            Some(Action::PreviousTrack)
        );
        assert_eq!(
            pad_action(Screen::Tracker, PadAction::Loop),
            Some(Action::NextTrack)
        );
        for screen in [Screen::Presets, Screen::Playback] {
            assert_eq!(
                pad_action(screen, PadAction::Pad),
                Some(Action::OpenTracker)
            );
        }
        assert_eq!(
            pad_action(Screen::Ideas, PadAction::Pad),
            Some(Action::Noop)
        );
        assert_eq!(
            pad_action(Screen::TrackerFiles, PadAction::Prog),
            Some(Action::Noop)
        );
        assert_eq!(
            pad_action(Screen::AudioRecorder, PadAction::Stop),
            Some(Action::AudioStop)
        );
        assert_eq!(
            pad_action(Screen::AudioRecorder, PadAction::Rec),
            Some(Action::AudioRecord)
        );
    }
}
