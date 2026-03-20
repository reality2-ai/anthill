# Security

## Trust group model

Anthill implements R2-TRUST — the same provisioning model designed for IoT sensor networks. The colony is a **trust group**. The server is the **queen**. Browsers and phones are **viewers** that must be provisioned to join.

![Trust Group](https://mermaid.ink/img/Z3JhcGggVEQKICAgIHN1YmdyYXBoIFRHW1RydXN0IEdyb3VwIC0gY29sb255XQogICAgICAgIFF1ZWVuW1NlcnZlciAtIHRoZSBRdWVlbiBjb2xvbnkua2V5XQogICAgICAgIEQxW1Bob25lIC0gY3JlZGVudGlhbCBpbiBsb2NhbFN0b3JhZ2VdCiAgICAgICAgRDJbTGFwdG9wIC0gY3JlZGVudGlhbCBpbiBsb2NhbFN0b3JhZ2VdCiAgICBlbmQKCiAgICBBZG1pbltBZG1pbiB0ZXJtaW5hbF0gLS0+fGFudGhpbGwgLS1qb2luLWNvZGV8IFF1ZWVuCiAgICBRdWVlbiAtLT58am9pbiBjb2RlIDUgbWluIG9uZS11c2V8IEQxCiAgICBEMSAtLT58WC1DcmVkZW50aWFsIGhlYWRlcnwgUXVlZW4KICAgIEQyIC0tPnxYLUNyZWRlbnRpYWwgaGVhZGVyfCBRdWVlbgoKICAgIE91dHNpZGVbVW5hdXRoZW50aWNhdGVkIGJyb3dzZXJdIC0uLT58NDAxIFVuYXV0aG9yaXplZHwgUXVlZW4=)

### How it works

1. **Colony key** — Ed25519 signing key, generated on first run (`colony.key`). The server is the key holder (the queen).

2. **Join codes** — 48-bit single-use tokens (`xxxx-xxxx-xxxx`), valid for 5 minutes. Generated via CLI (`anthill --qr-join` or `--join-code`) or from the web dashboard's Manage Devices → Add Device (QR).

3. **Device credentials** — Ed25519 private key seed. On join, the server generates a keypair, issues an R2-TRUST certificate, and returns the credential. Stored in the browser's `localStorage` and in `devices.toml` on the server.

4. **Authentication** — every API call requires `X-Credential` header. WebSocket messages are HMAC-SHA256 signed for transport integrity. No credential = 401.

### What's protected

| Route | Auth required |
|---|---|
| `GET /` (HTML, assets, icons) | No |
| `POST /api/auth/join` | No (needs join code) |
| `POST /api/auth/verify` | No (checks credential) |
| `GET /api/auth/status` | No (is colony empty) |
| `GET /ws` (WebSocket) | Yes (credential in query) |
| **All other `/api/*` routes** | **Yes (X-Credential header)** |

### Managing devices

From the web dashboard sidebar → **Manage Devices**:
- **Add Device (QR)** — generates a QR code with 5-minute countdown timer. Scan with phone camera to join.
- **Device list** — all provisioned devices with names, join dates, last seen
- **Revoke** — removes a device's credential (they'll need a new code to rejoin)

From the CLI:
```bash
anthill --qr-join                # QR code — scan with phone
anthill --qr-join --hostname X   # QR with custom hostname
anthill --join-code              # text code for manual entry
```

## Bot tokens are secrets

`ant.toml` files contain Telegram/Slack tokens. They live in `~/.config/anthill/ants/` which is outside the repo.

## Command execution

The AI backend runs with full permissions (`skip_permissions = true` by default). This means the ANT can run any shell command, edit any file, and access anything your user can.

**Always use the Telegram `allow` list** to restrict which chat IDs can interact. Without it, anyone who discovers your bot username can execute commands.

## The web dashboard has trust group auth

Unlike earlier versions, the web dashboard now requires authentication. Without a valid credential (obtained via a join code), all API calls return 401 Unauthorized and the WebSocket connection is refused.

The join screen is the only thing visible to unauthenticated users.

## Don't run as root

Anthill runs commands with the same permissions as the user running the service. Create a dedicated user if you want to limit access.

## Memory and knowledge graph

The knowledge graph (`knowledge.json`) and per-user memory files accumulate context over time — project details, relationships, preferences, conversation summaries. They're stored in the working directory and backed up to git.

**Encrypted backups** are available: set `encrypt_backups = true` in `ant.toml`. This encrypts `memory/` and `files/` using XChaCha20-Poly1305 before each git commit. The working directory stays plaintext; git history contains ciphertext.

## Git backup repositories

> **Warning:** If you use GitHub for workspace backups (`backup_remote`), make sure the repository is **private** — or enable `encrypt_backups`. A public unencrypted repo exposes the knowledge graph, memory files, and any files the AI creates.

Always use `gh repo create --private` when creating backup repos.

## Sensitive operation restriction

The following commands are restricted to the web dashboard and blocked from Telegram and Slack:

- `/analyse <file>` — thematic analysis
- `/specify <file>` — specification generation
- `/test-vectors <file>` — test vector generation

These commands operate on files in the workspace and produce structured output that is best reviewed in the web UI. Telegram and Slack lack the trust group authentication that the web dashboard provides, so sensitive operations that read and analyse workspace files are restricted to the more secure channel.

## Recommendations

1. **Use Tailscale** for the web dashboard — don't expose port 3000 to the public internet
2. **Use HTTPS** via `tailscale serve` — encrypts traffic, enables secure WebSocket
3. **Set `allow`** in Telegram config — restrict to your chat ID
4. **Review provisioned devices** periodically — revoke ones you don't recognise
5. **Keep `colony.key` safe** — anyone with this file can generate join codes
6. **Use private repos** for git backup remotes — memory and working artifacts may contain sensitive context
7. **Review memory files** periodically — remove any credentials or sensitive data that Claude may have stored
