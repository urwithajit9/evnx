//! Data models for the sync command.

/// Result of a sync operation (for dry-run preview)
#[derive(Debug, Clone)]
pub struct SyncPreview {
    pub target_file: String,
    pub action: SyncAction,
    pub variables: Vec<VarChange>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SyncAction {
    Add,
    Update,
    Remove,
    NoChange,
}

#[derive(Debug, Clone)]
pub struct VarChange {
    pub key: String,
    pub old_value: Option<String>,
    pub new_value: String,
    pub is_placeholder: bool,
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_action_equality() {
        assert_eq!(SyncAction::Add, SyncAction::Add);
        assert_ne!(SyncAction::Add, SyncAction::Remove);
    }

    #[test]
    fn test_var_change_clone() {
        let change = VarChange {
            key: "TEST".to_string(),
            old_value: Some("old".to_string()),
            new_value: "new".to_string(),
            is_placeholder: true,
        };
        let cloned = change.clone();
        assert_eq!(cloned.key, change.key);
        assert_eq!(cloned.is_placeholder, change.is_placeholder);
    }
}
