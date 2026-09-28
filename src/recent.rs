use crate::{cli::default_board_path, store::Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RecentBoard {
    pub name: String,
    pub path: PathBuf,
}

pub fn history_path() -> Result<PathBuf> {
    Ok(default_board_path()?.with_file_name("recent-boards.json"))
}

pub fn read(path: &std::path::Path) -> Result<Vec<RecentBoard>> {
    match fs::read(path) {
        Ok(data) => serde_json::from_slice(&data).context("Invalid recent-board history"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).context("Cannot read recent-board history"),
    }
}

pub fn remember_at(path: PathBuf, store: &Store) -> Result<()> {
    let board = store.read()?;
    let canonical = fs::canonicalize(&store.path).context("Cannot resolve board path")?;
    let history = Store::new(path);
    let _lock = history.lock()?;
    let mut entries = read(&history.path)?;
    entries.retain(|entry| entry.path != canonical);
    entries.insert(
        0,
        RecentBoard {
            name: board.name,
            path: canonical,
        },
    );
    entries.truncate(20);
    let mut temp = tempfile::NamedTempFile::new_in(history.parent())?;
    serde_json::to_writer_pretty(&mut temp, &entries)?;
    writeln!(temp)?;
    temp.as_file().sync_all()?;
    temp.persist(&history.path)
        .context("Cannot save recent-board history")?;
    #[cfg(unix)]
    fs::File::open(history.parent())?.sync_all()?;
    Ok(())
}

// History is optional metadata: a failure must not turn a saved mutation into an error.
pub fn remember(store: &Store) -> Result<()> {
    remember_at(history_path()?, store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Board;

    #[test]
    fn history_deduplicates_orders_and_preserves_boards() {
        let dir = tempfile::tempdir().unwrap();
        let history = dir.path().join("recent.json");
        for i in 0..22 {
            let store = Store::new(dir.path().join(format!("{i}.json")));
            store
                .init(Board::new(format!("Board {i}"), vec!["Todo".into()]).unwrap())
                .unwrap();
            remember_at(history.clone(), &store).unwrap();
        }
        let store = Store::new(dir.path().join("2.json"));
        let before = fs::read(&store.path).unwrap();
        remember_at(history.clone(), &store).unwrap();
        let entries = read(&history).unwrap();
        assert_eq!(entries.len(), 20);
        assert_eq!(entries[0].name, "Board 2");
        assert_eq!(entries[1].name, "Board 21");
        assert!(entries.iter().all(|e| e.path.is_absolute()));
        assert_eq!(fs::read(&store.path).unwrap(), before);
        fs::write(&history, b"broken").unwrap();
        assert!(remember_at(history.clone(), &store).is_err());
        assert_eq!(fs::read(history).unwrap(), b"broken");
        assert_eq!(fs::read(&store.path).unwrap(), before);
    }
}
