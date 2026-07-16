# Remote Model Access (WireGuard) — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan
> task-by-task.

## Status — 2026-07-16: executed, Tasks 1–9 done and verified

| Task | State | Note |
|------|-------|------|
| 1 DHCP reservation | ✅ | Step 3's verification was **rewritten mid-flight** — the original `dhclient -r` would have deconfigured the interface over the SSH session carrying it, and targeted the wrong DHCP client (NetworkManager owns it). |
| 2 Install WireGuard | ✅ | ufw **inactive** ⇒ Step 2 of Task 4 skipped. |
| 3 Keys | ✅ | Private keys generated in place, never printed. |
| 4 weebeastie `wg0` | ✅ | `wg-quick@wg0` **enabled at boot**. Invariant asserted: `127.0.0.1:8080`. |
| 5 Laptop, LAN endpoint | ✅ | The decomposition worked — proved the config with the router out of the picture. SSH host-key match also confirmed the tunnel lands on the same box. |
| 6 deSEC DDNS | ✅ | A only, **AAAA absent**, TTL 60. Empty `dig` was resolver negative-caching, not a fault. |
| 7 Port forward | ✅ | UDP 51820 → `192.168.1.22`. `:22` **not** forwarded. |
| 8 Outside test (NordVPN) | ✅ | **Handshake + model response.** Carrier NAT falsified. No MTU workaround needed (nesting was fine at 1420). |
| 9 Ergonomics | ✅ | Grew beyond plan: `~/.ssh/config` (both aliases) **+ root `Makefile` targets** — `make away` / `tunnels` / `wg-status` / `away-stop`. |
| 10 reresolve timer | ❌ **DROPPED** | YAGNI — see the task below for the reasoning. |
| 11 sshd hardening | ⬜ | Pending. |
| 12 Hotspot (Stage 2) | ⬜ | Pending — the only remaining test of *reality* vs a clean datacenter exit. Step 6 (`enable wg-quick@wg0`) is **cancelled**; see the task. |
| 13 Docs | ✅ | README §3 + §Remote access; CLAUDE.md; this table. |

**Also handled:** the design docs originally recorded the real home IP and reached **public**
GitHub. Redacted to `<HOME_IPV4>` / `<HOME_V6_PREFIX>` / `<DDNS_HOST>`, history rewritten,
force-pushed. ⚠️ The pre-rewrite commits **remain fetchable by SHA** until GitHub GCs them — a
support request is still outstanding. CLAUDE.md now carries a never-commit rule.

---

**Goal:** reach `llama-server`'s OpenAI-compatible API on `weebeastie` from a travel laptop
outside the home LAN, without changing the loopback-only binding.

**Architecture:** WireGuard terminates on `weebeastie` (`10.10.0.1`); the laptop
(`10.10.0.2`) tunnels to it and then runs the *existing* SSH local-forward
(`-L 8080:127.0.0.1:8080`) over that tunnel. `llama-server` stays bound to `127.0.0.1:8080`
and never learns any of this happened. WireGuard authenticates the tunnel (X25519), SSH
authenticates the user (existing ed25519 keys). Design + rationale:
`docs/plans/2026-07-16-remote-model-access-design.md`.

**Tech Stack:** WireGuard (in-kernel, `wireguard-tools`) · OpenSSH (already deployed) ·
deSEC.io DDNS + `systemd` timers · Proximus b-box (one UDP port-forward rule).

---

## Read this before Task 1

**This plan cannot be executed unattended.** Tasks 1 and 7 need your hands on the router's
admin page; Tasks 8 and 12 need you on a foreign network. Everything else is scripted.

**Three rules that matter more than the steps:**

1. **Never paste a private key into this session.** The transcript is a file. Commands below
   are written so private keys are generated in place and only *public* keys are ever
   displayed. Don't "helpfully" `cat` a private key to check it.
2. **Don't touch sshd until WireGuard works** (Task 11, deliberately near the end). We reach
   weebeastie *over SSH* to configure all of this. Hardening early risks locking yourself out
   of the box you're standing on, with no tunnel yet to recover through.
3. **Never test the port-forward from the LAN.** Consumer routers commonly lack NAT loopback,
   so an inside test against the public IP fails for reasons unrelated to your config. See
   design §Testing.

