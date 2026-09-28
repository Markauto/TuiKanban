use assert_cmd::{cargo::cargo_bin_cmd, Command};
use predicates::str::contains;
use serde_json::Value;
use std::path::Path;

fn cmd(file: &Path) -> Command {
    let mut command = cargo_bin_cmd!("kanban");
    command.arg("--file").arg(file);
    command
}
fn json(file: &Path, args: &[&str]) -> Value {
    let output = cmd(file)
        .arg("--json")
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).unwrap()
}
fn init(file: &Path) {
    cmd(file).args(["init", "Release"]).assert().success();
}

#[test]
fn complete_card_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("board.json");
    init(&file);
    let c = json(
        &file,
        &[
            "add",
            "Ship λ",
            "--description",
            "First line\nSecond line",
            "--tags",
            "release,Rust,rust",
            "--priority",
            "high",
            "--due",
            "2026-01-02",
        ],
    );
    assert_eq!(c["id"], 1);
    assert_eq!(c["tags"], serde_json::json!(["release", "Rust"]));
    assert_eq!(
        json(&file, &["show", "1"])["description"],
        "First line\nSecond line"
    );
    assert_eq!(
        json(&file, &["move", "1", "in progress"])["column"],
        "In Progress"
    );
    let c = json(
        &file,
        &[
            "edit",
            "1",
            "--title",
            "Shipped",
            "--priority",
            "urgent",
            "--tags",
            "",
            "--clear-due",
        ],
    );
    assert_eq!(c["title"], "Shipped");
    assert_eq!(c["tags"], serde_json::json!([]));
    assert!(c["due"].is_null());
    assert_eq!(json(&file, &["archive", "1"])["archived"], true);
    assert_eq!(json(&file, &["list"]), serde_json::json!([]));
    assert_eq!(
        json(&file, &["list", "--archived"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    json(&file, &["restore", "1"]);
    cmd(&file)
        .args(["delete", "1"])
        .assert()
        .failure()
        .stderr(contains("--yes"));
    assert_eq!(json(&file, &["delete", "1", "--yes"])["id"], 1);
    cmd(&file)
        .args(["show", "1"])
        .assert()
        .failure()
        .stderr(contains("not found"));
    assert_eq!(json(&file, &["add", "Next"])["id"], 2);
}
#[test]
fn filtering_sorting_and_stats() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("board.json");
    init(&file);
    json(
        &file,
        &["add", "Zulu", "--tags", "backend", "--priority", "low"],
    );
    json(
        &file,
        &[
            "add",
            "Alpha",
            "--tags",
            "Frontend",
            "--priority",
            "urgent",
            "--due",
            "2000-01-01",
            "--column",
            "Done",
        ],
    );
    assert_eq!(
        json(
            &file,
            &[
                "list",
                "--tag",
                "frontend",
                "--search",
                "ALPHA",
                "--column",
                "done",
                "--priority",
                "urgent",
                "--overdue"
            ]
        )[0]["id"],
        2
    );
    for sort in ["title", "due", "priority"] {
        assert_eq!(json(&file, &["list", "--sort", sort])[0]["id"], 2);
    }
    let stats = json(&file, &["stats"]);
    assert_eq!(stats["active"], 2);
    assert_eq!(stats["overdue"], 1);
    assert_eq!(stats["columns"]["Done"], 1);
    json(&file, &["archive", "2"]);
    assert_eq!(json(&file, &["list", "--all"]).as_array().unwrap().len(), 2);
    cmd(&file)
        .args(["list", "--column", "missing"])
        .assert()
        .failure();
}
#[test]
fn columns_preserve_active_and_archived_cards() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("board.json");
    init(&file);
    json(&file, &["add", "Task"]);
    json(&file, &["archive", "1"]);
    json(&file, &["column", "rename", "Todo", "Backlog"]);
    assert_eq!(json(&file, &["show", "1"])["column"], "Backlog");
    cmd(&file)
        .args(["column", "remove", "Backlog"])
        .assert()
        .failure();
    json(&file, &["column", "remove", "Backlog", "--move-to", "Done"]);
    assert_eq!(json(&file, &["show", "1"])["column"], "Done");
    json(&file, &["column", "add", "Review"]);
    assert_eq!(
        json(&file, &["column", "order", "Review", "1"])[0],
        "Review"
    );
    cmd(&file)
        .args(["column", "add", "review"])
        .assert()
        .failure();
    cmd(&file)
        .args(["column", "rename", "Review", "Done"])
        .assert()
        .failure();
    assert_eq!(json(&file, &["column", "list"])[0], "Review");
    json(&file, &["column", "remove", "Review"]);
    json(&file, &["column", "remove", "In Progress"]);
    cmd(&file)
        .args(["column", "remove", "Done"])
        .assert()
        .failure()
        .stderr(contains("last column"));
}
#[test]
fn import_export_roundtrip_and_validation() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("board.json");
    let other = dir.path().join("other.json");
    let backup = dir.path().join("backup.json");
    init(&file);
    json(&file, &["add", "Portable"]);
    let exported = cmd(&file)
        .arg("export")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    std::fs::write(&backup, &exported).unwrap();
    cmd(&other).arg("import").arg(&backup).assert().success();
    assert_eq!(json(&file, &["export"]), json(&other, &["export"]));
    cmd(&other).arg("import").arg(&backup).assert().failure();
    cmd(&other)
        .arg("import")
        .arg(&backup)
        .arg("--force")
        .assert()
        .success();
    let original = std::fs::read(&other).unwrap();
    let valid: Value = serde_json::from_slice(&exported).unwrap();
    for change in [
        "version",
        "next_id",
        "column",
        "id",
        "created_at",
        "title",
        "columns",
    ] {
        let mut data = valid.clone();
        match change {
            "version" => data["version"] = 99.into(),
            "next_id" => data["next_id"] = 1.into(),
            "column" => data["cards"][0]["column"] = "unknown".into(),
            "id" => data["cards"][0]["id"] = 0.into(),
            "created_at" => data["cards"][0]["created_at"] = "yesterday".into(),
            "title" => data["cards"][0]["title"] = " ".into(),
            "columns" => data["columns"] = serde_json::json!([]),
            _ => unreachable!(),
        }
        std::fs::write(&backup, serde_json::to_vec(&data).unwrap()).unwrap();
        cmd(&other)
            .arg("import")
            .arg(&backup)
            .arg("--force")
            .assert()
            .failure();
        assert_eq!(std::fs::read(&other).unwrap(), original);
    }
}
#[test]
fn failed_mutations_do_not_change_data() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("board.json");
    init(&file);
    json(&file, &["add", "Safe"]);
    let original = std::fs::read(&file).unwrap();
    for args in [
        vec!["add", " "],
        vec!["add", "Bad", "--due", "2026-02-30"],
        vec!["edit", "1", "--title", ""],
        vec!["move", "1", "Missing"],
        vec!["edit", "999", "--title", "No"],
        vec!["init"],
        vec!["column", "order", "Todo", "0"],
    ] {
        cmd(&file).args(args).assert().failure();
        assert_eq!(std::fs::read(&file).unwrap(), original);
    }
    std::fs::write(&file, "not json").unwrap();
    cmd(&file).args(["add", "No overwrite"]).assert().failure();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "not json");
}
#[test]
fn concurrent_writers_keep_every_card() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("board.json");
    init(&file);
    let mut children = vec![];
    for i in 0..20 {
        children.push(
            std::process::Command::new(assert_cmd::cargo::cargo_bin!("kanban"))
                .arg("--file")
                .arg(&file)
                .args(["add", &format!("Task {i}")])
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let board = json(&file, &["export"]);
    assert_eq!(board["cards"].as_array().unwrap().len(), 20);
    assert_eq!(board["next_id"], 21);
}
#[test]
fn help_completions_and_noninteractive_errors() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("nested/board.json");
    cmd(&file)
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("completions"));
    cmd(&file)
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(contains("complete"));
    cmd(&file)
        .arg("list")
        .assert()
        .failure()
        .stderr(contains("kanban init"));
    init(&file);
    cmd(&file)
        .arg("tui")
        .assert()
        .failure()
        .stderr(contains("interactive terminal"));
    cargo_bin_cmd!("kanban")
        .env("KANBAN_FILE", &file)
        .args(["--json", "stats"])
        .assert()
        .success()
        .stdout(contains("Release"));
}

