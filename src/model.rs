use anyhow::{bail, ensure, Context, Result};
use chrono::{NaiveDate, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    #[default]
    Medium,
    High,
    Urgent,
}
impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Low => "low",
                Self::Medium => "medium",
                Self::High => "high",
                Self::Urgent => "urgent",
            }
        )
    }
}
impl Priority {
    pub fn next(self) -> Self {
        match self {
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Urgent,
            Self::Urgent => Self::Low,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Card {
    pub id: u64,
    pub title: String,
    pub description: String,
    pub column: String,
    pub priority: Priority,
    pub tags: Vec<String>,
    pub due: Option<NaiveDate>,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}
impl Card {
    pub fn touch(&mut self) {
        self.updated_at = Utc::now().to_rfc3339();
    }
    pub fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        format!(
            "{} {} {} {} {}",
            self.id,
            self.title,
            self.description,
            self.column,
            self.tags.join(" ")
        )
        .to_lowercase()
        .contains(&query)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Board {
    pub version: u32,
    pub name: String,
    pub columns: Vec<String>,
    pub next_id: u64,
    pub cards: Vec<Card>,
}
impl Board {
    pub fn new(name: String, columns: Vec<String>) -> Result<Self> {
        let board = Self {
            version: 1,
            name,
            columns,
            next_id: 1,
            cards: vec![],
        };
        board.validate()?;
        Ok(board)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "Unsupported board version {}",
            self.version
        );
        valid_text(&self.name, "Board name")?;
        ensure!(
            !self.columns.is_empty(),
            "A board needs at least one column"
        );
        let mut columns = HashSet::new();
        for column in &self.columns {
            valid_text(column, "Column name")?;
            ensure!(
                columns.insert(column.to_lowercase()),
                "Duplicate column: {column}"
            );
        }
        let mut ids = HashSet::new();
        for card in &self.cards {
            ensure!(
                card.id > 0 && ids.insert(card.id),
                "Invalid or duplicate card ID {}",
                card.id
            );
            ensure!(
                self.columns.contains(&card.column),
                "Card #{} references missing column {}",
                card.id,
                card.column
            );
            valid_text(&card.title, "Title")?;
            for tag in &card.tags {
                valid_text(tag, "Tag")?;
            }
            chrono::DateTime::parse_from_rfc3339(&card.created_at)
                .context("Invalid creation timestamp")?;
            chrono::DateTime::parse_from_rfc3339(&card.updated_at)
                .context("Invalid update timestamp")?;
        }
        ensure!(
            self.next_id > self.cards.iter().map(|c| c.id).max().unwrap_or(0),
            "next_id must exceed all card IDs"
        );
        Ok(())
    }
    pub fn column(&self, name: &str) -> Result<String> {
        self.columns
            .iter()
            .find(|c| c.eq_ignore_ascii_case(name))
            .cloned()
            .with_context(|| {
                format!(
                    "Unknown column '{name}'. Available: {}",
                    self.columns.join(", ")
                )
            })
    }
    pub fn card(&self, id: u64) -> Result<&Card> {
        self.cards
            .iter()
            .find(|c| c.id == id)
            .with_context(|| format!("Card #{id} not found"))
    }
    pub fn card_mut(&mut self, id: u64) -> Result<&mut Card> {
        self.cards
            .iter_mut()
            .find(|c| c.id == id)
            .with_context(|| format!("Card #{id} not found"))
    }
    pub fn add(
        &mut self,
        title: String,
        column: &str,
        description: String,
        priority: Priority,
        tags: Vec<String>,
        due: Option<NaiveDate>,
    ) -> Result<u64> {
        valid_text(&title, "Title")?;
        let column = self.column(column)?;
        let id = self.next_id;
        self.next_id = id.checked_add(1).context("Card ID limit reached")?;
        let now = Utc::now().to_rfc3339();
        self.cards.push(Card {
            id,
            title: title.trim().into(),
            description,
            column,
            priority,
            tags,
            due,
            archived: false,
            created_at: now.clone(),
            updated_at: now,
        });
        Ok(id)
    }
    pub fn move_card(&mut self, id: u64, column: &str) -> Result<()> {
        let column = self.column(column)?;
        let card = self.card_mut(id)?;
        card.column = column;
        card.touch();
        Ok(())
    }
    pub fn remove_column(&mut self, name: &str, destination: Option<&str>) -> Result<()> {
        ensure!(self.columns.len() > 1, "Cannot remove the last column");
        let name = self.column(name)?;
        let destination = destination.map(|d| self.column(d)).transpose()?;
        ensure!(
            destination.as_ref() != Some(&name),
            "Destination must be a different column"
        );
        if self.cards.iter().any(|c| c.column == name) && destination.is_none() {
            bail!("Column contains cards (including archived). Supply --move-to COLUMN");
        }
        for card in self.cards.iter_mut().filter(|c| c.column == name) {
            card.column = destination.clone().unwrap();
            card.touch();
        }
        self.columns.retain(|c| c != &name);
        Ok(())
    }
}
pub fn valid_text(value: &str, label: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{label} cannot be empty");
    ensure!(
        !value.chars().any(char::is_control),
        "{label} cannot contain control characters"
    );
    Ok(())
}
pub fn tags(value: &str) -> Vec<String> {
    let mut result = vec![];
    for tag in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if !result.iter().any(|t: &String| t.eq_ignore_ascii_case(tag)) {
            result.push(tag.to_owned());
        }
    }
    result
}
pub fn date(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .context("Expected a valid date in YYYY-MM-DD format")
}
