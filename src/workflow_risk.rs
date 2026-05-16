use crate::config::WorkflowRiskConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowRiskDecision {
    Allow,
    Deny { reason: String },
    RequireApproval { reason: String },
}

pub fn workflow_risk_decision(
    config: &WorkflowRiskConfig,
    changed_files: &[String],
    approved: bool,
    runlet_labeled: bool,
) -> WorkflowRiskDecision {
    let matching_file = changed_files
        .iter()
        .find(|file| is_high_risk_path(config, file));
    let Some(file) = matching_file else {
        return WorkflowRiskDecision::Allow;
    };

    if config.require_approval_for_workflow_changes {
        return if approved {
            WorkflowRiskDecision::Allow
        } else {
            WorkflowRiskDecision::RequireApproval {
                reason: format!(
                    "pull request changes high-risk workflow path {file} and requires approval"
                ),
            }
        };
    }

    if config.deny_workflow_file_changes
        || (config.deny_runlet_label_if_workflow_changed && runlet_labeled)
    {
        return WorkflowRiskDecision::Deny {
            reason: format!("pull request changes high-risk workflow path {file}"),
        };
    }

    WorkflowRiskDecision::Allow
}

pub fn is_high_risk_path(config: &WorkflowRiskConfig, path: &str) -> bool {
    config
        .high_risk_paths
        .iter()
        .chain(config.additional_high_risk_paths.iter())
        .any(|pattern| path_matches_pattern(pattern, path))
}

fn path_matches_pattern(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim_matches('/');
    let path = path.trim_matches('/');
    if pattern == path || pattern == "**" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return path == prefix || path.starts_with(&format!("{prefix}/"));
    }
    if let Some(suffix) = pattern.strip_prefix("**/") {
        return path == suffix || path.ends_with(&format!("/{suffix}"));
    }
    if let Some(prefix) = pattern.strip_suffix("/*") {
        return path.strip_prefix(prefix).is_some_and(|rest| {
            rest.starts_with('/') && !rest[1..].contains('/') && !rest[1..].is_empty()
        });
    }
    if pattern.contains('*') {
        return wildcard_match(pattern.as_bytes(), path.as_bytes());
    }
    false
}

fn wildcard_match(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    if pattern[0] == b'*' {
        return wildcard_match(&pattern[1..], text)
            || (!text.is_empty() && wildcard_match(pattern, &text[1..]));
    }
    !text.is_empty() && pattern[0] == text[0] && wildcard_match(&pattern[1..], &text[1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_high_risk_workflow_changes() {
        let config = WorkflowRiskConfig::default();
        assert!(is_high_risk_path(&config, ".github/workflows/ci.yml"));
        assert!(is_high_risk_path(&config, "crates/foo/action.yaml"));
        assert!(is_high_risk_path(&config, "scripts/build.sh"));
        assert!(!is_high_risk_path(&config, "src/lib.rs"));
    }

    #[test]
    fn denies_or_holds_high_risk_pull_request_changes() {
        let config = WorkflowRiskConfig::default();
        assert!(matches!(
            workflow_risk_decision(
                &config,
                &[".github/workflows/ci.yml".to_string()],
                false,
                true,
            ),
            WorkflowRiskDecision::Deny { .. }
        ));

        let runlet_label_only = WorkflowRiskConfig {
            deny_workflow_file_changes: false,
            deny_runlet_label_if_workflow_changed: true,
            ..WorkflowRiskConfig::default()
        };
        assert_eq!(
            workflow_risk_decision(
                &runlet_label_only,
                &[".github/workflows/ci.yml".to_string()],
                false,
                false,
            ),
            WorkflowRiskDecision::Allow
        );
        assert!(matches!(
            workflow_risk_decision(
                &runlet_label_only,
                &[".github/workflows/ci.yml".to_string()],
                false,
                true,
            ),
            WorkflowRiskDecision::Deny { .. }
        ));

        let config = WorkflowRiskConfig {
            deny_workflow_file_changes: false,
            deny_runlet_label_if_workflow_changed: false,
            require_approval_for_workflow_changes: true,
            ..WorkflowRiskConfig::default()
        };
        assert!(matches!(
            workflow_risk_decision(
                &config,
                &[".github/workflows/ci.yml".to_string()],
                false,
                true,
            ),
            WorkflowRiskDecision::RequireApproval { .. }
        ));
        assert_eq!(
            workflow_risk_decision(
                &config,
                &[".github/workflows/ci.yml".to_string()],
                true,
                true,
            ),
            WorkflowRiskDecision::Allow
        );

        let approval_overrides_default_deny_flags = WorkflowRiskConfig {
            require_approval_for_workflow_changes: true,
            ..WorkflowRiskConfig::default()
        };
        assert_eq!(
            workflow_risk_decision(
                &approval_overrides_default_deny_flags,
                &[".github/workflows/ci.yml".to_string()],
                true,
                true,
            ),
            WorkflowRiskDecision::Allow
        );
    }
}
