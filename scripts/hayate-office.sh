#!/usr/bin/env bash
# Launcher used by the desktop entry: run the app from the project root via `just run`
# (which enters the Nix dev shell and injects the host GPU driver). Kept as a stable
# absolute target so the .desktop Exec does not depend on the caller's working directory.
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")/.."
exec just run
