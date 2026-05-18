# kvm-switch

Tiny cross-platform CLI that flips a monitor's active input via **DDC/CI**.

Built for monitors with a built-in **KVM switch** — where switching the video input also switches the monitor's USB hub. Bind the same hotkey on each computer and you have a one-keystroke KVM with no extra hardware.

Confirmed working on a **Gigabyte M32U** between Linux (DisplayPort) and a Mac (USB-C / DP-alt-mode). Should work on any DDC/CI-capable monitor — the values you put in the config will be monitor-specific.

## How it works

DDC/CI exposes a small set of "VCP" controls on most modern monitors. Feature `0x60` is **Input Source** — writing a value to it tells the monitor to switch inputs. On monitors with a built-in KVM, the USB hub follows.

`kvm-switch` is a thin wrapper around the [`ddc-hi`](https://crates.io/crates/ddc-hi) crate that:

1. Reads a per-host config file (`~/.config/kvm-switch/config.toml` on Linux, `~/Library/Application Support/kvm-switch/config.toml` on macOS).
2. Writes `target_input` to VCP `0x60` on the matching display.

Because the binary just sends "set input source = X", each host's config holds the value of **the other** host's input. Hit the hotkey here → the monitor flips to the other machine, and the hotkey on *that* machine flips it back.

## Install

### One-liner (Linux + macOS)

```sh
curl -fsSL https://raw.githubusercontent.com/jensenbox/kvm-switch/main/install.sh | sh
```

Downloads the latest signed release tarball for your OS+arch, verifies the SHA-256, and drops the binary in `~/.local/bin` (Linux) or `/usr/local/bin` (macOS, prompts for `sudo`). On Linux it also reminds you to add yourself to the `i2c` group.

Pin a version: `KVM_VERSION=v0.1.0 curl -fsSL .../install.sh | sh`.
Custom prefix: `KVM_PREFIX=/opt curl -fsSL .../install.sh | sh`.

### Homebrew (macOS / Linux)

```sh
brew install jensenbox/tap/kvm-switch
```

### Build from source

```sh
git clone https://github.com/jensenbox/kvm-switch
cd kvm-switch
cargo build --release
ln -s "$PWD/target/release/kvm-switch" ~/.local/bin/kvm-switch
```

### Linux: grant i2c access (one-time)

DDC/CI on Linux is exposed through `/dev/i2c-*`. Two options:

**Group membership (recommended):**

```sh
sudo usermod -aG i2c "$USER"
# then log out and back in for the new group to take effect
```

**Or a udev rule** (if you can't log out — but logout is simpler):

```sh
echo 'KERNEL=="i2c-[0-9]*", GROUP="i2c", MODE="0660"' | \
  sudo tee /etc/udev/rules.d/45-kvm-switch-i2c.rules
sudo udevadm control --reload && sudo udevadm trigger
```

After either, `kvm-switch list` should print your monitor without `sudo`.

### macOS

No special permissions needed — ddc-hi talks to `CoreDisplay` / `IOKit`. Just build and the binary works.

## Configure

Write `~/.config/kvm-switch/config.toml` (Linux) or `~/Library/Application Support/kvm-switch/config.toml` (macOS):

```toml
# The VCP 0x60 value to write when this machine triggers the switch.
# = the OTHER computer's monitor input.
target_input = 0x10

# Optional. Only needed if multiple DDC displays are connected.
# Use the exact model string from `kvm-switch list`.
# [monitor]
# model_name = "Gigabyte M32U"
```

Per-host examples (Gigabyte M32U):

| Host | `target_input` | Means |
|------|---------------|-------|
| Linux (on DP-1, `0x0f`) | `0x10` | "flip to Mac (USB-C)" |
| Mac (on USB-C, `0x10`) | `0x0f` | "flip to Linux (DP-1)" |

**Finding your monitor's values:** `kvm-switch list` to confirm it's seen, then probe with `kvm-switch set 0xNN` while watching the screen. Common ranges: `0x0f` / `0x10` (DisplayPort 1/2), `0x11` / `0x12` (HDMI 1/2), sometimes `0x1B` for USB-C. Capabilities reported by `ddcutil capabilities` are often incomplete — don't trust the advertised list as exhaustive.

## Usage

```sh
kvm-switch          # = `switch` — set VCP 0x60 to config's target_input
kvm-switch switch   # same thing
kvm-switch list     # list detected displays
kvm-switch get      # read current VCP 0x60
kvm-switch set 0x10 # write a value directly (hex)
```

## Hotkey binding

Pick one hotkey for both machines (e.g. `Ctrl+Alt+K`) and bind it on each OS.

### Linux

| Environment | Where |
|-------------|-------|
| GNOME | Settings → Keyboard → Keyboard Shortcuts → Custom |
| KDE Plasma | System Settings → Shortcuts → Custom Shortcuts |
| sway / i3 | `bindsym Ctrl+Mod1+k exec kvm-switch` in config |
| Hyprland | `bind = CTRL ALT, K, exec, kvm-switch` |

### macOS

Built-in shortcuts can't run arbitrary binaries; use one of:

- **Hammerspoon** (`~/.hammerspoon/init.lua`):

  ```lua
  hs.hotkey.bind({"ctrl", "alt"}, "K", function()
    hs.execute("/usr/local/bin/kvm-switch", true)
  end)
  ```

- **Karabiner-Elements** with a `complex_modifications` rule that runs `kvm-switch` on `ctrl+option+k`.

- **Raycast / Alfred** with a script command.

## Caveats

- Switching the video also switches the KVM's USB-B port. The moment you flip away, the source machine loses keyboard/mouse — there's no "switch back from the machine that just lost focus" unless you bind a hotkey on the other side too.
- DDC/CI write throughput is glacial (~50–200 ms) and some firmware briefly locks out further writes for ~1 s after a successful set. The binary doesn't retry — it just writes once.
- VCP `0x60` semantics vary across monitor firmware. The advertised capabilities can be incomplete (e.g. M32U firmware advertises only VGA/DVI/DP-1/HDMI-1 even though DP-2/HDMI-2/USB-C all work). Probe with `kvm-switch set 0xNN` to find your values.
- No retries, no daemon, no IPC. One binary, one config, one hotkey per host.

## License

MIT — see [LICENSE](LICENSE).