**No IP forwarding, no masquerade.** Most WireGuard guides tell you to enable
`net.ipv4.ip_forward` and add a NAT rule. **We don't, and shouldn't.** Those are for using
the box as a *gateway* into the LAN. Here the tunnel's only destination is weebeastie itself
(`AllowedIPs = 10.10.0.1/32`), so packets terminate at the interface. Skipping it keeps the
blast radius at one host instead of the whole LAN. If you find yourself adding a
`MASQUERADE` rule, stop — you've misread the design.

---

## Task 1: DHCP-reserve the box's LAN address

**Rationale:** the port-forward rule points at `192.168.1.22`. That address is currently a
DHCP *lease*, not a reservation — if it moves, the forward silently points at nothing.

**Step 1: Get the MAC address**

```bash
ssh filip@192.168.1.22 'ip -br link show enp3s0'
```

Note the MAC (second column).

**Step 2: Reserve it** — b-box admin (usually `http://192.168.1.1`) → DHCP / LAN settings →
add a static lease binding that MAC to `192.168.1.22`.

**Step 3: Verify it survives a lease renewal**

⚠️ **Do not** use `dhclient -r enp3s0 && dhclient enp3s0` (an earlier draft of this plan did).
Two reasons: (a) releasing the lease deconfigures the interface *over the SSH connection
carrying the command* — if the renew half doesn't land, weebeastie has no IP and needs
**physical access** to recover; (b) the interface is managed by **NetworkManager**
(`Wired connection 1`) using its internal DHCP client, so a manual `dhclient` fights NM
rather than testing it.

Instead, bounce the NM connection **detached**, so an SSH drop can't strand the box:

```bash
ssh filip@192.168.1.22 'sudo systemd-run --on-active=3 --unit=nm-bounce \
  nmcli connection up "Wired connection 1"'
sleep 20
ssh filip@192.168.1.22 'ip -4 -br addr show enp3s0'
```

Expected: still `192.168.1.22/24`. `systemd-run` detaches the bounce from the SSH session, so
the reconnect happens regardless of what the connection drop does to our shell.

If the second `ssh` refuses to connect, wait another 20s and retry — NM is still settling.

**Do not commit anything.** No repo changes in this task.

---

## Task 2: Install WireGuard on both machines

**Step 1: weebeastie**

```bash
ssh filip@192.168.1.22 'sudo apt-get update && sudo apt-get install -y wireguard wireguard-tools'
```

**Step 2: laptop**

```bash
sudo apt-get install -y wireguard wireguard-tools
```

**Step 3: Verify — the module loads**

```bash
ssh filip@192.168.1.22 'sudo modprobe wireguard && lsmod | grep -c wireguard'
```

Expected: `1` or higher. Kernel 6.8 has WireGuard built in; a failure here means something is
badly wrong, stop and diagnose.

**Step 4: Check whether a host firewall is in play**

```bash
ssh filip@192.168.1.22 'sudo ufw status 2>/dev/null || echo "ufw not installed"'
```

Note the answer. If it says `Status: active`, you'll need `sudo ufw allow 51820/udp` in
Task 4 — if it says inactive or not installed, skip that.

---

## Task 3: Generate keys (private keys never displayed)

**Step 1: weebeastie**

```bash
ssh filip@192.168.1.22 'sudo install -d -m 700 /etc/wireguard && \
  sudo sh -c "umask 077; wg genkey > /etc/wireguard/privatekey; \
              wg pubkey < /etc/wireguard/privatekey > /etc/wireguard/publickey" && \
  echo "weebeastie public key:" && sudo cat /etc/wireguard/publickey'
```

**Step 2: laptop**

```bash
sudo install -d -m 700 /etc/wireguard && \
sudo sh -c 'umask 077; wg genkey > /etc/wireguard/privatekey; \
            wg pubkey < /etc/wireguard/privatekey > /etc/wireguard/publickey' && \
echo "laptop public key:" && sudo cat /etc/wireguard/publickey
```

**Step 3: Verify the private keys are unreadable and unprinted**

```bash
ssh filip@192.168.1.22 'sudo stat -c "%a %n" /etc/wireguard/privatekey'
```

Expected: `600 /etc/wireguard/privatekey`.

Record both **public** keys — you need each on the other machine. Public keys are safe to
display and safe in a transcript; that's the whole point of the asymmetry.

---

## Task 4: Configure and start WireGuard on weebeastie

**Files:** Create `/etc/wireguard/wg0.conf` on weebeastie (root-only, **never** in git).

**Step 1: Write the config** — substitute `<laptop public>` with Task 3 Step 2's output. The
private key is interpolated by root at write time, so it never transits this session.

