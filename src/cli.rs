use crate::{
    model::{self, Board, Card, Priority},
    store::Store,
    tui,
};
use anyhow::{ensure, Context, Result};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use std::{
    io::{self, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    name = "kanban",
    version,
    about = "A local-first kanban board for your terminal",
    after_help = "Run without a subcommand to open the TUI. Start with: kanban init"
)]
pub struct Cli {
    /// Board file (default: $XDG_DATA_HOME/kanban/board.json or ~/.local/share/kanban/board.json)
    #[arg(short, long, global = true, env = "KANBAN_FILE")]
    pub file: Option<PathBuf>,
    /// Emit machine-readable JSON for command results
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}
#[derive(Subcommand)]
pub enum Command {
    /// Create a board without overwriting existing data
    Init {
        #[arg(default_value = "My board")]
        name: String,
        #[arg(long, value_delimiter = ',', default_value = "Todo,In Progress,Done")]
        columns: Vec<String>,
    },
    /// Open the interactive board (also the default command)
    Tui,
    /// Create a card
    Add {
        title: String,
        #[arg(short, long)]
        column: Option<String>,
        #[arg(short, long, default_value = "")]
        description: String,
        #[arg(short, long, value_enum, default_value = "medium")]
        priority: Priority,
        #[arg(short, long, default_value = "")]
        tags: String,
        #[arg(long, value_parser = model::date)]
        due: Option<chrono::NaiveDate>,
    },
    /// List cards with optional filters
    List(Filter),
    /// Show all details of one card
    Show { id: u64 },
    /// Update card fields; unspecified fields are preserved
    Edit(Edit),
    /// Move a card to another column
    Move { id: u64, column: String },
    /// Archive a card, keeping its history
    Archive { id: u64 },
    /// Restore an archived card
    Restore { id: u64 },
    /// Permanently delete a card (requires --yes)
    Delete {
        id: u64,
        #[arg(short, long)]
        yes: bool,
    },
    /// Manage board columns
    Column {
        #[command(subcommand)]
        command: ColumnCommand,
    },
    /// Show counts by column and priority
    Stats,
    /// Export the complete board as JSON to stdout
    Export,
    /// Import a validated JSON board; replacement requires --force
    Import {
        path: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Generate shell completion scripts
    Completions { shell: clap_complete::Shell },
}
#[derive(Args, Default)]
pub struct Filter {
    #[arg(short, long)]
    column: Option<String>,
    #[arg(short, long, value_enum)]
    priority: Option<Priority>,
    #[arg(short, long)]
    tag: Option<String>,
    /// Case-insensitive search across ID, title, description, column and tags
    #[arg(short, long)]
    search: Option<String>,
    /// Show only archived cards
    #[arg(long, conflicts_with = "all")]
    archived: bool,
    /// Include both active and archived cards
    #[arg(long)]
    all: bool,
    /// Cards whose due date is before today (UTC)
    #[arg(long)]
    overdue: bool,
    #[arg(long, value_enum, default_value = "id")]
    sort: Sort,
}
#[derive(Clone, Copy, Default, ValueEnum)]
enum Sort {
    #[default]
    Id,
    Priority,
    Due,
    Title,
}
#[derive(Args)]
pub struct Edit {
    id: u64,
    #[arg(long)]
    title: Option<String>,
    #[arg(short, long)]
    description: Option<String>,
    #[arg(short, long)]
    column: Option<String>,
    #[arg(short, long, value_enum)]
    priority: Option<Priority>,
    /// Replace tags with a comma-separated list; empty string clears tags
    #[arg(short, long)]
    tags: Option<String>,
    #[arg(long, value_parser = model::date, conflicts_with = "clear_due")]
    due: Option<chrono::NaiveDate>,
    #[arg(long)]
    clear_due: bool,
}
#[derive(Subcommand)]
pub enum ColumnCommand {
    /// Stack a column below its predecessor
    Stack {
        name: String,
    },
    /// Display a column in its own horizontal lane
    Unstack {
        name: String,
    },
    List,
    Add {
        name: String,
    },
    Rename {
        name: String,
        new_name: String,
    },
    /// Remove a column, optionally moving all its cards elsewhere
    Remove {
        name: String,
        #[arg(long)]
        move_to: Option<String>,
    },
    /// Set a column's position (1-based)
    Order {
        name: String,
        position: usize,
    },
}
fn emit(value: &impl serde::Serialize) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, value)?;
    writeln!(out)?;
    Ok(())
}
// Strip terminal control characters from human-facing data; JSON preserves originals.
fn plain(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
fn result_card(card: &Card, json: bool, action: &str) -> Result<()> {
    if json {
        emit(card)
    } else {
        println!(
            "{action} #{}: {} [{}]",
            card.id,
            plain(&card.title),
            plain(&card.column)
        );
        Ok(())
    }
}
fn default_board_path() -> Result<PathBuf> {
    // XDG paths must be absolute; empty or relative values are ignored.
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return Ok(data_home.join("kanban/board.json"));
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .context("Cannot determine the data directory; set HOME, XDG_DATA_HOME, KANBAN_FILE, or use --file")?;
    Ok(home.join(".local/share/kanban/board.json"))
}

pub fn run(cli: Cli) -> Result<()> {
    let command = cli.command.unwrap_or(Command::Tui);
    if let Command::Completions { shell } = &command {
        clap_complete::generate(*shell, &mut Cli::command(), "kanban", &mut io::stdout());
        return Ok(());
    }
    let path = match cli.file {
        Some(path) => path,
        None => default_board_path()?,
    };
    let store = Store::new(path);
    match command {
        Command::Completions { .. } => unreachable!("handled before resolving the board path"),
        Command::Tui => {
            ensure!(
                !cli.json,
                "--json is for CLI commands, not the interactive TUI"
            );
            tui::run(&store)?;
        }
        Command::Init { name, columns } => {
            let board = Board::new(
                name.trim().into(),
                columns.into_iter().map(|c| c.trim().into()).collect(),
            )?;
            store.init(board.clone())?;
            if cli.json {
                emit(&board)?;
            } else {
                println!("Created {} at {}", board.name, store.path.display());
            }
        }
        Command::Add {
            title,
            column,
            description,
            priority,
            tags,
            due,
        } => {
            let card = store.update(|b| {
                let col = column.unwrap_or_else(|| b.columns[0].clone());
                let id = b.add(title, &col, description, priority, model::tags(&tags), due)?;
                Ok(b.card(id)?.clone())
            })?;
            result_card(&card, cli.json, "Added")?;
        }
        Command::List(filter) => {
            let board = store.read()?;
            let column = filter
                .column
                .as_deref()
                .map(|c| board.column(c))
                .transpose()?;
            let today = chrono::Utc::now().date_naive();
            let mut cards: Vec<_> = board
                .cards
                .iter()
                .filter(|c| {
                    (filter.all || c.archived == filter.archived)
                        && column.as_ref().is_none_or(|v| v == &c.column)
                        && filter.priority.is_none_or(|p| p == c.priority)
                        && filter
                            .tag
                            .as_ref()
                            .is_none_or(|t| c.tags.iter().any(|v| v.eq_ignore_ascii_case(t)))
                        && filter.search.as_ref().is_none_or(|q| c.matches(q))
                        && (!filter.overdue || c.due.is_some_and(|d| d < today))
                })
                .collect();
            match filter.sort {
                Sort::Id => cards.sort_by_key(|c| c.id),
                Sort::Priority => {
                    cards.sort_by_key(|c| (std::cmp::Reverse(c.priority as u8), c.id))
                }
                Sort::Due => cards.sort_by_key(|c| (c.due.is_none(), c.due, c.id)),
                Sort::Title => cards.sort_by_key(|c| c.title.to_lowercase()),
            }
            if cli.json {
                emit(&cards)?;
            } else if cards.is_empty() {
                println!("No matching cards.");
            } else {
                println!(
                    "{:<6} {:<18} {:<8} {:<10} TITLE / TAGS",
                    "ID", "COLUMN", "PRIORITY", "DUE"
                );
                for c in cards {
                    println!(
                        "{:<6} {:<18} {:<8} {:<10} {}{}{}",
                        c.id,
                        plain(&c.column),
                        c.priority.to_string(),
                        c.due.map(|d| d.to_string()).unwrap_or_else(|| "-".into()),
                        plain(&c.title),
                        if c.archived { " [archived]" } else { "" },
                        if c.tags.is_empty() {
                            String::new()
                        } else {
                            format!("  #{}", c.tags.join(" #"))
                        }
                    );
                }
            }
        }
        Command::Show { id } => {
            let board = store.read()?;
            let c = board.card(id)?;
            if cli.json {
                emit(c)?;
            } else {
                println!("#{} {}\nColumn: {}{}\nPriority: {}\nTags: {}\nDue: {}\nCreated: {}\nUpdated: {}\n\n{}", c.id, plain(&c.title), plain(&c.column), if c.archived { " (archived)" } else { "" }, c.priority, c.tags.join(", "), c.due.map(|d| d.to_string()).unwrap_or_else(|| "-".into()), c.created_at, c.updated_at, c.description.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect::<String>());
            }
        }
        Command::Edit(edit) => {
            let card = store.update(|b| {
                let column = edit.column.as_deref().map(|c| b.column(c)).transpose()?;
                let c = b.card_mut(edit.id)?;
                if let Some(title) = edit.title {
                    c.title = title.trim().into();
                }
                if let Some(description) = edit.description {
                    c.description = description;
                }
                if let Some(column) = column {
                    c.column = column;
                }
                if let Some(priority) = edit.priority {
                    c.priority = priority;
                }
                if let Some(tags) = edit.tags {
                    c.tags = model::tags(&tags);
                }
                if edit.clear_due {
                    c.due = None;
                } else if let Some(due) = edit.due {
                    c.due = Some(due);
                }
                c.touch();
                Ok(c.clone())
            })?;
            result_card(&card, cli.json, "Updated")?;
        }
        Command::Move { id, column } => {
            let card = store.update(|b| {
                b.move_card(id, &column)?;
                Ok(b.card(id)?.clone())
            })?;
            result_card(&card, cli.json, "Moved")?;
        }
        command @ (Command::Archive { .. } | Command::Restore { .. }) => {
            let archived = matches!(command, Command::Archive { .. });
            let id = match command {
                Command::Archive { id } | Command::Restore { id } => id,
                _ => unreachable!(),
            };
            let card = store.update(|b| {
                let c = b.card_mut(id)?;
                c.archived = archived;
                c.touch();
                Ok(c.clone())
            })?;
            result_card(
                &card,
                cli.json,
                if archived { "Archived" } else { "Restored" },
            )?;
        }
        Command::Delete { id, yes } => {
            ensure!(
                yes,
                "Permanent deletion requires --yes; use `archive` to keep the card"
            );
            let card = store.update(|b| {
                let c = b.card(id)?.clone();
                b.cards.retain(|c| c.id != id);
                Ok(c)
            })?;
            result_card(&card, cli.json, "Deleted")?;
        }
        Command::Column { command } => {
            if matches!(command, ColumnCommand::List) {
                let board = store.read()?;
                if cli.json {
                    emit(&board.columns)?;
                } else {
                    for (i, c) in board.columns.iter().enumerate() {
                        println!("{}. {}", i + 1, c);
                    }
                }
            } else {
                let columns = store.update(|b| {
                    match command {
                        ColumnCommand::Add { name } => b.add_column(&name)?,
                        ColumnCommand::Rename { name, new_name } => {
                            b.rename_column(&name, &new_name)?
                        }
                        ColumnCommand::Remove { name, move_to } => {
                            b.remove_column(&name, move_to.as_deref())?
                        }
                        ColumnCommand::Order { name, position } => {
                            b.order_column(&name, position)?
                        }
                        ColumnCommand::Stack { name } => b.stack_column(&name, true)?,
                        ColumnCommand::Unstack { name } => b.stack_column(&name, false)?,
                        ColumnCommand::List => unreachable!(),
                    }
                    Ok(b.columns.clone())
                })?;
                if cli.json {
                    emit(&columns)?;
                } else {
                    println!("Columns: {}", columns.join(" → "));
                }
            }
        }
        Command::Stats => {
            let b = store.read()?;
            let active: Vec<_> = b.cards.iter().filter(|c| !c.archived).collect();
            let columns: serde_json::Map<_, _> = b
                .columns
                .iter()
                .map(|col| {
                    (
                        col.clone(),
                        serde_json::json!(active.iter().filter(|c| &c.column == col).count()),
                    )
                })
                .collect();
            let priorities: serde_json::Map<_, _> = [
                Priority::Low,
                Priority::Medium,
                Priority::High,
                Priority::Urgent,
            ]
            .iter()
            .map(|p| {
                (
                    p.to_string(),
                    serde_json::json!(active.iter().filter(|c| c.priority == *p).count()),
                )
            })
            .collect();
            let overdue = active
                .iter()
                .filter(|c| c.due.is_some_and(|d| d < chrono::Utc::now().date_naive()))
                .count();
            if cli.json {
                emit(
                    &serde_json::json!({"name": b.name, "active": active.len(), "archived": b.cards.len() - active.len(), "overdue": overdue, "columns": columns, "priorities": priorities}),
                )?;
            } else {
                println!(
                    "{}: {} active · {} archived · {} overdue",
                    b.name,
                    active.len(),
                    b.cards.len() - active.len(),
                    overdue
                );
                for col in &b.columns {
                    println!("  {col}: {}", columns[col]);
                }
                println!(
                    "Priorities: {}",
                    priorities
                        .iter()
                        .map(|(k, v)| format!("{k} {v}"))
                        .collect::<Vec<_>>()
                        .join(" · ")
                );
            }
        }
        Command::Export => emit(&store.read()?)?,
        Command::Import { path, force } => {
            let data =
                std::fs::read(&path).with_context(|| format!("Cannot read {}", path.display()))?;
            let board: Board = serde_json::from_slice(&data).context("Invalid import JSON")?;
            store.replace(board.clone(), force)?;
            if cli.json {
                emit(&board)?;
            } else {
                println!(
                    "Imported {} cards into {}",
                    board.cards.len(),
                    store.path.display()
                );
            }
        }
    }
    Ok(())
}
