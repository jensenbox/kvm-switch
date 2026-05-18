#!/usr/bin/env sh
# kvm-switch installer
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/jensenbox/kvm-switch/main/install.sh | sh
#
# Env vars:
#   KVM_VERSION   Specific version (e.g. v0.1.0). Default: latest release.
#   KVM_PREFIX    Install prefix. Default: ~/.local on Linux, /usr/local on macOS.
#   KVM_FORCE     If non-empty, overwrite existing binary without prompting.

set -eu

REPO="jensenbox/kvm-switch"
BIN="kvm-switch"

# ---------- helpers ----------

say() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"
}

# ---------- detect platform ----------

uname_s="$(uname -s)"
uname_m="$(uname -m)"

case "$uname_s" in
    Linux)  os="unknown-linux-gnu"; default_prefix="${HOME}/.local" ;;
    Darwin) os="apple-darwin";      default_prefix="/usr/local" ;;
    *)      die "unsupported OS: $uname_s" ;;
esac

case "$uname_m" in
    x86_64|amd64)
        if [ "$uname_s" = "Darwin" ]; then
            die "Intel Macs are not built in releases (build from source: cargo build --release)"
        fi
        arch="x86_64"
        ;;
    arm64|aarch64)
        if [ "$uname_s" = "Linux" ]; then
            die "Linux aarch64 is not built in releases yet (build from source: cargo build --release)"
        fi
        arch="aarch64"
        ;;
    *) die "unsupported arch: $uname_m" ;;
esac

target="${arch}-${os}"
prefix="${KVM_PREFIX:-$default_prefix}"
bindir="${prefix}/bin"

# ---------- tools ----------

need uname
need tar
need sha256sum 2>/dev/null || need shasum
DL=""
if command -v curl >/dev/null 2>&1; then
    DL="curl"
elif command -v wget >/dev/null 2>&1; then
    DL="wget"
else
    die "need curl or wget"
fi

dl() {
    # dl <url> <dest>
    if [ "$DL" = "curl" ]; then
        curl -fsSL "$1" -o "$2"
    else
        wget -qO "$2" "$1"
    fi
}

sha256_check() {
    # sha256_check <file> <expected>
    file="$1"; expected="$2"
    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$file" | awk '{print $1}')"
    else
        actual="$(shasum -a 256 "$file" | awk '{print $1}')"
    fi
    [ "$actual" = "$expected" ] || die "sha256 mismatch for $file (expected $expected, got $actual)"
}

# ---------- pick version ----------

if [ -n "${KVM_VERSION:-}" ]; then
    version="$KVM_VERSION"
    case "$version" in v*) ;; *) version="v$version" ;; esac
else
    say "Looking up latest release..."
    # Use the redirect on /releases/latest to find the tag without an API token.
    latest_url="https://github.com/${REPO}/releases/latest"
    if [ "$DL" = "curl" ]; then
        resolved="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$latest_url")"
    else
        resolved="$(wget --max-redirect=5 --server-response -q -O /dev/null "$latest_url" 2>&1 \
            | awk '/^  Location: / {print $2}' | tail -n1)"
    fi
    [ -n "$resolved" ] || die "could not resolve latest release"
    version="${resolved##*/tag/}"
fi

[ -n "$version" ] || die "no version determined"
say "Installing kvm-switch $version for $target"

# ---------- download + verify ----------

asset="${BIN}-${version#v}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/download/${version}/${asset}"
sha_url="${url}.sha256"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "  $url"
dl "$url" "$tmp/$asset"

say "  verifying checksum"
dl "$sha_url" "$tmp/$asset.sha256"
expected="$(awk '{print $1}' "$tmp/$asset.sha256")"
sha256_check "$tmp/$asset" "$expected"

say "  extracting"
tar -xzf "$tmp/$asset" -C "$tmp"

src="$tmp/${BIN}-${version#v}-${target}/${BIN}"
[ -x "$src" ] || die "binary not found in archive: $src"

# ---------- install ----------

mkdir -p "$bindir" 2>/dev/null || true
dst="$bindir/$BIN"

needs_sudo=0
if ! ( : > "$dst" ) 2>/dev/null; then
    needs_sudo=1
fi
rm -f "$dst" 2>/dev/null || true

if [ -e "$dst" ] && [ -z "${KVM_FORCE:-}" ]; then
    warn "$dst already exists (set KVM_FORCE=1 to overwrite)"
    exit 1
fi

if [ "$needs_sudo" = "1" ]; then
    say "  installing to $dst (sudo)"
    sudo install -m 0755 "$src" "$dst"
else
    install -m 0755 "$src" "$dst"
    say "  installed to $dst"
fi

# ---------- next steps ----------

cat <<EOF

Installed kvm-switch $version.

Next steps:

  1. Write your config at:
EOF

if [ "$uname_s" = "Linux" ]; then
    cfg="${XDG_CONFIG_HOME:-$HOME/.config}/kvm-switch/config.toml"
else
    cfg="$HOME/Library/Application Support/kvm-switch/config.toml"
fi

cat <<EOF
       $cfg

     Example:
       target_input = 0x10  # the OTHER computer's VCP 0x60 value

  2. Probe your monitor and find the right value:
       kvm-switch list
       kvm-switch set 0x10   # try values; the screen will switch when right

EOF

if [ "$uname_s" = "Linux" ]; then
    cat <<'EOF'
  3. Grant i2c access (Linux only, one-time):
       sudo usermod -aG i2c "$USER"
       # then log out and back in for the group to take effect

EOF
fi

case ":$PATH:" in
    *":$bindir:"*) ;;
    *) say "  note: $bindir is not on your PATH — add it to your shell rc" ;;
esac
