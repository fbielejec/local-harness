# Remote Access to the Local Model — Design

**Date:** 2026-07-16
**Status:** **IMPLEMENTED and verified from outside** (2026-07-16). Two items outstanding:
sshd hardening, and Stage-2 verification from a phone hotspot. Runbook: `README.md` §3 +
§Remote access. Task-by-task record:
`docs/plans/2026-07-16-remote-model-access-implementation-plan.md`.
**Topic:** reach `llama-server`'s OpenAI-compatible API on `weebeastie` from a travel laptop
outside the home LAN, without weakening the loopback-only posture.

## Outcome (2026-07-16)

Works. `make away` from an external network → WireGuard handshake → SSH forward →
`llama-server` answering on `localhost:8080`, with the loopback-only invariant intact
(asserted, not assumed: `ss -lntp` still shows `127.0.0.1:8080` and nothing else).

**Settled by measurement, not argument:** the public IPv4 is **not** behind carrier NAT — the
one fact unfalsifiable from inside the LAN. A handshake arriving from a NordVPN exit disproved
it outright.

**Three things the design got wrong, corrected by building it:**

1. **`reresolve-dns` on a timer → dropped as YAGNI.** See the Endpoint-resolution section below.
   Bouncing the tunnel re-resolves; the timer was slower than the manual fix and rescued nothing.
2. **`systemctl enable wg-quick@wg0` on the laptop → dropped.** An auto-started tunnel is
   permanently broken *at home*. Replaced by explicit `make away` with a fail-fast reachability
   check. (The server-side enable was right and stayed.)
3. **DDNS-before-outside-test → reordered.** The original order tested the port-forward and
   deSEC simultaneously, so a failure would have implicated both — exactly the mistake the
   LAN-endpoint-first step was designed to avoid. Corrected to: forward → verify → DDNS → verify.

## Goal

From a café, point a laptop's `OPENAI_BASE_URL` at the home model and have `qwen`, the evals,
and any OpenAI client work exactly as they do at home. Nothing else in the harness changes.

**Explicitly out of scope** (deliberate YAGNI — revisit only if wanted):

- Open WebUI (`:3000`) from outside. It stays LAN-only.
- Shell/admin access as a *goal*. SSH is the transport here, so it comes along for free, but
  it isn't the thing being built.
- Driving an agent session at home from a phone (the claude-code "Remote Control" pattern).
  See *Prior art* below for why that's a dead end anyway.

## Decisions (what & why)

| Question | Decision | Why |
|----------|----------|-----|
| What's reachable | **The API only** (`:8080/v1`, later `:8081/v1`) | Narrowest scope that meets the goal |
| Transport | **WireGuard** (self-hosted, on weebeastie) | In-kernel, ~4k LoC, no daemon, no account, no third party. Silent to port scanners |
| Auth to the model | **SSH keys** (existing `~/.ssh`) | Already exist; `llama-server` gains no auth surface of its own |
| llama-server binding | **UNCHANGED — `127.0.0.1:8080`** | The load-bearing invariant. The model is never bound to any network interface, mesh or LAN |
| Tunnel command | `ssh -fN -L 8080:127.0.0.1:8080 filip@10.10.0.1` | Byte-identical to today's, only the host changes |
| Endpoint address family | **IPv4** (`<HOME_IPV4>`) | Café wifi is frequently IPv4-only. Native v6 exists but is unreachable from v4-only networks |
| Endpoint stability | **DDNS hostname**, not a raw IP | RIPE says the address is *dynamic*. Guards the lockout failure mode (below) |
| DDNS provider | **deSEC.io** | Nonprofit, open source, DNSSEC, no account harvesting. Sees an IP + hostname; no traffic |
| Router involvement | **One rule:** `UDP 51820 → 192.168.1.22` | Router stays dumb; WireGuard terminates on the box we actually want |
| Client routing | `AllowedIPs = 10.10.0.1/32` | Split tunnel — only the box goes through WireGuard, so Proton VPN can stay on |
| NordVPN Meshnet | **Rejected** (see *Rejected alternatives*) | Closed-source root daemon on weebeastie; iptables conflict with Docker; its only real advantage (NAT traversal) isn't needed |

## Architecture

```
travel laptop (café)                        weebeastie (home, Proximus BE)
                                              wg0  10.10.0.1  ◀── UDP 51820 fwd
  qwen-code                                     │
  OPENAI_BASE_URL=localhost:8080/v1             ▼
        │                                     sshd  ◀── reachable via wg0 (and LAN)
        ▼                                       │
  ssh -fN -L 8080:127.0.0.1:8080 \              ▼
      filip@10.10.0.1               ═══════▶  127.0.0.1:8080  llama-server
   wg0 10.10.0.2                              127.0.0.1:8081  rag-server (later)
        └── WireGuard (X25519) ──┘
            + SSH (RSA-4096, rsa-sha2-512)
```

