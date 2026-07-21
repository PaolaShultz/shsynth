//! Non-audible controller discovery and MIDI learn.

use crate::control::CONTROLS;
use crate::pads::{ControllerButton, ControllerLayout, PadAction, PadConfig};
use anyhow::{anyhow, bail, Context, Result};
use midir::{Ignore, MidiInput, MidiInputConnection};
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn input_names() -> Result<Vec<String>> {
    let input = MidiInput::new("SHR-DAW controller discovery")?;
    input
        .ports()
        .iter()
        .map(|port| input.port_name(port).map_err(anyhow::Error::from))
        .collect()
}

pub fn resolve_input(wanted: Option<&str>) -> Result<String> {
    let names = input_names()?;
    resolve_input_name(&names, wanted)
}

pub fn resolve_input_name(names: &[String], wanted: Option<&str>) -> Result<String> {
    if let Some(wanted) = wanted {
        let wanted_lower = wanted.to_ascii_lowercase();
        let matches = names
            .iter()
            .filter(|name| name.to_ascii_lowercase().contains(&wanted_lower))
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [name] => Ok((*name).clone()),
            [] => bail!("MIDI input not found: {wanted}"),
            _ => bail!("MIDI input match is ambiguous: {wanted}"),
        };
    }
    let candidates = names
        .iter()
        .filter(|name| {
            let lower = name.to_ascii_lowercase();
            !lower.contains("midi through") && !lower.contains("shr-daw")
        })
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [name] => Ok((*name).clone()),
        [] => bail!("no external MIDI input detected"),
        _ => bail!(
            "more than one MIDI input detected; pass part of the port name:\n{}",
            candidates
                .iter()
                .map(|name| format!("  {name}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LearnRole {
    AbsoluteControl(usize),
    EncoderClockwise,
    EncoderCounterClockwise,
    EncoderClick,
    Pad(usize),
    Confirm,
}

const FIRST_OPTIONAL_STEP: usize = 3;
const CONTROL_STEP_START: usize = FIRST_OPTIONAL_STEP;
const BUTTON_STEP_START: usize = CONTROL_STEP_START + CONTROLS.len();
const CONFIRM_STEP: usize = BUTTON_STEP_START + COMMAND_ACTIONS.len();
const TOTAL_STEPS: usize = CONFIRM_STEP + 1;
const COMMAND_ACTIONS: [PadAction; 9] = [
    PadAction::Page1,
    PadAction::Page2,
    PadAction::Page3,
    PadAction::Page4,
    PadAction::CyclePage,
    PadAction::Item1,
    PadAction::Item2,
    PadAction::Item3,
    PadAction::Item4,
];

impl LearnRole {
    pub fn label(self) -> String {
        match self {
            Self::AbsoluteControl(index) => {
                format!("CONTROL {} · {}", index + 1, CONTROLS[index].name)
            }
            Self::EncoderClockwise => "MASTER ENCODER · TURN RIGHT".into(),
            Self::EncoderCounterClockwise => "MASTER ENCODER · TURN LEFT".into(),
            Self::EncoderClick => "MASTER ENCODER · CLICK".into(),
            Self::Pad(4) => "PAGE SWITCH · BUTTON OR MODIFIER + CONTROL".into(),
            Self::Pad(index) => format!("COMMAND BUTTON · {}", COMMAND_ACTIONS[index]),
            Self::Confirm => "REVIEW AND SAVE".into(),
        }
    }

    pub const fn skippable(self) -> bool {
        matches!(self, Self::AbsoluteControl(_) | Self::Pad(_))
    }
}

#[derive(Clone, Debug)]
pub struct LearnSession {
    draft: PadConfig,
    step: usize,
    feedback: String,
    state: LearnState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LearnInput {
    Cc { channel: u8, cc: u8 },
    Note { channel: u8, note: u8 },
}

#[derive(Clone, Copy, Debug)]
enum LearnState {
    EntryQuiet { deadline: Instant },
    Armed,
    Settling { cc: u8, deadline: Instant },
    ButtonHeld { input: LearnInput },
    CycleCandidate { modifier: LearnInput },
    CycleConfirm { candidate: LearnInput },
    CycleConfirmHeld { candidate: LearnInput },
    CycleChordHeld { modifier: LearnInput },
    PostRelease { deadline: Instant },
    NavigationSettling { cc: u8, deadline: Instant },
    SaveButtonHeld { input: LearnInput, saved: bool },
    Saved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LearnAction {
    None,
    Save,
    FinishSaved,
}

const ENTRY_QUIET: Duration = Duration::from_millis(120);
const GESTURE_SETTLE: Duration = Duration::from_millis(120);

impl LearnInput {
    fn from_message(message: &[u8]) -> Option<Self> {
        if message.len() < 3 {
            return None;
        }
        match message[0] & 0xf0 {
            0xb0 => Some(Self::Cc {
                channel: message[0] & 0x0f,
                cc: message[1],
            }),
            0x90 if message[2] > 0 => Some(Self::Note {
                channel: message[0] & 0x0f,
                note: message[1],
            }),
            _ => None,
        }
    }

    fn matches_message(self, message: &[u8]) -> bool {
        if message.len() < 3 {
            return false;
        }
        match self {
            Self::Cc { channel, cc } => {
                message[0] & 0xf0 == 0xb0 && message[0] & 0x0f == channel && message[1] == cc
            }
            Self::Note { channel, note } => {
                matches!(message[0] & 0xf0, 0x80 | 0x90 | 0xa0)
                    && message[0] & 0x0f == channel
                    && message[1] == note
            }
        }
    }

    fn is_release(self, message: &[u8]) -> bool {
        if !self.matches_message(message) {
            return false;
        }
        match self {
            Self::Cc { .. } => message[2] == 0,
            Self::Note { .. } => {
                message[0] & 0xf0 == 0x80 || (message[0] & 0xf0 == 0x90 && message[2] == 0)
            }
        }
    }

    fn controller_button(self) -> ControllerButton {
        match self {
            Self::Cc { channel, cc } => ControllerButton::Cc { channel, cc },
            Self::Note { channel, note } => ControllerButton::Note { channel, note },
        }
    }
}

impl LearnSession {
    pub fn new(input_name: &str) -> Self {
        Self::new_at(input_name, Instant::now())
    }

    pub fn new_at(input_name: &str, now: Instant) -> Self {
        let mut draft = PadConfig::unmapped(stable_input_match(input_name));
        draft.profile = Some("learned".into());
        draft.layout = ControllerLayout::Four;
        Self {
            draft,
            step: 0,
            feedback: "Release the opening control · waiting for quiet".into(),
            state: LearnState::EntryQuiet {
                deadline: now + ENTRY_QUIET,
            },
        }
    }

    pub fn role(&self) -> LearnRole {
        match self.step {
            0 => LearnRole::EncoderCounterClockwise,
            1 => LearnRole::EncoderClockwise,
            2 => LearnRole::EncoderClick,
            CONTROL_STEP_START..BUTTON_STEP_START => {
                LearnRole::AbsoluteControl(self.step - CONTROL_STEP_START)
            }
            BUTTON_STEP_START..CONFIRM_STEP => LearnRole::Pad(self.step - BUTTON_STEP_START),
            _ => LearnRole::Confirm,
        }
    }

    pub fn progress(&self) -> (usize, usize) {
        (self.step.min(CONFIRM_STEP) + 1, TOTAL_STEPS)
    }

    pub fn feedback(&self) -> &str {
        &self.feedback
    }

    pub fn draft(&self) -> &PadConfig {
        &self.draft
    }

    pub fn tick(&mut self, now: Instant) {
        match self.state {
            LearnState::EntryQuiet { deadline } if now >= deadline => {
                self.state = LearnState::Armed;
                self.feedback = format!("Ready · {}", self.role().label());
            }
            LearnState::Settling { deadline, .. } if now >= deadline => {
                self.advance_after_capture();
            }
            LearnState::NavigationSettling { deadline, .. } if now >= deadline => {
                self.state = LearnState::Armed;
            }
            LearnState::PostRelease { deadline } if now >= deadline => {
                self.advance_after_capture();
            }
            _ => {}
        }
    }

    pub fn retry(&mut self) {
        self.retry_at(Instant::now());
    }

    pub fn retry_at(&mut self, now: Instant) {
        if matches!(
            self.state,
            LearnState::Saved | LearnState::SaveButtonHeld { saved: true, .. }
        ) {
            self.feedback = "Profile saved · release the encoder to exit".into();
            return;
        }
        self.clear_current_mapping();
        self.state = LearnState::EntryQuiet {
            deadline: now + ENTRY_QUIET,
        };
        self.feedback = format!(
            "Retry · release control, then wait for {}",
            self.role().label()
        );
    }

    pub fn previous(&mut self) -> bool {
        if !matches!(self.state, LearnState::Armed) {
            return false;
        }
        if self.step <= FIRST_OPTIONAL_STEP {
            self.feedback = "Master encoder setup is complete · browse optional mappings".into();
            return false;
        }
        self.step_backward();
        self.feedback = format!("Selected {}", self.role().label());
        true
    }

    pub fn skip(&mut self) -> bool {
        if !matches!(self.state, LearnState::Armed) {
            return false;
        }
        if !self.role().skippable() {
            self.feedback = if self.can_finish() {
                "Click the encoder or press Enter to save and exit".into()
            } else {
                "Learn the master encoder first · Esc cancels".into()
            };
            return false;
        }
        let skipped = self.role().label();
        self.step_forward();
        self.feedback = format!("Skipped {skipped}");
        true
    }

    pub fn receive(&mut self, message: &[u8], now: Instant) -> LearnAction {
        match self.state {
            LearnState::EntryQuiet { ref mut deadline } => {
                if message_marks_activity(message) {
                    *deadline = now + ENTRY_QUIET;
                }
                return LearnAction::None;
            }
            LearnState::Settling {
                cc,
                ref mut deadline,
            }
            | LearnState::NavigationSettling {
                cc,
                ref mut deadline,
            } => {
                if cc_message(message, cc) {
                    *deadline = now + GESTURE_SETTLE;
                }
                return LearnAction::None;
            }
            LearnState::ButtonHeld { input } => {
                if input.is_release(message) {
                    self.advance_after_capture();
                }
                return LearnAction::None;
            }
            LearnState::CycleCandidate { modifier } => {
                if modifier.is_release(message) {
                    self.state = LearnState::CycleConfirm {
                        candidate: modifier,
                    };
                    self.feedback =
                        "No chord seen · press the same page-switch button again to confirm".into();
                } else if let Some(trigger) = LearnInput::from_message(message) {
                    if trigger != modifier {
                        self.draft.page_cycle_modifier = Some(modifier.controller_button());
                        self.draft.page_cycle_trigger = Some(trigger.controller_button());
                        self.draft.layout = ControllerLayout::Five;
                        self.state = LearnState::CycleChordHeld { modifier };
                        self.feedback = format!(
                            "Learned {} + {} = page-cycle · OK · release modifier",
                            learn_input_description(modifier),
                            learn_input_description(trigger)
                        );
                    }
                }
                return LearnAction::None;
            }
            LearnState::CycleConfirm { candidate } => {
                if let Some(input) = LearnInput::from_message(message) {
                    if input == candidate {
                        self.state = LearnState::CycleConfirmHeld { candidate };
                        self.feedback =
                            "Release to confirm this button, or keep holding and use a trigger"
                                .into();
                    } else {
                        self.state = LearnState::CycleCandidate { modifier: input };
                        self.feedback =
                            "Modifier held · now move or press the page-switch control".into();
                    }
                }
                return LearnAction::None;
            }
            LearnState::CycleConfirmHeld { candidate } => {
                if candidate.is_release(message) {
                    match self.learn_pad_input(4, candidate) {
                        Ok(description) => {
                            self.feedback = format!("Learned {description} · OK");
                            self.state = LearnState::PostRelease {
                                deadline: now + GESTURE_SETTLE,
                            };
                        }
                        Err(error) => {
                            self.state = LearnState::Armed;
                            self.feedback = error;
                        }
                    }
                } else if let Some(trigger) = LearnInput::from_message(message) {
                    if trigger != candidate {
                        self.draft.page_cycle_modifier = Some(candidate.controller_button());
                        self.draft.page_cycle_trigger = Some(trigger.controller_button());
                        self.draft.layout = ControllerLayout::Five;
                        self.state = LearnState::CycleChordHeld {
                            modifier: candidate,
                        };
                        self.feedback = format!(
                            "Learned {} + {} = page-cycle · OK · release modifier",
                            learn_input_description(candidate),
                            learn_input_description(trigger)
                        );
                    }
                }
                return LearnAction::None;
            }
            LearnState::CycleChordHeld { modifier } => {
                if modifier.is_release(message) {
                    self.advance_after_capture();
                }
                return LearnAction::None;
            }
            LearnState::PostRelease { .. } => return LearnAction::None,
            LearnState::SaveButtonHeld { input, saved } => {
                if input.is_release(message) {
                    if saved {
                        self.state = LearnState::Saved;
                        return LearnAction::FinishSaved;
                    }
                    self.state = LearnState::Armed;
                    self.feedback = "Save failed · release received · ready to retry".into();
                }
                return LearnAction::None;
            }
            LearnState::Saved => return LearnAction::None,
            LearnState::Armed => {}
        }

        if self.can_finish() {
            let navigation = {
                let cc_action = self.draft.encoder_action(message);
                if cc_action.0 {
                    cc_action
                } else {
                    self.draft.encoder_note_action(message)
                }
            };
            if navigation.0 {
                match navigation.1 {
                    Some(crate::pads::EncoderAction::Up) => {
                        self.previous();
                        if let Some(cc) = cc_number(message) {
                            self.state = LearnState::NavigationSettling {
                                cc,
                                deadline: now + GESTURE_SETTLE,
                            };
                        }
                    }
                    Some(crate::pads::EncoderAction::Down) => {
                        self.skip();
                        if let Some(cc) = cc_number(message) {
                            self.state = LearnState::NavigationSettling {
                                cc,
                                deadline: now + GESTURE_SETTLE,
                            };
                        }
                    }
                    Some(crate::pads::EncoderAction::Select) => {
                        if let Some(input) = LearnInput::from_message(message) {
                            self.state = LearnState::SaveButtonHeld {
                                input,
                                saved: false,
                            };
                            self.feedback = "Save requested · keep the encoder held".into();
                            return LearnAction::Save;
                        }
                    }
                    None => {}
                }
                return LearnAction::None;
            }
        }

        let role = self.role();
        if self.role_is_mapped() || !message_is_relevant(role, message) {
            return LearnAction::None;
        }
        if role == LearnRole::Pad(4) {
            if let Some(modifier) = LearnInput::from_message(message) {
                self.state = LearnState::CycleCandidate { modifier };
                self.feedback = "Modifier held · now move or press the page-switch control".into();
            }
            return LearnAction::None;
        }
        let accepted = match role {
            LearnRole::AbsoluteControl(index) => self.learn_absolute(index, message),
            LearnRole::EncoderCounterClockwise => self.learn_encoder_counterclockwise(message),
            LearnRole::EncoderClockwise => self.learn_encoder_clockwise(message),
            LearnRole::EncoderClick => self.learn_click(message),
            LearnRole::Pad(index) => self.learn_pad(index, message),
            LearnRole::Confirm => return LearnAction::None,
        };
        match accepted {
            Ok(description) => {
                if matches!(role, LearnRole::EncoderClick | LearnRole::Pad(_)) {
                    let Some(input) = LearnInput::from_message(message) else {
                        return LearnAction::None;
                    };
                    self.state = LearnState::ButtonHeld { input };
                    self.feedback = format!("Learned {description} · OK · release to continue");
                } else {
                    let Some(cc) = cc_number(message) else {
                        return LearnAction::None;
                    };
                    self.state = LearnState::Settling {
                        cc,
                        deadline: now + GESTURE_SETTLE,
                    };
                    self.feedback = format!("Learned {description} · OK · finish movement");
                }
                LearnAction::None
            }
            Err(message) => {
                self.feedback = message;
                LearnAction::None
            }
        }
    }

    pub fn mark_save_result(&mut self, saved: bool) {
        if let LearnState::SaveButtonHeld {
            saved: ref mut state,
            ..
        } = self.state
        {
            *state = saved;
            self.feedback = if saved {
                "Profile saved and activated · release encoder to exit".into()
            } else {
                "Save failed · release encoder before retrying".into()
            };
        }
    }

    pub fn save_committed(&self) -> bool {
        matches!(
            self.state,
            LearnState::SaveButtonHeld { saved: true, .. } | LearnState::Saved
        )
    }

    fn advance_after_capture(&mut self) {
        self.step_forward();
        self.state = LearnState::Armed;
        self.feedback = if self.role() == LearnRole::Confirm {
            "Learning complete · click the encoder or press Enter to save".into()
        } else {
            format!("Ready · {}", self.role().label())
        };
    }

    fn step_forward(&mut self) {
        self.step = (self.step + 1).min(CONFIRM_STEP);
        if self.role() == LearnRole::Pad(4) && !self.cycle_page_role_needed() {
            self.step = (self.step + 1).min(CONFIRM_STEP);
        }
    }

    fn step_backward(&mut self) {
        self.step = self.step.saturating_sub(1);
        if self.role() == LearnRole::Pad(4) && !self.cycle_page_role_needed() {
            self.step = self.step.saturating_sub(1);
        }
    }

    fn cycle_page_role_needed(&self) -> bool {
        !self
            .draft
            .pads
            .values()
            .chain(self.draft.cc_buttons.values())
            .any(|action| {
                matches!(
                    action,
                    PadAction::Page1 | PadAction::Page2 | PadAction::Page3 | PadAction::Page4
                )
            })
    }

    fn learn_absolute(&mut self, index: usize, message: &[u8]) -> Result<String, String> {
        if message.len() < 3 || message[0] & 0xf0 != 0xb0 {
            return Err("Expected an absolute knob/fader CC".into());
        }
        let cc = message[1];
        if used_ccs(&self.draft).contains(&cc) {
            return Err(format!("Conflict · CC {cc} is already assigned · retry"));
        }
        self.draft.controls.insert(cc, CONTROLS[index].cc);
        Ok(format!("CC {cc} = {}", CONTROLS[index].name))
    }

    fn learn_encoder_clockwise(&mut self, message: &[u8]) -> Result<String, String> {
        let Some(cc) = self.draft.encoder_relative_cc else {
            return Err("Learn the counterclockwise direction first".into());
        };
        if message.len() < 3 || message[0] & 0xf0 != 0xb0 || message[1] != cc {
            return Err(format!("Expected the same encoder CC {cc}"));
        }
        let expected_less = self.draft.encoder_relative_reverse;
        if message[2] == 0 || message[2] == 64 || (message[2] < 64) != expected_less {
            return Err("Direction conflict · turn the encoder right and retry".into());
        }
        Ok(format!("CC {cc} value {} = right", message[2]))
    }

    fn learn_encoder_counterclockwise(&mut self, message: &[u8]) -> Result<String, String> {
        if message.len() < 3 || message[0] & 0xf0 != 0xb0 || matches!(message[2], 0 | 64) {
            return Err("Expected a moving relative CC (not neutral 0 or 64)".into());
        }
        let cc = message[1];
        if used_ccs(&self.draft).contains(&cc) {
            return Err(format!("Conflict · CC {cc} is already assigned · retry"));
        }
        self.draft.encoder_relative_cc = Some(cc);
        self.draft.encoder_relative_reverse = message[2] > 64;
        Ok(format!("CC {cc} value {} = left", message[2]))
    }

    fn learn_click(&mut self, message: &[u8]) -> Result<String, String> {
        let button = button_from_message(message, &used_ccs(&self.draft), &used_notes(&self.draft))
            .ok_or_else(|| "Expected an unused CC or note press".to_owned())?;
        match button {
            Button::Cc { cc, channel } => {
                self.draft.encoder_press_cc = Some(cc);
                self.draft.encoder_press_channel = Some(channel);
                Ok(format!("CC {cc} ch {} = encoder click", channel + 1))
            }
            Button::Note { note, channel } => {
                self.draft.encoder_press_note = Some(note);
                self.draft.encoder_press_channel = Some(channel);
                Ok(format!("note {note} ch {} = encoder click", channel + 1))
            }
        }
    }

    fn learn_pad(&mut self, index: usize, message: &[u8]) -> Result<String, String> {
        let input = LearnInput::from_message(message)
            .ok_or_else(|| "Conflict or release · press an unused pad/button".to_owned())?;
        self.learn_pad_input(index, input)
    }

    fn learn_pad_input(&mut self, index: usize, input: LearnInput) -> Result<String, String> {
        let action = COMMAND_ACTIONS[index];
        match input {
            LearnInput::Cc { cc, channel } => {
                if used_ccs(&self.draft).contains(&cc) {
                    return Err(format!("Conflict · CC {cc} is already assigned · retry"));
                }
                self.draft.cc_buttons.insert(cc, action);
                self.draft.cc_button_channels.insert(cc, channel);
            }
            LearnInput::Note { note, channel } => {
                if used_notes(&self.draft).contains(&note) {
                    return Err(format!(
                        "Conflict · note {note} is already assigned · retry"
                    ));
                }
                self.draft.pads.insert(note, action);
                self.draft.pad_channels.insert(note, channel);
            }
        }
        self.draft.layout = inferred_layout(&self.draft);
        Ok(format!("{} = {action}", learn_input_description(input)))
    }

    pub fn validated_config(&self) -> Result<PadConfig> {
        if !self.can_finish() {
            bail!("learn the master encoder left, right, and click before saving");
        }
        self.draft.validate()?;
        Ok(self.draft.clone())
    }

    pub fn can_finish(&self) -> bool {
        self.draft.encoder_relative_cc.is_some()
            && (self.draft.encoder_press_cc.is_some() || self.draft.encoder_press_note.is_some())
    }

    fn role_is_mapped(&self) -> bool {
        match self.role() {
            LearnRole::AbsoluteControl(index) => self
                .draft
                .controls
                .values()
                .any(|target| *target == CONTROLS[index].cc),
            LearnRole::Pad(index) => {
                self.draft
                    .pads
                    .values()
                    .chain(self.draft.cc_buttons.values())
                    .any(|action| *action == COMMAND_ACTIONS[index])
                    || (index == 4
                        && self.draft.page_cycle_modifier.is_some()
                        && self.draft.page_cycle_trigger.is_some())
            }
            _ => false,
        }
    }

    fn clear_current_mapping(&mut self) {
        match self.role() {
            LearnRole::EncoderCounterClockwise => {
                self.draft.encoder_relative_cc = None;
                self.draft.encoder_relative_reverse = false;
            }
            LearnRole::EncoderClick => {
                self.draft.encoder_press_cc = None;
                self.draft.encoder_press_note = None;
                self.draft.encoder_press_channel = None;
            }
            LearnRole::AbsoluteControl(index) => {
                let target = CONTROLS[index].cc;
                self.draft.controls.retain(|_, mapped| *mapped != target);
            }
            LearnRole::Pad(index) => {
                let action = COMMAND_ACTIONS[index];
                if index == 4 {
                    self.draft.page_cycle_modifier = None;
                    self.draft.page_cycle_trigger = None;
                }
                let notes = self
                    .draft
                    .pads
                    .iter()
                    .filter_map(|(note, mapped)| (*mapped == action).then_some(*note))
                    .collect::<Vec<_>>();
                for note in notes {
                    self.draft.pads.remove(&note);
                    self.draft.pad_channels.remove(&note);
                }
                let ccs = self
                    .draft
                    .cc_buttons
                    .iter()
                    .filter_map(|(cc, mapped)| (*mapped == action).then_some(*cc))
                    .collect::<Vec<_>>();
                for cc in ccs {
                    self.draft.cc_buttons.remove(&cc);
                    self.draft.cc_button_channels.remove(&cc);
                }
                self.draft.layout = inferred_layout(&self.draft);
            }
            LearnRole::EncoderClockwise | LearnRole::Confirm => {}
        }
    }
}

fn cc_number(message: &[u8]) -> Option<u8> {
    (message.len() >= 3 && message[0] & 0xf0 == 0xb0).then_some(message[1])
}

fn cc_message(message: &[u8], cc: u8) -> bool {
    cc_number(message) == Some(cc)
}

fn message_marks_activity(message: &[u8]) -> bool {
    message.len() >= 3 && matches!(message[0] & 0xf0, 0x80 | 0x90 | 0xb0)
}

fn learn_input_description(input: LearnInput) -> String {
    match input {
        LearnInput::Cc { channel, cc } => format!("CC {cc} ch {}", channel + 1),
        LearnInput::Note { channel, note } => format!("note {note} ch {}", channel + 1),
    }
}

fn inferred_layout(config: &PadConfig) -> ControllerLayout {
    let actions = config.pads.values().chain(config.cc_buttons.values());
    if actions.clone().any(|action| {
        matches!(
            action,
            PadAction::Page1 | PadAction::Page2 | PadAction::Page3 | PadAction::Page4
        )
    }) {
        ControllerLayout::Eight
    } else if actions
        .clone()
        .any(|action| *action == PadAction::CyclePage)
        || (config.page_cycle_modifier.is_some() && config.page_cycle_trigger.is_some())
    {
        ControllerLayout::Five
    } else {
        ControllerLayout::Four
    }
}

fn message_is_relevant(role: LearnRole, message: &[u8]) -> bool {
    if message.len() < 3 {
        return false;
    }
    match role {
        LearnRole::AbsoluteControl(_) => message[0] & 0xf0 == 0xb0,
        LearnRole::EncoderClockwise => message[0] & 0xf0 == 0xb0,
        LearnRole::EncoderCounterClockwise => message[0] & 0xf0 == 0xb0 && message[2] != 64,
        LearnRole::EncoderClick | LearnRole::Pad(_) => {
            message[2] > 0 && matches!(message[0] & 0xf0, 0x90 | 0xb0)
        }
        LearnRole::Confirm => false,
    }
}

pub fn stable_input_match(name: &str) -> String {
    name.split_whitespace()
        .filter(|part| {
            let token = part.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != ':'
            });
            let Some((left, right)) = token.split_once(':') else {
                return true;
            };
            !(left.chars().all(|c| c.is_ascii_digit()) && right.chars().all(|c| c.is_ascii_digit()))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn learn(config: &mut PadConfig, input_name: &str) -> Result<()> {
    let (connection, receiver) = listen(input_name)?;
    let _connection = connection;
    config.input_match = Some(stable_input_match(input_name));
    println!("Listening to {input_name}. MIDI is not being forwarded to an instrument.");

    let missing = CONTROLS
        .iter()
        .filter(|control| !config.controls.values().any(|target| *target == control.cc))
        .count();
    if missing > 0 {
        let count = ask_number(
            &format!("Additional knobs/faders to learn (0-{missing}) [0]: "),
            0,
            missing,
        )?;
        let targets = CONTROLS
            .iter()
            .filter(|control| !config.controls.values().any(|target| *target == control.cc))
            .take(count)
            .copied()
            .collect::<Vec<_>>();
        for control in targets {
            let cc = capture_cc(
                &receiver,
                &format!("Move the control for {}", control.name),
                &used_ccs(config),
            )?;
            config.controls.insert(cc, control.cc);
            println!("  CC {cc} -> {}", control.name);
        }
    }

    if config.encoder_relative_cc.is_none() && ask_yes_no("Learn a main endless encoder? [y/N]: ")?
    {
        let (cc, value) = capture_cc_value(
            &receiver,
            "Turn the main encoder clockwise",
            &used_ccs(config),
        )?;
        if value == 64 {
            bail!("encoder sent only its stationary value; turn it farther and retry");
        }
        config.encoder_relative_cc = Some(cc);
        config.encoder_relative_reverse = value < 64;
        println!("  encoder CC {cc}; direction convention detected");
    }

    if config.encoder_press_cc.is_none()
        && config.encoder_press_note.is_none()
        && ask_yes_no("Learn the main encoder press/select? [y/N]: ")?
    {
        match capture_button(
            &receiver,
            "Press the main encoder",
            &used_ccs(config),
            &used_notes(config),
        )? {
            Button::Cc { cc, .. } => config.encoder_press_cc = Some(cc),
            Button::Note { note, .. } => config.encoder_press_note = Some(note),
        }
    }

    let layout = ask_number("Command buttons available (0, 4, 5, or 8) [0]: ", 0, 8)?;
    if !matches!(layout, 0 | 4 | 5 | 8) {
        bail!("command-button count must be 0, 4, 5, or 8");
    }
    if layout == 0 {
        config.layout = ControllerLayout::Four;
        config.pads.clear();
        config.pad_channels.clear();
        config.cc_buttons.clear();
        config.cc_button_channels.clear();
        config.page_cycle_modifier = None;
        config.page_cycle_trigger = None;
        config.lock_cc = None;
    }
    if layout > 0 {
        config.layout = match layout {
            4 => ControllerLayout::Four,
            5 => ControllerLayout::Five,
            8 => ControllerLayout::Eight,
            _ => unreachable!(),
        };
        config.pads.clear();
        config.pad_channels.clear();
        config.cc_buttons.clear();
        config.cc_button_channels.clear();
        config.page_cycle_modifier = None;
        config.page_cycle_trigger = None;
        let actions: &[PadAction] = match layout {
            4 => &[
                PadAction::Item1,
                PadAction::Item2,
                PadAction::Item3,
                PadAction::Item4,
            ],
            5 => &[
                PadAction::CyclePage,
                PadAction::Item1,
                PadAction::Item2,
                PadAction::Item3,
                PadAction::Item4,
            ],
            8 => &[
                PadAction::Page1,
                PadAction::Page2,
                PadAction::Page3,
                PadAction::Page4,
                PadAction::Item1,
                PadAction::Item2,
                PadAction::Item3,
                PadAction::Item4,
            ],
            _ => unreachable!(),
        };
        for &action in actions {
            let binding = capture_button(
                &receiver,
                &format!("Press the button for {action}"),
                &used_ccs(config),
                &used_notes(config),
            )?;
            match binding {
                Button::Cc { cc, channel } => {
                    config.cc_buttons.insert(cc, action);
                    config.cc_button_channels.insert(cc, channel);
                }
                Button::Note { note, channel } => {
                    config.pads.insert(note, action);
                    config.pad_channels.insert(note, channel);
                }
            }
        }
    }

    if config.lock_cc.is_none() && ask_yes_no("Learn an optional command-button lock CC? [y/N]: ")?
    {
        config.lock_cc = Some(capture_cc(
            &receiver,
            "Press the lock control",
            &used_ccs(config),
        )?);
    }
    Ok(())
}

pub fn backup(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    for revision in 0..1000 {
        let suffix = if revision == 0 {
            format!("conf.bak-{stamp}")
        } else {
            format!("conf.bak-{stamp}-{revision}")
        };
        let backup = path.with_extension(suffix);
        let mut destination = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        let result = (|| -> Result<()> {
            let mut source = std::fs::File::open(path)?;
            io::copy(&mut source, &mut destination)?;
            destination.sync_all()?;
            std::fs::set_permissions(&backup, source.metadata()?.permissions())?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&backup);
        }
        result?;
        return Ok(Some(backup));
    }
    bail!("could not allocate a unique controller backup name")
}

enum Button {
    Cc { cc: u8, channel: u8 },
    Note { note: u8, channel: u8 },
}

fn listen(input_name: &str) -> Result<(MidiInputConnection<()>, Receiver<Vec<u8>>)> {
    let mut input = MidiInput::new("SHR-DAW MIDI learn")?;
    input.ignore(Ignore::None);
    let port = input
        .ports()
        .into_iter()
        .find(|port| input.port_name(port).ok().as_deref() == Some(input_name))
        .with_context(|| format!("MIDI input disappeared: {input_name}"))?;
    let (sender, receiver) = mpsc::channel();
    let connection = input
        .connect(
            &port,
            "SHR-DAW MIDI learn",
            move |_stamp, message, _| {
                let _ = sender.send(message.to_vec());
            },
            (),
        )
        .map_err(|error| anyhow!("open MIDI input for learning: {error}"))?;
    Ok((connection, receiver))
}

fn capture_cc(receiver: &Receiver<Vec<u8>>, prompt: &str, used: &HashSet<u8>) -> Result<u8> {
    capture_cc_value(receiver, prompt, used).map(|(cc, _)| cc)
}

fn capture_cc_value(
    receiver: &Receiver<Vec<u8>>,
    prompt: &str,
    used: &HashSet<u8>,
) -> Result<(u8, u8)> {
    receiver.try_iter().for_each(drop);
    println!("{prompt} …");
    loop {
        let message = receiver.recv().context("MIDI learn input closed")?;
        if message.len() >= 3 && message[0] & 0xf0 == 0xb0 && !used.contains(&message[1]) {
            return Ok((message[1], message[2]));
        }
    }
}

fn capture_button(
    receiver: &Receiver<Vec<u8>>,
    prompt: &str,
    used_ccs: &HashSet<u8>,
    used_notes: &HashSet<u8>,
) -> Result<Button> {
    receiver.try_iter().for_each(drop);
    println!("{prompt} …");
    loop {
        let message = receiver.recv().context("MIDI learn input closed")?;
        if let Some(button) = button_from_message(&message, used_ccs, used_notes) {
            return Ok(button);
        }
    }
}

fn button_from_message(
    message: &[u8],
    used_ccs: &HashSet<u8>,
    used_notes: &HashSet<u8>,
) -> Option<Button> {
    if message.len() < 3 || message[2] == 0 {
        return None;
    }
    match message[0] & 0xf0 {
        0xb0 if !used_ccs.contains(&message[1]) => Some(Button::Cc {
            cc: message[1],
            channel: message[0] & 0x0f,
        }),
        0x90 if !used_notes.contains(&message[1]) => Some(Button::Note {
            note: message[1],
            channel: message[0] & 0x0f,
        }),
        _ => None,
    }
}

fn used_ccs(config: &PadConfig) -> HashSet<u8> {
    config
        .controls
        .keys()
        .chain(config.cc_buttons.keys())
        .copied()
        .chain(
            [
                config.encoder_relative_cc,
                config.encoder_press_cc,
                config.lock_cc,
            ]
            .into_iter()
            .flatten(),
        )
        .chain(
            [config.page_cycle_modifier, config.page_cycle_trigger]
                .into_iter()
                .flatten()
                .filter_map(|button| match button {
                    ControllerButton::Cc { cc, .. } => Some(cc),
                    ControllerButton::Note { .. } => None,
                }),
        )
        .collect()
}

fn used_notes(config: &PadConfig) -> HashSet<u8> {
    config
        .pads
        .keys()
        .copied()
        .chain(config.encoder_press_note)
        .chain(
            [config.page_cycle_modifier, config.page_cycle_trigger]
                .into_iter()
                .flatten()
                .filter_map(|button| match button {
                    ControllerButton::Note { note, .. } => Some(note),
                    ControllerButton::Cc { .. } => None,
                }),
        )
        .collect()
}

fn ask_yes_no(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn ask_number(prompt: &str, default: usize, maximum: usize) -> Result<usize> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if answer.trim().is_empty() {
        return Ok(default);
    }
    let value = answer
        .trim()
        .parse::<usize>()
        .context("expected a number")?;
    if value > maximum {
        bail!("value must be no more than {maximum}");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unstable_alsa_address_is_removed_from_saved_match() {
        assert_eq!(
            stable_input_match("MiniLab3 MIDI:MiniLab3 MIDI 1 24:0"),
            "MiniLab3 MIDI:MiniLab3 MIDI 1"
        );
    }

    #[test]
    fn button_learning_retains_observed_note_and_cc_channels() {
        match button_from_message(&[0x99, 36, 100], &HashSet::new(), &HashSet::new()).unwrap() {
            Button::Note { note, channel } => {
                assert_eq!((note, channel), (36, 9));
            }
            Button::Cc { .. } => panic!("learned note as CC"),
        }

        match button_from_message(&[0xb2, 44, 127], &HashSet::new(), &HashSet::new()).unwrap() {
            Button::Cc { cc, channel } => {
                assert_eq!((cc, channel), (44, 2));
            }
            Button::Note { .. } => panic!("learned CC as note"),
        }
    }

    #[test]
    fn repeated_backups_do_not_overwrite_each_other() {
        let base =
            std::env::temp_dir().join(format!("shsynth-controller-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join("controller.conf");
        std::fs::write(&path, "first").unwrap();
        let first = backup(&path).unwrap().unwrap();
        std::fs::write(&path, "second").unwrap();
        let second = backup(&path).unwrap().unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(first).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(second).unwrap(), "second");
        let _ = std::fs::remove_dir_all(base);
    }

    struct Harness {
        learn: LearnSession,
        now: Instant,
    }

    impl Harness {
        fn new() -> Self {
            let start = Instant::now();
            let mut learn = LearnSession::new_at("Test Controller MIDI 44:0", start);
            let now = start + ENTRY_QUIET;
            learn.tick(now);
            Self { learn, now }
        }

        fn send(&mut self, message: &[u8]) -> LearnAction {
            self.now += Duration::from_millis(1);
            self.learn.receive(message, self.now)
        }

        fn settle(&mut self) {
            self.now += GESTURE_SETTLE + Duration::from_millis(1);
            self.learn.tick(self.now);
        }

        fn learn_master(&mut self, rotary: u8, click: u8, high_low: bool) {
            let (left, right, neutral) = if high_low { (125, 1, 0) } else { (63, 65, 64) };
            self.send(&[0xb0, rotary, left]);
            self.send(&[0xb0, rotary, neutral]);
            self.settle();
            assert_eq!(self.learn.role(), LearnRole::EncoderClockwise);
            self.send(&[0xb0, rotary, right]);
            self.send(&[0xb0, rotary, neutral]);
            self.settle();
            assert_eq!(self.learn.role(), LearnRole::EncoderClick);
            self.send(&[0xb0, click, 127]);
            assert!(self.learn.feedback().contains("OK"));
            self.send(&[0xb0, click, 0]);
            assert_eq!(self.learn.role(), LearnRole::AbsoluteControl(0));
        }

        fn skip_controls(&mut self) {
            for _ in 0..CONTROLS.len() {
                assert!(self.learn.skip());
            }
            assert_eq!(self.learn.role(), LearnRole::Pad(0));
        }
    }

    #[test]
    fn opening_click_release_is_quarantined_before_rotary_left_arms() {
        let start = Instant::now();
        let mut learn = LearnSession::new_at("Controller", start);
        learn.receive(&[0xb0, 118, 0], start + Duration::from_millis(20));
        learn.tick(start + ENTRY_QUIET);
        assert_eq!(learn.role(), LearnRole::EncoderCounterClockwise);
        assert_eq!(learn.draft().encoder_relative_cc, None);
        learn.tick(start + Duration::from_millis(20) + ENTRY_QUIET);
        learn.receive(&[0xb0, 28, 63], start + Duration::from_millis(141));
        assert_eq!(learn.draft().encoder_relative_cc, Some(28));
    }

    #[test]
    fn left_neutral_cannot_satisfy_right_and_right_waits_for_settle() {
        let mut h = Harness::new();
        h.send(&[0xb0, 28, 63]);
        let success = h.learn.feedback().to_owned();
        h.send(&[0xb0, 28, 64]);
        h.send(&[0xb0, 28, 65]);
        assert_eq!(h.learn.role(), LearnRole::EncoderCounterClockwise);
        assert_eq!(h.learn.feedback(), success);
        h.settle();
        assert_eq!(h.learn.role(), LearnRole::EncoderClockwise);
        h.send(&[0xb0, 28, 65]);
        assert!(h.learn.feedback().contains("right"));
    }

    #[test]
    fn click_press_release_is_consumed_before_control_one_arms() {
        let mut h = Harness::new();
        h.send(&[0xb0, 28, 63]);
        h.settle();
        h.send(&[0xb0, 28, 65]);
        h.settle();
        h.send(&[0xb0, 118, 127]);
        assert_eq!(h.learn.role(), LearnRole::EncoderClick);
        h.send(&[0xb0, 118, 127]);
        assert_eq!(h.learn.role(), LearnRole::EncoderClick);
        h.send(&[0xb0, 118, 0]);
        assert_eq!(h.learn.role(), LearnRole::AbsoluteControl(0));
        assert_eq!(h.learn.draft().encoder_press_cc, Some(118));
    }

    #[test]
    fn absolute_stream_stays_on_control_one_then_advances_once() {
        let mut h = Harness::new();
        h.learn_master(28, 118, false);
        h.send(&[0xb0, 10, 20]);
        let success = h.learn.feedback().to_owned();
        for value in [21, 22, 23] {
            h.send(&[0xb0, 10, value]);
            assert_eq!(h.learn.role(), LearnRole::AbsoluteControl(0));
            assert_eq!(h.learn.feedback(), success);
        }
        h.settle();
        assert_eq!(h.learn.role(), LearnRole::AbsoluteControl(1));
        h.settle();
        assert_eq!(h.learn.role(), LearnRole::AbsoluteControl(1));
    }

    #[test]
    fn next_control_packet_during_settle_is_not_taken_by_control_one() {
        let mut h = Harness::new();
        h.learn_master(28, 118, false);
        h.send(&[0xb0, 10, 20]);
        h.send(&[0xb0, 11, 30]);
        h.settle();
        assert_eq!(h.learn.role(), LearnRole::AbsoluteControl(1));
        assert_eq!(h.learn.draft().controls.len(), 1);
        h.send(&[0xb0, 11, 31]);
        assert_eq!(h.learn.draft().controls.len(), 2);
        assert_eq!(h.learn.draft().controls[&11], CONTROLS[1].cc);
    }

    #[test]
    fn cc_button_press_and_release_advance_exactly_once() {
        let mut h = Harness::new();
        h.learn_master(28, 118, false);
        h.skip_controls();
        h.send(&[0xb2, 44, 127]);
        h.send(&[0xb2, 44, 127]);
        assert_eq!(h.learn.role(), LearnRole::Pad(0));
        h.send(&[0xb2, 44, 0]);
        assert_eq!(h.learn.role(), LearnRole::Pad(1));
        h.send(&[0xb2, 44, 0]);
        assert_eq!(h.learn.role(), LearnRole::Pad(1));
    }

    #[test]
    fn note_off_and_velocity_zero_release_each_advance_once() {
        let mut h = Harness::new();
        h.learn_master(28, 118, false);
        h.skip_controls();
        h.send(&[0x99, 36, 100]);
        h.send(&[0x89, 36, 0]);
        assert_eq!(h.learn.role(), LearnRole::Pad(1));
        h.send(&[0x99, 37, 100]);
        h.send(&[0x99, 37, 0]);
        assert_eq!(h.learn.role(), LearnRole::Pad(2));
        h.send(&[0x99, 37, 0]);
        assert_eq!(h.learn.role(), LearnRole::Pad(2));
    }

    #[test]
    fn multi_packet_rotary_browse_gesture_skips_one_role() {
        let mut h = Harness::new();
        h.learn_master(28, 118, false);
        for value in [65, 66, 67, 64] {
            h.send(&[0xb0, 28, value]);
        }
        assert_eq!(h.learn.role(), LearnRole::AbsoluteControl(1));
        h.settle();
        assert_eq!(h.learn.role(), LearnRole::AbsoluteControl(1));
    }

    #[test]
    fn save_click_repeats_and_release_produce_one_action() {
        let mut h = Harness::new();
        h.learn_master(28, 118, false);
        assert_eq!(h.send(&[0xb0, 118, 127]), LearnAction::Save);
        assert_eq!(h.send(&[0xb0, 118, 127]), LearnAction::None);
        h.learn.mark_save_result(true);
        assert_eq!(h.send(&[0xb0, 118, 0]), LearnAction::FinishSaved);
        assert_eq!(h.send(&[0xb0, 118, 0]), LearnAction::None);
    }

    #[test]
    fn retry_clears_only_current_role_and_reentry_has_fresh_quarantine() {
        let mut h = Harness::new();
        h.learn_master(28, 118, false);
        h.send(&[0xb0, 10, 20]);
        h.learn.retry_at(h.now);
        assert_eq!(h.learn.draft().encoder_relative_cc, Some(28));
        assert_eq!(h.learn.draft().encoder_press_cc, Some(118));
        assert!(h.learn.draft().controls.is_empty());
        h.send(&[0xb0, 10, 21]);
        assert!(h.learn.draft().controls.is_empty());
        h.now += ENTRY_QUIET;
        h.learn.tick(h.now);
        h.send(&[0xb0, 11, 30]);
        assert_eq!(h.learn.draft().controls[&11], CONTROLS[0].cc);

        let mut reentered = LearnSession::new_at("Controller", h.now);
        reentered.receive(&[0xb0, 118, 0], h.now + Duration::from_millis(1));
        assert_eq!(reentered.draft().encoder_relative_cc, None);
        assert_eq!(reentered.role(), LearnRole::EncoderCounterClockwise);
    }

    #[test]
    fn minilab_daw_and_user_mode_encoder_pairs_both_learn() {
        for (rotary, click) in [(28, 118), (114, 115)] {
            let mut h = Harness::new();
            h.learn_master(rotary, click, false);
            let config = h.learn.validated_config().unwrap();
            assert_eq!(config.encoder_relative_cc, Some(rotary));
            assert_eq!(config.encoder_press_cc, Some(click));
        }
    }

    #[test]
    fn high_low_encoder_reset_zero_is_part_of_the_gesture() {
        let mut h = Harness::new();
        h.learn_master(114, 115, true);
        assert!(h.learn.draft().encoder_relative_reverse);
        for value in [1, 2, 3, 0] {
            h.send(&[0xb0, 114, value]);
        }
        assert_eq!(h.learn.role(), LearnRole::AbsoluteControl(1));
        h.settle();
        assert_eq!(h.learn.role(), LearnRole::AbsoluteControl(1));
    }

    #[test]
    fn trailing_traffic_cannot_replace_accepted_success_with_conflict() {
        let mut h = Harness::new();
        h.learn_master(28, 118, false);
        h.send(&[0xb0, 10, 20]);
        let success = h.learn.feedback().to_owned();
        h.send(&[0xb0, 10, 21]);
        h.send(&[0xb0, 28, 64]);
        assert_eq!(h.learn.feedback(), success);
        assert!(success.contains("OK"));
    }

    #[test]
    fn optional_command_roles_still_infer_five_button_layout() {
        let mut h = Harness::new();
        h.learn_master(28, 118, false);
        h.skip_controls();
        for _ in 0..4 {
            assert!(h.learn.skip());
        }
        assert_eq!(h.learn.role(), LearnRole::Pad(4));
        h.send(&[0x99, 40, 100]);
        h.send(&[0x89, 40, 0]);
        h.send(&[0x99, 40, 100]);
        assert!(h.learn.draft().pads.is_empty());
        h.send(&[0x89, 40, 0]);
        assert_eq!(h.learn.draft().layout, ControllerLayout::Five);
        assert_eq!(h.learn.draft().pads[&40], PadAction::CyclePage);
    }

    #[test]
    fn four_dedicated_page_buttons_bypass_page_cycle_role() {
        let mut h = Harness::new();
        h.learn_master(28, 118, false);
        h.skip_controls();
        for note in 36..=39 {
            h.send(&[0x99, note, 100]);
            h.send(&[0x89, note, 0]);
        }
        assert_eq!(h.learn.role(), LearnRole::Pad(5));
        assert_eq!(h.learn.draft().layout, ControllerLayout::Eight);
        assert!(!h
            .learn
            .draft()
            .pads
            .values()
            .any(|action| *action == PadAction::CyclePage));
    }

    #[test]
    fn page_cycle_appears_only_after_all_four_page_buttons_are_skipped() {
        let mut h = Harness::new();
        h.learn_master(28, 118, false);
        h.skip_controls();
        for _ in 0..4 {
            assert!(h.learn.skip());
        }
        assert_eq!(h.learn.role(), LearnRole::Pad(4));

        h.send(&[0xb0, 44, 127]);
        h.send(&[0xb0, 44, 0]);
        assert_eq!(h.learn.role(), LearnRole::Pad(4));
        assert!(h.learn.draft().cc_buttons.is_empty());
        h.send(&[0xb0, 44, 127]);
        h.send(&[0xb0, 44, 0]);
        h.settle();
        assert_eq!(h.learn.role(), LearnRole::Pad(5));
        assert_eq!(h.learn.draft().cc_buttons[&44], PadAction::CyclePage);
    }

    #[test]
    fn page_cycle_chord_ignores_modifier_press_and_may_reuse_a_control() {
        let mut h = Harness::new();
        h.learn_master(28, 118, false);
        h.send(&[0xb0, 10, 40]);
        h.settle();
        for _ in 1..CONTROLS.len() {
            assert!(h.learn.skip());
        }
        assert_eq!(h.learn.role(), LearnRole::Pad(0));
        for _ in 0..4 {
            assert!(h.learn.skip());
        }
        assert_eq!(h.learn.role(), LearnRole::Pad(4));

        h.send(&[0xb0, 27, 127]);
        h.send(&[0xb0, 27, 0]);
        assert_eq!(h.learn.draft().page_cycle_modifier, None);
        h.send(&[0xb0, 27, 127]);
        assert_eq!(h.learn.draft().page_cycle_modifier, None);
        h.send(&[0xb0, 10, 70]);
        assert_eq!(
            h.learn.draft().page_cycle_modifier,
            Some(ControllerButton::Cc { channel: 0, cc: 27 })
        );
        assert_eq!(
            h.learn.draft().page_cycle_trigger,
            Some(ControllerButton::Cc { channel: 0, cc: 10 })
        );
        assert!(h.learn.feedback().contains("OK"));
        h.send(&[0xb0, 10, 71]);
        assert_eq!(h.learn.role(), LearnRole::Pad(4));
        h.send(&[0xb0, 27, 0]);
        assert_eq!(h.learn.role(), LearnRole::Pad(5));
        h.learn.validated_config().unwrap();
    }
}
