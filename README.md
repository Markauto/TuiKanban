# TuiKanban

A local-first kanban application written in Rust. `kanban` combines an interactive terminal board with a scriptable CLI. No server, account, or database setup required.

- Keyboard navigation, card editor, full-card viewer, live search, and responsive columns
- Descriptions, four priorities, tags, due dates, archiving, and deletion confirmation
- Custom columns with rename, reorder, and safe migration of cards
- Filtered/sorted listings, statistics, JSON output, import/export, and shell completions
- Atomic JSON saves and locks to protect concurrent CLI/TUI changes

## Install and start

Install a current stable [Rust toolchain](https://rustup.rs/) and a system C linker (`build-essential` on Debian/Ubuntu, Xcode command-line tools on macOS, or Visual Studio C++ build tools on Windows).

```sh
cargo install --path . --locked
kanban init "My project"
kanban add "Build something useful" --priority high --tags rust,feature
kanban
```

Or run from the checkout with `cargo run -- init "My project"`, then `cargo run`.

The default board lives at `$XDG_DATA_HOME/kanban/board.json`, falling back to
`~/.local/share/kanban/board.json` when `XDG_DATA_HOME` is unset, empty, or relative.
This location is shared regardless of which directory you run `kanban` from.
`kanban init` creates the directory and board; initialization never overwrites an
existing file.

Use `--file PATH` or `KANBAN_FILE` for another board. Precedence is `--file`, then
`KANBAN_FILE`, then the default data directory. Relative overrides resolve against
the current directory. Each file is an independent board.

Existing `.kanban.json` files are left untouched and are not automatically loaded
or migrated. Open one with `kanban --file .kanban.json`, or copy it into the new
default location with `kanban import .kanban.json` (refuses to replace an existing
board unless `--force` is supplied).

## Arch Linux package

Build and install from this checkout with `base-devel` and `rust` installed:

```sh
cd packaging/arch
makepkg -si
```

The local `PKGBUILD` snapshots the checkout, fetches the locked Cargo dependencies,
then builds and tests with network access disabled in Cargo. It installs `kanban`,
the MIT license, this README, and Bash/Zsh/Fish completions. Run makepkg from
`packaging/arch`, where its `src/` and `pkg/` build directories are separate from
the application's sources. This recipe uses local sources; it is not an AUR
release recipe. Supported package architectures: x86_64 and aarch64.

## Terminal controls

| Key | Action |
| --- | --- |
| Arrow keys / `h j k l` | Navigate columns and cards |
| `Home` / `End` | Select first / last card |
| `n` / `e` | Create / edit a card |
| `Enter` | View details; arrows scroll |
| `H` / `L` | Move selected card left / right |
| `p` | Cycle priority |
| `a` | Archive, or restore in archive view |
| `v` | Switch between active and archived cards |
| `d` | Delete after confirmation |
| `/` | Search ID, title, description, column, and tags |
| `Esc` | Clear filter / close dialog |
| `r` | Reload |
| `?` | Help (scroll with arrows) |
| `q` / `Ctrl+C` | Quit |

In text fields, Left/Right and Home/End move the cursor; Backspace/Delete remove characters. In the editor, use `Tab` / `Shift+Tab` to change fields, arrows or Space to change priority/column, `Enter` for a description newline, `Ctrl+S` to save, and `Esc` to cancel. Blank tags or due date clear that field. Columns page horizontally on smaller terminals. The TUI needs at least 30×10 cells; 100×30 or larger is recommended.

Changes save immediately. The TUI refreshes external changes automatically. If another session changes a card while its editor is open, saving reports a conflict; cancel and reopen to use the latest version.

## CLI examples

```sh
kanban init "Launch" --columns "Backlog,Doing,Review,Done"
kanban add "Ship v1" --column Backlog --description "Prepare release notes" \
  --priority urgent --tags release,rust --due 2026-12-01
kanban list --column Backlog --tag rust --sort priority
kanban list --search release --overdue
kanban list --sort due --json
kanban show 1
kanban edit 1 --title "Ship v1.0" --priority high --tags release --clear-due
kanban move 1 Doing
kanban archive 1
kanban list --archived
kanban restore 1
kanban delete 1 --yes

kanban column add Blocked
kanban column rename Backlog Todo
kanban column order Blocked 2
kanban column remove Blocked --move-to Todo
kanban stats --json

kanban export > backup.json
kanban --file restored.json import backup.json
kanban import backup.json --force
kanban --file personal.json init "Personal"
kanban --file personal.json
```

`--json` produces structured results for every data command. `export` always writes complete JSON to stdout. Errors go to stderr with a nonzero exit status. `list` defaults to active cards; `--archived` shows only archived cards and `--all` includes both. Priority sorts urgent first, and due-date sorting puts undated cards last. Overdue means before today's UTC date, independent of column name (archive completed work to omit it from active statistics).

Column names match without regard to case and must be unique. Removing a populated column requires `--move-to`, including when it contains archived cards. IDs are stable and never reused after deletion. Tags are comma-separated and deduplicated without regard to case. Card order in the TUI is creation order.

Run `kanban --help` or `kanban <command> --help` for all options. Generate shell completions with:

```sh
kanban completions bash > kanban.bash
source kanban.bash
# Also supports zsh, fish, powershell, and elvish.
```

## Storage and backups

The versioned JSON document contains the board name, ordered columns, next ID, and all active and archived cards. Import validates the complete document before changing the destination. Replacement requires `--force`. Export to a **different path** from your live board: shell redirection truncates its destination before the command runs.

Writes acquire a sibling `.lock` file, reload the latest board, validate it, and atomically replace it via a temporary file in the same directory. Lock files may remain on disk; locks are released by the operating system when the process exits. Keep the lock file in place while any session is running. Use a local filesystem with reliable file locking; distributed/network filesystems are not supported. Back up with `kanban export`; this app does not provide sync or automatic backup history.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
python3 scripts/pty_smoke.py target/release/kanban # Unix terminal smoke test
```

Tests cover CLI workflows, validation and failed-write preservation, JSON round trips, concurrent writers, TUI rendering at different sizes, keyboard actions, and editor conflicts. A Unix PTY smoke test exercises the real terminal event loop and verifies terminal restoration after `q` and `Ctrl+C`. CI runs formatting, Clippy, tests, and the PTY smoke test on Linux.

Built with [Ratatui](https://ratatui.rs/), Crossterm, Clap, and Serde. MIT licensed.
