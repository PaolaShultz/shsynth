mod audio_recorder;
mod chord;
mod config;
mod control;
mod controller_learn;
mod controller_profile;
mod device_profile;
mod engine;
mod geometry;
mod help;
mod loop_player;
mod midi;
mod navigation;
mod pads;
mod preset;
mod recording;
mod scale;
mod sequencer;
mod ui;

use anyhow::{bail, Context, Result};
use std::env;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    if let Err(e) = real_main() {
        eprintln!("shr: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let state = state_dir();
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("config") {
        return config_command(&args[1..], &state);
    }
    let runtime = config::RuntimeConfig::load(&state.join("shsynth.conf"))?;
    let preset_dir = preset_dir(&runtime)?;
    let catalogs = preset::discover_all(&runtime, &preset_dir);
    let presets = catalogs
        .iter()
        .flat_map(|catalog| catalog.presets.iter().cloned())
        .collect::<Vec<_>>();
    match args.first().map(String::as_str).unwrap_or("menu") {
        "menu" => ui::run(&catalogs, &state, &runtime),
        "list" => {
            for catalog in &catalogs {
                if let Some(reason) = &catalog.unavailable {
                    println!("[{} unavailable: {reason}]", catalog.backend.label());
                }
                for p in &catalog.presets {
                    println!("{}:{}", catalog.backend.label(), p.name);
                }
            }
            Ok(())
        }
        "status" => {
            println!("{}", engine::status(&state));
            Ok(())
        }
        "doctor" => doctor(&runtime, &preset_dir, &state),
        "stop" => engine::stop_managed(&state),
        "log" | "logs" => show_log(&state, args.get(1)),
        "ideas" => ideas_command(&args[1..], &presets, &state, &runtime),
        "pads" => pads_command(&args[1..], &state),
        "casio" => casio_command(&args[1..], &runtime),
        "start" => {
            let arg = args.get(1).context("usage: shr start PRESET")?;
            let p = preset::resolve(&presets, arg)
                .with_context(|| format!("unknown preset (use ENGINE:NAME): {arg}"))?;
            start_daemon(p, &state, &runtime)
        }
        "daemon" => {
            let arg = args.get(1).context("internal daemon missing preset")?;
            let p = preset::resolve(&presets, arg)
                .with_context(|| format!("unknown preset: {arg}"))?
                .clone();
            engine::daemon(p, state, runtime)
        }
        "help" | "-h" | "--help" => {
            usage();
            Ok(())
        }
        other => {
            usage();
            bail!("unknown command: {other}")
        }
    }
}

fn preset_dir(config: &config::RuntimeConfig) -> Result<PathBuf> {
    if let Some(path) = env::var_os("SHSYNTH_PRESET_DIR") {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = &config.preset_dir {
        return Ok(path.clone());
    }
    let beside_exe = env::current_exe()?
        .parent()
        .unwrap_or(Path::new("."))
        .join("../share/shsynth/presets/synthv1");
    if beside_exe.is_dir() {
        return Ok(beside_exe);
    }
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("presets/synthv1"))
}

fn state_dir() -> PathBuf {
    if let Some(path) = env::var_os("SHSYNTH_STATE_DIR") {
        return PathBuf::from(path);
    }
    env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".local/state")
        })
        .join("shsynth")
}

fn start_daemon(
    preset: &preset::Preset,
    state: &Path,
    config: &config::RuntimeConfig,
) -> Result<()> {
    engine::stop_managed(state)?;
    fs::create_dir_all(state)?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.join("engine.log"))?;
    let exe = env::current_exe()?;
    let child = Command::new(exe)
        .args([
            "daemon",
            &format!("{}:{}", preset.backend.label(), preset.name),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    let deadline = Instant::now() + config.startup_timeout + Duration::from_secs(1);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
        let status = engine::status(state);
        if status.starts_with("Running:") {
            println!("Loaded {}:{}.", preset.backend.label(), preset.name);
            return Ok(());
        }
        if unsafe { libc::kill(child.id() as i32, 0) } != 0 {
            break;
        }
    }
    bail!(
        "engine failed to start; see {}",
        state.join("engine.log").display()
    )
}

