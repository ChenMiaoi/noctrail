# Noctrail

Noctrail is a Rust-native terminal project being rebooted around a
stable terminal-core-first roadmap.

The active plan lives in [docs/real-plan.md](docs/real-plan.md). The old
[docs/plan.md](docs/plan.md) file is retained only as a legacy pointer.

## Goals

- Build a reliable cross-platform terminal first.
- Keep workspace, layout, and visual polish ahead of agent features.
- Add agent workflows behind explicit permission, redaction, and review boundaries.
- Keep AI features optional; the terminal must remain useful with agent features disabled.

## Repository Layout

```text
crates/
  noctrail-cli/   # `noctrail` command-line entry point
docs/
  real-plan.md    # active restart roadmap
  plan.md         # legacy pointer to the new roadmap
```

More crates will be added only when there is clear implementation ownership for the boundary.

## Development

Install stable Rust, then run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

Run the development CLI:

```sh
cargo run -p noctrail-cli -- doctor
```

## License

Licensed under the GNU Lesser General Public License, version 3 or later
(`LGPL-3.0-or-later`). See [LICENSE](LICENSE).
