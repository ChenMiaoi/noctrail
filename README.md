# Noctrail

Noctrail is a Rust-native terminal project being rebooted around a
stable terminal-core-first roadmap.

The active plan lives in [docs/roadmap.md](docs/roadmap.md). The old
[docs/plan.md](docs/plan.md) and
[docs/real-plan.md](docs/real-plan.md) files are retained only as
legacy pointers.

## Goals

- Build a reliable cross-platform terminal first.
- Keep workspace, layout, and visual polish ahead of agent features.
- Add agent workflows behind explicit permission, redaction, and review boundaries.
- Keep AI features optional; the terminal must remain useful with agent features disabled.

## Repository Layout

```text
crates/
  noctrail-app/   # desktop app shell
  noctrail-cli/   # `noctrail` command-line entry point
  noctrail-config/ # configuration boundary
  noctrail-layout/ # workspace and pane layout boundary
  noctrail-pty/    # PTY/process boundary
  noctrail-render/ # render plan and backend boundary
  noctrail-runtime/ # pane runtime registry boundary
  noctrail-term/   # terminal state-machine boundary
docs/
  roadmap.md      # active engineering roadmap
  plan.md         # legacy pointer to the active roadmap
  real-plan.md    # legacy pointer to the active roadmap
```

The skeleton stops here until each boundary has an owner and a test
slice.

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

## Configuration

Noctrail reads an optional TOML config with `noctrail-app --config <path>`.
The schema reference lives in [docs/config-reference.md](docs/config-reference.md),
and a runnable example lives at
[examples/noctrail.example.toml](examples/noctrail.example.toml).
The current beta gate lives in [docs/release-blockers.md](docs/release-blockers.md).
The privacy and security model lives in
[docs/privacy-security.md](docs/privacy-security.md).

## License

Licensed under the GNU Lesser General Public License, version 3 or later
(`LGPL-3.0-or-later`). See [LICENSE](LICENSE).
