use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use ddc_hi::{Ddc, Display};
use serde::Deserialize;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

const VCP_INPUT_SOURCE: u8 = 0x60;

#[derive(Parser)]
#[command(version, about = "Switch monitor input via DDC/CI")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,

    #[arg(long, help = "Override config path")]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Cmd {
    Switch,
    List,
    Get,
    Set {
        #[arg(value_parser = parse_hex_u16)]
        value: u16,
    },
    /// Interactive setup — detects the other computer's input by watching for an OSD switch
    Setup {
        /// Overwrite an existing config without prompting
        #[arg(long)]
        force: bool,
        /// Max seconds to wait for the input change after the prompt
        #[arg(long, default_value_t = 60)]
        timeout: u64,
    },
    /// Self-update from the latest GitHub release
    Update {
        /// Only check for a newer version; don't install
        #[arg(long)]
        check: bool,
        /// Skip the confirmation prompt
        #[arg(short = 'y', long = "yes")]
        no_confirm: bool,
    },
}

fn parse_hex_u16(s: &str) -> Result<u16, String> {
    let s = s.trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(s, 16).map_err(|e| format!("not a hex u16: {e}"))
}

#[derive(Deserialize)]
struct Config {
    target_input: u16,
    monitor: Option<MonitorFilter>,
}

#[derive(Deserialize, Default)]
struct MonitorFilter {
    model_name: Option<String>,
}

fn default_config_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "kvm-switch")
        .ok_or_else(|| anyhow!("could not determine config dir"))?;
    Ok(dirs.config_dir().join("config.toml"))
}

fn load_config(path: &PathBuf) -> Result<Config> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading config {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parsing config {}", path.display()))
}

