# Spec Confidence Dashboard

A Thurisaz-inspired tracking system for specification quality. Every spec is a claim. Reviews, tests, and implementation are evidence. Confidence is the posterior.

## Quick Start

```bash
bash dashboard/build.sh
open dashboard/index.html
```

## Confidence Model

| Evidence | Score |
|----------|-------|
| Spec exists with version | +0.10 |
| First reviewer approved | +0.15 |
| Second reviewer approved | +0.10 |
| Test vectors written | +0.15 |
| Automated tests passing | +0.15 |
| Implementation complete | +0.10 |
| Integration tests passing | +0.10 |
| Field validation | +0.05 |
| No open issues | +0.10 |
| **Maximum** | **1.00** |

## Confidence Decay

- **Dependency changed**: -0.15 per affected dependency
- **Open issue**: -0.03 per issue
- **Version bump**: resets to 0.15 (author self-review only)

## Color Bands

- 🔴 Red: < 0.30
- 🟡 Yellow: 0.30 – 0.59
- 🟢 Green: 0.60 – 0.84
- ✅ Full: ≥ 0.85

## Recording Reviews

```bash
# Approve a spec
python3 dashboard/review.py R2-WIRE --reviewer Roy --status approved

# Flag an issue
python3 dashboard/review.py R2-AUTH --issue "§2.5 escalation freshness too short"

# Resolve an issue
python3 dashboard/review.py R2-AUTH --resolve-issue 0

# Mark evidence
python3 dashboard/review.py R2-FNV --set test_vectors=true
python3 dashboard/review.py R2-FNV --set tests_passing=true
python3 dashboard/review.py R2-FNV --set implementation=true
```

## Auto-Rebuild

The watcher script polls every 5 minutes and rebuilds on changes:

```bash
nohup bash dashboard/watch.sh >> dashboard/watch.log 2>&1 &
```

## Rules

1. **No human shall edit generated code directly.** All changes flow through specs → test vectors → generation → validation.
2. **Emergency hotfixes must be back-propagated to specs within 24 hours** or they are automatically reverted.
3. **Confidence is earned, not granted.** A spec starts at 0.10 and climbs only through evidence.
4. **Dependency changes propagate.** If R2-FNV changes, everything that depends on it drops in confidence.

## Live dashboard server & inline saving

Run the lightweight HTTP server to get inline checkbox saving, reviewer metadata, the review queue, and JSON APIs:

```bash
python3 dashboard/server.py --port 8080
```

Visit `http://<host>:8080/dashboard/` and the UI will stream edits straight into `web/dashboard/spec-meta/reviews.json`. If the
server is offline the UI falls back to the CLI command panel.

**API endpoints**

| Method | Path            | Description                           |
|--------|-----------------|---------------------------------------|
| GET    | /api/ping       | Health check                          |
| POST   | /api/evidence   | `{ "spec": "R2-WIRE", "changes": {"testing.unit": true} }` |
| POST   | /api/review     | `{ "spec": "R2-WIRE", "reviewer": "Roy", "status": "approved", "note": "optional" }` |
| POST   | /api/assignment | `{ "spec": "R2-WIRE", "slot": "review1", "reviewer": "Mariko Ops" }` (empty reviewer clears override) |
| GET    | /api/spec/<code>| Returns the latest spec payload (optional helper) |

## Reviewer roster

Reviewer slots (1st/2nd/expert) are defined in `dashboard/spec-meta/reviewer_roster.json`. The `_defaults`
section applies per-prefix (R2/MK/TH) and you can override per spec:

```json
{
  "_defaults": {
    "R2": {"review1": "Roy Davies", "review2": "Alfred (AI)", "expert": "External auditor"}
  },
  "R2-CBOR": {"expert": "Crypto reviewer"}
}
```

The dashboard surface shows both the assigned reviewer and the person who actually signed off, with status pills.

Click the **Review queue** button in the header to see unfulfilled reviewer slots, reassign owners, and log approvals without leaving the dashboard.
