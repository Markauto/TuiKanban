use crate::model::Board;
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub struct Store {
    pub path: PathBuf,
}
impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub(crate) fn parent(&self) -> &Path {
        self.path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    }
    pub(crate) fn lock(&self) -> Result<File> {
        fs::create_dir_all(self.parent()).context("Cannot create board directory")?;
        let mut name = self.path.as_os_str().to_owned();
        name.push(".lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(PathBuf::from(name))
            .context("Cannot open board lock")?;
        file.lock_exclusive().context("Cannot lock board")?;
        Ok(file)
    }
    pub fn read(&self) -> Result<Board> {
        let data = fs::read(&self.path).with_context(|| {
            format!(
                "Cannot read {}. Create a board with `kanban init`",
                self.path.display()
            )
        })?;
        let board: Board = serde_json::from_slice(&data)
            .context("Invalid board JSON; the file was not changed")?;
        board.validate()?;
        Ok(board)
    }
    fn save(&self, board: &Board) -> Result<()> {
        board.validate()?;
        let mut temp = tempfile::NamedTempFile::new_in(self.parent())?;
        serde_json::to_writer_pretty(&mut temp, board)?;
        writeln!(temp)?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path)
            .context("Cannot atomically save board")?;
        #[cfg(unix)]
        File::open(self.parent())?.sync_all()?;
        Ok(())
    }
    pub fn init(&self, board: Board) -> Result<()> {
        let _lock = self.lock()?;
        ensure!(
            !self.path.exists(),
            "Board already exists at {}",
            self.path.display()
        );
        self.save(&board)
    }
    pub fn update<T>(&self, change: impl FnOnce(&mut Board) -> Result<T>) -> Result<T> {
        let _lock = self.lock()?;
        let mut board = self.read()?;
        let value = change(&mut board)?;
        self.save(&board)?;
        Ok(value)
    }
    pub fn replace(&self, board: Board, force: bool) -> Result<()> {
        let _lock = self.lock()?;
        ensure!(
            force || !self.path.exists(),
            "Board exists; use --force to replace it"
        );
        self.save(&board)
    }
}