**Two key types, two jobs.** WireGuard uses X25519 (ECDH). SSH authenticates the *user* with
the existing **RSA-4096** key (`~/.ssh/id_rsa`) — verified 2026-07-16; an earlier draft of this
doc said ed25519, having confused it with weebeastie's ed25519 **host** key, which is a
different key doing a different job. Signatures negotiate `rsa-sha2-512` (the `ssh-rsa` label
in `authorized_keys` is the *key type*, not the deprecated SHA-1 signature algorithm), so
RSA-4096 here is sound and needs no churn.

The two key types are unrelated and non-interchangeable: generate WireGuard keys fresh (two
commands) and leave the SSH key alone. The answer wouldn't change with an ed25519 SSH key
either — ed25519 and X25519 are birationally equivalent so conversion is *mathematically*
possible, but WireGuard's tooling doesn't accept it, and reusing one keypair across a
signature scheme and a key-agreement scheme is a genuine footgun.

**Why SSH-over-WireGuard rather than binding to `wg0`.** The shortcut is `--host 10.10.0.1`
on llama-server, skipping SSH. Rejected: it would need llama.cpp's `--api-key` (a bearer
string in a config file, not a credential system), and it puts an unauthenticated inference
endpoint one firewall mistake from the LAN. Going through SSH means auth is a keypair that
already exists, encryption is doubled, and the loopback-only invariant survives intact.

## Network facts (measured 2026-07-16)

| Fact | Value | Consequence |
|------|-------|-------------|
| Public IPv4 | `<HOME_IPV4>` | **Not CGNAT** (`100.64.0.0/10`) ⇒ port forwarding works |
| RIPE `inetnum` | `193.121.64.0 – 193.121.127.255`, Proximus NV/SA, **"xDSL customers (dynamic)"** | The IP is *sticky*, not static ⇒ DDNS required |
| Native IPv6 | `<HOME_V6_PREFIX>::/64`, RA-delegated, `FIA Pools - Dynamic address space` | Usable, but not as the primary endpoint |
| v6 addresses | one `temporary dynamic` (rotates ~daily, `use_tempaddr=2`), one RFC-7217 stable-privacy | Neither is dependable; the "stable" one is stable only while the prefix is |
| RA lease | `valid_lft ~86400s` | Prefix renews daily; a router reboot may hand out a new one |

**Unfalsifiable from inside:** whether `<HOME_IPV4>` is the router's WAN address or a
carrier NAT using public-range pools. The two look identical from the LAN. Proximus doesn't
carrier-NAT residential xDSL, so this is very likely fine — but the real test is whether a
handshake lands after the port forward is configured. Cheap to falsify; do it first.

## The lockout failure mode

The one genuinely bad failure in this design, and the reason DDNS is not optional:

1. You're abroad. Proximus resyncs the line. The home IP changes.
2. Your client config hardcodes the old `Endpoint`. The tunnel is dead.
3. Fixing it requires knowing the new IP — which requires access to the house.

Chicken-and-egg, and a sticky IP makes it *rare enough that you'll have forgotten about it*
by the time it bites. DDNS makes the endpoint self-healing.

**Gotcha:** `wg-quick` resolves `Endpoint` **once, at interface start, and never
re-resolves.** A live tunnel will sit pointed at a stale address indefinitely.

**The fix is a bounce, not a daemon.** `wg-quick down && up` re-resolves (it calls `wg setconf`,
which resolves hostnames at that moment) — so `make away-stop && make away` is the whole remedy.
Every `make away` therefore starts with a fresh DNS answer; the only exposure is an IP change
*mid-session*.

**`reresolve-dns` on a timer: considered and rejected (YAGNI).** `wireguard-tools` ships one
(`/usr/share/doc/wireguard-tools/examples/reresolve-dns/`) and the obvious move is to run it on
a timer. Three reasons not to:

1. **It rescues nothing.** When the tunnel dies the SSH session and its forward die with it.
   Auto-healing the WireGuard layer underneath doesn't bring the session back — you re-establish
   SSH by hand regardless, and that means running `make away`, which re-resolves anyway.
2. **It's slower than the thing it replaces.** The script only acts once a handshake is >135 s
   stale, and a 2-min timer tops that up: ~4 min worst case, versus ~10 s to bounce it yourself.
