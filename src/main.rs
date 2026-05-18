use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use ddc_hi::{Ddc, Display};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

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
        None | Some(Cmd::Switch) => {
            let path = cli.config.map_or_else(default_config_path, Ok)?;
            let cfg = load_config(&path)?;
            cmd_set(cfg.target_input, cfg.monitor.as_ref())
        }
    }
}
