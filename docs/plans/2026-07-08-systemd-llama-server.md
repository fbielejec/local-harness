# systemd llama-server Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Implements:** the **server half of Phase E** ("Make it durable") from `docs/plans/2026-07-06-local-coding-harness-design.md`. Two deliberate divergences from that doc's Phase E note, both confirmed with the user 2026-07-08:
- **system** service, not the doc's "systemd *user* service" — starts at boot with no login; the doc's wording was a preliminary note and gets updated to match (Task 5).
- **`autossh` tunnel service is deferred** — it is the *remaining* open item of Phase E, tracked as a TODO, not built here (scope = remote server only).

**Goal:** Wrap the current winning llama-server config (IQ4_XS) as a system-level systemd service on the remote host `weebeastie` so it starts automatically on boot and restarts on crash.

**Architecture:** A single `/etc/systemd/system/llama-server.service` unit runs `llama-server` as `User=filip` with the exact IQ4_XS flag set from `CLAUDE.md`, pointed at the **local GGUF path** (not `-hf`) so boot has zero network dependency. `Restart=always` rides out crashes and the boot-time GPU-not-ready race. The unit file is version-controlled in the repo at `deploy/llama-server.service`; installation to `/etc/systemd/system/` is a privileged step the user runs (sudo needs a password).

**Tech Stack:** systemd 255 (Ubuntu 24.04 / Mint 22.1), llama.cpp CUDA build, SSH/scp to `filip@192.168.1.22`.

**Convention:** This is ops work, not TDD app code — "tests" are verification commands (unit active, port bound, health OK, `enabled` for boot). Steps prefixed **[YOU]** require the user to run them in their own terminal because sudo prompts for a password; all other steps Claude runs.

---

## Pre-flight facts (already verified)

- Remote user `filip`, **no passwordless sudo**, systemd 255.
- nvidia devices are `crw-rw-rw-` (world-accessible) → a service as `User=filip` gets the GPU.
- Local GGUF (resolved from the `-hf` cache):
  `/home/filip/models/models--unsloth--Qwen3-Coder-30B-A3B-Instruct-GGUF/snapshots/b17cb02dd882d5b6ab62fc777ad2995f19668350/Qwen3-Coder-30B-A3B-Instruct-IQ4_XS.gguf`
- Binary: `/home/filip/Programs/llama.cpp/build/bin/llama-server` (exists, executable).
- Server currently running as an `nohup` process (must be stopped before the service can bind :8080).
- No existing `llama*` units (system or user).

---

## Task 1: Author the unit file in the repo

**Files:**
- Create: `deploy/llama-server.service`

**Step 1: Write the unit file**

```ini
[Unit]
Description=llama.cpp server — Qwen3-Coder-30B-A3B IQ4_XS (local coding harness)
After=network.target
Wants=network.target

[Service]
Type=simple
User=filip
Group=filip
WorkingDirectory=/home/filip/Programs/llama.cpp/build/bin
ExecStart=/home/filip/Programs/llama.cpp/build/bin/llama-server \
  --model /home/filip/models/models--unsloth--Qwen3-Coder-30B-A3B-Instruct-GGUF/snapshots/b17cb02dd882d5b6ab62fc777ad2995f19668350/Qwen3-Coder-30B-A3B-Instruct-IQ4_XS.gguf \
  --host 127.0.0.1 --port 8080 \
  --threads 10 --parallel 1 --ctx-size 32768 \
  --n-gpu-layers 99 --cpu-moe \
  -fa on --cache-type-k q8_0 --cache-type-v q8_0 \
  --no-mmap --jinja
Restart=always
RestartSec=10
TimeoutStartSec=300

[Install]
WantedBy=multi-user.target
```

**Why each non-obvious line (do not "simplify" these away):**
- `--model <local path>` not `-hf …:IQ4_XS` — identical weights, but no HuggingFace revision check that could hang/fail an offline boot.
- `--host 127.0.0.1` — llama-server stays loopback-only (self-sovereignty goal; access is via SSH tunnel).
- `--parallel 1` — essential; keeps the single KV slot warm across turns (documented finding).
- `--cpu-moe` — expert FFNs in RAM; experts don't fit the 4 GB card.
- `Restart=always` + `RestartSec=10` — survives crashes and the boot GPU-not-ready race; 10s spacing never trips systemd's default start-burst limiter.
- `TimeoutStartSec=300` — `--no-mmap` loads ~15 GiB into RAM before binding; default 90s would wrongly mark it failed.

**Step 2: Verify it parses cleanly (local dry check)**

Run:
```bash
systemd-analyze verify /home/filip/CloudStation/LLMs/local_coding_harness/deploy/llama-server.service
```
Expected: **no output** (silence = valid). Warnings about the executable path not existing on the *laptop* are acceptable/ignorable — the unit runs on the remote. If it errors on syntax, fix and re-run.

**Step 3: Commit**

```bash
cd /home/filip/CloudStation/LLMs/local_coding_harness
git add deploy/llama-server.service
git commit -m "deploy: add systemd unit for llama-server IQ4_XS winner"
```

---

## Task 2: Stage the unit on the remote

**Step 1: Copy the unit to the remote home (no sudo needed)**

Run:
```bash
scp /home/filip/CloudStation/LLMs/local_coding_harness/deploy/llama-server.service filip@192.168.1.22:~/llama-server.service
```
Expected: `llama-server.service 100% ...`

