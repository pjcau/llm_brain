---
title: VPS deployment
sidebar_position: 5
---

# VPS deployment

What runs on the VPS: `brain serve` — the proxy (`/v1/*`, client-key
auth) and the board — under systemd, behind Caddy with automatic HTTPS.
The tools' own logs stay on the laptop; everything that goes through the
proxy is recorded on the server.

:::note Live since 2026-09-20
**Contabo** VPS (x86_64, 4 vCPU, 8 GB, Ubuntu 24.04), hostname
`brain.<ip-with-dashes>.sslip.io` (no domain needed —
[hostname vs domain](./analysis/hosting-costs.md#hostname-not-a-domain)).
IP, hostname and board credentials are in the git-ignored
`deploy/server.local.env`. Alternative: on Hetzner (the original plan,
CAX11 ARM) paste `deploy/cloud-init.yaml` as the server's cloud config and
skip step 1; use the `aarch64` binary.
:::

## 1. Prepare the server (over SSH, as root)

Contabo has no cloud-init in the order flow, so the steps of
`deploy/cloud-init.yaml` are run by hand; order the server with your SSH
public key.

```bash
apt-get update && apt-get -y upgrade
apt-get install -y ufw fail2ban unattended-upgrades git curl ca-certificates \
  debian-keyring debian-archive-keyring apt-transport-https
useradd --system --home /opt/llm_brain --shell /usr/sbin/nologin brain
mkdir -p /opt/llm_brain/data /opt/llm_brain/config
# SSH keys only. Contabo's 50-cloud-init.conf sets PasswordAuthentication yes
# and the first file wins, so the hardening file must sort before it.
printf 'PasswordAuthentication no\nPermitRootLogin prohibit-password\nKbdInteractiveAuthentication no\n' \
  > /etc/ssh/sshd_config.d/00-hardening.conf && systemctl restart ssh
# Caddy (official repo): the two curl lines from deploy/cloud-init.yaml, then
apt-get update && apt-get install -y caddy
ufw default deny incoming && ufw default allow outgoing
ufw allow OpenSSH && ufw allow 80/tcp && ufw allow 443/tcp && ufw --force enable
systemctl enable --now fail2ban
```

Plus the unattended-upgrades file from `cloud-init.yaml`
(`/etc/apt/apt.conf.d/20auto-upgrades`).

## 2. Install `brain`

Binaries are built by CI for every `v*` tag (`.github/workflows/release.yml`):
`brain-x86_64-unknown-linux-gnu` and `brain-aarch64-unknown-linux-gnu`,
each with a `.sha256`.

```bash
cd /opt/llm_brain
B=brain-x86_64-unknown-linux-gnu
curl -fsSLO https://github.com/pjcau/llm_brain/releases/latest/download/$B
curl -fsSLO https://github.com/pjcau/llm_brain/releases/latest/download/$B.sha256
sha256sum -c $B.sha256 && install -m 0755 $B brain
git clone --depth 1 https://github.com/pjcau/llm_brain /tmp/src && cp -r /tmp/src/config /opt/llm_brain/
```

Secrets: `/opt/llm_brain/.env` (mode 0600, owner `brain`) with the
`OPENROUTER_KEY_*` lines, `BRAIN_HOME=/opt/llm_brain`,
`BRAIN_DB=/opt/llm_brain/data/brain.db`. Copied with `scp` from the
laptop; never through git or chat ([secrets](./architecture/secrets.md)).

```bash
cp /tmp/src/deploy/brain.service /etc/systemd/system/   # brain serve --bind 127.0.0.1:8080 --refresh 600
cp /tmp/src/deploy/Caddyfile /etc/caddy/Caddyfile       # then edit: hostname, bcrypt hash, API routes (below)
chown -R brain:brain /opt/llm_brain && chmod 600 /opt/llm_brain/.env
systemctl daemon-reload && systemctl enable --now brain && systemctl reload caddy
curl -s https://brain.<ip-with-dashes>.sslip.io/health   # → ok
```

The Caddyfile on the server sends `/v1/*`, `/api/hello` and `/health`
straight to brain (the proxy's own key auth applies, `flush_interval -1`
for SSE) and keeps **basic auth** on everything else (the board;
`caddy hash-password` for the hash, credentials in `server.local.env`).

Client keys are issued on the server ([Phase 1](./phase-1.md#issue-a-key-on-the-server-admin-only)):

```bash
cd /opt/llm_brain && sudo -u brain env BRAIN_HOME=/opt/llm_brain BRAIN_DB=/opt/llm_brain/data/brain.db \
  ./brain keys create --profile dev --name laptop
```

## Update

New release, or changed `config/*.yaml` (read at start only):

```bash
# binary
cd /opt/llm_brain && B=brain-x86_64-unknown-linux-gnu
curl -fsSLO https://github.com/pjcau/llm_brain/releases/latest/download/$B
curl -fsSLO https://github.com/pjcau/llm_brain/releases/latest/download/$B.sha256
sha256sum -c $B.sha256 && systemctl stop brain && install -m 0755 $B brain && systemctl start brain
# config, from the laptop
scp config/profiles.yaml config/tiers.yaml root@<vps>:/opt/llm_brain/config/ && ssh root@<vps> systemctl restart brain
```

After changing a profile's `daily_limit_usd`, also run `brain upstream
sync` (laptop, management key) so the OpenRouter key's own limit follows.

## What is exposed

| Route | Auth |
|-------|------|
| `/v1/*` | client key, checked by `brain` |
| `/api/hello` | none (Claude Code's connectivity probe, 204) |
| `/health` | open |
| `/`, `/api/summary` (board) | Caddy basic auth |

Caddy on 80/443, everything else closed by ufw; `brain` listens on
`127.0.0.1:8080` only.

## Backups: not set up

Litestream is **not installed**: the `.deb` asset name changed upstream
and the install was deferred. The SQLite file (`data/brain.db`: client
keys, requests) is currently not replicated. The intended setup is in
`deploy/litestream.yml` (S3-compatible bucket, credentials in
`/etc/default/litestream`, restore with `litestream restore -o
/opt/llm_brain/data/brain.db s3://llm-brain-backup/brain.db`).