// Keep environment changes in child processes so parallel tests remain isolated.
fn isolated_cmd(home: &Path, cwd: &Path) -> Command {
    let mut command = cargo_bin_cmd!("kanban");
    command
        .env_remove("KANBAN_FILE")
        .env_remove("XDG_DATA_HOME")
        .env("HOME", home)
        .env("USERPROFILE", home)
        .current_dir(cwd);
    command
}

#[test]
fn default_board_is_shared_across_working_directories() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let legacy = first.join(".kanban.json");
    std::fs::write(&legacy, "legacy board stays untouched").unwrap();
    isolated_cmd(&home, &first)
        .args(["init", "Shared"])
        .assert()
        .success();
    assert!(home.join(".local/share/kanban/board.json").exists());
    isolated_cmd(&home, &second)
        .args(["--json", "stats"])
        .assert()
        .success()
        .stdout(contains("Shared"));
    assert_eq!(
        std::fs::read_to_string(&legacy).unwrap(),
        "legacy board stays untouched"
    );
    assert!(!second.join(".kanban.json").exists());
}

#[test]
fn xdg_data_home_and_explicit_overrides_have_correct_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let data = dir.path().join("data");
    isolated_cmd(&home, dir.path())
        .env("XDG_DATA_HOME", &data)
        .args(["init", "XDG"])
        .assert()
        .success();
    assert!(data.join("kanban/board.json").exists());
    assert!(!home.exists());
    isolated_cmd(&home, dir.path())
        .env("XDG_DATA_HOME", &data)
        .env("KANBAN_FILE", "env.json")
        .args(["init", "Environment"])
        .assert()
        .success();
    isolated_cmd(&home, dir.path())
        .env("XDG_DATA_HOME", &data)
        .env("KANBAN_FILE", "env.json")
        .args(["--file", "explicit.json", "init", "Explicit"])
        .assert()
        .success();
    assert_eq!(
        json(&dir.path().join("env.json"), &["export"])["name"],
        "Environment"
    );
    assert_eq!(
        json(&dir.path().join("explicit.json"), &["export"])["name"],
        "Explicit"
    );
    assert_eq!(
        json(&data.join("kanban/board.json"), &["export"])["name"],
        "XDG"
    );
}

