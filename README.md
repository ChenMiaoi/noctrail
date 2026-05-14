# Noctrail

Noctrail is a Rust-native, GPU-oriented, agent-aware terminal project inspired by Hyprland-style workflows.

The project is at the repository bootstrap stage. The product plan lives in [docs/plan.md](docs/plan.md).

## Goals

- Build a reliable cross-platform terminal first.
- Add Hyprland-inspired workspaces, panes, themes, and keyboard-first UI.
- Add agent workflows behind explicit permission, redaction, and review boundaries.
- Keep AI features optional; the terminal must remain useful with agent features disabled.

## Repository Layout

```text
crates/
  noctrail-cli/   # `noctrail` command-line entry point
docs/
  plan.md         # product and engineering plan
```

More crates will be added only when there is real implementation ownership for the boundary.

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