```bash
ssh filip@192.168.1.22 'sudo sh -c "umask 077; cat > /etc/wireguard/wg0.conf <<EOF
[Interface]
Address    = 10.10.0.1/24
ListenPort = 51820
PrivateKey = \$(cat /etc/wireguard/privatekey)

[Peer]
PublicKey  = <laptop public>
AllowedIPs = 10.10.0.2/32
EOF"'
```

**Step 2: If and only if ufw was active in Task 2 Step 4**

```bash
ssh filip@192.168.1.22 'sudo ufw allow 51820/udp'
```

**Step 3: Start it, and make it survive reboot**

```bash
ssh filip@192.168.1.22 'sudo systemctl enable --now wg-quick@wg0'
```

**Step 4: Verify the interface exists and is listening**

```bash
ssh filip@192.168.1.22 'sudo wg show; ip -4 -br addr show wg0; sudo ss -lunp | grep 51820'
```

Expected: `wg0` holds `10.10.0.1/24`; `wg show` lists one peer with
`latest handshake: (none)` — correct, nothing has dialed in yet; a UDP socket on `51820`.

**Step 5: Verify the invariant still holds**

```bash
ssh filip@192.168.1.22 'ss -lntp | grep -E ":8080|:3000"'
```

Expected: `llama-server` still bound to **`127.0.0.1:8080`** and nothing else. If it shows
`0.0.0.0:8080`, stop — the design's central invariant is broken.

---

## Task 5: Configure the laptop — LAN endpoint first (deliberately)

**Rationale — this is the key decomposition.** There are two independent unknowns: *is my
WireGuard config right?* and *is my port-forward right?* Pointing the endpoint at the **LAN**
address first tests the config alone, with the router entirely out of the picture. If it
works here and fails later, the delta is unambiguously the router. Testing both at once and
debugging the union is how people lose an evening.

**Files:** Create `/etc/wireguard/wg0.conf` on the laptop.

**Step 1: Write the config** — `<weebeastie public>` from Task 3 Step 1. Note the endpoint is
the **LAN** IP for now; Task 8 swaps it.

```bash
sudo sh -c 'umask 077; cat > /etc/wireguard/wg0.conf <<EOF
[Interface]
Address    = 10.10.0.2/24
PrivateKey = $(cat /etc/wireguard/privatekey)

[Peer]
PublicKey  = <weebeastie public>
Endpoint   = 192.168.1.22:51820
AllowedIPs = 10.10.0.1/32
PersistentKeepalive = 25
EOF'
```

**Step 2: Bring it up** (`up`, not `enable` — this config is temporary)

```bash
sudo wg-quick up wg0
```

**Step 3: Verify a handshake lands**

```bash
sudo wg show
```

Expected: `latest handshake:` shows a time, and **`transfer:` is non-zero in BOTH
directions**. Growing TX with flat RX means packets are leaving and nothing is coming back —
that's a one-way failure, not success.

**Step 4: Verify the tunnel carries traffic**

```bash
ping -c3 10.10.0.1
ssh filip@10.10.0.1 'systemctl is-active llama-server'
```

Expected: replies, then `active`.

**Step 5: Verify the whole point of the exercise**

```bash
ssh -fN -L 8080:127.0.0.1:8080 filip@10.10.0.1
curl -s localhost:8080/v1/models | head -c 200
```

Expected: JSON naming the IQ4_XS model. **This is the deliverable** — everything after this
task is about making it work from *outside*, not about making it work.

**Step 6: Tear down before proceeding**

```bash
pkill -f "ssh -fN -L 8080" ; sudo wg-quick down wg0
```

---

## Task 6: Set up deSEC DDNS

**Rationale:** the design's lockout failure mode. RIPE marks the address dynamic; if it moves
while you're abroad, a hardcoded endpoint is unrecoverable — fixing it needs the new IP, and
learning the new IP needs access to the house.

**Step 1** — register at <https://desec.io>, create a `dedyn.io` domain, generate a token.

**Step 2: Store the credentials** (root-only, not in git)

```bash
ssh filip@192.168.1.22 'sudo sh -c "umask 077; cat > /etc/desec-updater.env <<EOF
DESEC_HOST=<your-name>.dedyn.io
DESEC_TOKEN=<your token>
EOF"'
```

**Step 3: The updater script**

Note `curl -4` and `myipv6=` — both deliberate, see Step 5.

