use crate::config::CacheConfig;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("cache writes are not allowed for untrusted jobs")]
    UntrustedWriteDenied,
    #[error("failed to create cache namespace {path}: {source}")]
    CreateNamespace {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheMount {
    pub namespace: String,
    pub path: PathBuf,
    pub writable: bool,
}

pub fn namespace_for_repository(repository: &str) -> String {
    let safe_name = repository
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let digest = Sha256::digest(repository.as_bytes());
    format!("{safe_name}-{}", &hex::encode(digest)[..16])
}

pub fn prepare_cache_mount(
    config: &CacheConfig,
    repository: &str,
    cache_write: bool,
) -> Result<Option<CacheMount>, CacheError> {
    if !config.enable {
        return Ok(None);
    }
    let namespace = namespace_for_repository(repository);
    let path = config.path.join(&namespace);
    fs::create_dir_all(&path).map_err(|source| CacheError::CreateNamespace {
        path: path.clone(),
        source,
    })?;
    Ok(Some(CacheMount {
        namespace,
        path,
        writable: cache_write,
    }))
}

pub fn assert_untrusted_cache_write_allowed(
    config: &CacheConfig,
    cache_write: bool,
    trusted: bool,
) -> Result<(), CacheError> {
    if cache_write && !trusted && !config.allow_untrusted_write {
        Err(CacheError::UntrustedWriteDenied)
    } else {
        Ok(())
    }
}

pub fn path_is_inside_cache_root(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CacheBackend;

    #[test]
    fn repository_namespaces_are_filesystem_safe() {
        assert!(namespace_for_repository("github:org/project").starts_with("github_org_project-"));
        assert_ne!(
            namespace_for_repository("github:org/project.name"),
            namespace_for_repository("github:org/project_name")
        );
    }

    #[test]
    fn rejects_untrusted_cache_writes_by_default() {
        let config = CacheConfig {
            enable: true,
            backend: CacheBackend::Local,
            path: "/tmp/runlet-cache".into(),
            allow_untrusted_write: false,
        };
        assert!(matches!(
            assert_untrusted_cache_write_allowed(&config, true, false),
            Err(CacheError::UntrustedWriteDenied)
        ));
    }
}