fn show_log(state: &Path, count: Option<&String>) -> Result<()> {
    let n = count.and_then(|s| s.parse::<usize>().ok()).unwrap_or(80);
    let text = fs::read_to_string(state.join("engine.log")).unwrap_or_default();
    let lines: Vec<_> = text.lines().collect();
    for line in &lines[lines.len().saturating_sub(n)..] {
        println!("{line}");
    }
    Ok(())
}

fn usage() {
    println!("Usage: shr [menu|list|status|doctor|start PRESET|stop|log [LINES]|ideas COMMAND|pads COMMAND|casio diagnostic|config init]\n\nController setup: shr pads ports|profiles|auto [PORT]|learn [PORT]|update\nWith no arguments, opens the terminal instrument browser.");
}

fn casio_command(args: &[String], config: &config::RuntimeConfig) -> Result<()> {
    match args.first().map(String::as_str).unwrap_or("diagnostic") {
        "diagnostic" | "status" | "dry-run" => {
            print!("{}", sequencer::diagnostic(&config.external_midi)?);
            Ok(())
        }
        other => {
            bail!("unknown Casio command {other}; only non-transmitting diagnostic is available")
        }
    }
}

fn doctor(config: &config::RuntimeConfig, preset_dir: &Path, state: &Path) -> Result<()> {
    let mut problems = 0;
    let mut check = |ok: bool, message: String| {
        println!("[{}] {message}", if ok { "ok" } else { "!!" });
        if !ok {
            problems += 1;
        }
    };
    check(
        command_exists(&config.synth_command),
        format!("synth command: {}", config.synth_command),
    );
    for command in ["jack_lsp", "jack_connect", "aconnect"] {
        check(
            command_exists(command),
            format!("required command: {command}"),
        );
    }
    check(
        preset_dir.is_dir(),
        format!("preset directory: {}", preset_dir.display()),
    );
    check(
        state.join("shsynth.conf").is_file(),
        format!("runtime config: {}", state.join("shsynth.conf").display()),
    );
    check(
        state.join("controller.conf").is_file(),
        format!(
            "controller config: {}",
            state.join("controller.conf").display()
        ),
    );
    let jack = Command::new("jack_lsp").output().ok();
    let jack_ready = jack
        .as_ref()
        .map(|output| output.status.success())
        .unwrap_or(false);
    check(jack_ready, "JACK server reachable".into());
    if jack_ready && config.audio_autoconnect {
        let ports = String::from_utf8_lossy(&jack.as_ref().unwrap().stdout);
        for output in &config.audio_outputs {
            check(
                ports.lines().any(|port| port == output),
                format!("JACK output: {output}"),
            );
        }
    }
    if let Some(cpu) = config.audio_engine_cpu {
        check(
            Path::new(&format!("/sys/devices/system/cpu/cpu{cpu}")).is_dir(),
            format!("configured audio CPU is online: {cpu}"),
        );
        let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();
        check(
            cmdline
                .split_whitespace()
                .any(|arg| arg == format!("nohz_full={cpu}")),
            format!("audio CPU {cpu} is isolated (reboot after shr-audio-tune)"),
        );
        let governors = fs::read_dir("/sys/devices/system/cpu/cpufreq")
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("policy"))
            .map(|entry| fs::read_to_string(entry.path().join("scaling_governor")))
            .collect::<std::io::Result<Vec<_>>>()
            .unwrap_or_default();
        check(
            !governors.is_empty() && governors.iter().all(|value| value.trim() == "performance"),
            "CPU frequency governor: performance".into(),
        );
    }
    if config.midi_autoconnect {
        let controller = pads::PadConfig::load(&state.join("controller.conf")).unwrap_or_default();
        let wanted = controller
            .input_match
            .map(|input| vec![input])
            .unwrap_or_else(|| config.midi_input_matches.clone());
        let ports = Command::new("aconnect")
            .arg("-l")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).to_lowercase())
            .unwrap_or_default();
        check(
            wanted
                .iter()
                .any(|wanted| ports.contains(&wanted.to_lowercase())),
            format!("MIDI input match: {}", wanted.join(", ")),
        );
    }
    if problems > 0 {
        bail!("doctor found {problems} problem(s)");
    }
    Ok(())
}

