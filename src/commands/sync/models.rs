//! The sync command's report types: names, actions and the text to show.
//! No value lives here (WP-17): the preview rows hold the line as it prints,
//! built in `executor.rs` where the values are.

use super::executor::VarChange;

/// What a dry run shows: the target, the action, and one row per variable.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncPreview {
    pub target_file: String,
    pub action: SyncAction,
    pub variables: Vec<PreviewRow>,
    pub warnings: Vec<String>,
}

/// One line of the preview.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PreviewRow {
    pub key: String,
    /// The text the preview prints after `KEY =`: a placeholder, or the value
    /// of a variable that is not a credential. Decided in `VarChange::row`, never
    /// the value of a credential.
    pub shown: String,
    pub is_placeholder: bool,
}

impl SyncPreview {
    pub fn new(
        target_file: String,
        action: SyncAction,
        changes: &[VarChange],
        warnings: Vec<String>,
    ) -> Self {
        Self {
            target_file,
            action,
            variables: changes.iter().map(VarChange::row).collect(),
            warnings,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum SyncAction {
    Add,
    Update,
    Remove,
    NoChange,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_action_equality() {
        assert_eq!(SyncAction::Add, SyncAction::Add);
        assert_ne!(SyncAction::Add, SyncAction::Remove);
    }
}