3. **It only pays off for unattended connections** — a network mount, `autossh`, a headless box
   that must dial home. None exist here; a human runs `make away` on sitting down.

Revisit only if something long-lived and unattended ever depends on the tunnel.

## Configuration

`/etc/wireguard/wg0.conf` — **weebeastie**:

```ini
[Interface]
Address    = 10.10.0.1/24
ListenPort = 51820
PrivateKey = <weebeastie private>

[Peer]                              # travel laptop
PublicKey  = <laptop public>
AllowedIPs = 10.10.0.2/32
```

`/etc/wireguard/wg0.conf` — **travel laptop**:

```ini
[Interface]
Address    = 10.10.0.2/24
PrivateKey = <laptop private>

[Peer]
PublicKey  = <weebeastie public>
Endpoint   = <ddns-name>.dedyn.io:51820
AllowedIPs = 10.10.0.1/32           # split tunnel — only the box
PersistentKeepalive = 25            # holds the NAT mapping open from behind café wifi
```

Keys: `wg genkey | tee privatekey | wg pubkey > publickey` on each machine. Private keys
never leave their machine; only public keys are exchanged. **Nothing here goes in git** —
`/etc/wireguard/` is root-only (`chmod 600`) and the configs contain private keys.

Enable: `sudo systemctl enable --now wg-quick@wg0` on both.

One peer block per travel device, each with its own `10.10.0.x/32`. WireGuard has no key
distribution to manage — adding a device is a keygen plus a stanza on both ends.

## sshd hardening (now load-bearing)

Today sshd only faces a trusted LAN. It's about to face the tunnel, so the posture matters
more even though WireGuard is doing the real gatekeeping:

- `PasswordAuthentication no`, `PubkeyAuthentication yes` — key-only.
- `PermitRootLogin no`.
- **Do not** forward `22` on the router. sshd should be reachable via `wg0` and the LAN only.
  WireGuard is the front door; SSH behind it. An unauthenticated WireGuard packet gets *no
  reply at all*, so the box stays invisible to internet-wide scanners — which is exactly the
  property we'd throw away by exposing `22`.

## Testing

### Never test the forward from the LAN

The obvious test — laptop on the LAN, dial `<HOME_IPV4>:51820` — is a trap. Many consumer
routers don't implement NAT loopback (hairpinning), so packets from inside addressed to the
WAN IP are simply dropped. **You get a false negative and go hunting a bug that isn't there.**
The packets must arrive from genuinely outside.

### Stage 1 — NordVPN as the test harness (convenient; kills the carrier-NAT question)

Connect the **laptop** to NordVPN, stay physically on the home LAN, then bring up `wg0`.
Traffic exits at Nord's node and re-enters at `<HOME_IPV4>` from the real internet, so the
port-forward is exercised on the real path with no hairpin involved.

This is a legitimate use of Nord and **not** a contradiction of its rejection above: the
objection was to a closed-source root daemon on *weebeastie*, the box holding the models and
the RAG index. On the travel laptop, as a temporary test client, that objection doesn't apply.

What Stage 1 proves: the port-forward rule, a foreign source address, and — decisively — that
`<HOME_IPV4>` is not behind a carrier NAT. A handshake here settles the one fact we
couldn't establish from inside.

Two artifacts **of the test setup**, not of the real design:

- **MTU.** NordLynx is WireGuard, so this nests WireGuard-in-WireGuard: ~80 bytes of extra
  encapsulation. Classic symptom is a **handshake that succeeds while data silently stalls**.
  That's MTU, not auth. Set `MTU = 1280` under `[Interface]` to confirm before chasing
  anything else. Won't occur unnested.
- **Kill switch.** Nord's kill switch can blackhole the nested tunnel. Leave it off for the
  test.

### Stage 2 — phone hotspot (realism; do this before depending on it)

**A Stage 1 pass does not predict a café pass.** Nord exit nodes are clean, well-connected
datacenter networks. Real café wifi is worse in exactly the ways that bite this design:
outbound UDP on odd ports sometimes blocked, captive portals, restrictive NAT, odd MTUs.

A phone hotspot is the better realism test and is nearly free: your carrier almost certainly
CGNATs you, which is a realistic stand-in for café conditions and a *harder* test than Nord.

### The checks (run under Stage 1, then again under Stage 2)

1. **Handshake:** `sudo wg show` → non-zero `latest handshake`, growing `transfer` **in both
   directions**. Growing TX with flat RX = packets leaving, nothing coming back ⇒ the forward
   isn't landing.
