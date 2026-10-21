#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
if [[ -x "$HOME/.cargo/bin/cargo" ]]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
export STRATA_DEV=1
export RUST_BACKTRACE="${RUST_BACKTRACE:-1}"

native_dependencies_available() {
  command -v pkg-config >/dev/null 2>&1 \
    && pkg-config --exists fontconfig \
    && pkg-config --exists 'gtk4 >= 4.12' \
    && pkg-config --exists gtksourceview-5 \
    && pkg-config --exists poppler-glib \
    && pkg-config --exists 'libheif >= 1.17'
}

run_as_root() {
  if (( EUID == 0 )); then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    echo "Native development dependencies are missing, and sudo is unavailable." >&2
    return 1
  fi
}

install_native_dependencies() {
  echo "Native development dependencies are missing; installing them now..."

  if command -v pacman >/dev/null 2>&1; then
    run_as_root pacman -S --needed base-devel rust fontconfig gtk4 gtksourceview5 poppler-glib libheif
  elif command -v apt-get >/dev/null 2>&1; then
    run_as_root apt-get update
    run_as_root apt-get install -y build-essential cargo rustc pkg-config \
      libfontconfig1-dev libgtk-4-dev libgtksourceview-5-dev libpoppler-glib-dev libheif-dev
  elif command -v dnf >/dev/null 2>&1; then
    run_as_root dnf install -y gcc gcc-c++ make rust cargo pkgconf-pkg-config \
      fontconfig-devel gtk4-devel gtksourceview5-devel poppler-glib-devel libheif-devel
  else
    echo "Unsupported package manager. Install the native dependencies listed in README.md." >&2
    return 1
  fi

  if ! native_dependencies_available; then
    echo "Native dependency installation completed, but required pkg-config libraries are still unavailable." >&2
    return 1
  fi
}

if ! command -v cargo >/dev/null 2>&1 || ! native_dependencies_available; then
  install_native_dependencies
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "Cargo is unavailable after installing development dependencies." >&2
  exit 1
fi

if ! cargo watch --version >/dev/null 2>&1; then
  echo "cargo-watch is not installed; installing it now..."
  cargo install --locked cargo-watch
fi

log_directory="${XDG_STATE_HOME:-$HOME/.local/state}/hermes"
mkdir -p "$log_directory"
exec > >(tee -a "$log_directory/dev.log") 2>&1
build_command="build --jobs 2"
build_runner=(nice -n 10)
if command -v ionice >/dev/null 2>&1; then
  build_runner+=(ionice -c 3)
fi
run_command="run"
binary="target/debug/strata"
if [[ "${HERMES_DEV_RELEASE:-0}" == "1" ]]; then
  build_command="build --release --jobs 2"
  run_command="run --release"
  binary="target/release/strata"
fi
if [[ "${HERMES_DEV_AUTO_RESTART:-0}" == "1" ]]; then
  exec cargo watch \
    --watch src \
    --watch data \
    --watch Cargo.toml \
    --watch Cargo.lock \
    --watch build.rs \
    --exec "$run_command"
fi
if [[ "${HERMES_DEV_RELEASE:-0}" == "1" ]]; then
  "${build_runner[@]}" cargo build --release --jobs 2
else
  "${build_runner[@]}" cargo build --jobs 2
fi
"$binary" &
echo "Hermes stays open while edits rebuild in the background. Reopen Hermes when ready to use the new build."
exec cargo watch \
  --postpone \
  --watch src \
  --watch data \
  --watch Cargo.toml \
  --watch Cargo.lock \
  --watch build.rs \
  --shell "${build_runner[*]} cargo $build_command"
