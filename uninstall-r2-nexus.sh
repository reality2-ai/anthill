#!/bin/bash
# Remove the old r2-nexus installation and migrate config to anthill.
#
# Usage:
#   ./uninstall-r2-nexus.sh
#
# This script:
#   1. Stops and removes the r2-nexus systemd service
#   2. Removes the r2-nexus binary
#   3. Optionally migrates config from ~/.config/r2-nexus to ~/.config/anthill

set -e

echo "=== Removing old r2-nexus installation ==="
echo ""

# Stop and disable the service.
if systemctl is-active --quiet r2-nexus 2>/dev/null; then
    echo "Stopping r2-nexus service..."
    sudo systemctl stop r2-nexus
fi

if systemctl is-enabled --quiet r2-nexus 2>/dev/null; then
    echo "Disabling r2-nexus service..."
    sudo systemctl disable r2-nexus
fi

if [ -f /etc/systemd/system/r2-nexus.service ]; then
    echo "Removing systemd service file..."
    sudo rm /etc/systemd/system/r2-nexus.service
    sudo systemctl daemon-reload
fi

# Remove the binary.
if [ -f /usr/local/bin/r2-nexus ]; then
    echo "Removing /usr/local/bin/r2-nexus..."
    sudo rm /usr/local/bin/r2-nexus
fi

echo ""
echo "=== r2-nexus service and binary removed ==="
echo ""

# Offer to migrate config.
OLD_CONFIG="$HOME/.config/r2-nexus"
NEW_CONFIG="$HOME/.config/anthill"

if [ -d "$OLD_CONFIG" ]; then
    echo "Found old config at $OLD_CONFIG"
    echo ""

    if [ -d "$NEW_CONFIG" ]; then
        echo "  $NEW_CONFIG already exists — skipping migration."
        echo "  You can manually move files from $OLD_CONFIG if needed."
    else
        echo "  Migrating to $NEW_CONFIG..."

        # Copy the directory.
        cp -r "$OLD_CONFIG" "$NEW_CONFIG"

        # Rename bots/ → ants/ if needed.
        if [ -d "$NEW_CONFIG/bots" ] && [ ! -d "$NEW_CONFIG/ants" ]; then
            mv "$NEW_CONFIG/bots" "$NEW_CONFIG/ants"
            echo "  Renamed bots/ → ants/"
        fi

        # Rename bot.toml → ant.toml in each ant directory.
        for dir in "$NEW_CONFIG/ants"/*/; do
            if [ -f "${dir}bot.toml" ] && [ ! -f "${dir}ant.toml" ]; then
                mv "${dir}bot.toml" "${dir}ant.toml"
                echo "  Renamed $(basename "$dir")/bot.toml → ant.toml"
            fi
        done

        # Update supervisor.toml if needed.
        if [ -f "$NEW_CONFIG/supervisor.toml" ]; then
            sed -i 's/bots_dir = "bots"/ants_dir = "ants"/' "$NEW_CONFIG/supervisor.toml"
            echo "  Updated supervisor.toml (bots_dir → ants_dir)"
        fi

        echo ""
        echo "  Migration complete: $NEW_CONFIG"
        echo "  Review ant.toml files — you may want to add a 'name' field."
    fi

    echo ""
    echo "  The old config is still at $OLD_CONFIG"
    echo "  Remove it manually when you're satisfied: rm -rf $OLD_CONFIG"
fi

echo ""
echo "Done! Install anthill with: cd /path/to/anthill && ./install.sh"
