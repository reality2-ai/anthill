# Security

## Bot tokens are secrets

`ant.toml` files contain Telegram bot tokens. They should never be committed to git. The `.gitignore` excludes `anthill.toml` (single-bot mode). For production, `ant.toml` files live in `~/.config/anthill/ants/` which is outside the repo.

## skip_permissions gives full shell access

`skip_permissions = true` passes `--dangerously-skip-permissions` to Claude Code. This means the ANT can run any shell command, edit any file, and access anything your user can.

**Always use the `allow` list** to restrict which Telegram chat IDs can talk to the ANT. Without it, anyone who discovers your bot username can execute commands on your machine.

## The web dashboard has no authentication

If you can reach port 3000, you can interact with any ANT. Use Tailscale to restrict access — only devices on your Tailscale network can connect. Don't expose port 3000 to the public internet.

## Commands run as your user

Anthill runs commands with the same permissions as the user running the service. Don't run it as root. Create a dedicated user if you want to limit access.

## Memory files may contain sensitive context

Per-user memory files accumulate context over time — project names, file paths, preferences, decisions. They're stored in the working directory and backed up to git. They are not encrypted.

## Recommendations

1. **Set `allow`** in every `ant.toml` — restrict to your chat ID
2. **Use Tailscale** for the web dashboard — don't expose port 3000 publicly
3. **Use HTTPS** via `tailscale serve` — encrypts traffic and enables secure WebSocket
4. **Don't run as root** — use a regular user account
5. **Review memory files** periodically — they may contain context you don't want backed up
6. **Use private repos** for git backup remotes — memory and working artifacts may contain sensitive information