#[test]
fn empty_or_relative_xdg_values_use_home() {
    for value in ["", "relative/data"] {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        isolated_cmd(&home, dir.path())
            .env("XDG_DATA_HOME", value)
            .arg("init")
            .assert()
            .success();
        assert!(home.join(".local/share/kanban/board.json").exists());
        assert!(!dir.path().join("relative").exists());
    }
}

#[test]
fn missing_home_requires_a_path_but_does_not_break_help_or_completions() {
    let dir = tempfile::tempdir().unwrap();
    let without_home = || {
        let mut command = isolated_cmd(dir.path(), dir.path());
        command.env_remove("HOME").env_remove("USERPROFILE");
        command
    };
    without_home()
        .arg("init")
        .assert()
        .failure()
        .stderr(contains("--file"));
    without_home().arg("--help").assert().success();
    without_home()
        .args(["completions", "bash"])
        .assert()
        .success();
    without_home()
        .args(["--file", "override.json", "init"])
        .assert()
        .success();
    without_home()
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .arg("init")
        .assert()
        .success();
}

#[test]
fn column_layout_survives_renames_ordering_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("board.json");
    init(&file);
    json(&file, &["add", "Keep me", "--column", "In Progress"]);
    json(&file, &["archive", "1"]);
    cmd(&file)
        .args(["column", "stack", "in progress"])
        .assert()
        .success();
    cmd(&file)
        .args(["column", "rename", "In Progress", "Working"])
        .assert()
        .success();
    let exported = json(&file, &["export"]);
    assert_eq!(exported["stacked_columns"], serde_json::json!(["Working"]));
    assert_eq!(exported["cards"][0]["column"], "Working");
    let before = std::fs::read(&file).unwrap();
    for args in [
        vec!["column", "stack", "Todo"],
        vec!["column", "rename", "Working", "Todo"],
        vec!["column", "add", "  "],
    ] {
        cmd(&file).args(args).assert().failure();
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }
    let copy = dir.path().join("copy.json");
    cmd(&copy).arg("import").arg(&file).assert().success();
    assert_eq!(json(&copy, &["export"]), exported);
    cmd(&file)
        .args(["column", "order", "Working", "1"])
        .assert()
        .success();
    assert!(json(&file, &["export"]).get("stacked_columns").is_none());
    cmd(&file)
        .args(["column", "stack", "Todo"])
        .assert()
        .success();
    cmd(&file)
        .args(["column", "remove", "Working", "--move-to", "Todo"])
        .assert()
        .success();
    assert!(json(&file, &["export"]).get("stacked_columns").is_none());
    assert_eq!(json(&file, &["show", "1"])["column"], "Todo");
    cmd(&file)
        .args(["column", "stack", "Done"])
        .assert()
        .success();
    cmd(&file)
        .args(["column", "unstack", "Done"])
        .assert()
        .success();
    assert!(json(&file, &["export"]).get("stacked_columns").is_none());
}

#[test]
fn legacy_boards_load_and_invalid_stacks_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("board.json");
    init(&file);
    let mut board = json(&file, &["export"]);
    assert!(board.get("stacked_columns").is_none());
    cmd(&file).args(["list"]).assert().success();
    let source = dir.path().join("bad.json");
    let before = std::fs::read(&file).unwrap();
    for stacks in [
        serde_json::json!(["Missing"]),
        serde_json::json!(["Todo"]),
        serde_json::json!(["Done", "Done"]),
    ] {
        board["stacked_columns"] = stacks;
        std::fs::write(&source, serde_json::to_vec(&board).unwrap()).unwrap();
        cmd(&file)
            .arg("import")
            .arg(&source)
            .arg("--force")
            .assert()
            .failure();
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }
}
