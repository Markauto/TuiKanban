# Repository Guidelines

## Project Structure & Module Organization

TuiKanban is a Rust 2021 application with a single binary, `kanban`.
- `src/main.rs`: entry point and error reporting.
- `src/cli.rs`: Clap commands, output formatting, and board-path resolution.
- `src/model.rs`: board/card types and validation shared by both interfaces.
- `src/store.rs`: file locking and atomic JSON persistence.
- `src/tui.rs`: Ratatui/Crossterm rendering, keyboard handling, and unit tests.
- `tests/cli.rs`: CLI integration tests; `scripts/pty_smoke.py`: Unix terminal smoke test.
- `packaging/arch/PKGBUILD`: local-checkout Arch package; `.github/workflows/ci.yml`: Linux checks.

## Build, Test, and Development Commands

Use stable Rust and a system C linker.
- `cargo build --locked`: build the debug binary.
- `cargo build --release --locked`: build the optimized binary.
- `cargo run -- --file /tmp/kanban-dev.json init "Development"`: create a separate development board.
- `cargo run -- --file /tmp/kanban-dev.json`: open that board interactively.
- `cargo fmt --check`: verify formatting; `cargo fmt` applies it.
- `cargo clippy --locked --all-targets -- -D warnings`: lint all targets.
- `cargo test --locked`: run unit and integration tests.
- `python3 scripts/pty_smoke.py`: test the real terminal after a debug build.
- From `packaging/arch/`, run `makepkg -si` to build and install locally.

## Coding Style & Naming Conventions

Follow rustfmt defaults: four-space indentation, `snake_case` functions/modules, `PascalCase` types, and `SCREAMING_SNAKE_CASE` constants. Use `anyhow::Result` with actionable error context. Keep validation in the model and persistence in the store so CLI and TUI behavior stays consistent. Preserve atomic writes, lock handling, and terminal cleanup.

## Testing Guidelines

Use Rust `#[test]`, `assert_cmd`, `predicates`, and temporary directories. Name tests after observable behavior, such as `failed_mutations_do_not_change_data`. Add regression coverage for changed behavior; no numeric coverage threshold is configured. Isolate environment overrides in child processes. For TUI changes, exercise rendering, keyboard behavior, and terminal restoration. Run the CI commands above before submitting.

## Commit & Pull Request Guidelines

There are no commits yet, so no historical commit convention exists. Use concise, imperative subjects describing one coherent change. PRs should explain the problem, resulting behavior, validation performed, and related issues. Include a terminal capture for visible TUI changes and update README examples when commands or defaults change.

## Configuration & Data Safety

Path precedence is `--file`, `KANBAN_FILE`, then `$XDG_DATA_HOME/kanban/board.json`, falling back to `~/.local/share/kanban/board.json`. Use explicit temporary paths during development. Preserve existing boards; never redirect exports onto the live board file.