```bash
ssh filip@192.168.1.22 'sudo sh -c "cat > /usr/local/bin/desec-update <<\"EOF\"
#!/bin/sh
set -eu
. /etc/desec-updater.env
exec curl -4 -fsS --max-time 30 \
  --header \"Authorization: Token \$DESEC_TOKEN\" \
  \"https://update.dedyn.io/?hostname=\$DESEC_HOST&myipv6=\"
EOF
chmod 700 /usr/local/bin/desec-update"'
```

**Step 4: Timer + unit**

```bash
ssh filip@192.168.1.22 'sudo sh -c "cat > /etc/systemd/system/desec-update.service <<EOF
[Unit]
Description=Update deSEC DDNS record
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=/usr/local/bin/desec-update
EOF
cat > /etc/systemd/system/desec-update.timer <<EOF
[Unit]
Description=Update deSEC DDNS record every 5 minutes

[Timer]
OnBootSec=1min
OnUnitActiveSec=5min

[Install]
WantedBy=timers.target
EOF
systemctl daemon-reload
systemctl enable --now desec-update.timer"'
```

**Step 5: Verify — and understand why `myipv6=` matters**

```bash
ssh filip@192.168.1.22 'sudo systemctl start desec-update && sudo journalctl -u desec-update -n5 --no-pager'
dig +short A    <your-name>.dedyn.io      # expect <HOME_IPV4>
dig +short AAAA <your-name>.dedyn.io      # expect EMPTY — this is the important one
```

**The AAAA record must be empty.** weebeastie has native IPv6, and deSEC would happily
publish an AAAA. If it exists, `wg-quick` may resolve the endpoint to IPv6 — which works
beautifully at home and then **fails on IPv4-only café wifi**, i.e. exactly when you need it
and can't debug it. `curl -4` forces v4 detection; `myipv6=` deletes the AAAA.

**Caveat:** this proves the updater *runs*, not that it *updates*. Only an actual IP change
proves that. See Open questions.

---

## Task 7: Forward the port on the router

**Step 1** — b-box admin → NAT / port forwarding → add: **protocol UDP, external port 51820,
internal host `192.168.1.22`, internal port 51820**.

**Step 2: Do NOT forward port 22.** Deliberate. WireGuard is the front door; SSH lives behind
it. An unauthenticated WireGuard packet gets *no reply at all*, so the box stays invisible to
internet-wide scanners — precisely the property you'd throw away by exposing 22.

**Step 3:** no verification here. It cannot be tested from the LAN (hairpinning). Task 8 is
the test.

---

## Task 8: Stage 1 — verify from outside via NordVPN

**Rationale:** routes your packets out to Nord and back in via `<HOME_IPV4>` from the real
internet, so the forward is exercised on the real path with no hairpin. This also settles the
one fact unfalsifiable from inside: whether that address is behind a carrier NAT.

Using Nord here is **not** a contradiction of rejecting it in the design — the objection was
to a closed-source root daemon on *weebeastie*. As a temporary test client on the laptop,
that objection doesn't apply.

**Step 1: Point the endpoint at the DDNS name**

```bash
sudo sed -i 's|^Endpoint .*|Endpoint   = <your-name>.dedyn.io:51820|' /etc/wireguard/wg0.conf
grep Endpoint /etc/wireguard/wg0.conf
```

**Step 2: Connect NordVPN on the laptop, kill switch OFF** (it can blackhole the nested
tunnel). Verify you're actually exiting elsewhere:

```bash
curl -4 -s ifconfig.me    # expect a Nord IP, NOT <HOME_IPV4>
```

**Step 3: Bring up the tunnel**

```bash
sudo wg-quick up wg0
sudo wg show
```

Expected: a handshake, non-zero transfer **both** ways.

**Step 4: Run the full check**

```bash
ssh filip@10.10.0.1 'systemctl is-active llama-server'
ssh -fN -L 8080:127.0.0.1:8080 filip@10.10.0.1
curl -s localhost:8080/v1/models | head -c 200
```

**Step 5: If the handshake succeeds but data stalls — that's MTU, not auth**

NordLynx is WireGuard, so this nests WireGuard-in-WireGuard: ~80 bytes extra encapsulation.
Classic symptom is a clean handshake followed by silence.

```bash
sudo wg-quick down wg0
sudo sed -i '/^\[Interface\]/a MTU = 1280' /etc/wireguard/wg0.conf
sudo wg-quick up wg0
```