fn pick_display(filter: Option<&MonitorFilter>) -> Result<Display> {
    let mut matched: Vec<Display> = Display::enumerate()
        .into_iter()
        .filter(|d| match filter.and_then(|f| f.model_name.as_ref()) {
            None => true,
            Some(want) => d.info.model_name.as_deref() == Some(want.as_str()),
        })
        .collect();

    match matched.len() {
        0 => bail!("no matching display found"),
        1 => Ok(matched.remove(0)),
        n => bail!(
            "{n} displays match — tighten [monitor].model_name in the config (candidates: {})",
            matched
                .iter()
                .map(|d| d.info.model_name.clone().unwrap_or_else(|| "(unnamed)".into()))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn cmd_list() -> Result<()> {
    let displays: Vec<Display> = Display::enumerate().into_iter().collect();
    if displays.is_empty() {
        println!("no displays detected");
        return Ok(());
    }
    for d in displays {
        println!(
            "- backend={:?} id={} mfg={} model={}",
            d.info.backend,
            d.info.id,
            d.info.manufacturer_id.as_deref().unwrap_or("?"),
            d.info.model_name.as_deref().unwrap_or("?"),
        );
    }
    Ok(())
}

fn cmd_get(filter: Option<&MonitorFilter>) -> Result<()> {
    let mut display = pick_display(filter)?;
    let v = display
        .handle
        .get_vcp_feature(VCP_INPUT_SOURCE)
        .with_context(|| "reading VCP 0x60")?;
    println!(
        "VCP 0x60 = 0x{:02x} (mh={:02x} ml={:02x} sh={:02x} sl={:02x})",
        v.value(),
        v.mh,
        v.ml,
        v.sh,
        v.sl,
    );
    Ok(())
}

fn cmd_set(value: u16, filter: Option<&MonitorFilter>) -> Result<()> {
    let mut display = pick_display(filter)?;
    display
        .handle
        .set_vcp_feature(VCP_INPUT_SOURCE, value)
        .with_context(|| format!("setting VCP 0x60 = 0x{:02x}", value))?;
    println!("set VCP 0x60 = 0x{:02x}", value);
    Ok(())
}

fn prompt(question: &str) -> Result<String> {
    print!("{question}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn cmd_setup(force: bool, timeout_secs: u64, config_path: PathBuf) -> Result<()> {
    if config_path.exists() && !force {
        let answer = prompt(&format!(
            "Config {} already exists. Overwrite? [y/N] ",
            config_path.display()
        ))?;
        if !matches!(answer.to_lowercase().as_str(), "y" | "yes") {
            bail!("aborted — config not written");
        }
    }

    let displays: Vec<Display> = Display::enumerate().into_iter().collect();
    if displays.is_empty() {
        bail!(
            "no DDC/CI displays detected. On Linux you need read/write access \
             to /dev/i2c-* — see https://github.com/jensenbox/kvm-switch#install"
        );
    }

    let chosen = if displays.len() == 1 {
        displays.into_iter().next().unwrap()
    } else {
        println!("Multiple displays detected:");
        for (i, d) in displays.iter().enumerate() {
            println!(
                "  [{i}] {} (mfg={}, id={})",
                d.info.model_name.as_deref().unwrap_or("?"),
                d.info.manufacturer_id.as_deref().unwrap_or("?"),
                d.info.id,
            );
        }
        let answer = prompt("Pick the monitor with the KVM by number: ")?;
        let idx: usize = answer.parse().context("not a number")?;
        displays
            .into_iter()
            .nth(idx)
            .ok_or_else(|| anyhow!("no display at index {idx}"))?
    };

    let model = chosen
        .info
        .model_name
        .clone()
        .unwrap_or_else(|| "(unknown)".into());
    println!("Using display: {model}");

    let mut display = chosen;
    let initial = display
        .handle
        .get_vcp_feature(VCP_INPUT_SOURCE)
        .context("reading current input source")?;
    let local_value = initial.value();
    println!("Current input (this machine): 0x{local_value:02x}");
    println!();
    println!(
        "Now switch the monitor to the OTHER computer using the OSD joystick. \
         You don't need to switch back — this script will detect the change \
         and write the config silently. Re-run from the other machine to \
         configure the reverse direction."
    );
    println!();
    print!("Waiting up to {timeout_secs}s for the input to change");
    io::stdout().flush().ok();

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut target_value: Option<u16> = None;
    while Instant::now() < deadline {
        sleep(Duration::from_millis(500));
        match display.handle.get_vcp_feature(VCP_INPUT_SOURCE) {
            Ok(v) if v.value() != local_value => {
                target_value = Some(v.value());
                break;
            }
            _ => {
                print!(".");
                io::stdout().flush().ok();
            }
        }
    }
    println!();

    let target_value = target_value.ok_or_else(|| {
        anyhow!("timed out — no input change detected within {timeout_secs}s")
    })?;
    println!("Detected other input: 0x{target_value:02x}");

    let body = format!(
        "# Written by `kvm-switch setup`. Hit your hotkey on this machine\n\
         # and the monitor flips to `target_input`.\n\
         target_input = 0x{target_value:02x}\n\
         \n\
         [monitor]\n\
         model_name = \"{model}\"\n",
    );

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&config_path, body).with_context(|| format!("writing {}", config_path.display()))?;
    println!();
    println!("Wrote {}", config_path.display());
    println!();
    println!("Test: run `kvm-switch` on this machine — the monitor flips to 0x{target_value:02x}.");
    println!("Then run `kvm-switch setup` on the OTHER machine to set up the reverse.");
    Ok(())
}

fn cmd_update(check_only: bool, no_confirm: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("kvm-switch v{current}");

    if let Ok(exe) = std::env::current_exe() {
        let p = exe.to_string_lossy();
        if p.contains("/Cellar/") || p.contains("/opt/homebrew/") || p.contains("/linuxbrew/") {
            eprintln!(
                "warning: this binary lives at {p} — that looks like a Homebrew install.\n\
                 Prefer `brew upgrade jensenbox/tap/kvm-switch` so brew's bookkeeping stays correct."
            );
        }
    }

    let builder = self_update::backends::github::Update::configure()
        .repo_owner("jensenbox")
        .repo_name("kvm-switch")
        .bin_name("kvm-switch")
        .show_download_progress(true)
        .current_version(current)
        .no_confirm(no_confirm)
        .build()
        .context("configuring self_update")?;

    if check_only {
        let release = builder
            .get_latest_release()
            .context("fetching latest release")?;
        println!("Latest release: v{}", release.version);
        let newer = self_update::version::bump_is_compatible(current, &release.version)
            .unwrap_or(false);
        if newer {
            println!(
                "An update is available — run `kvm-switch update` (or `-y` to skip the prompt) to install."
            );
        } else if release.version == current {
            println!("Up to date.");
        } else {
            println!("You're ahead of the latest release (running a dev or pre-release build).");
        }
        return Ok(());
    }

    let status = builder.update().context("running self-update")?;
    match status {
        self_update::Status::UpToDate(v) => println!("Already on the latest version (v{v})."),
        self_update::Status::Updated(v) => println!("Updated to v{v}."),
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.cmd {
        Some(Cmd::List) => cmd_list(),
        Some(Cmd::Get) => {
            let path = cli.config.map_or_else(default_config_path, Ok)?;
            let cfg = load_config(&path).ok();
            cmd_get(cfg.as_ref().and_then(|c| c.monitor.as_ref()))
        }
        Some(Cmd::Set { value }) => {
            let path = cli.config.map_or_else(default_config_path, Ok)?;
            let cfg = load_config(&path).ok();
            cmd_set(value, cfg.as_ref().and_then(|c| c.monitor.as_ref()))
        }
        Some(Cmd::Setup { force, timeout }) => {
            let path = cli.config.map_or_else(default_config_path, Ok)?;
            cmd_setup(force, timeout, path)
        }
        Some(Cmd::Update { check, no_confirm }) => cmd_update(check, no_confirm),
        None | Some(Cmd::Switch) => {
            let path = cli.config.map_or_else(default_config_path, Ok)?;
            let cfg = load_config(&path)?;
            cmd_set(cfg.target_input, cfg.monitor.as_ref())
        }
    }
}
