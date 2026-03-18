# Troubleshooting

## ANT doesn't respond to Telegram messages

1. Check the service: `sudo systemctl status anthill`
2. Check logs: `journalctl -u anthill -f`
3. Verify the bot token in `ant.toml`
4. Check your chat ID is in `allow` (or remove `allow` to allow everyone)

## "Failed to run claude" error

1. `which claude` — is it installed?
2. `claude -p "hello"` — does it work directly?
3. Check PATH in the systemd service includes `~/.local/bin` and `~/.cargo/bin`
4. Run `claude` interactively once to authenticate

## Claude asks for permission (can't execute)

Set `skip_permissions = true` in `[claude]` section of `ant.toml`.

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
