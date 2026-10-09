mod common;

use std::fs;
use std::path::PathBuf;

use common::file_names;
use scratchpad_core::{Config, ThemePreference, WindowBounds};

fn config_with(json: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::write(&path, json).unwrap();
    (dir, path)
}

#[test]
fn a_missing_file_gives_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    assert_eq!(Config::load(&path), Config::default());
    assert!(!path.exists(), "loading must not create the file");
}

#[test]
fn save_and_load_round_trip_every_field() {
    let dir = tempfile::tempdir().unwrap();
    // The folder does not exist yet, like on a first run.
    let path = dir.path().join("Scratchpad").join("config.json");
    let config = Config {
        window: Some(WindowBounds {
            x: -1200.5,
            y: 40.0,
            width: 960.0,
            height: 700.25,
            maximized: true,
        }),
        sidebar_width: Some(248.5),
        last_opened_note: Some(dir.path().join("Meeting Notes.md")),
        theme: ThemePreference::Dark,
        notes_dir: Some(dir.path().join("my notes")),
        sidebar_collapsed: true,
        status_bar_hidden: true,
        zoom_percent: Some(150),
    };

    config.save(&path).unwrap();

    assert_eq!(Config::load(&path), config);
    assert_eq!(file_names(path.parent().unwrap()), ["config.json"]);
}

#[test]
fn saving_replaces_the_previous_config() {
    let (_dir, path) = config_with(br#"{"theme": "light", "sidebar_width": 100}"#);
    Config {
        theme: ThemePreference::Dark,
        ..Config::default()
    }
    .save(&path)
    .unwrap();

    let loaded = Config::load(&path);
    assert_eq!(loaded.theme, ThemePreference::Dark);
    assert_eq!(loaded.sidebar_width, None);
}

#[test]
fn loads_the_documented_file_format() {
    let (_dir, path) = config_with(
        br#"{
            "window": {"x": 10, "y": 20, "width": 800, "height": 600},
            "sidebar_width": 260,
            "last_opened_note": "C:/Notes/Todo.md",
            "theme": "light",
            "notes_dir": "C:/Notes",
            "sidebar_collapsed": true,
            "status_bar_hidden": true,
            "zoom_percent": 120,
            "from_the_future": [1, 2, 3]
        }"#,
    );
    assert_eq!(
        Config::load(&path),
        Config {
            window: Some(WindowBounds {
                x: 10.0,
                y: 20.0,
                width: 800.0,
                height: 600.0,
                maximized: false,
            }),
            sidebar_width: Some(260.0),
            last_opened_note: Some("C:/Notes/Todo.md".into()),
            theme: ThemePreference::Light,
            notes_dir: Some("C:/Notes".into()),
            sidebar_collapsed: true,
            status_bar_hidden: true,
            zoom_percent: Some(120),
        }
    );
}

#[test]
fn a_partial_file_keeps_what_is_present_and_defaults_the_rest() {
    let (_dir, path) = config_with(br#"{"theme": "dark"}"#);
    assert_eq!(
        Config::load(&path),
        Config {
            theme: ThemePreference::Dark,
            ..Config::default()
        }
    );
}

#[test]
fn invalid_fields_fall_back_individually() {
    let (_dir, path) = config_with(
        br#"{
            "window": {"x": "left", "y": 0, "width": 1, "height": 1},
            "sidebar_width": "wide",
            "theme": "solarized",
            "last_opened_note": 42,
            "notes_dir": "C:/Notes",
            "zoom_percent": -5
        }"#,
    );
    assert_eq!(
        Config::load(&path),
        Config {
            notes_dir: Some("C:/Notes".into()),
            ..Config::default()
        }
    );
}

#[test]
fn unusable_files_give_defaults() {
    for contents in [
        &b""[..],
        b"not json at all",
        b"{\"theme\": \"dark\"",
        b"[1, 2, 3]",
        b"\xFF\xFE\x00garbage",
        b"null",
    ] {
        let (_dir, path) = config_with(contents);
        assert_eq!(Config::load(&path), Config::default(), "{contents:?}");
    }
}

#[test]
fn a_configured_notes_folder_replaces_the_default_one() {
    let config = Config {
        notes_dir: Some("D:/Elsewhere".into()),
        ..Config::default()
    };
    assert_eq!(config.notes_dir_or_default(), PathBuf::from("D:/Elsewhere"));
    assert_eq!(
        Config::default().notes_dir_or_default(),
        scratchpad_core::default_notes_dir()
    );
}

#[test]
fn a_directory_in_place_of_the_file_gives_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::create_dir(&path).unwrap();
    assert_eq!(Config::load(&path), Config::default());
}

#[test]
fn a_byte_order_mark_from_a_text_editor_is_tolerated() {
    let (_dir, path) = config_with(b"\xEF\xBB\xBF{\"theme\": \"dark\"}");
    assert_eq!(Config::load(&path).theme, ThemePreference::Dark);
}

#[test]
fn a_failed_save_reports_the_error_and_keeps_the_old_file() {
    let (dir, path) = config_with(br#"{"theme": "dark"}"#);
    // A file where the config folder should be makes creating it impossible.
    let blocked = dir
        .path()
        .join("config.json")
        .join("nested")
        .join("config.json");

    let error = Config::default().save(&blocked).unwrap_err();

    assert!(error.to_string().contains("config"), "{error}");
    assert_eq!(Config::load(&path).theme, ThemePreference::Dark);
}
