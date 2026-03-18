#!/bin/bash
# Install anthill binary and systemd service.
#
# Usage:
#   cd r2-core/tools/anthill
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

BINARY="../../target/release/anthill"

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
echo "Done! Next steps:"
echo ""
echo "  1. Create an ant:"
echo "     mkdir -p $CONFIG_DIR/ants/my-ant"
echo "     cp config-example/ants/dev-assistant/ant.toml $CONFIG_DIR/ants/my-ant/ant.toml"
echo ""
echo "  2. Edit the config:"
echo "     \$EDITOR $CONFIG_DIR/ants/my-ant/ant.toml"
echo ""
echo "  3. Start the service:"
echo "     sudo systemctl enable --now anthill"
echo ""
echo "  4. Check logs:"
echo "     journalctl -u anthill -f"