**This is an artifact of the test, not the design** — it won't occur unnested. If 1280 fixes
it, remove the line again before Task 12 and re-test there.

**Step 6: If there is no handshake at all** — the forward isn't landing. Check, in order: the
b-box rule says **UDP** (not TCP); `dig +short A <name>.dedyn.io` matches weebeastie's
`curl -4 -s ifconfig.me`; `sudo ss -lunp | grep 51820` on weebeastie still shows the socket;
ufw. If all are clean, carrier NAT is back on the table and the design's fallback (Meshnet)
applies.

**Step 7: Disconnect NordVPN, tear down.**

---

## Task 9: Ergonomics — one command instead of four

**Files:** Create `~/.ssh/config` entry on the laptop.

**Step 1:**

```
Host weebeastie-remote
    HostName 10.10.0.1
    User filip
    LocalForward 8080 127.0.0.1:8080
    ExitOnForwardFailure yes
```

`ExitOnForwardFailure yes` is load-bearing: without it, SSH connects happily while the
forward silently fails, and you get a confusing `curl: connection refused` against a *live*
session.

**Step 2: Verify**

```bash
sudo wg-quick up wg0 && ssh -fN weebeastie-remote && curl -s localhost:8080/v1/models | head -c 100
```

**Step 3: Commit** — this is the first repo change in the plan.

```bash
git add README.md && git commit -m "docs: record remote access runbook"
```

---

## Task 10: ~~Endpoint re-resolution on the laptop~~ — DROPPED (YAGNI)

**Kept as a record of the reasoning, not as work to do.**

The original task installed `reresolve-dns` (ships with `wireguard-tools` at
`/usr/share/doc/wireguard-tools/examples/reresolve-dns/`) on a 2-minute systemd timer, on the
premise that `wg-quick` resolves `Endpoint` **once at interface start and never re-resolves**,
so DDNS alone can't heal a live tunnel.

The premise is true. The conclusion didn't follow. **Dropped because:**

1. **It rescues nothing.** When the tunnel dies, the SSH session and its forward die with it.
   Healing the WireGuard layer underneath doesn't restore the session — you re-establish SSH by
   hand anyway, which means `make away`, which re-resolves. The window it covers is already
   covered.
2. **It's slower than the alternative.** The script only acts once a handshake is >135 s stale,
   plus up to 2 min of timer: ~4 min worst case. A manual `make away-stop && make away` is ~10 s.
   The automation is slower than the thing it automates.
3. **It only pays for unattended connections** — a network mount, `autossh`, a headless box that
   must dial home. None exist here.

**What replaces it:** a line in the README — *tunnel died? `make away-stop && make away`* — and
the `make away` target itself, which resolves fresh on every invocation.

**Revisit if** something long-lived and unattended ever depends on the tunnel staying up.

**If an earlier run already installed it:**

```bash
sudo systemctl disable --now wg-reresolve.timer
sudo rm -f /etc/systemd/system/wg-reresolve.{timer,service} /usr/local/bin/wg-reresolve-dns
sudo systemctl daemon-reload
```

---

## Task 11: Harden sshd (last, on purpose)

**Rationale:** sshd now faces the tunnel, not just a trusted LAN. Doing this earlier risks
locking yourself out of the box you're configuring, before a tunnel exists to recover
through. WireGuard is the real gatekeeper; this is defence in depth.

**Step 1: Confirm key auth works before changing anything**

```bash
ssh -o PasswordAuthentication=no filip@192.168.1.22 'echo key-auth-ok'
```

Expected: `key-auth-ok`. **If this fails, STOP.** Fix key auth first — the next step will
lock you out otherwise.

**Step 2: Keep an escape hatch open.** In a *separate terminal*, hold a live root-capable
session to weebeastie for the duration of this task. Do not close it until Step 5 passes.

**Step 3: Apply**

```bash
ssh filip@192.168.1.22 'sudo sh -c "cat > /etc/ssh/sshd_config.d/99-hardening.conf <<EOF
PasswordAuthentication no
PubkeyAuthentication yes
PermitRootLogin no
KbdInteractiveAuthentication no
EOF
sshd -t"'
```

`sshd -t` validates the config. **If it errors, do not restart.**

**Step 4: Restart**

```bash
ssh filip@192.168.1.22 'sudo systemctl restart ssh'
```

**Step 5: Verify from a NEW session**

```bash
ssh filip@192.168.1.22 'echo still-in'
```

