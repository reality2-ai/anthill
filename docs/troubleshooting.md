# Troubleshooting

## ANT doesn't respond to Telegram messages

1. Check the service: `sudo systemctl status anthill`
2. Check logs: `journalctl -u anthill -f`
3. Verify the bot token in `ant.toml`
4. Check your chat ID is in `allow` (or remove `allow` to allow everyone)

## "Failed to run claude/codex" error

1. Check the AI backend is installed: `which claude` or `which codex`
2. Test it directly: `claude -p "hello"` or `codex exec "hello"`
3. Check PATH in the service config includes `~/.local/bin`, `~/.cargo/bin`, `~/.npm-global/bin`
4. Run the backend interactively once to authenticate

## AI asks for permission (can't execute)

This is handled automatically — `skip_permissions` defaults to true.

## Web dashboard not loading

1. Check logs for "Web server listening on" message
2. Verify you can reach the server: `ping <tailscale-ip>`
3. Check both devices are on Tailscale: `tailscale status`
4. Test directly: `curl http://localhost:3000`
5. Check HTTPS proxy: `tailscale serve status`

## Chat history missing after rename

History is keyed by directory name (stable ID), not the display name. If you renamed the directory under `ants/`, rename the history file to match:

```bash
mv ~/.config/anthill/history/old-name.jsonl ~/.config/anthill/history/new-name.jsonl
```

## Binary can't be overwritten during install

```bash
sudo systemctl stop anthill
./install.sh
sudo systemctl start anthill
```

## Systemd service fails to start

```bash
journalctl -u anthill -n 50 --no-pager
```

Common issues:
- Binary not found → re-run `./install.sh`
- Config directory missing → `mkdir -p ~/.config/anthill/ants`
- No ANTS configured → create at least one `ants/<name>/ant.toml`
- PATH missing claude → check `Environment=PATH=...` in the service file

## Long responses get cut off

Telegram has a 4096-character limit. Anthill splits long messages automatically. If content is still missing, check logs for errors.

## Crash on messages with macrons or special characters (UTF-8)

**Fixed.** Earlier versions could panic when slicing strings at byte offsets that landed inside multi-byte UTF-8 characters (e.g. Māori macrons like ā, ē, ī, ō, ū, or emoji). All string slicing now uses character or word boundaries. If you see a panic mentioning "byte index is not a char boundary", update to the latest build.

## ANT doesn't respond (web UI) — no error, no spinner

1. Check the ANT status indicator in the sidebar: green = running, red = stopped, grey = configured but not started.
2. If the ANT is stopped or grey, start it from the ANT settings or restart Anthill.
3. If you send a message to a stopped ANT, the web UI now shows an error message ("ANT is not running"). If you don't see this message, update to the latest build.
4. Check the Workers tab for stall warnings (yellow) — the task may be running but slow.
5. Check logs: `journalctl -u anthill -f` for backend errors.
