# GNOME Quick Web Apps — dev tasks. Run inside `nix develop`.
set shell := ["bash", "-uc"]

# Build the whole workspace (manager + runner + helper).
build:
    cargo build --workspace

# Run the unit tests.
test:
    cargo test -p qwa-core

# Launch the GTK4 manager (editor).
manager:
    cargo run -p gnome-quick-web-apps

# Symlink the CEF runtime next to the runner so cef_dir() resolves it.
deploy-cef: build
    #!/usr/bin/env bash
    set -euo pipefail
    out=$(find target -type d -path '*cef-dll-sys*/out/cef_linux_x86_64' | head -1)
    test -n "$out" || { echo "CEF build output not found"; exit 1; }
    ln -sfn "$out" target/debug/cef
    echo "linked target/debug/cef -> $out"

# Run a web app by id in the CEF window (create it in the manager first).
run id: deploy-cef
    cd target/debug && LD_LIBRARY_PATH="$PWD/cef:${LD_LIBRARY_PATH:-}" ./gnome-quick-web-apps-runner {{id}}
