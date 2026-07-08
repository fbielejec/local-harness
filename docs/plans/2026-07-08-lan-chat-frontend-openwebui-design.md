# LAN Chat Frontend (Open WebUI) — Design

**Date:** 2026-07-08
**Status:** validated, implementing
**Topic:** a reboot-surviving, LAN-accessible web chat UI in front of the local Qwen model.

## Goal

Give any device on the home LAN (phone, tablet, laptop browser) a ChatGPT-style UI that
talks to the local `llama-server` (Qwen3-Coder-30B IQ4_XS) on `weebeastie`. Must survive
reboots. Keep the model private; put only the chat UI on the network.

## Decisions (what & why)

| Question | Decision | Why |
|----------|----------|-----|
| What is chatted with | The local LLM | Self-hosted ChatGPT for the household |
| Frontend | **Open WebUI** (Docker) | Best UX for ~zero custom code; mobile-responsive; per-user history/memory |
| Host | **weebeastie** | Always-on box where the model lives; laptop is a client |
| Networking | **`network_mode: host`** | `llama-server` is loopback-only (`127.0.0.1:8080`); a bridged container can't reach a loopback service. Host net lets the container use `localhost:8080` **and** exposes port 3000 to the LAN. Model stays private. |
| Port | **3000** | 8080 is taken by llama-server under host networking |
| Reboot survival | `restart: unless-stopped` + docker already `enabled` at boot | No extra systemd unit needed |
| Isolation posture | Pragmatic + cheap hardening | Trusted single-user home LAN |
| Auth | **Fixed accounts, signup locked** | Two users (filip admin + spouse) + a shared `guest` account |
| Provisioning | Manual, once | Passwords never touch git or transcripts; accounts persist in the volume |
| Memory | Per-user, on by default | Separate accounts ⇒ separate memories, automatically |
| Document RAG | **Off for now** (no embedder downloaded) | "Leave the retriever model for later"; enable later via admin panel |
| Web search | **On, via Exa** | User has an Exa API key; a deliberate, consented LAN-exit |
| Image pin | `ghcr.io/open-webui/open-webui:v0.10.2` | Reproducibility, matching the pinned GGUF ethos |

## Architecture

```
Any LAN device (browser)
        │  http://192.168.1.22:3000
        ▼
weebeastie (always-on)
  ┌─────────────────────────────────────────────┐
  │  Open WebUI  (Docker, network_mode: host)    │
  │     binds 0.0.0.0:3000  ── LAN-facing        │
  │        │ http://localhost:8080/v1            │
  │        ▼                                      │
  │  llama-server (systemd, 127.0.0.1:8080)      │  ← unchanged, still loopback-only
  │     Qwen3-Coder-30B IQ4_XS                    │
  └─────────────────────────────────────────────┘
```

**Trust boundary:** anyone who can reach `192.168.1.22:3000` can use the model. That is the
home LAN, the intended audience. The model port itself stays unreachable from other devices.

## Sandboxing posture (how contained is "the model in chat")

Three layers:

1. **The model** only turns tokens into tokens — zero host access inherently.
2. **Generated/interpreted code** runs in the **viewer's browser** (default engine `pyodide`,
   WASM in a sandboxed opaque-origin iframe), not on weebeastie. Rendered HTML/Artifacts are
   sandboxed iframes too. `ENABLE_CODE_INTERPRETER`/`ENABLE_CODE_EXECUTION` default on but
   client-side-sandboxed.
3. **The container** is the host-side boundary. Docker isolates its **filesystem** (sees only
   image + named volume — not `~`, repos, or `~/models`) and **processes**. The deliberate
   gap: `network_mode: host` removes **network** isolation (it can reach any localhost service
   and the LAN), and the official image runs as **root** inside the container.

**Would break the sandbox — all opt-in, all OFF by default:** server-side code execution
(`CODE_EXECUTION_ENGINE=jupyter` / Open Terminal) and admin-installed **Tools/Functions**
(server-side Python running in-container at root, host-networked). Be deliberate before
enabling any of these.

**Cheap hardening applied:** `security_opt: no-new-privileges:true`, `cap_drop: [ALL]`
(verify-and-relax if the entrypoint needs a capability). Not pursued: dropping host networking
(needs re-architecting model reachability), non-root user, read-only rootfs (Open WebUI writes
in several places).

## Configuration (docker-compose env)

Full file lives at `deploy/openwebui/docker-compose.yml`. Secrets (`WEBUI_SECRET_KEY`,
`EXA_API_KEY`) live in a gitignored `deploy/openwebui/.env`; `.env.example` is committed.

Key groups:

