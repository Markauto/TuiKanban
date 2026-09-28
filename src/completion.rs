//! Completion uses Clap's command definitions so hints track the CLI.
use crate::{cli::Cli, model::Board};
use clap::CommandFactory;
use std::ops::Range;

pub struct Suggestion {
    pub value: String,
    pub description: String,
}
pub struct Completion {
    pub suggestions: Vec<Suggestion>,
    pub hint: String,
    range: Range<usize>,
    option_prefix: String,
}
impl Completion {
    pub fn apply(&self, selected: usize, text: &mut String, cursor: &mut usize) {
        if let Some(item) = self.suggestions.get(selected) {
            let value = shlex::try_quote(&item.value).expect("completion values contain no NUL");
            let mut replacement = format!("{}{value}", self.option_prefix);
            let whitespace = text[self.range.end..]
                .chars()
                .next()
                .filter(|c| c.is_whitespace());
            if whitespace.is_none() {
                replacement.push(' ');
            }
            *cursor = self.range.start + replacement.len() + whitespace.map_or(0, char::len_utf8);
            text.replace_range(self.range.clone(), &replacement);
        }
    }
}

struct Token {
    range: Range<usize>,
    value: String,
}
// Keep byte ranges for cursor replacement; tolerate unfinished quotes while typing.
fn tokens(text: &str) -> Vec<Token> {
    let mut result = Vec::new();
    let mut start = None;
    let mut value = String::new();
    let mut quote = None;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if escaped {
            value.push(c);
            escaped = false;
        } else if c == '\\'
            && (quote.is_none()
                || (quote == Some('"')
                    && text[i + 1..]
                        .chars()
                        .next()
                        .is_none_or(|next| matches!(next, '$' | '`' | '"' | '\\' | '\n'))))
        {
            start.get_or_insert(i);
            escaped = true;
        } else if Some(c) == quote {
            quote = None;
        } else if quote.is_none() && matches!(c, '\'' | '"') {
            start.get_or_insert(i);
            quote = Some(c);
        } else if quote.is_none() && c.is_whitespace() {
            if let Some(begin) = start.take() {
                result.push(Token {
                    range: begin..i,
                    value: std::mem::take(&mut value),
                });
            }
        } else {
            start.get_or_insert(i);
            value.push(c);
        }
    }
    if let Some(begin) = start {
        result.push(Token {
            range: begin..text.len(),
            value,
        });
    }
    result
}

fn option<'a>(command: &'a clap::Command, word: &str) -> Option<&'a clap::Arg> {
    command.get_arguments().find(|arg| {
        arg.get_long()
            .is_some_and(|long| word == format!("--{long}"))
            || arg
                .get_short()
                .is_some_and(|short| word == format!("-{short}"))
    })
}

fn parsed_option<'a, 'b>(
    command: &'a clap::Command,
    word: &'b str,
) -> Option<(&'a clap::Arg, Option<&'b str>)> {
    if let Some((flag, value)) = word.split_once('=') {
        return option(command, flag).map(|arg| (arg, Some(value)));
    }
    if word.starts_with('-')
        && !word.starts_with("--")
        && word.len() > 2
        && word.is_char_boundary(2)
    {
        if let Some(arg) = option(command, &word[..2]).filter(|arg| arg.get_action().takes_values())
        {
            return Some((arg, Some(&word[2..])));
        }
    }
    option(command, word).map(|arg| (arg, None))
}

