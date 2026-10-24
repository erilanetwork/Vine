# Contributing to Vine

Thanks for wanting to help. Bug reports, fixes, new features and docs are all welcome.

Before starting on something big, open an issue first so we can agree on the approach. It saves you from writing a PR that ends up going a different way.

## Setting up

You need Rust 1.96 or newer, installed through [rustup](https://rustup.rs).

```sh
git clone https://github.com/erilanetwork/Vine.git
cd Vine
cargo build
cargo run
```

`cargo run` writes `vine.toml` and the other runtime files into the repo root. They are ignored by git, so you can tweak them freely while testing.

For testing against a real backend, a [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin) server with Vine forwarding works best. A vanilla server in offline mode works for anything that does not touch forwarding.

## Project layout

| Path | What lives there |
| --- | --- |
| `src/` | The proxy binary: backend connections and server switching, with client handling, the server list and the console in `src/utils` |
| `crates/vine-protocol` | Packet definitions and the wire format for every supported version |
| `crates/vine-network` | Socket setup, PROXY protocol and network config |
| `crates/vine-auth` | Mojang session authentication |
| `crates/vine-security` | Rate limiting and input sanitizing |
| `crates/vine-config` | `vine.toml` and permissions |
| `crates/vine-i18n` | Translations, with `en_us.json` as the source of truth |
| `crates/vine-moderation` | Bans and the whitelist, stored as JSON |
| `crates/vine-plugin` | Built in commands, health, queues, metrics and the WebAssembly plugin host |
| `crates/vine-plugin-sdk` | The Rust SDK plugins are written with |
| `examples/` | Example plugins, built for `wasm32-wasip2` |
| `tests/` | End to end tests that run the proxy against a fake backend |
| `wit/` | The plugin interface |

## Before opening a PR

CI runs all of these, so running them locally saves a round trip:

```sh
cargo fmt --all
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
typos
```

CI also runs [`cargo machete`](https://github.com/bnjbvr/cargo-machete) for unused dependencies and [`cargo deny check advisories`](https://github.com/EmbarkStudios/cargo-deny) for known vulnerabilities. Install them with `cargo install` if you touch dependencies.

## Guidelines

- Keep a PR focused on one thing. Unrelated cleanups go in their own PR.
- Match the style of the code around your change. `rustfmt` handles formatting, the rest is naming and structure.
- Write comments when they explain something the code cannot, like a protocol quirk or why a workaround exists. Skip comments that repeat the code.
- Add tests for logic that can be tested without a running client, following the existing `#[cfg(test)]` modules. Behavior that needs a connection belongs in `tests/proxy.rs`, which drives a real proxy with a fake client and backend.
- The plugin host test in `crates/vine-plugin/tests/sdk_plugin.rs` skips itself until the example plugin is built with `cargo build -p hello-plugin --target wasm32-wasip2 --release`.
- Avoid allocations in packet hot paths. Most of the proxy's time goes to moving bytes between client and backend.
- Never log secrets, forwarding keys or session tokens.
- Changes to `wit/` break existing plugins, so call them out clearly in the PR, and keep `vine-plugin-sdk` and the example plugin building.

### Translations

Every player facing message goes through `vine-i18n`. Add new keys to `crates/vine-i18n/src/en_us.json`; the files in `lang/` are written from it at start and are not tracked.

### Protocol versions

Packet ids and version tables live in `crates/vine-protocol`. When adding a new Minecraft version, check that server switching still works for the oldest and newest versions, since that is where things usually break.

## Commits

Use a short imperative title in sentence case, without a `type:` prefix or a trailing period, for example `Add a status command`. One purpose per commit.

## Releases

Each release has its notes in `changelogs/<version>.md`, for example `changelogs/0.1.0.md`. To cut one, bump the version in `Cargo.toml`, add its changelog and push a tag with the same version. The release workflow refuses a tag that does not match `Cargo.toml`, builds every platform and uses the changelog as the release body.

## Reporting bugs

Use the bug report template and include the Vine version, the client version, the backend software and the relevant part of the log. For security issues, follow [SECURITY.md](SECURITY.md) instead of opening a public issue.

## License

By contributing, you agree that your contributions are licensed under [GPL-3.0](LICENSE).
