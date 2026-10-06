use super::*;
use crate::db;
use rusqlite::Connection;

fn database() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(db::SCHEMA).unwrap();
    connection
}

#[test]
fn startup_shows_wizard_until_never_show_is_saved() {
    for stage in [
        None,
        Some(OnboardingStage::Welcome),
        Some(OnboardingStage::Importing),
        Some(OnboardingStage::Tips),
        Some(OnboardingStage::Complete),
    ] {
        let mut prefs = OnboardingPreferences {
            stage,
            ..Default::default()
        };
        let expected = StartupDecision::Welcome;
        assert_eq!(startup_decision(&prefs), expected, "stage={stage:?}");
        prefs.tips_dismissed = true;
        assert_eq!(startup_decision(&prefs), expected);
        prefs.never_show = true;
        assert_eq!(startup_decision(&prefs), StartupDecision::Hidden);
    }
}

#[test]
fn preferences_roundtrip_and_tips_dismissal_preserves_import_recovery() {
    let connection = database();
    assert_eq!(
        load_preferences(&connection).unwrap(),
        OnboardingPreferences::default()
    );
    save_never_show(&connection, true).unwrap();
    assert!(load_preferences(&connection).unwrap().never_show);
    save_never_show(&connection, false).unwrap();
    save_stage(&connection, OnboardingStage::Importing, Some("/photos")).unwrap();
    dismiss_tips(&connection).unwrap();
    let prefs = load_preferences(&connection).unwrap();
    assert!(!prefs.never_show);
    assert!(prefs.tips_dismissed);
    assert_eq!(prefs.stage, Some(OnboardingStage::Importing));
    assert_eq!(prefs.root.as_deref(), Some("/photos"));
    finish_onboarding(&connection).unwrap();
    let prefs = load_preferences(&connection).unwrap();
    assert_eq!(prefs.stage, Some(OnboardingStage::Complete));
    assert!(prefs.tips_dismissed);
    assert_eq!(prefs.root.as_deref(), Some("/photos"));
}

#[test]
fn saved_preference_survives_reopening_database() {
    let path = std::env::temp_dir().join(format!("pic-onboarding-{}.sqlite", std::process::id()));
    {
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(db::SCHEMA).unwrap();
        save_never_show(&connection, true).unwrap();
    }
    let connection = Connection::open(&path).unwrap();
    assert!(load_preferences(&connection).unwrap().never_show);
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn unknown_stage_is_hidden_and_failed_writes_return_errors() {
    let connection = database();
    db::set_setting(&connection, "onboarding-stage", "future-version").unwrap();
    assert_eq!(
        load_preferences(&connection).unwrap().stage,
        Some(OnboardingStage::Complete)
    );
    connection.execute_batch("DROP TABLE settings").unwrap();
    assert!(save_never_show(&connection, true).is_err());
    assert!(load_preferences(&connection).is_err());
}