pub fn complete(text: &str, cursor: usize, board: &Board) -> Completion {
    let all = tokens(text);
    let current = all
        .iter()
        .find(|token| token.range.start <= cursor && cursor <= token.range.end);
    let range = current.map_or(cursor..cursor, |token| token.range.clone());
    let prefix = tokens(&text[range.start..cursor])
        .pop()
        .map_or(String::new(), |t| t.value);
    let mut root = Cli::command();
    root.build();
    let mut command = &root;
    let mut path = vec!["kanban"];
    let mut pending = None;
    let mut position = 1;
    let mut used = Vec::new();
    let mut end_options = false;
    for token in all.iter().take_while(|t| t.range.end <= range.start) {
        let word = token.value.as_str();
        if pending.take().is_some() {
            continue;
        }
        if word == "--" {
            end_options = true;
            continue;
        }
        if !end_options {
            if let Some((arg, value)) = parsed_option(command, word) {
                used.push(arg.get_id().as_str());
                if arg.get_action().takes_values() && value.is_none() {
                    pending = Some(arg);
                }
                continue;
            }
            if let Some(sub) = command.find_subcommand(word) {
                command = sub;
                path.push(sub.get_name());
                position = 1;
                continue;
            }
        }
        position += 1;
    }
    let mut option_prefix = String::new();
    let mut prefix = prefix;
    if !end_options && pending.is_none() {
        if let Some((arg, Some(value))) = parsed_option(command, &prefix) {
            if arg.get_action().takes_values() {
                pending = Some(arg);
                option_prefix = prefix[..prefix.len() - value.len()].to_string();
                prefix = value.to_string();
            }
        }
    }
    let positional = command
        .get_positionals()
        .find(|arg| arg.get_index() == Some(position));
    let argument = pending.or(positional);
    let mut suggestions = Vec::new();
    let mut add = |value: String, description: String| {
        if !value.contains('\0') && value.to_lowercase().starts_with(&prefix.to_lowercase()) {
            suggestions.push(Suggestion { value, description });
        }
    };
    if pending.is_none() && !end_options {
        for sub in command
            .get_subcommands()
            .filter(|s| !s.is_hide_set() && s.get_name() != "tui")
        {
            add(
                sub.get_name().into(),
                sub.get_about().map(ToString::to_string).unwrap_or_default(),
            );
        }
        if path.len() == 1 {
            add("quit".into(), "Quit the app".into());
        }
        for arg in command.get_arguments().filter(|arg| {
            !arg.is_hide_set()
                && arg.get_id() != "file"
                && !used.contains(&arg.get_id().as_str())
                && !command
                    .get_arg_conflicts_with(arg)
                    .iter()
                    .any(|conflict| used.contains(&conflict.get_id().as_str()))
        }) {
            if let Some(long) = arg.get_long() {
                add(
                    format!("--{long}"),
                    arg.get_help().map(ToString::to_string).unwrap_or_default(),
                );
            }
            if prefix.starts_with('-') && !prefix.starts_with("--") {
                if let Some(short) = arg.get_short() {
                    add(
                        format!("-{short}"),
                        arg.get_help().map(ToString::to_string).unwrap_or_default(),
                    );
                }
            }
        }
    }
    if let Some(arg) = argument.filter(|_| pending.is_some() || !prefix.starts_with('-')) {
        if let Some(values) = arg.get_value_parser().possible_values() {
            for value in values.filter(|v| !v.is_hide_set()) {
                add(
                    value.get_name().into(),
                    value
                        .get_help()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                );
            }
        }
        match arg.get_id().as_str() {
            "column" | "move_to" => {
                for column in &board.columns {
                    add(column.clone(), "Board column".into());
                }
            }
            "name" if path.get(1) == Some(&"column") && command.get_name() != "add" => {
                for column in &board.columns {
                    add(column.clone(), "Board column".into());
                }
            }
            "id" => {
                for card in &board.cards {
                    add(
                        card.id.to_string(),
                        format!(
                            "{} [{}]{}",
                            card.title,
                            card.column,
                            if card.archived { " (archived)" } else { "" }
                        ),
                    );
                }
            }
            "tag" => {
                let mut tags: Vec<_> = board.cards.iter().flat_map(|c| c.tags.iter()).collect();
                tags.sort();
                tags.dedup();
                for tag in tags {
                    add(tag.clone(), "Board tag".into());
                }
            }
            _ => {}
        }
    }
    suggestions.sort_by_key(|item| item.value.starts_with('-'));
    let mut display = command.clone().bin_name(path.join(" "));
    let mut hint = display.render_usage().to_string();
    if let Some(arg) = argument {
        hint.push_str(&format!(
            "\n<{}>: {}",
            arg.get_id(),
            arg.get_help()
                .map(ToString::to_string)
                .unwrap_or_else(|| "Enter a value".into())
        ));
    } else if let Some(about) = command.get_about() {
        hint.push_str(&format!("\n{about}"));
    }
    Completion {
        suggestions,
        hint,
        range,
        option_prefix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Priority;

    fn board() -> Board {
        let mut board = Board::new(
            "Test".into(),
            vec!["Todo".into(), "In Progress".into(), "Review λ".into()],
        )
        .unwrap();
        board
            .add(
                "Ship release".into(),
                "Todo",
                String::new(),
                Priority::High,
                vec!["rust".into()],
                None,
            )
            .unwrap();
        board
    }
    fn values(line: &str) -> Vec<String> {
        complete(line, line.len(), &board())
            .suggestions
            .into_iter()
            .map(|s| s.value)
            .collect()
    }
    fn accept(line: &str, cursor: usize, value: &str) -> (String, usize) {
        let completion = complete(line, cursor, &board());
        let selected = completion
            .suggestions
            .iter()
            .position(|s| s.value == value)
            .unwrap_or_else(|| panic!("Missing {value} for {line}"));
        let mut result = line.to_string();
        let mut cursor = cursor;
        completion.apply(selected, &mut result, &mut cursor);
        (result, cursor)
    }
    #[test]
    fn commands_options_and_enum_values_follow_cli_definitions() {
        assert!(values("").contains(&"add".into()));
        assert!(!values("").contains(&"tui".into()));
        assert!(!values("").contains(&"--file".into()));
        assert_eq!(values("col"), ["column"]);
        assert_eq!(values("column ren"), ["rename"]);
        assert_eq!(values("add \"A title\" --pri"), ["--priority"]);
        assert_eq!(values("add x -p h"), ["high"]);
        assert_eq!(values("add x -ph"), ["high"]);
        assert!(values("add x -phigh --column ").contains(&"Todo".into()));
        assert_eq!(values("add x --priority=h"), ["high"]);
        assert_eq!(values("list --sort d"), ["due"]);
        assert!(!values("list --all ").contains(&"--archived".into()));
        assert!(!values("add x --priority high ").contains(&"--priority".into()));
        assert!(complete("move ", 5, &board())
            .hint
            .contains("<ID> <COLUMN>"));
    }
    #[test]
    fn board_values_and_descriptions_match_argument_context() {
        assert_eq!(values("move 1 In"), ["In Progress"]);
        assert_eq!(values("column rename Re"), ["Review λ"]);
        assert!(values("column rename Todo ")
            .iter()
            .all(|s| !s.contains("Progress")));
        assert_eq!(values("column remove Todo --move-to In"), ["In Progress"]);
        assert_eq!(values("list --tag r"), ["rust"]);
        let completion = complete("edit ", 5, &board());
        assert_eq!(completion.suggestions[0].value, "1");
        assert!(completion.suggestions[0]
            .description
            .contains("Ship release"));
        assert!(values("add --priority high title --column ").contains(&"Todo".into()));
        assert!(!values("add -- ").iter().any(|v| v.starts_with('-')));
    }
    #[test]
    fn accepting_handles_quotes_unicode_and_the_middle_of_a_line() {
        for line in ["move 1 In", "move 1 \"In", "move 1 'In", "move 1 In\\ "] {
            let (result, _) = accept(line, line.len(), "In Progress");
            assert_eq!(shlex::split(&result).unwrap(), ["move", "1", "In Progress"]);
        }
        let line = "add x --column=\"Re";
        let (result, cursor) = accept(line, line.len(), "Review λ");
        assert_eq!(cursor, result.len());
        assert_eq!(
            shlex::split(&result).unwrap(),
            ["add", "x", "--column=Review λ"]
        );
        let (result, cursor) = accept("move 1 Incomplete --json", 9, "In Progress");
        assert_eq!(
            shlex::split(&result).unwrap(),
            ["move", "1", "In Progress", "--json"]
        );
        assert!(result[cursor..].starts_with("--json"));
        let line = "move 1 \"Review λ\"";
        let (result, _) = accept(line, line.len() - 1, "Review λ");
        assert_eq!(shlex::split(&result).unwrap(), ["move", "1", "Review λ"]);
    }
}
