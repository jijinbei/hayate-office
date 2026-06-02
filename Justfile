# HayateOffice task runner.
#
# Core recipes (test/build/fmt) are pure Rust and run with your system cargo.
# gpui recipes (build-app/run/clippy) auto-enter the Nix dev shell (flake.nix) for the
# Wayland/Vulkan/X11 build deps. `run` additionally injects the host GPU driver via
# nix-gl-host so the window works on non-NixOS (e.g. Ubuntu + NVIDIA).

set shell := ["bash", "-uc"]

# List available recipes.
default:
    @just --list

# Run core-crate tests (pure Rust; no gpui).
test:
    cargo test -p hayate-ir -p hayate-model -p hayate-core -p hayate-render -p hayate-format

# Generate visual debug snapshots (gpui-free): runs edit scenarios and writes debug-shots/*.png.
shots:
    cargo run -p hayate-shot

# Build the core crates (pure Rust).
build:
    cargo build -p hayate-ir -p hayate-model -p hayate-render -p hayate-format

# Format all crates.
fmt:
    cargo fmt --all

# Format-check + core tests: a quick pre-commit gate.
check:
    cargo fmt --all --check
    cargo test -p hayate-ir -p hayate-model -p hayate-render -p hayate-format

# Compile the gpui app inside the Nix dev shell.
build-app:
    nix develop --command cargo build -p hayate-app

# Run the gpui interaction E2E tests (headless, inside the Nix dev shell).
e2e:
    nix develop --command cargo test -p hayate-app

# Run the gpui app. Injects the host GPU driver (non-NixOS) via nix-gl-host.
# Falls back to a plain run if nix-gl-host is unavailable (e.g. on NixOS).
run *ARGS:
    nix develop --command bash -uc 'nix run github:numtide/nix-gl-host -- cargo run -p hayate-app {{ARGS}}'

# Run the app without driver injection (use on NixOS, or to debug nix-gl-host issues).
run-plain *ARGS:
    nix develop --command cargo run -p hayate-app {{ARGS}}

# Install a desktop entry so the Wayland/GNOME taskbar shows the logo (assets/logo2.png).
# The compositor matches the window's app_id ("hayate-office") to this entry to find the icon.
install-desktop:
    #!/usr/bin/env bash
    set -euo pipefail
    repo="$(pwd)"
    apps="$HOME/.local/share/applications"
    mkdir -p "$apps"
    chmod +x "$repo/scripts/hayate-office.sh"
    cat > "$apps/hayate-office.desktop" <<EOF
    [Desktop Entry]
    Type=Application
    Name=HayateOffice
    Comment=Fast, lightweight presentation editor
    Exec=$repo/scripts/hayate-office.sh
    Icon=$repo/assets/logo2.png
    Terminal=false
    Categories=Office;Presentation;
    StartupWMClass=hayate-office
    EOF
    update-desktop-database "$apps" 2>/dev/null || true
    echo "Installed $apps/hayate-office.desktop (Icon=$repo/assets/logo2.png)"

# Lint the whole workspace (gpui app included).
clippy:
    nix develop --command cargo clippy --workspace --all-targets

# Print Vulkan adapter info from inside the dev shell (diagnostics).
vulkan-info:
    nix develop --command bash -uc 'nix run github:numtide/nix-gl-host -- vulkaninfo --summary'
