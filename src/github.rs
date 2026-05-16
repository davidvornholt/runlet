use crate::config::GitHubConfig;
use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::blocking::Client as HttpClient;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::fs;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitHubError {
    #[error("repository id must look like github:owner/repo, got {0}")]
    InvalidRepository(String),
    #[error("failed to read GitHub App key: {0}")]
    ReadKey(std::io::Error),
    #[error("failed to parse GitHub App key: {0}")]
    ParseKey(jsonwebtoken::errors::Error),
    #[error("failed to create GitHub App JWT: {0}")]
    Jwt(jsonwebtoken::errors::Error),
    #[error("GitHub request failed: {0}")]
    Request(reqwest::Error),
    #[error("GitHub pull request {number} changed-file list reached GitHub's 3000-file API cap")]
    PullRequestFileListTruncated { number: u64 },
    #[error("GitHub API returned {status}: {body}")]
    Api { status: StatusCode, body: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryId {
    pub owner: String,
    pub name: String,
}

impl RepositoryId {
    pub fn parse(value: &str) -> Result<Self, GitHubError> {
        let Some(path) = value.strip_prefix("github:") else {
            return Err(GitHubError::InvalidRepository(value.to_string()));
        };
        let Some((owner, name)) = path.split_once('/') else {
            return Err(GitHubError::InvalidRepository(value.to_string()));
        };
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            return Err(GitHubError::InvalidRepository(value.to_string()));
        }
        Ok(Self {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }

    pub fn repo_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }
}

#[derive(Debug, Serialize)]
struct JwtClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

#[derive(Debug, Deserialize)]
struct InstallationTokenResponse {
    token: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RegistrationToken {
    pub token: String,
    pub expires_at: String,
}

#[derive(Debug, Deserialize)]
struct RunnersResponse {
    runners: Vec<Runner>,
}

#[derive(Debug, Deserialize)]
struct Runner {
    id: u64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct PullRequestFile {
    filename: String,
    previous_filename: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PullRequestResponse {
    labels: Vec<PullRequestLabel>,
}

#[derive(Debug, Deserialize)]
struct PullRequestLabel {
    name: String,
}

#[derive(Clone)]
pub struct GitHubClient {
    config: GitHubConfig,
    http: HttpClient,
}

impl GitHubClient {
    pub fn new(config: GitHubConfig) -> Self {
        Self {
            config,
            http: HttpClient::new(),
        }
    }

    pub fn app_jwt(&self) -> Result<String, GitHubError> {
        let key = fs::read(&self.config.private_key_file).map_err(GitHubError::ReadKey)?;
        let encoding_key = EncodingKey::from_rsa_pem(&key).map_err(GitHubError::ParseKey)?;
        let now = Utc::now().timestamp();
        let claims = JwtClaims {
            iat: now - 60,
            exp: now + 9 * 60,
            iss: self.config.app_id.to_string(),
        };
        encode(&Header::new(Algorithm::RS256), &claims, &encoding_key).map_err(GitHubError::Jwt)
    }

    pub fn installation_token(&self) -> Result<String, GitHubError> {
        let jwt = self.app_jwt()?;
        let url = format!(
            "{}/app/installations/{}/access_tokens",
            self.config.api_base_url.trim_end_matches('/'),
            self.config.installation_id
        );
        let response = self
            .http
            .post(url)
            .bearer_auth(jwt)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "runlet")
            .send()
            .map_err(GitHubError::Request)?;
        parse_response::<InstallationTokenResponse>(response).map(|response| response.token)
    }

    pub fn create_registration_token(
        &self,
        repository: &RepositoryId,
    ) -> Result<RegistrationToken, GitHubError> {
        let installation_token = self.installation_token()?;
        let url = format!(
            "{}/repos/{}/{}/actions/runners/registration-token",
            self.config.api_base_url.trim_end_matches('/'),
            repository.owner,
            repository.name
        );
        let response = self
            .http
            .post(url)
            .bearer_auth(installation_token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "runlet")
            .send()
            .map_err(GitHubError::Request)?;
        parse_response(response)
    }

    pub fn pull_request_changed_files(
        &self,
        repository: &RepositoryId,
        pull_request_number: u64,
    ) -> Result<Vec<String>, GitHubError> {
        const PER_PAGE: usize = 100;
        const MAX_FILES: usize = 3000;
        let installation_token = self.installation_token()?;
        let mut files = Vec::new();
        for page in 1.. {
            let url = format!(
                "{}/repos/{}/{}/pulls/{}/files?per_page={PER_PAGE}&page={page}",
                self.config.api_base_url.trim_end_matches('/'),
                repository.owner,
                repository.name,
                pull_request_number
            );
            let response = self
                .http
                .get(url)
                .bearer_auth(&installation_token)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("User-Agent", "runlet")
                .send()
                .map_err(GitHubError::Request)?;
            let page_files = parse_response::<Vec<PullRequestFile>>(response)?;
            let count = page_files.len();
            for file in page_files {
                files.push(file.filename);
                if let Some(previous_filename) = file.previous_filename {
                    files.push(previous_filename);
                }
            }
            if page >= MAX_FILES / PER_PAGE && count == PER_PAGE {
                return Err(GitHubError::PullRequestFileListTruncated {
                    number: pull_request_number,
                });
            }
            if count < PER_PAGE {
                return Ok(files);
            }
        }
        unreachable!("unbounded changed-file pagination loop must return from inside the loop")
    }

    pub fn pull_request_has_label(
        &self,
        repository: &RepositoryId,
        pull_request_number: u64,
        label: &str,
    ) -> Result<bool, GitHubError> {
        let installation_token = self.installation_token()?;
        let url = format!(
            "{}/repos/{}/{}/pulls/{}",
            self.config.api_base_url.trim_end_matches('/'),
            repository.owner,
            repository.name,
            pull_request_number
        );
        let response = self
            .http
            .get(url)
            .bearer_auth(&installation_token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "runlet")
            .send()
            .map_err(GitHubError::Request)?;
        let pull_request = parse_response::<PullRequestResponse>(response)?;
        Ok(pull_request.labels.iter().any(|item| item.name == label))
    }

    pub fn remove_runner_by_name(
        &self,
        repository: &RepositoryId,
        runner_name: &str,
    ) -> Result<(), GitHubError> {
        let installation_token = self.installation_token()?;
        let Some(runner_id) =
            self.runner_id_by_name_with_token(repository, runner_name, &installation_token)?
        else {
            return Ok(());
        };
        self.remove_runner_with_token(repository, runner_id, &installation_token)
    }

    fn runner_id_by_name_with_token(
        &self,
        repository: &RepositoryId,
        runner_name: &str,
        token: &str,
    ) -> Result<Option<u64>, GitHubError> {
        const PER_PAGE: usize = 100;

        for page in 1.. {
            let url = format!(
                "{}/repos/{}/{}/actions/runners?per_page={PER_PAGE}&page={page}",
                self.config.api_base_url.trim_end_matches('/'),
                repository.owner,
                repository.name
            );
            let response = self
                .http
                .get(url)
                .bearer_auth(token)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("User-Agent", "runlet")
                .send()
                .map_err(GitHubError::Request)?;
            let runners = parse_response::<RunnersResponse>(response)?;
            let count = runners.runners.len();
            if let Some(runner) = runners
                .runners
                .into_iter()
                .find(|runner| runner.name == runner_name)
            {
                return Ok(Some(runner.id));
            }
            if count < PER_PAGE {
                return Ok(None);
            }
        }
        unreachable!("unbounded runner pagination loop must return from inside the loop")
    }

    fn remove_runner_with_token(
        &self,
        repository: &RepositoryId,
        runner_id: u64,
        token: &str,
    ) -> Result<(), GitHubError> {
        let url = format!(
            "{}/repos/{}/{}/actions/runners/{}",
            self.config.api_base_url.trim_end_matches('/'),
            repository.owner,
            repository.name,
            runner_id
        );
        let response = self
            .http
            .delete(url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "runlet")
            .send()
            .map_err(GitHubError::Request)?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(GitHubError::Api {
                status: response.status(),
                body: response.text().unwrap_or_default(),
            })
        }
    }
}

fn parse_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::blocking::Response,
) -> Result<T, GitHubError> {
    let status = response.status();
    if status.is_success() {
        response.json::<T>().map_err(GitHubError::Request)
    } else {
        Err(GitHubError::Api {
            status,
            body: response.text().unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repository_id() {
        let repository = RepositoryId::parse("github:openai/runlet").unwrap();
        assert_eq!(repository.owner, "openai");
        assert_eq!(repository.name, "runlet");
        assert_eq!(repository.repo_url(), "https://github.com/openai/runlet");
    }

    #[test]
    fn rejects_invalid_repository_id() {
        assert!(RepositoryId::parse("openai/runlet").is_err());
        assert!(RepositoryId::parse("github:openai").is_err());
        assert!(RepositoryId::parse("github:openai/runlet/extra").is_err());
    }
}