**Step 2: Confirm it landed**

Run:
```bash
ssh filip@192.168.1.22 'ls -l ~/llama-server.service'
```
Expected: file present, ~700 bytes.

---

## Task 3: Install + enable the service (privileged — user runs)

> These steps need root and sudo prompts for a password, so **the user runs them.** In this Claude Code session the user types the line prefixed with `!` so the SSH TTY (`-t`) can carry the password prompt. The `pkill` stops the current nohup server so the service can bind :8080 (this drops the warm KV cache — the next request pays the one-time ~190s prefill; expected).

**Step 1: [YOU] Stop the old server, install, enable, start**

```bash
! ssh -t filip@192.168.1.22 '
  pkill -f "build/bin/llama-server"; sleep 2;
  sudo install -m 0644 ~/llama-server.service /etc/systemd/system/llama-server.service &&
  sudo systemctl daemon-reload &&
  sudo systemctl enable --now llama-server &&
  echo "INSTALL_OK"
'
```
Expected: ends with `INSTALL_OK`. (You'll be prompted for the sudo password once.)

---

## Task 4: Verify (running now + survives reboot)

**Step 1: Unit is active and enabled**

Run:
```bash
ssh filip@192.168.1.22 'systemctl is-active llama-server; systemctl is-enabled llama-server'
```
Expected:
```
active
enabled
```
(`enabled` = it will come back after reboot — the core requirement.)

**Step 2: Model finished loading, port is bound, health OK**

The service takes up to ~5 min to finish the ~15 GiB load + first bind. Poll health:
```bash
ssh filip@192.168.1.22 'for i in $(seq 1 60); do
  if curl -sf http://127.0.0.1:8080/health >/dev/null; then echo "HEALTH_OK after ${i}0s"; break; fi
  sleep 10
done; curl -s http://127.0.0.1:8080/health'
```
Expected: `HEALTH_OK ...` then `{"status":"ok"}`.

**Step 3: Confirm only ONE llama-server is running (no stray nohup)**

Run:
```bash
ssh filip@192.168.1.22 'pgrep -af "build/bin/llama-server"'
```
Expected: exactly one process, and its parent is systemd (not a bash/nohup wrapper). Cross-check the flags include `--model .../IQ4_XS.gguf`.

**Step 4: End-to-end through the harness (optional but recommended)**

On the laptop, ensure the tunnel is up (`ssh -fN -L 8080:127.0.0.1:8080 filip@192.168.1.22`), then:
```bash
cd ~/qwen-scratch
export OPENAI_BASE_URL="http://localhost:8080/v1" OPENAI_API_KEY=dummy \
       OPENAI_MODEL="unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS"
qwen -p "Use your tools to read notes.txt and tell me the secret word."
```
Expected: answer contains `artichoke`.

**Step 5 (optional, definitive): reboot test**

Only if the user wants hard proof of boot survival:
```bash
! ssh -t filip@192.168.1.22 'sudo reboot'
# wait ~60-90s, then re-run Task 4 Step 1 and Step 2.
```

---

## Task 5: Update docs + commit

**Files:**
- Modify: `CLAUDE.md` — the "Operating the server" section.
- Modify: `docs/plans/2026-07-06-local-coding-harness-design.md` — Phase E (line ~148-149).

**Step 0: Reconcile Phase E in the design doc**

In `2026-07-06-local-coding-harness-design.md`, update the Phase E line so it reflects reality:
- Change "systemd **user** service" → "systemd **system** service" (`deploy/llama-server.service`), and mark the server half **done (2026-07-08)**.
- Leave the **`autossh` tunnel service** called out as the **remaining Phase E item** (not yet built).

**Step 1: Replace the manual-launch runbook with systemd operations**

Add a note that the production server is now a systemd unit and give the new commands:
```bash
# manage
sudo systemctl {start,stop,restart,status} llama-server
# logs (replaces `tail -f ~/llama-server.log`)
journalctl -fu llama-server
# the manual nohup launch block is retained below for reference / one-off benchmarking
```
Keep the existing manual `nohup … llama-server …` block in the doc labelled "reference / one-off benchmarking" — the autoperf loop still launches servers by hand.

**Step 2: Note the config source of truth**

Add: unit file lives at `deploy/llama-server.service` (repo) → `/etc/systemd/system/llama-server.service` (remote). Any flag change must update both.

**Step 3: Commit**

```bash
cd /home/filip/CloudStation/LLMs/local_coding_harness
git add CLAUDE.md
git commit -m "docs: switch server runbook to systemd (llama-server.service)"
```

---

## Rollback

If anything misbehaves, revert to the pre-systemd manual server:
```bash
! ssh -t filip@192.168.1.22 '
  sudo systemctl disable --now llama-server &&
  echo "disabled"
'
# then relaunch manually per CLAUDE.md nohup block if needed.
```
The unit file stays at `/etc/systemd/system/` (inert once disabled); `sudo rm` it only if you want it fully gone.

---

## Done when

- `systemctl is-enabled llama-server` → `enabled` (survives reboot ✅ — the ask)
- `systemctl is-active llama-server` → `active`
- `/health` returns ok; exactly one systemd-parented llama-server on the IQ4_XS local model
- `CLAUDE.md` runbook updated; both commits landed
