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
docker compose -f docker/docker-compose.yml up -d
```

The compose file builds the image from source and keeps its files in the `vine-data` volume. Prebuilt images are published to `ghcr.io/erilanetwork/vine`; swap the `build` section for the commented `image` line to use one. The console is attached, so `docker attach vine-vine-1` gives you the Vine prompt.

### Pterodactyl

Import [`egg-vine.json`](egg-vine.json) as a new egg. The install script downloads the musl build for the node's architecture, checks it against the release checksums and writes the default config.

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

Plugins are WebAssembly components dropped into `plugins/` as `.wasm` files. The interface they build against is in [`wit/`](wit). Plugins can listen to proxy events, register commands with tab completion, send messages, move players and use the translation system. Each one gets its own memory limit and call timeout, and a plugin that hangs is disabled instead of stalling the proxy.

`/vine reload` reloads them in place, so you can update a plugin without kicking anyone.

## Contributing

Bug reports, ideas and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a PR, and see [SECURITY.md](SECURITY.md) for reporting vulnerabilities privately.

## License

Vine is licensed under GPL-3.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
