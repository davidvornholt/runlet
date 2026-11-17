use crate::config::CacheConfig;
use sha2::{Digest, Sha256};
use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("failed to create cache namespace {path}: {source}")]
    CreateNamespace {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "cache root {path} has mode {mode:o}, so other host users could change cached files; make it 0770 or stricter"
    )]
    UnprotectedRoot { path: PathBuf, mode: u32 },
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
    let create_error = |source| CacheError::CreateNamespace {
        path: path.clone(),
        source,
    };
    DirBuilder::new()
        .recursive(true)
        .mode(0o770)
        .create(&config.path)
        .map_err(create_error)?;
    let root_mode = fs::metadata(&config.path)
        .map_err(create_error)?
        .permissions()
        .mode()
        & 0o777;
    if root_mode & 0o007 != 0 {
        return Err(CacheError::UnprotectedRoot {
            path: config.path.clone(),
            mode: root_mode,
        });
    }
    fs::create_dir_all(&path).map_err(create_error)?;
    // The container's runner user maps to a subordinate ID, so it can only
    // write here through the "other" bits. The private cache root keeps
    // other host users out.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).map_err(create_error)?;
    Ok(Some(CacheMount {
        namespace,
        path,
        writable: cache_write,
    }))
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

    fn cache_config(path: PathBuf) -> CacheConfig {
        CacheConfig {
            enable: true,
            backend: CacheBackend::Local,
            path,
        }
    }

    #[test]
    fn prepares_cache_namespace_writable_by_mapped_runner_user() {
        let directory = tempfile::tempdir().expect("tempdir should be created");
        let config = cache_config(directory.path().join("cache"));

        let mount = prepare_cache_mount(&config, "github:org/project", false)
            .expect("cache mount should be prepared")
            .expect("cache should be enabled");

        let mode = fs::metadata(mount.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o777);
        let root_mode = fs::metadata(&config.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            root_mode & 0o007,
            0,
            "a new cache root must keep other users out"
        );
    }

    #[test]
    fn refuses_cache_root_other_users_can_enter() {
        let directory = tempfile::tempdir().expect("tempdir should be created");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let config = cache_config(directory.path().to_path_buf());

        let error = prepare_cache_mount(&config, "github:org/project", true)
            .expect_err("a cache root other users can enter must be refused");

        assert!(matches!(
            error,
            CacheError::UnprotectedRoot { mode: 0o755, .. }
        ));
        assert!(!config
            .path
            .join(namespace_for_repository("github:org/project"))
            .exists());
    }
}
