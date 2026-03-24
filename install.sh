#!/bin/bash
# Install anthill binary and auto-start service.
# Supports Linux (systemd) and macOS (launchd).
#
# Usage:
#   cd anthill
#   ./install.sh            # production — info-level logging
#   ./install.sh dev        # development — debug-level logging

set -e

MODE="${1:-prod}"
USER_NAME="$(whoami)"
USER_HOME="$HOME"
CONFIG_DIR="$USER_HOME/.config/anthill"
OS="$(uname -s)"

if [ "$MODE" = "dev" ]; then
    RUST_LOG_LEVEL="debug"
    echo "Installing in DEV mode (debug logging)"
else
    RUST_LOG_LEVEL="info"
    echo "Installing in production mode"
fi

echo "User: $USER_NAME ($USER_HOME)"
echo "Platform: $OS"
echo ""

# --- Build ---
echo "Building release binary..."
cargo build -p anthill --release

BINARY="target/release/anthill"

# --- Install binary ---
if [ "$OS" = "Darwin" ]; then
    INSTALL_DIR="/usr/local/bin"
    # Ensure /usr/local/bin exists (not always present on macOS).
    if [ ! -d "$INSTALL_DIR" ]; then
        sudo mkdir -p "$INSTALL_DIR"
    fi
    echo "Installing binary to $INSTALL_DIR/anthill..."
    sudo cp "$BINARY" "$INSTALL_DIR/anthill"
    sudo chmod 755 "$INSTALL_DIR/anthill"
else
    INSTALL_DIR="/usr/local/bin"
    # Stop the service if running (binary can't be overwritten while running).
    if command -v systemctl &>/dev/null && systemctl is-active --quiet anthill 2>/dev/null; then
        echo "Stopping anthill service..."
        sudo systemctl stop anthill
        sleep 1
    fi
    echo "Installing binary to $INSTALL_DIR/anthill..."
    sudo cp "$BINARY" "$INSTALL_DIR/anthill"
    sudo chmod 755 "$INSTALL_DIR/anthill"
fi

# --- Config directory ---
echo "Setting up config directory at $CONFIG_DIR..."
mkdir -p "$CONFIG_DIR/ants"

if [ ! -f "$CONFIG_DIR/supervisor.toml" ]; then
    cp config-example/supervisor.toml "$CONFIG_DIR/supervisor.toml"
    echo "  Created supervisor.toml"
else
    echo "  supervisor.toml already exists, skipping"
fi

# --- Platform-specific service ---
if [ "$OS" = "Darwin" ]; then
    # macOS — launchd
    PLIST_NAME="ai.reality2.anthill"
    PLIST_DIR="$USER_HOME/Library/LaunchAgents"
    PLIST_FILE="$PLIST_DIR/$PLIST_NAME.plist"

    echo ""
    echo "Generating launchd plist (RUST_LOG=$RUST_LOG_LEVEL)..."
    mkdir -p "$PLIST_DIR"
    sed -e "s|@@BINARY@@|$INSTALL_DIR/anthill|g" \
        -e "s|@@CONFIG@@|$CONFIG_DIR|g" \
        -e "s|@@HOME@@|$USER_HOME|g" \
        -e "s|<string>info</string>|<string>$RUST_LOG_LEVEL</string>|g" \
        anthill.plist.template > "$PLIST_FILE"

    # Unload if already loaded, then load.
    launchctl bootout "gui/$(id -u)/$PLIST_NAME" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$PLIST_FILE"

    echo ""
    echo "============================================"
    echo "  Anthill installed successfully! (macOS)"
    echo "============================================"
    echo ""
    echo "  Config: $CONFIG_DIR"
    echo "  Binary: $INSTALL_DIR/anthill"
    echo "  Service: $PLIST_FILE"
    echo ""

    # Check if any ants exist.
    ANT_COUNT=$(find "$CONFIG_DIR/ants" -name "ant.toml" 2>/dev/null | wc -l | tr -d ' ')
    if [ "$ANT_COUNT" -eq 0 ]; then
        echo "  No ANTS configured yet. Create your first:"
        echo ""
        echo "    mkdir -p $CONFIG_DIR/ants/my-ant"
        echo "    cp config-example/ants/dev-assistant/ant.toml $CONFIG_DIR/ants/my-ant/ant.toml"
        echo "    \$EDITOR $CONFIG_DIR/ants/my-ant/ant.toml"
        echo ""
        echo "  Or create one from the web dashboard after starting."
        echo ""
    else
        echo "  Found $ANT_COUNT ANT(s) configured."
        echo ""
    fi

    echo "  Service commands:"
    echo "    launchctl kickstart gui/$(id -u)/$PLIST_NAME   # restart"
    echo "    launchctl kill SIGTERM gui/$(id -u)/$PLIST_NAME # stop"
    echo "    launchctl bootout gui/$(id -u)/$PLIST_NAME     # unload"
    echo ""

    if [ "$MODE" = "dev" ]; then
        echo "  DEV mode — debug logging enabled."
        echo "  Re-install without 'dev' for production: ./install.sh"
        echo ""
        echo "  Add a device:"
        echo "    anthill --qr-join                 Show QR code to scan with phone"
        echo "    anthill --join-code               Generate a text join code"
        echo ""
        echo "  Web dashboard:"
        echo "    http://localhost:3000"
        echo ""
        echo "  Starting log tail (Ctrl+C to exit)..."
        echo "============================================"
        echo ""
        exec tail -f "$CONFIG_DIR/anthill.log" "$CONFIG_DIR/anthill.err"
    else
        echo "  Logs:"
        echo "    tail -f $CONFIG_DIR/anthill.log"
        echo "    tail -f $CONFIG_DIR/anthill.err"
        echo ""
        echo "  Add a device:"
        echo "    anthill --qr-join                 Show QR code to scan with phone"
        echo "    anthill --join-code               Generate a text join code"
        echo ""
        echo "  Web dashboard:"
        echo "    http://localhost:3000"
        echo ""
    fi

