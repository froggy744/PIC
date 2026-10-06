pub(crate) mod view;
use anyhow::Result;
use rusqlite::Connection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OnboardingStage {
    Welcome,
    Importing,
    Tips,
    Complete,
}
impl OnboardingStage {
    fn key(self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::Importing => "importing",
            Self::Tips => "tips",
            Self::Complete => "complete",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OnboardingPreferences {
    pub never_show: bool,
    pub stage: Option<OnboardingStage>,
    pub root: Option<String>,
    pub tips_dismissed: bool,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct StartupFacts {
    pub has_photo_records: bool,
    pub has_folder_records: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartupDecision {
    Hidden,
    Welcome,
    RecoverImport,
    Tips,
}

pub(crate) fn load_preferences(connection: &Connection) -> Result<OnboardingPreferences> {
    let read = |key| crate::db::setting(connection, key);
    let stage = read("onboarding-stage")?.map(|value| match value.as_str() {
        "welcome" => OnboardingStage::Welcome,
        "importing" => OnboardingStage::Importing,
        "tips" => OnboardingStage::Tips,
        _ => OnboardingStage::Complete,
    });
    Ok(OnboardingPreferences {
        never_show: read("onboarding-never-show")?.as_deref() == Some("true"),
        stage,
        root: read("onboarding-root")?,
        tips_dismissed: read("onboarding-tips-dismissed")?.as_deref() == Some("true"),
    })
}
pub(crate) fn save_never_show(connection: &Connection, value: bool) -> Result<()> {
    crate::db::set_setting(
        connection,
        "onboarding-never-show",
        if value { "true" } else { "false" },
    )
}
pub(crate) fn save_stage(
    connection: &Connection,
    stage: OnboardingStage,
    root: Option<&str>,
) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    crate::db::set_setting(&transaction, "onboarding-stage", stage.key())?;
    if let Some(root) = root {
        crate::db::set_setting(&transaction, "onboarding-root", root)?;
    }
    transaction.commit()?;
    Ok(())
}
pub(crate) fn dismiss_tips(connection: &Connection) -> Result<()> {
    crate::db::set_setting(connection, "onboarding-tips-dismissed", "true")
}
pub(crate) fn finish_onboarding(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    crate::db::set_setting(&transaction, "onboarding-stage", "complete")?;
    dismiss_tips(&transaction)?;
    transaction.commit()?;
    Ok(())
}
pub(crate) fn startup_facts(connection: &Connection) -> Result<StartupFacts> {
    Ok(StartupFacts {
        has_photo_records: connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM photos)",
            [],
            |row| row.get(0),
        )?,
        has_folder_records: connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM folders)",
            [],
            |row| row.get(0),
        )?,
    })
}
pub(crate) fn startup_decision(
    prefs: &OnboardingPreferences,
    facts: StartupFacts,
) -> StartupDecision {
    if prefs.never_show || prefs.stage == Some(OnboardingStage::Complete) {
        return StartupDecision::Hidden;
    }
    match prefs.stage {
        Some(OnboardingStage::Importing) => StartupDecision::RecoverImport,
        Some(OnboardingStage::Tips) if facts.has_photo_records && !prefs.tips_dismissed => {
            StartupDecision::Tips
        }
        Some(OnboardingStage::Tips) => StartupDecision::RecoverImport,
        _ if !facts.has_photo_records && !facts.has_folder_records => StartupDecision::Welcome,
        _ => StartupDecision::Hidden,
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod gtk_tests;