fn command_exists(program: &str) -> bool {
    let path = Path::new(program);
    if path.components().count() > 1 {
        return path.is_file();
    }
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

fn config_command(args: &[String], state: &Path) -> Result<()> {
    match args.first().map(String::as_str).unwrap_or("paths") {
        "paths" => {
            println!("{}", state.join("shsynth.conf").display());
            println!("{}", state.join("controller.conf").display());
            Ok(())
        }
        "init" => {
            let runtime_path = state.join("shsynth.conf");
            let controller_path = state.join("controller.conf");
            let force = args.get(1).map(String::as_str) == Some("--force");
            if force || !runtime_path.exists() {
                config::RuntimeConfig::default().save(&runtime_path)?;
                println!("Created {}", runtime_path.display());
            } else {
                println!("Kept {}", runtime_path.display());
            }
            if force || !controller_path.exists() {
                pads::PadConfig::default().save(&controller_path)?;
                println!("Created {}", controller_path.display());
            } else {
                println!("Kept {}", controller_path.display());
            }
            Ok(())
        }
        other => bail!("unknown config command: {other}"),
    }
}

fn ideas_command(
    args: &[String],
    presets: &[preset::Preset],
    state: &Path,
    config: &config::RuntimeConfig,
) -> Result<()> {
    let base = recording::ideas_dir();
    match args.first().map(String::as_str).unwrap_or("list") {
        "list" => {
            if let Ok(entries) = fs::read_dir(&base) {
                let mut names = entries
                    .filter_map(Result::ok)
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                names.sort();
                for n in names {
                    println!("{n}");
                }
            }
            Ok(())
        }
        "inspect" => {
            let n = args.get(1).context("usage: shr ideas inspect NAME")?;
            print!(
                "{}",
                fs::read_to_string(base.join(recording::safe_name(n)).join("metadata.json"))?
            );
            Ok(())
        }
        "delete" => {
            let n = args.get(1).context("usage: shr ideas delete NAME --yes")?;
            if args.get(2).map(String::as_str) != Some("--yes") {
                bail!("deletion requires --yes");
            }
            fs::remove_dir_all(base.join(recording::safe_name(n)))?;
            Ok(())
        }
        "play" => {
            let n = args.get(1).context("usage: shr ideas play NAME")?;
            let (p, events) = recording::load(&base, n)?;
            let (tx, _) = std::sync::mpsc::channel();
            let router = engine::MidiRouter::start(state, config, tx)?;
            if let Ok(mut backend) = router.backend().lock() {
                *backend = p.backend;
            }
            router.arm_pickup(&engine::initial_values(&p)?);
            let engine = engine::Engine::start(&p, state, router.output(), config)?;
            let stop = std::sync::atomic::AtomicBool::new(false);
            recording::play_events(
                &events,
                |m| {
                    let _ = engine.send(m);
                },
                &stop,
            );
            drop(engine);
            Ok(())
        }
        other => {
            let _ = presets;
            bail!("unknown ideas command: {other}")
        }
    }
}

fn pads_command(args: &[String], state: &Path) -> Result<()> {
    let path = state.join("controller.conf");
    let mut config = pads::PadConfig::load(&path)?;
    match args.first().map(String::as_str).unwrap_or("list") {
        "ports" => {
            for name in controller_learn::input_names()? {
                println!("{name}");
            }
            Ok(())
        }
        "profiles" => {
            for profile in controller_profile::Catalog::discover().profiles() {
                println!("{}: {} [{}]", profile.id, profile.name, profile.source);
            }
            Ok(())
        }
        "auto" | "detect" => {
            let input = controller_learn::resolve_input(args.get(1).map(String::as_str))?;
            config.input_match = Some(controller_learn::stable_input_match(&input));
            if let Some(profile) = controller_profile::Catalog::discover().matching(&input) {
                profile.apply(&mut config, &controller_learn::stable_input_match(&input))?;
                if let Some(backup) = controller_learn::backup(&path)? {
                    println!("Backed up {}", backup.display());
                }
                config.save(&path)?;
                println!("Loaded known profile {} for {input}", profile.name);
            } else {
                if let Some(backup) = controller_learn::backup(&path)? {
                    println!("Backed up {}", backup.display());
                }
                config.save(&path)?;
                println!("Selected {input}; no known profile. Run `shr pads learn`.");
            }
            Ok(())
        }
        "learn" => {
            let input = controller_learn::resolve_input(
                args.get(1)
                    .map(String::as_str)
                    .or(config.input_match.as_deref()),
            )?;
            if config.controls.is_empty() && config.pads.is_empty() && config.cc_buttons.is_empty()
            {
                if let Some(profile) = controller_profile::Catalog::discover().matching(&input) {
                    profile.apply(&mut config, &controller_learn::stable_input_match(&input))?;
                    println!("Started with known profile {}.", profile.name);
                }
            }
            controller_learn::learn(&mut config, &input)?;
            if let Some(backup) = controller_learn::backup(&path)? {
                println!("Backed up {}", backup.display());
            }
            config.save(&path)?;
            println!("Saved learned controller mapping to {}", path.display());
            Ok(())
        }
        "update" => update_controller_profiles(),
        "list" => {
            println!("input: {}", config.input_match.as_deref().unwrap_or("auto"));
            println!(
                "menu layout: {} buttons",
                match config.layout {
                    pads::ControllerLayout::Eight => 8,
                    pads::ControllerLayout::Five => 5,
                    pads::ControllerLayout::Four => 4,
                }
            );
            println!(
                "encoder: turn CC {}, press CC {}; pad lock CC {}",
                config
                    .encoder_relative_cc
                    .map(|cc| cc.to_string())
                    .unwrap_or_else(|| "off".into()),
                config
                    .encoder_press_cc
                    .map(|cc| cc.to_string())
                    .unwrap_or_else(|| "off".into()),
                config
                    .lock_cc
                    .map(|cc| cc.to_string())
                    .unwrap_or_else(|| "off".into())
            );
            let mut controls = config.controls.iter().collect::<Vec<_>>();
            controls.sort_by_key(|x| x.0);
            for (incoming, target) in controls {
                println!("cc {incoming} -> mapped CC {target}");
            }
            let mut v = config.pads.iter().collect::<Vec<_>>();
            v.sort_by_key(|x| x.0);
            for (n, a) in v {
                println!("note {n}: {a}");
            }
            let mut v = config.cc_buttons.iter().collect::<Vec<_>>();
            v.sort_by_key(|x| x.0);
            for (cc, action) in v {
                println!("button CC {cc}: {action}");
            }
            Ok(())
        }
        "set" => {
            let n: u8 = args
                .get(1)
                .context("usage: shr pads set NOTE ACTION")?
                .parse()?;
            let a = args
                .get(2)
                .context("usage: shr pads set NOTE ACTION")?
                .parse()?;
            config.pads.insert(n, a);
            config.save(&path)
        }
        "clear" => {
            let n: u8 = args.get(1).context("usage: shr pads clear NOTE")?.parse()?;
            config.pads.remove(&n);
            config.save(&path)
        }
        "input" => {
            let name = args.get(1).context("usage: shr pads input PORT_MATCH")?;
            config.input_match = Some(name.clone());
            config.save(&path)
        }
        "layout" => {
            config.layout = match args.get(1).map(String::as_str) {
                Some("8" | "eight") => pads::ControllerLayout::Eight,
                Some("5" | "five") => pads::ControllerLayout::Five,
                Some("4" | "four") => pads::ControllerLayout::Four,
                _ => bail!("usage: shr pads layout 8|5|4"),
            };
            config.save(&path)
        }
        "cc" => {
            let incoming: u8 = args
                .get(1)
                .context("usage: shr pads cc INCOMING TARGET")?
                .parse()?;
            let target: u8 = args
                .get(2)
                .context("usage: shr pads cc INCOMING TARGET")?
                .parse()?;
            if control::by_cc(target).is_none() {
                bail!("TARGET must be one of the 12 mapped CC numbers");
            }
            config.controls.insert(incoming, target);
            config.save(&path)
        }
        other => bail!("unknown pads command: {other}"),
    }
}

fn update_controller_profiles() -> Result<()> {
    let path = controller_profile::user_catalog_path();
    let parent = path.parent().context("controller profile directory")?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    let status = Command::new("curl")
        .args([
            "--proto",
            "=https",
            "--tlsv1.2",
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            controller_profile::UPDATE_URL,
            "--output",
        ])
        .arg(&temporary)
        .status()
        .context("run curl to update controller profiles")?;
    if !status.success() {
        let _ = fs::remove_file(&temporary);
        bail!("controller profile download failed");
    }
    let count = controller_profile::validate_catalog(&temporary)?;
    fs::rename(&temporary, &path)?;
    println!(
        "Installed {count} controller profiles at {}",
        path.display()
    );
    Ok(())
}
