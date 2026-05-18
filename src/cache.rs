use crate::config::CacheConfig;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
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
    fs::set_permissions(&path, fs::Permissions::from_mode(0o770)).map_err(|source| {
        CacheError::CreateNamespace {
            path: path.clone(),
            source,
        }
    })?;
    Ok(Some(CacheMount {
        namespace,
        path,
        writable: cache_write,
    }))
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
    fn prepares_group_writable_cache_namespace() {
        let directory = tempfile::tempdir().expect("tempdir should be created");
        let config = CacheConfig {
            enable: true,
            backend: CacheBackend::Local,
            path: directory.path().to_path_buf(),
        };

        let mount = prepare_cache_mount(&config, "github:org/project", false)
            .expect("cache mount should be prepared")
            .expect("cache should be enabled");

        let mode = fs::metadata(mount.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o770);
    }
}
