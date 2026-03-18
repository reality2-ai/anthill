#!/bin/bash
# Install anthill binary and systemd service.
#
# Usage:
#   cd anthill
#   ./install.sh

set -e

USER_NAME="$(whoami)"
USER_HOME="$HOME"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="$USER_HOME/.config/anthill"

echo "Installing for user: $USER_NAME ($USER_HOME)"
echo ""

echo "Building release binary..."
cargo build -p anthill --release

BINARY="target/release/anthill"

# Stop the service if running (binary can't be overwritten while running).
if systemctl is-active --quiet anthill 2>/dev/null; then
    echo "Stopping anthill service..."
    sudo systemctl stop anthill
    sleep 1
fi

echo "Installing binary to $INSTALL_DIR/anthill..."
sudo cp "$BINARY" "$INSTALL_DIR/anthill"
sudo chmod 755 "$INSTALL_DIR/anthill"

echo "Setting up config directory at $CONFIG_DIR..."
mkdir -p "$CONFIG_DIR/ants"

if [ ! -f "$CONFIG_DIR/supervisor.toml" ]; then
    cp config-example/supervisor.toml "$CONFIG_DIR/supervisor.toml"
    echo "  Created supervisor.toml"
else
    echo "  supervisor.toml already exists, skipping"
fi

echo ""
echo "Generating systemd service for user '$USER_NAME'..."
sed -e "s|@@USER@@|$USER_NAME|g" \
    -e "s|@@HOME@@|$USER_HOME|g" \
    anthill.service.template > /tmp/anthill.service
sudo cp /tmp/anthill.service /etc/systemd/system/anthill.service
rm /tmp/anthill.service
sudo systemctl daemon-reload

echo ""
echo "============================================"
echo "  Anthill installed successfully!"
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
echo "  Generate a join code:"
echo "    anthill --join-code"
echo ""
echo "  Check logs:"
echo "    journalctl -u anthill -f"
echo ""
echo "  Web dashboard:"
echo "    http://localhost:3000 (or your Tailscale HTTPS URL)"
echo ""