- **Connection:** `OPENAI_API_BASE_URL=http://localhost:8080/v1`, `OPENAI_API_KEY=dummy`,
  `ENABLE_OLLAMA_API=false`.
- **Network:** `PORT=3000`, `WEBUI_URL=http://192.168.1.22:3000`.
- **Auth:** `WEBUI_AUTH=true`, `ENABLE_SIGNUP=false` (first-admin bootstrap still allowed;
  admin creates the other two), `DEFAULT_USER_ROLE=pending`, `WEBUI_SECRET_KEY=${...}`.
- **Memory:** `ENABLE_MEMORIES=true`, `ENABLE_MEMORY_SYSTEM_CONTEXT=true` (per-user,
  char-budget injection ⇒ **no embedder needed**).
- **Document RAG off:** `RAG_EMBEDDING_ENGINE=openai` (external engine ⇒ no local
  SentenceTransformers model loaded at startup, so nothing is downloaded),
  `BYPASS_EMBEDDING_AND_RETRIEVAL=true`.
- **Web search (Exa):** `ENABLE_WEB_SEARCH=true`, `WEB_SEARCH_ENGINE=exa`,
  `EXA_API_KEY=${...}`, `BYPASS_WEB_SEARCH_EMBEDDING_AND_RETRIEVAL=true` (full-content
  injection ⇒ still no embedder), `WEB_SEARCH_RESULT_COUNT=3`,
  `ENABLE_WEB_SEARCH_CONFIRMATION=true` (explicit consent before each outbound query).
- **Nothing else leaves the LAN:** `ENABLE_VERSION_UPDATE_CHECK=false`,
  `ENABLE_COMMUNITY_SHARING=false`, `ENABLE_DIRECT_CONNECTIONS=false`,
  `ANONYMIZED_TELEMETRY=false`, `DO_NOT_TRACK=true`.

### Why NOT `OFFLINE_MODE`

`OFFLINE_MODE=true` / `HF_HUB_OFFLINE=1` block HF downloads but **abort a fresh install** with
`No embedding model is loaded` (the default SentenceTransformers engine loads the embedder at
startup). The `RAG_EMBEDDING_ENGINE=openai` + bypass-flags approach reaches "no download now"
without crashing, and keeps chat + web search + memory fully working.

### ConfigVar caveat

Most of these keys are "ConfigVar": the compose env only **seeds the first boot**; afterwards
the volume/DB is authoritative and changes are made in the **Admin Panel** (or by wiping the
volume). The compose file therefore documents *intent* + the initial seed.

## Provisioning the 3 accounts (manual, once)

Signup ships already locked (`enable_signup=false`), but `onboarding=true` lets the *first*
account self-create as admin. So:

1. First run: open `http://192.168.1.22:3000`, register **filip** → becomes admin.
2. Admin Panel → Users → add **spouse** (role user) and **guest** (role user, shared password).
3. Admin Panel → set the Qwen model's **Function Calling = Native** so `search_web` and the
   memory tools fire (llama-server's `--jinja` supports tool calling).
4. Admin Panel → Settings → Web Search → paste the **Exa API key**. This is authoritative:
   `EXA_API_KEY` is a ConfigVar already seeded empty on first boot, so editing `.env` + re-up
   won't override the DB value. Record it in `.env` too for reproducibility (volume re-seed).

Per-user memory is automatic. Note: *autonomous* memory (model self-deciding what to save) is
model-dependent and may be inconsistent with a small local model; **manual** memory
(Settings → Personalization → Memory) is deterministic.

## Reboot survival & ops

- `restart: unless-stopped` + `systemctl is-enabled docker` = `enabled` ⇒ auto-returns on boot.
- Runs from `~/openwebui/` on weebeastie.
- Ops: `docker compose ps` / `logs -f` / `restart` / `down`.
- Update: bump the image tag, `docker compose pull && docker compose up -d`.

## Verification (evidence before "done")

1. `docker compose logs` shows a clean startup with **no** HuggingFace/sentence-transformers
   download line.
2. Login page loads from the laptop at `http://192.168.1.22:3000`.
3. Register admin → send a chat → response streams from Qwen (proves `localhost:8080` wiring).
4. Web search toggle → ask something current → confirmation prompt fires → Exa results appear.
5. Add a memory as filip → confirm it persists and injects; confirm spouse's account doesn't
   see it.
6. Optional: `sudo reboot` weebeastie → container auto-returns.

## Out of scope / later

- Document RAG (enable embedder later — admin panel or flip the env keys).
- HTTPS/TLS (home LAN, plain HTTP for now).
- Redis (JWT revocation on logout — acceptable for home use).
- Client hardening of the Qwen-Code CLI (tracked separately in CLAUDE.md TODOs).