elif [ "$OS" = "FreeBSD" ] || [ "$OS" = "OpenBSD" ] || [ "$OS" = "NetBSD" ]; then
    # BSD — rc.d
    RC_DIR="/usr/local/etc/rc.d"
    RC_FILE="$RC_DIR/anthill"

    echo ""
    echo "Generating rc.d script..."
    sed -e "s|@@USER@@|$USER_NAME|g" \
        -e "s|@@HOME@@|$USER_HOME|g" \
        anthill.rc.template > /tmp/anthill.rc
    sudo cp /tmp/anthill.rc "$RC_FILE"
    sudo chmod 755 "$RC_FILE"
    rm /tmp/anthill.rc

    echo ""
    echo "============================================"
    echo "  Anthill installed successfully! (BSD)"
    echo "============================================"
    echo ""
    echo "  Config: $CONFIG_DIR"
    echo "  Binary: $INSTALL_DIR/anthill"
    echo ""
    echo "  Enable in /etc/rc.conf:"
    echo "    echo 'anthill_enable=\"YES\"' >> /etc/rc.conf"
    echo ""
    echo "  Service commands:"
    echo "    service anthill start"
    echo "    service anthill stop"
    echo "    service anthill restart"
    echo ""
    echo "  Add a device:"
    echo "    anthill --qr-join                 Show QR code to scan with phone"
    echo "    anthill --join-code               Generate a text join code"
    echo ""
    echo "  Web dashboard:"
    echo "    http://localhost:3000"
    echo ""

else
    # Linux — systemd
    echo ""
    echo "Generating systemd service for user '$USER_NAME' (RUST_LOG=$RUST_LOG_LEVEL)..."
    sed -e "s|@@USER@@|$USER_NAME|g" \
        -e "s|@@HOME@@|$USER_HOME|g" \
        -e "s|RUST_LOG=info|RUST_LOG=$RUST_LOG_LEVEL|g" \
        anthill.service.template > /tmp/anthill.service
    sudo cp /tmp/anthill.service /etc/systemd/system/anthill.service
    rm /tmp/anthill.service
    sudo systemctl daemon-reload

    echo ""
    echo "============================================"
    echo "  Anthill installed successfully! (Linux)"
    echo "============================================"
    echo ""
    echo "  Config: $CONFIG_DIR"
    echo "  Binary: $INSTALL_DIR/anthill"
    echo ""

    # Check if any ants exist.
    ANT_COUNT=$(find "$CONFIG_DIR/ants" -name "ant.toml" 2>/dev/null | wc -l)
    if [ "$ANT_COUNT" -eq 0 ]; then
        echo "  No ANTS configured yet. Create your first:"
        echo ""
        echo "    mkdir -p $CONFIG_DIR/ants/my-ant"
        echo "    cp config-example/ants/dev-assistant/ant.toml $CONFIG_DIR/ants/my-ant/ant.toml"
        echo "    \$EDITOR $CONFIG_DIR/ants/my-ant/ant.toml"
        echo ""
        echo "  Or create one from the web dashboard after starting."
        echo ""
    else
        echo "  Found $ANT_COUNT ANT(s) configured."
        echo ""
    fi

    # Start/restart the service.
    if systemctl is-enabled --quiet anthill 2>/dev/null; then
        echo "  Starting anthill service..."
        sudo systemctl start anthill
        echo "  Service started."
    else
        echo "  Enabling and starting anthill service..."
        sudo systemctl enable --now anthill
        echo "  Service enabled and started."
    fi
    echo ""

    echo "  Set up HTTPS (first time only):"
    echo "    sudo tailscale serve --bg http://localhost:3000"
    echo ""
    echo "  Add a device:"
    echo "    anthill --qr-join                 Show QR code to scan with phone"
    echo "    anthill --join-code               Generate a text join code"
    echo ""
    if [ "$MODE" = "dev" ]; then
        echo "  DEV mode — debug logging enabled."
        echo "  Re-install without 'dev' for production: ./install.sh"
        echo ""
        echo "  Web dashboard:"
        echo "    http://localhost:3000 (or your Tailscale HTTPS URL)"
        echo ""
        echo "  Starting log tail (Ctrl+C to exit)..."
        echo "============================================"
        echo ""
        sleep 1
        exec journalctl -u anthill -f
    else
        echo "  Check logs:"
        echo "    journalctl -u anthill -f"
        echo "    journalctl -u anthill -n 200 | grep -i 'registry\|backend\|AI config'"
        echo ""
        echo "  Web dashboard:"
        echo "    http://localhost:3000 (or your Tailscale HTTPS URL)"
        echo ""
    fi
        echo "  DEV mode — debug logging enabled."
        echo "  Re-install without 'dev' for production: ./install.sh"
    fi
    echo ""
    echo "  Web dashboard:"
    echo "    http://localhost:3000 (or your Tailscale HTTPS URL)"
    echo ""
fi

# Common commands for all platforms.
echo "  ──────────────────────────────────"
echo "  CLI commands:"
echo ""
echo "    anthill --qr-join                Scan QR with phone to join colony"
echo "    anthill --qr-join --hostname X   QR with custom hostname in URL"
echo "    anthill --join-code              Generate a text join code"
echo "    anthill --export-key             Show colony key (for password manager)"
echo "    anthill --export-key --qr        Show colony key as QR code"
echo "    anthill --import-key <key>       Restore colony key from backup"
echo "    anthill --help                   Show all options"
echo ""
