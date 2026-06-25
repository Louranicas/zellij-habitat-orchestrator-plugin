#!/bin/bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
TARGET_DIR="/tmp/habitat-zellij-target"
PLUGIN_DIR="$HOME/.config/zellij/plugins"
WASM_NAME="habitat_plugin.wasm"
PLUGIN_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_DIR/Cargo.toml" | head -n 1)"
VERSIONED_WASM_NAME="habitat-plugin-v${PLUGIN_VERSION}.wasm"

echo "Building habitat-plugin for wasm32-wasip1..."
CARGO_TARGET_DIR="$TARGET_DIR" cargo build \
    --manifest-path "$PROJECT_DIR/Cargo.toml" \
    --target wasm32-wasip1 \
    --release \
    -p habitat-plugin \
    2>&1

WASM_PATH="$TARGET_DIR/wasm32-wasip1/release/$WASM_NAME"

if [[ ! -f "$WASM_PATH" ]]; then
    echo "ERROR: WASM binary not found at $WASM_PATH"
    exit 1
fi

SIZE=$(du -h "$WASM_PATH" | cut -f1)
echo "Built: $WASM_PATH ($SIZE)"

mkdir -p "$PLUGIN_DIR"
/usr/bin/cp -f "$WASM_PATH" "$PLUGIN_DIR/habitat-plugin.wasm"
/usr/bin/cp -f "$WASM_PATH" "$PLUGIN_DIR/$VERSIONED_WASM_NAME"
echo "Deployed: $PLUGIN_DIR/habitat-plugin.wasm"
echo "Deployed: $PLUGIN_DIR/$VERSIONED_WASM_NAME"

if command -v zellij >/dev/null 2>&1; then
    zellij action start-or-reload-plugin "file:$PLUGIN_DIR/$VERSIONED_WASM_NAME" 2>/dev/null && \
        echo "Hot-reloaded in active session" || \
        echo "No active Zellij session for hot-reload (deploy only)"
fi

echo "Done. Launch with:"
echo "  zellij --layout $PROJECT_DIR/layouts/habitat-fleet.kdl"
echo "Force live replacement from the target pane with:"
echo "  zellij plugin --skip-plugin-cache --in-place --close-replaced-pane --configuration modules=orchestrator_kernel --configuration role=orchestrator_kernel --configuration sidecar_cli=/home/louranicas/.local/bin/orch-kernelctl --configuration kernel_poll=5 -- file:$PLUGIN_DIR/$VERSIONED_WASM_NAME"
