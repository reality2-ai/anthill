#!/bin/bash
# Auto-rebuild dashboard on repo changes
# Polls every 5 minutes, pulls, rebuilds if changes detected
# Run: nohup bash dashboard/watch.sh &

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_DIR"

SSH_CMD="ssh -i $HOME/.ssh/id_ed25519_bitbucket -o IdentitiesOnly=yes"

while true; do
    # Fetch latest
    GIT_SSH_COMMAND="$SSH_CMD" git fetch origin main --quiet 2>/dev/null
    
    LOCAL=$(git rev-parse HEAD)
    REMOTE=$(git rev-parse origin/main)
    
    if [ "$LOCAL" != "$REMOTE" ]; then
        echo "$(date '+%Y-%m-%d %H:%M:%S') Changes detected, rebuilding..."
        GIT_SSH_COMMAND="$SSH_CMD" git pull --quiet origin main 2>/dev/null
        bash dashboard/build.sh
        echo "$(date '+%Y-%m-%d %H:%M:%S') Dashboard rebuilt."
    fi
    
    sleep 300  # 5 minutes
done
