---
title: VPS deployment
sidebar_position: 5
---

# VPS deployment

What runs on the VPS today: the `brain` binary with the **board** (budget
per profile from OpenRouter's counters, benchmark rows). The tools' logs
(aider chat, Claude Code sessions) stay on the laptop until Phase 1's proxy
moves every request through the VPS. The infrastructure below — Caddy/TLS,
systemd, Litestream, the release pipeline — is the same the proxy will use.

:::note Deployed on 2026-09-20
Provider actually chosen: **Contabo** (x86_64, 4 vCPU, 8 GB, Ubuntu 24.04.5).
Contabo has no cloud-init in the order flow, so the steps of
`deploy/cloud-init.yaml` were run over SSH; one Contabo-specific detail:
`/etc/ssh/sshd_config.d/50-cloud-init.conf` sets `PasswordAuthentication yes`
and wins over later files, so the hardening file must sort first
(`00-hardening.conf`). The board is live at `https://brain.<ip-with-dashes>.sslip.io/`
(IP and access notes in the git-ignored `deploy/server.local.env`).
:::

## 1. Create the server (you)

Hetzner Cloud → new project → server (or any provider with Ubuntu 24.04):

| Setting | Value |
|---------|-------|
| Location | Falkenstein or Nuremberg (EU) |
| Image | Ubuntu 24.04 |
| Type | **CAX11** (2 vCPU Ampere ARM, 4 GB) — [why](./analysis/hosting-costs.md) |
| SSH key | your public key (no password login) |
| Cloud config | paste `deploy/cloud-init.yaml` |
| Firewall | the cloud-init sets ufw (22/80/443); a Hetzner firewall with the same rules is a free extra layer |

Optional: create a Hetzner **Object Storage** bucket (`llm-brain-backup`) and
an S3 key pair for Litestream. Backblaze B2 works the same.

Put the IP, plan and hostname in `deploy/server.local.env` (git-ignored,
never committed; template created on first setup). No domain is needed: the hostname is
`brain.<ip-with-dashes>.sslip.io` until you want a real one
([hostname vs domain](./analysis/hosting-costs.md#hetzner-cax11-or-lightsail-5-and-is-a-domain-needed)).

## 2. Install (from the laptop, over SSH)

Binaries are built by CI for every `v*` tag (`.github/workflows/release.yml`):
`brain-aarch64-unknown-linux-gnu` for CAX, `brain-x86_64-unknown-linux-gnu`
for x86, with `.sha256` files. On the server:

```bash
# as root, after cloud-init finished (cloud-init status --wait)
cd /opt/llm_brain
curl -fsSLO https://github.com/pjcau/llm_brain/releases/latest/download/brain-aarch64-unknown-linux-gnu
curl -fsSLO https://github.com/pjcau/llm_brain/releases/latest/download/brain-aarch64-unknown-linux-gnu.sha256
sha256sum -c brain-aarch64-unknown-linux-gnu.sha256 && install -m 0755 brain-aarch64-unknown-linux-gnu brain   # x86: the x86_64 files
git clone --depth 1 https://github.com/pjcau/llm_brain /tmp/src && cp -r /tmp/src/config /opt/llm_brain/
```

Secrets: create `/opt/llm_brain/.env` (mode 0600, owner `brain`) with the
`OPENROUTER_KEY_*` lines and `BRAIN_HOME=/opt/llm_brain`,
`BRAIN_DB=/opt/llm_brain/data/brain.db`. Copy it with `scp` from the laptop
or paste it; it never goes through git or chat
([secrets](./architecture/secrets.md)).

```bash
cp /tmp/src/deploy/brain.service /etc/systemd/system/
cp /tmp/src/deploy/Caddyfile /etc/caddy/Caddyfile   # edit the hostname line
chown -R brain:brain /opt/llm_brain && chmod 600 /opt/llm_brain/.env
systemctl daemon-reload && systemctl enable --now brain && systemctl reload caddy
curl -s https://brain.<ip-with-dashes>.sslip.io/health   # → ok
```

Backups (optional now, required for the proxy): `/etc/litestream.yml` from
`deploy/litestream.yml`, credentials in `/etc/default/litestream`,
`systemctl enable --now litestream`. Restore = `litestream restore -o
/opt/llm_brain/data/brain.db s3://llm-brain-backup/brain.db`.

## 3. Update

```bash
systemctl stop brain && curl -fsSLo /opt/llm_brain/brain https://github.com/pjcau/llm_brain/releases/latest/download/brain-aarch64-unknown-linux-gnu && chmod 0755 /opt/llm_brain/brain && systemctl start brain
```

## What is exposed

Caddy on 80/443 with automatic HTTPS, everything else closed by ufw;
`brain` listens on `127.0.0.1:8080` only. The board has no auth yet: until
the Phase 1 key middleware exists, keep it behind Tailscale or limit it in
the Caddyfile (`@lan remote_ip <your-ip>`), or accept that it shows spend
figures only.
