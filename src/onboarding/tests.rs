use super::*;
use crate::db;
use rusqlite::Connection;

fn database() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(db::SCHEMA).unwrap();
    connection
}

#[test]
fn startup_only_automatically_opens_for_fresh_or_recorded_incomplete_setup() {
    let fresh = StartupFacts {
        has_photo_records: false,
        has_folder_records: false,
    };
    let prefs = OnboardingPreferences::default();
    assert_eq!(startup_decision(&prefs, fresh), StartupDecision::Welcome);
    for facts in [
        StartupFacts {
            has_photo_records: true,
            ..fresh
        },
        StartupFacts {
            has_folder_records: true,
            ..fresh
        },
    ] {
        assert_eq!(startup_decision(&prefs, facts), StartupDecision::Hidden);
    }
    for stage in [
        OnboardingStage::Welcome,
        OnboardingStage::Importing,
        OnboardingStage::Tips,
        OnboardingStage::Complete,
    ] {
        let prefs = OnboardingPreferences {
            stage: Some(stage),
            never_show: true,
            ..Default::default()
        };
        assert_eq!(startup_decision(&prefs, fresh), StartupDecision::Hidden);
    }
    let prefs = OnboardingPreferences {
        stage: Some(OnboardingStage::Importing),
        ..Default::default()
    };
    assert_eq!(
        startup_decision(&prefs, fresh),
        StartupDecision::RecoverImport
    );
    let prefs = OnboardingPreferences {
        stage: Some(OnboardingStage::Tips),
        ..Default::default()
    };
    assert_eq!(
        startup_decision(&prefs, fresh),
        StartupDecision::RecoverImport
    );
    assert_eq!(
        startup_decision(
            &prefs,
            StartupFacts {
                has_photo_records: true,
                ..fresh
            }
        ),
        StartupDecision::Tips
    );
    assert_eq!(
        startup_decision(
            &OnboardingPreferences {
                stage: Some(OnboardingStage::Complete),
                ..Default::default()
            },
            fresh
        ),
        StartupDecision::Hidden
    );
}

#[test]
fn trashed_photos_and_registered_empty_folders_are_existing_libraries() {
    let connection = database();
    connection
        .execute("INSERT INTO photos(path,trashed) VALUES('/gone.jpg',1)", [])
        .unwrap();
    assert!(startup_facts(&connection).unwrap().has_photo_records);
    connection.execute("DELETE FROM photos", []).unwrap();
    db::mark_import_root(&connection, "/offline/root").unwrap();
    assert!(startup_facts(&connection).unwrap().has_folder_records);
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
