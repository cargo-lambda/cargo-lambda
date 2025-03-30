#!/bin/sh

abort() {
  printf "%s\n" "$@"
  exit 1
}

# string formatters
if [ -t 1 ]; then
  tty_escape() { printf "\033[%sm" "$1"; }
else
  tty_escape() { :; }
fi
tty_mkbold() { tty_escape "1;$1"; }
tty_blue="$(tty_mkbold 34)"
tty_bold="$(tty_mkbold 39)"
tty_reset="$(tty_escape 0)"

ohai() {
  printf "${tty_blue}==>${tty_bold} %s${tty_reset}\n" "$1"
}

download() {
  if command -v curl > /dev/null 2>&1; then
    curl -fsSL "$1"
  else
    wget -qO- "$1"
  fi
}

expand() {
  mkdir -p "$2"
  if [ "${platform}" = "windows-x64" ]; then
    unzip "$1" -d "$2"
  else
    tar -xf "$1" -C "$2"
  fi
}

detect_platform() {
  local platform
  platform="$(uname -s | tr '[:upper:]' '[:lower:]')"

  case "${platform}" in
    linux) platform="unknown-linux-musl" ;;
    darwin) platform="apple-darwin" ;;
    windows) platform="windows-x64" ;;
    mingw*) platform="windows-x64" ;;
  esac

  printf '%s' "${platform}"
}

detect_arch() {
  local arch
  arch="$(uname -m | tr '[:upper:]' '[:lower:]')"

  case "${arch}" in
    x86_64 | amd64) arch="x86_64" ;;
    armv*) arch="aarch64" ;;
    arm64 | aarch64) arch="aarch64" ;;
  esac

  case "$arch" in
    x86_64*) ;;
    aarch64*) ;;
    *) return 1
  esac
  printf '%s' "${arch}"
}

download_and_install() {
  local platform arch version_json version archive_url tmp_dir target name
  platform="$(detect_platform)"

  case "${platform}" in
    unknown-linux-musl | apple-darwin)
      arch="$(detect_arch)" || abort "Sorry! Cargo Lambda currently only provides pre-built binaries for x86_64/arm64 architectures."
      target="${arch}-${platform}" ;;
    windows-x64) 
      target="${platform}" ;;
    *) return 1
  esac

  if [ -z "${CARGO_LAMBDA_VERSION}" ]; then
    version_json="$(download "https://www.cargo-lambda.info/latest-version.json")" || abort "Download Error!"
    version="$(echo "$version_json" | grep -o '"latest":[[:space:]]*"[0-9.]*"' | grep -o '[0-9.]*')"
  else
    version="${CARGO_LAMBDA_VERSION}"
  fi

  name="cargo-lambda-v${version}.${target}"
  if [ "${platform}" = "windows-x64" ]; then
    name="${name}.zip"
  else
    name="${name}.tar.gz"
  fi

  archive_url="https://github.com/cargo-lambda/cargo-lambda/releases/download/v${version}/${name}"

  tmp_dir="$(mktemp -d)" || abort "Tmpdir Error!"
  trap 'rm -rf "$tmp_dir"' EXIT INT TERM HUP

  ohai "Downloading Cargo Lambda version ${version}"
  # download the binary to the specified directory
  download "$archive_url" > "$tmp_dir/$name"  || return 1

  CARGO_HOME="${CARGO_HOME:-"$HOME/.cargo"}"
  expand "$tmp_dir/$name" "$CARGO_HOME/bin"

  ohai "Cargo Lambda was installed successfully to ${CARGO_HOME}/bin"

  ohai "Checking Zig installation"
  $CARGO_HOME/bin/cargo-lambda lambda system --install-zig

  ohai "Installation complete! Run 'cargo lambda --help' to get started"
}

download_and_install || abort "Install Error!"
