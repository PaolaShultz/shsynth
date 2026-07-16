//! Non-audible controller discovery and MIDI learn.

use crate::control::CONTROLS;
use crate::pads::{ControllerLayout, PadAction, PadConfig};
use anyhow::{anyhow, bail, Context, Result};
use midir::{Ignore, MidiInput, MidiInputConnection};
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{SystemTime, UNIX_EPOCH};

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
            Button::Cc(cc) => config.encoder_press_cc = Some(cc),
            Button::Note(note) => config.encoder_press_note = Some(note),
        }
    }

    let layout = ask_number("Command buttons available (0, 4, 5, or 8) [0]: ", 0, 8)?;
    if !matches!(layout, 0 | 4 | 5 | 8) {
        bail!("command-button count must be 0, 4, 5, or 8");
    }
    if layout == 0 {
        config.layout = ControllerLayout::Four;
        config.pads.clear();
        config.cc_buttons.clear();
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
        config.cc_buttons.clear();
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
                Button::Cc(cc) => {
                    config.cc_buttons.insert(cc, action);
                }
                Button::Note(note) => {
                    config.pads.insert(note, action);
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
    Cc(u8),
    Note(u8),
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
        if message.len() < 3 || message[2] == 0 {
            continue;
        }
        match message[0] & 0xf0 {
            0xb0 if !used_ccs.contains(&message[1]) => return Ok(Button::Cc(message[1])),
            0x90 if !used_notes.contains(&message[1]) => return Ok(Button::Note(message[1])),
            _ => {}
        }
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
        .collect()
}

fn used_notes(config: &PadConfig) -> HashSet<u8> {
    config
        .pads
        .keys()
        .copied()
        .chain(config.encoder_press_note)
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
}
