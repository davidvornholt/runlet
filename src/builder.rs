use crate::process::ProcessSpec;
use std::ffi::OsString;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageBuilder {
    Podman,
    Buildah,
    BuildKit,
    Nix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageBuildRequest {
    pub builder: ImageBuilder,
    pub context: PathBuf,
    pub tag: String,
    pub push: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ImageBuildError {
    #[error("image build context must not be empty")]
    EmptyContext,
    #[error("image tag must not be empty")]
    EmptyTag,
}

pub fn build_image(request: &ImageBuildRequest) -> Result<ProcessSpec, ImageBuildError> {
    if request.context.as_os_str().is_empty() {
        return Err(ImageBuildError::EmptyContext);
    }
    if request.tag.is_empty() {
        return Err(ImageBuildError::EmptyTag);
    }

    let spec = match request.builder {
        ImageBuilder::Podman => ProcessSpec {
            program: OsString::from("podman"),
            args: vec![
                "build".into(),
                "--tag".into(),
                request.tag.clone().into(),
                request.context.as_os_str().to_os_string(),
            ],
        },
        ImageBuilder::Buildah => ProcessSpec {
            program: OsString::from("buildah"),
            args: vec![
                "bud".into(),
                "--tag".into(),
                request.tag.clone().into(),
                request.context.as_os_str().to_os_string(),
            ],
        },
        ImageBuilder::BuildKit => {
            let push = request.push.is_some();
            let name = request.push.as_ref().unwrap_or(&request.tag);
            ProcessSpec {
                program: OsString::from("buildctl"),
                args: vec![
                    "build".into(),
                    "--frontend".into(),
                    "dockerfile.v0".into(),
                    "--local".into(),
                    format!("context={}", request.context.display()).into(),
                    "--local".into(),
                    format!("dockerfile={}", request.context.display()).into(),
                    "--output".into(),
                    format!("type=image,name={name},push={push}").into(),
                ],
            }
        }
        ImageBuilder::Nix => ProcessSpec {
            program: OsString::from("nix"),
            args: vec![
                "build".into(),
                format!("{}#{}", request.context.display(), request.tag).into(),
            ],
        },
    };
    Ok(spec)
}

pub fn push_image(source: &str, destination: &str) -> Option<ProcessSpec> {
    if source.is_empty() || destination.is_empty() {
        return None;
    }
    Some(ProcessSpec {
        program: OsString::from("podman"),
        args: vec!["push".into(), source.into(), destination.into()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn podman_build_does_not_use_docker_socket_or_shell() {
        let spec = build_image(&ImageBuildRequest {
            builder: ImageBuilder::Podman,
            context: ".".into(),
            tag: "ghcr.io/org/app:latest".to_string(),
            push: None,
        })
        .unwrap();

        assert_eq!(spec.program, "podman");
        assert!(!spec.args.iter().any(|arg| arg == "/var/run/docker.sock"));
        assert!(!spec.args.iter().any(|arg| arg == "sh"));
        assert!(!spec.args.iter().any(|arg| arg == "-c"));
    }

    #[test]
    fn buildkit_push_uses_requested_destination() {
        let spec = build_image(&ImageBuildRequest {
            builder: ImageBuilder::BuildKit,
            context: ".".into(),
            tag: "ghcr.io/org/app:local".to_string(),
            push: Some("ghcr.io/org/app:latest".to_string()),
        })
        .unwrap();

        assert!(spec
            .args
            .iter()
            .any(|arg| arg == "type=image,name=ghcr.io/org/app:latest,push=true"));
        assert!(!spec
            .args
            .iter()
            .any(|arg| arg == "type=image,name=ghcr.io/org/app:local,push=true"));
    }

    #[test]
    fn push_uses_source_and_destination_without_docker_socket() {
        let spec = push_image("ghcr.io/org/app:local", "ghcr.io/org/app:latest").unwrap();

        assert_eq!(spec.program, "podman");
        assert_eq!(
            spec.args,
            ["push", "ghcr.io/org/app:local", "ghcr.io/org/app:latest"]
        );
        assert!(!spec.args.iter().any(|arg| arg == "/var/run/docker.sock"));
    }
}
