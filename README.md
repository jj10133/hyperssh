# hyperssh

SSH to any machine without an IP address, port forwarding, or VPN.

hyperssh tunnels SSH over [HyperDHT](https://github.com/holepunchto/hyperdht), the
peer-to-peer network behind Hyperswarm. You dial a machine by its public key, the
DHT helps the two sides find each other and punch through NAT, and your normal
`ssh` runs over the resulting encrypted connection. No servers of your own, no
accounts.

It is a [pigeons](https://github.com/n0-computer/pigeons) alternative built on
[peeroxide](https://github.com/Rightbracket/peeroxide) (a pure-Rust Hyperswarm
implementation) instead of iroh.

```
 ssh ──ProxyCommand──▶ hyperssh connect ═══ HyperDHT / UDX ═══▶ hyperssh serve ──TCP──▶ sshd :22
       (your laptop)                                                (your server)
```

> **Status:** early. The tunnel works end to end on a single machine. Testing
> across two different networks (real NAT traversal) is still to do.

## Install

Requires Rust 1.85 or newer (the project uses the 2024 edition).

```bash
git clone <this repo>
cd hyperssh
cargo install --path .
```

This installs `hyperssh` to `~/.cargo/bin`. Install before running `hyperssh add`,
because the generated ssh config entry stores the absolute path of the binary that
created it.

## Quick start

**On the machine you want to SSH into** (make sure sshd is running):

```bash
hyperssh serve
# hyperssh is serving. id: a446bea9a37db3787a8c0f7fc4c3272255d95329dd2da54bfb095beed569c8ac
```

Copy the 64-character id. It is persistent, so it stays the same across restarts.

**On the machine you're connecting from:**

```bash
hyperssh add --id a446bea9a37db3787a8c0f7fc4c3272255d95329dd2da54bfb095beed569c8ac --name my-server
ssh youruser@my-server
```

That's it. Everything `ssh` normally does (keys, agent, `scp`, `rsync`, port
forwards) works over the tunnel.

## Commands

| Command | What it does |
|---|---|
| `hyperssh serve` | Accept tunnels and forward them to the local sshd. |
| `hyperssh connect --stdio <ID>` | Connect to a server and pipe the tunnel over stdin/stdout. This is what ssh runs as its `ProxyCommand`. |
| `hyperssh add --id <ID> --name <NAME>` | Add a `Host` entry to `~/.ssh/config`. |
| `hyperssh list` | List configured routes. |
| `hyperssh remove <NAME>` | Remove a route. |
| `hyperssh id` | Print this machine's server id (creates the key if needed). |

The older names `roost` (for `serve`) and `fly` (for `connect`) still work as hidden
aliases.

### `serve` options

| Option | Default | Description |
|---|---|---|
| `--ssh-port <PORT>` | `22` | Port your sshd listens on. |
| `--ephemeral` | off | Use a throwaway identity. A new id is generated on every start and nothing is saved. |
| `--identity <FILE>` | platform config dir | Use a specific identity file. |

### `connect` options

| Option | Default | Description |
|---|---|---|
| `--stdio` | required | Pipe over stdin/stdout (ProxyCommand mode). A local-port mode is not implemented yet. |
| `--identity <FILE>` | ephemeral | Optional persistent client identity. By default each connection uses a fresh key. |

### Without touching your ssh config

```bash
ssh -o ProxyCommand="hyperssh connect --stdio <ID>" youruser@anything
```

## How it works

- **Identity.** Each server has an Ed25519 key pair. The public key (hex) is its
  id. The private seed is stored with `0600` permissions at:
  - Linux: `~/.config/hyperssh/roost.key`
  - macOS: `~/Library/Application Support/hyperssh/roost.key`
- **Discovery.** The server announces itself on a DHT topic derived from its
  public key. A client computes the same topic from the id it was given and looks
  for peers there.
- **Authentication of the server.** Anyone can announce on a topic, so the client
  only accepts a connection whose Noise-authenticated remote public key equals the
  id it asked for.
- **Transport.** The connection is Noise-encrypted over UDX (reliable UDP with
  congestion control), with hole punching handled by HyperDHT.
- **Forwarding.** The server connects each incoming tunnel to `127.0.0.1:<ssh-port>`
  and copies bytes both ways. SSH itself is unmodified and runs end to end inside
  the tunnel.

## Security model

- **Anyone with the server id can reach your sshd's port.** The id is not a
  secret in the way a password is, but treat it like an unlisted address: share it
  only with people you'd allow to attempt a login.
- **SSH authentication still applies.** Keys, passwords, and certificates are
  checked by sshd as usual. Use key-only auth and keep sshd hardened as you would
  for any internet-facing machine.
- **Keep `roost.key` private.** Anyone who copies it can impersonate your server.
- **No client allowlist yet.** Restricting which client keys may connect is
  planned. For now, clients use a fresh ephemeral key by default.

## Limitations

- Connections take a few seconds to set up, because every `ssh` invocation starts
  a fresh DHT node and bootstraps. A long-lived local daemon could remove this.
- HyperDHT hole punching works on most home networks, but there is no
  iroh-style relay fallback. Two peers that are both behind hard (symmetric)
  NATs may fail to connect.
- Requires internet access to reach the public HyperDHT bootstrap nodes, even when
  both machines are on the same network.
- `connect` only supports `--stdio` mode.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| `Timed out looking for server` | The server isn't running yet or hasn't announced. Wait about 10 seconds after `serve` starts and retry. Check that the id matches. |
| Server logs `Could not reach sshd at 127.0.0.1:22` | sshd isn't running, or listens on another port. Use `--ssh-port`. On macOS, enable System Settings > General > Sharing > Remote Login. |
| Nothing happens on macOS | Allow incoming connections for `hyperssh` when the firewall prompt appears. The DHT uses UDP. |
| Anything else | Run both sides with `RUST_LOG=peeroxide=debug,hyperssh=debug` and read the stderr. |

Logs go to stderr so that stdout carries only tunnel data in `--stdio` mode.

## Testing on one machine

Run these in three terminals from the project directory:

```bash
# 1. a stand-in for sshd (restart it before each test, nc accepts one connection)
nc -l 2222

# 2. the server, forwarding to that port
cargo run -- serve --ephemeral --ssh-port 2222

# 3. the client (use the id printed by step 2)
cargo run -- connect --stdio <ID>
```

Lines typed in terminal 3 should appear in terminal 1, and vice versa. For a real
ssh test, enable sshd locally, run `cargo run -- serve --ephemeral`, and use the
`ProxyCommand` form above against `localhost`.

## Roadmap

- Client key allowlist on the server
- `connect` in local-port mode (non-ssh clients)
- Long-lived local daemon for instant connects
- Service install (systemd, launchd)

## Credits

- [pigeons](https://github.com/n0-computer/pigeons) by n0, the project this mirrors
- [peeroxide](https://github.com/Rightbracket/peeroxide), the Rust Hyperswarm stack
- [HyperDHT / Hyperswarm](https://github.com/holepunchto/hyperdht) by Holepunch