Expected: `still-in`. Only now close the escape-hatch terminal.

---

## Task 12: Stage 2 — verify from a phone hotspot

**Rationale: a Stage 1 pass does not predict a café pass.** Nord exit nodes are clean,
well-connected datacenter networks. Your carrier almost certainly CGNATs you, which is a
realistic stand-in for café conditions and a genuinely *harder* test. This is the last task
because it's the only one that tests reality.

**Step 1:** tether the laptop to the phone. Wifi off, **NordVPN off**, no LAN path — verify
you're genuinely isolated from home:

```bash
ping -c1 -W2 192.168.1.22 || echo "good: no LAN path"
```

**Step 2: Remove the MTU workaround if Task 8 Step 5 added it**

```bash
sudo sed -i '/^MTU = 1280/d' /etc/wireguard/wg0.conf
```

**Step 3: Full run**

```bash
sudo wg-quick up wg0 && sudo wg show
ssh -fN weebeastie-remote
curl -s localhost:8080/v1/models | head -c 200
```

**Step 4: End-to-end — the existing smoke test, unchanged**

```bash
cd ~/qwen-scratch
export OPENAI_BASE_URL="http://localhost:8080/v1" OPENAI_API_KEY=dummy \
       OPENAI_MODEL="unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS"
qwen -p "Use your tools to read notes.txt and tell me the secret word."
```

Expected: `artichoke`. That it's the *same* smoke test with the *same* env vars is the proof
that the design's invariant held — the harness cannot tell it's not at home.

**Step 5: Regression — from the LAN, tunnel down**

```bash
sudo wg-quick down wg0
curl -s -m3 http://192.168.1.22:3000 >/dev/null && echo "Open WebUI: OK"
curl -s -m3 http://192.168.1.22:8080/v1/models && echo "BROKEN: model exposed on LAN" || echo "loopback invariant: OK"
```

The second check asserts the invariant rather than assuming it.

**Step 6: Do NOT enable wg-quick@wg0 at boot**

An earlier draft of this plan ended here with `systemctl enable wg-quick@wg0`. **That is
wrong**, and running the plan proved it: the endpoint is the home *public* IP and the router
does not hairpin, so an auto-started tunnel is **permanently broken at home** — which is where
the laptop lives most of the time. You'd get a live `wg0`, a dead `10.10.0.1`, and `ssh`
hanging for minutes on the one network where everything is actually fine.

Use the Makefile targets instead — explicit, and they fail fast with an explanation:

```bash
make tunnels      # at home  — straight over the LAN
make away         # off-LAN  — wg-quick up (+ reachability check) then all tunnels over it
make away-stop    # tear it all down
make wg-status    # endpoint / handshake / transfer
```

---

## Task 13: Update the docs

**Files:** Modify `CLAUDE.md` (mark the TODO `[x]`, record what was learned), `README.md`
(runbook entry, matching the existing chronological style).

Record in particular: whether Stage 1 needed the MTU workaround, whether Stage 2 passed on
first try, and the observed carrier-NAT answer. **Do not commit any config containing
keys or tokens** — `/etc/wireguard/` and `/etc/desec-updater.env` are root-only and live
outside the repo by design.

```bash
git add CLAUDE.md README.md && git commit -m "docs: remote model access via WireGuard — implemented and verified"
```

---

## Open questions (carry forward)

- **The DDNS updater is unproven until the IP actually moves.** Tasks 6/10 verify it *runs*;
  neither verifies it *recovers*. The honest test is a b-box reboot to force a new lease, then
  confirming the record follows within ~5 minutes and the tunnel re-establishes. Worth doing
  deliberately at home rather than discovering it abroad — that's the lockout scenario the
  whole DDNS apparatus exists to prevent.
- **`rag-server` on `:8081`** — one more `LocalForward` in `~/.ssh/config`. Free once this works.
- **IPv6 as fallback** — if a network ever gives you v4-only-*outbound* trouble, the design
  notes a one-line `Endpoint` change. Requires publishing the AAAA that Task 6 deliberately
  deletes, and a b-box firewall rule permitting inbound UDP to weebeastie's v6 address.
- **Sandboxing** (pre-existing, in CLAUDE.md TODOs): the eval gate still runs `qwen --yolo`
  unsandboxed. Remote access doesn't worsen this, but it does widen who can reach a box that
  auto-executes agent tool calls. A compromised travel laptop now has a path in.
