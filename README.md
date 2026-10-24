<div align="center">
  <h1><img src="https://github.com/Pumpkin-MC.png" alt="Pumpkin" width="112" align="absmiddle"> <img src=".github/vine-title.svg" alt="Vine" height="78" align="absmiddle"></h1>

  <p>A fast, multi-version Minecraft Java Edition proxy written in Rust, built on the <a href="https://github.com/Pumpkin-MC/Pumpkin">Pumpkin</a> crates.</p>

  <a href="https://github.com/erilanetwork/Vine/actions/workflows/rust.yml"><img src="https://github.com/erilanetwork/Vine/actions/workflows/rust.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/erilanetwork/Vine/releases"><img src="https://img.shields.io/github/v/release/erilanetwork/Vine?include_prereleases&sort=semver" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/erilanetwork/Vine" alt="License"></a>
</div>

## Features

- **Every version from 1.7 to 26.x** on one proxy, with server switching on all of them
- **Vine forwarding** for Pumpkin backends, signed with Ed25519 so a backend can trust the player data it receives
- **WebAssembly plugins** that run sandboxed, with memory and time limits, and reload without a restart
- **Forced hosts** to route players by the domain they joined with, plus per host MOTDs and icons
- **Health checks** that ping every backend and send new players around servers that are down
- **Queues** for backends with a player limit, with priority and bypass permissions
- **Bans, kicks, a whitelist and maintenance mode**, kept in plain JSON files next to the config
- **Live config reloads** with `/vine config reload`, no restart and nobody kicked
- **Prometheus metrics** for players, backends, queues and logins
- **Permissions** with groups, inheritance, wildcards and negated nodes, all in `vine.toml`
- **Built in protection** with per IP connection rate limits, packet flood limits and username validation
- **PROXY protocol** v1 and v2 for running behind HAProxy, Cloudflare Spectrum and similar
- **Per player translations** loaded from `lang/`
- **Interactive console** with tab completion and command hints

## Getting started

### Prebuilt binaries

Grab a binary for your platform from the [releases page](https://github.com/erilanetwork/Vine/releases). The `nightly` release is rebuilt from every push to `master`. Every release ships a `checksums.sha256` file.

| Platform | File |
| --- | --- |
| Linux x86_64 | `vine-X64-Linux`, or `vine-X64-Linux-musl` for a static build |
| Linux ARM64 | `vine-ARM64-Linux`, or `vine-ARM64-Linux-musl` for a static build |
| Windows | `vine-X64-Windows.exe`, `vine-ARM64-Windows.exe` |
| macOS (Apple Silicon) | `vine-ARM64-macOS` |

```sh
chmod +x vine-X64-Linux
./vine-X64-Linux
```

### Docker

```sh
docker run -it -p 25565:25565 -v vine-data:/vine ghcr.io/erilanetwork/vine:latest
```

Or with compose:

```sh
docker compose -f docker/docker-compose.yml up -d
```

The compose file builds the image from source and keeps its files in the `vine-data` volume. Prebuilt images for x86_64 and ARM64 are published to `ghcr.io/erilanetwork/vine` and Docker Hub as `xrookiefight/vine`, tagged `latest`, the release version and `master`. Swap the `build` section for the commented `image` line to use one. The console is attached, so `docker attach vine-vine-1` gives you the Vine prompt.

### From source

Vine needs Rust 1.96 or newer.

```sh
git clone https://github.com/erilanetwork/Vine.git
cd Vine
cargo build --release --locked
./target/release/vine
```

## First start

The first start writes the default files next to the config:

| File | What it is |
| --- | --- |
| `vine.toml` | The whole proxy config |
| `lang/en_us.json` | Default translations, edit or add more languages here |
| `vine-forwarding.key` | Forwarding signing key, only written when a backend uses Vine forwarding |

Run `vine --init` to write these files and exit without starting the proxy. It also prints the public key each Vine forwarded backend needs. A different config path can be passed as the last argument, for example `vine /etc/vine/vine.toml`.

`VINE_BIND_ADDRESS` overrides `server.bind_address`, which is handy for hosting panels that pick the port at start.

> [!IMPORTANT]
> Keep `vine-forwarding.key` private. Anyone holding it can log into your backends as any player.

## Configuration

Everything lives in `vine.toml`. The generated file is the reference; this is the short tour.

```toml
[server]
bind_address = "0.0.0.0:25565"
online_mode = true
compression_threshold = 256
default_locale = "en_us"

[motd]
text = "A Vine Server"
max_players = 100

[routing]
default_server = "lobby"
fallback_server = "survival"

[routing.forced_hosts]
"survival.example.com" = "survival"

[servers.lobby]
address = "127.0.0.1:25566"
forwarding = "vine"

[servers.vanilla]
address = "127.0.0.1:25568"
forwarding = "none"
```

| Section | Covers |
| --- | --- |
| `[server]` | Listen address, online mode, compression, shutdown timeout, forwarding key file, default language |
| `[network]` | TCP options, timeouts, backend retries, PROXY protocol and its trusted proxies |
| `[motd]` | Server list text, player count, version string, sample players and icon |
| `[security]` | Connection rate limits, packet flood and size limits, username validation |
| `[routing]` | Default and fallback servers, forced hosts |
| `[servers.<name>]` | Backend address, forwarding mode, an optional own secret, MOTD and icon |
| `[permissions]` | Groups, inheritance and per player assignments |
| `[plugins]` | Plugin folder, memory limit and call timeout |
| `[health]` | Backend health check interval, timeout and how many failures mark a server down |
| `[maintenance]` | Maintenance mode switch, its server list text and version label |
| `[whitelist]` | Whitelist switch; the players themselves live in `whitelist.json` |
| `[metrics]` | Prometheus endpoint switch and address |

`/vine config reload` applies most changes right away. The listen address, `[network]`, `[plugins]`, `[metrics]` and the connection rate limits need a restart, and the reload says so when one of them changed.

Setting `max_players` on a backend caps it at that many players. Anyone using `/server` to join a full backend waits in its queue and is moved over once a slot opens; `vine.queue.priority` puts a player ahead of the line and `vine.queue.bypass` skips it.

### Bans and whitelist

Bans live in `banned-players.json` and `banned-ips.json`, the whitelist in `whitelist.json`, all next to `vine.toml`. The commands below keep them up to date, but they are plain JSON and fine to edit by hand while the proxy is stopped. Durations look like `30m`, `12h`, `1d12h` or `2w`.

### Metrics

With `[metrics] enabled = true`, Vine serves Prometheus metrics at `http://127.0.0.1:9225/metrics`. The endpoint has no authentication, so keep it on a private address.

### Forwarding

Each backend picks a `forwarding` mode:

- `vine` sends the player's real IP, UUID and skin to a Pumpkin backend, signed with the key from `vine-forwarding.key` (or the backend's own `secret`). Put the public key printed by `vine --init` into `networking.proxy.vine.public_key` in Pumpkin's config.
- `none` connects without forwarding, for vanilla servers in offline mode. Make sure such a backend can only be reached through the proxy.

