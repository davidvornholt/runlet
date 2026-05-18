use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RepositoryConfig {
    pub enabled: bool,
    pub public_pull_requests: PublicPullRequestConfig,
    pub trusted_branches: Vec<String>,
    pub trusted_jobs: TrustedJobsConfig,
    pub workflow_risk: WorkflowRiskConfig,
}

impl Default for RepositoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            public_pull_requests: PublicPullRequestConfig::default(),
            trusted_branches: vec!["main".to_string()],
            trusted_jobs: TrustedJobsConfig::default(),
            workflow_risk: WorkflowRiskConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct PublicPullRequestConfig {
    pub enabled: bool,
    pub timeout: String,
}

impl Default for PublicPullRequestConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout: "15m".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkPolicy {
    Strict,
    Restricted,
    Normal,
    Offline,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct TrustedJobsConfig {
    pub allow_registry_push: bool,
    pub allow_deploy: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct WorkflowRiskConfig {
    pub require_approval_for_workflow_changes: bool,
    pub approval_label: String,
    pub high_risk_paths: Vec<String>,
    pub additional_high_risk_paths: Vec<String>,
}

impl Default for WorkflowRiskConfig {
    fn default() -> Self {
        Self {
            require_approval_for_workflow_changes: false,
            approval_label: "runlet-approved-workflow-change".to_string(),
            high_risk_paths: vec![
                ".github/workflows/**".to_string(),
                ".github/actions/**".to_string(),
                "**/action.yml".to_string(),
                "**/action.yaml".to_string(),
                "scripts/**".to_string(),
            ],
            additional_high_risk_paths: Vec::new(),
        }
    }
}