2. **Tunnel:** `ssh filip@10.10.0.1 'systemctl is-active llama-server'` → `active`.
3. **API:** with the forward up, `curl -s localhost:8080/v1/models` on the laptop.
4. **End-to-end:** the existing smoke test — `qwen -p "…read notes.txt…"` → `artichoke`.
5. **Regression (from the LAN, tunnel down):** Open WebUI still answers on
   `192.168.1.22:3000`, and `llama-server` is still **not** reachable at `192.168.1.22:8080`
   — the loopback-only invariant, asserted rather than assumed.
6. **DDNS:** `dig +short <ddns-name>.dedyn.io` matches `curl -4 -s ifconfig.me` on weebeastie.
   The updater is only proven by a *change*, so this is a weak check until the IP actually
   moves — see Open questions.

## Rejected alternatives

| Option | Why rejected |
|--------|--------------|
| **NordVPN Meshnet** | Initially chosen as "easiest", then abandoned once costed honestly: a **closed-source root daemon** on the box holding the models, RAG index, and MEP-office PDFs — a bigger concession than the control-plane metadata question it was picked over. Also rewrites iptables, which Docker (Open WebUI) also does. Its one real advantage is NAT traversal, which a non-CGNAT public IP makes unnecessary. Kept as fallback **only** if port forwarding turns out to be impossible. |
| **Proton VPN / NordVPN as VPNs** | Category error. Commercial VPNs solve *egress* privacy (hiding traffic from café wifi); this needs *ingress* (reaching a box behind home NAT). Running one *on* weebeastie makes it less reachable, not more. Proton's port forwarding assigns a random port that changes on reconnect — built for torrent clients, not a stable endpoint. |
| **Cloudflare Tunnel** | Third party terminates TLS and sees prompts in plaintext. Contradicts the project's premise. |
| **Exposing `:8080` with TLS + bearer auth** | Puts an inference endpoint on the public internet behind a config-file string. Breaks the loopback-only invariant for no gain over SSH. |
| **Plain SSH on a forwarded `:22`** | Works, zero new software — but announces itself to internet-wide scanners. WireGuard's silence is worth two config files. |
| **IPv6 as primary endpoint** | Café wifi is frequently IPv4-only; an unreachable endpoint is worthless. Kept as a documented fallback (one-line `Endpoint` change). |

## Prior art: claude-code Remote Control (why it's not reusable)

Researched 2026-07-16. Three independent blockers, any one fatal:

1. **The relay *is* the API.** Remote Control requires claude.ai OAuth against
   `api.anthropic.com` and, as of v2.1.196, disables itself if `ANTHROPIC_BASE_URL` points
   elsewhere. A llama-server setup trips this automatically. No separation between "relay my
   session" and "run my model".
2. **The transcript is persisted at Anthropic.** Execution stays local, but every message,
   response, and tool call is stored server-side for cross-device sync. ZDR orgs are barred
   from the feature — Anthropic's own answer to "can this be no-egress?" is no.
3. **Our harness is Qwen-Code**, not Claude Code.

[Issue #25746](https://github.com/anthropics/claude-code/issues/25746) requested exactly this
use case (a `--serve` mode reachable over a private mesh); closed as *not planned*.

**The one idea worth stealing** — should the scope ever widen to driving an agent at home
from a phone — is the **topology**: all three Anthropic designs (Remote Control, self-hosted
sandbox workers, Channels) converge on *the machine doing the work dials out and polls;
nothing inbound is opened*. That buys NAT traversal, which we don't need here. Noted for
if/when the "drive an agent session at home" scope returns.

Sources: [remote-control](https://code.claude.com/docs/en/remote-control) ·
[channels](https://code.claude.com/docs/en/channels) ·
[self-hosted-sandboxes](https://platform.claude.com/docs/en/managed-agents/self-hosted-sandboxes)

## Open questions

- **Does the Proximus b-box allow a UDP port-forward to a fixed LAN IP?** Expected yes.
  Needs `192.168.1.22` to be a DHCP reservation, or the forward breaks on lease change.
- **Prefix stability**, if IPv6 is ever promoted to primary. Unmeasured; would need watching
  across a router reboot.
- **`rag-server` on `:8081`** — same treatment, one more `-L`. Free once this works.
- **Sandboxing** — unrelated to this design but adjacent: the eval gate still runs
  `qwen --yolo` unsandboxed. Remote access doesn't worsen it, but it does mean a compromised
  travel laptop reaches a box that auto-executes agent tool calls. Tracked in CLAUDE.md TODOs.