## Commands

| Command | Permission | Description |
| --- | --- | --- |
| `/server [name]` | `vine.command.server` | Show your current server, or switch to another one |
| `/send <player\|all> <server>` | `vine.command.send` | Move players to another server |
| `/list` | `vine.command.list` | List every player on the proxy |
| `/glist` | `vine.command.glist` | List players grouped by server |
| `/status` | `vine.command.status` | Show version, uptime and player counts |
| `/help [command]` | `vine.command.help` | Show command help |
| `/vine plugins` | `vine.command.plugins` | List loaded plugins |
| `/vine reload [plugin]` | `vine.command.plugins` | Reload all plugins or a single one |
| `/vine config reload` | `vine.command.reload` | Read `vine.toml` again |
| `/queue [leave]` | none | Show or give up your place in a queue |
| `/kick <player> [reason]` | `vine.command.kick` | Disconnect a player |
| `/ban <player> [duration] [reason]` | `vine.command.ban` | Ban a player, permanently without a duration |
| `/unban <player>` | `vine.command.ban` | Lift a ban |
| `/banip <ip\|player> [duration] [reason]` | `vine.command.banip` | Ban an address, or the one a player is on |
| `/unbanip <ip>` | `vine.command.banip` | Lift an IP ban |
| `/banlist` | `vine.command.ban` | List bans |
| `/maintenance <on\|off>` | `vine.command.maintenance` | Toggle maintenance mode, kicking players without `vine.maintenance.bypass` |
| `/whitelist <on\|off\|list\|add\|remove>` | `vine.command.whitelist` | Manage the whitelist; `vine.whitelist.bypass` gets past it |

The console has every permission and also accepts `stop` (or `end`) to shut the proxy down.

### Permissions

```toml
[permissions]
default_group = "default"

[permissions.groups.default]
permissions = ["vine.command.server", "vine.command.list"]

[permissions.groups.mod]
inherits = ["default"]
permissions = ["vine.command.*", "-vine.command.plugins"]

[permissions.users]
Alex = ["mod"]
Steve = { groups = ["mod"], permissions = ["vine.command.plugins"] }
```

Every player gets the default group. `*` matches everything, `vine.command.*` matches every node under it and a leading `-` takes a node away.

## Plugins

Plugins are WebAssembly components dropped into `plugins/` as `.wasm` files. The interface they build against is in [`crates/vine-plugin-wit/wit`](crates/vine-plugin-wit/wit). Plugins can listen to proxy events, register commands with tab completion, send messages, move players and use the translation system. Each one gets its own memory limit and call timeout, and a plugin that hangs is disabled instead of stalling the proxy.

`/vine reload` reloads them in place, so you can update a plugin without kicking anyone.

The easiest way to write one is in Rust with [`vine-plugin-sdk`](crates/vine-plugin-sdk). [`examples/hello-plugin`](examples/hello-plugin) is a small working plugin that greets players, adds a command and changes the server list. Build it with:

```sh
rustup target add wasm32-wasip2
cargo build -p hello-plugin --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/hello_plugin.wasm plugins/
```

## Contributing

Bug reports, ideas and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a PR, and see [SECURITY.md](SECURITY.md) for reporting vulnerabilities privately.

## License

Vine is licensed under GPL-3.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
