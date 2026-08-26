//! Microsandbox runtime — every `run_container` boots a fresh microVM via
//! the `msb` CLI (<https://github.com/superradcompany/microsandbox>).
//!
//! Compared to Docker this gives hardware-level isolation per step with
//! sub-second boots (libkrun on macOS/Apple Silicon, KVM on Linux) and no
//! daemon dependency. Compared to emulation it contains the blast radius:
//! nothing a workflow does can touch the host.
//!
//! Semantics mirror `DockerRuntime`: `working_dir` is the container-visible
//! path (the volume list carries the host mapping), an empty `cmd` runs the
//! image's own ENTRYPOINT/CMD, and stdout/stderr/exit code are returned
//! verbatim. Known limits: no Dockerfile builds (msb consumes OCI images),
//! and `runs.entrypoint` overrides are best-effort because msb preserves the
//! image's entrypoint.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use wrkflw_runtime::container::{ContainerError, ContainerOutput, ContainerRuntime};

pub struct MicrosandboxRuntime;

impl Default for MicrosandboxRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl MicrosandboxRuntime {
    pub fn new() -> Self {
        MicrosandboxRuntime
    }

    /// Whether the `msb` CLI is present and answers.
    pub fn is_available() -> bool {
        std::process::Command::new("msb")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// msb mounts refuse symlinked sources (macOS `/tmp` -> `/private/tmp`),
    /// so host paths are canonicalized before mounting.
    fn canonical_host(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }
}

#[async_trait]
impl ContainerRuntime for MicrosandboxRuntime {
    async fn run_container(
        &self,
        image: &str,
        cmd: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
        volumes: &[(&Path, &Path)],
        entrypoint: Option<&str>,
    ) -> Result<ContainerOutput, ContainerError> {
        let mut args: Vec<String> = vec!["run".to_string(), image.to_string()];

        for (key, value) in env_vars {
            // PATH accumulates guest-side additions (GITHUB_PATH entries from
            // setup actions) plus host noise; the standard bins are appended
            // so shells inside the VM always resolve, and host-only segments
            // stay inert.
            if *key == "PATH" {
                args.push("-e".to_string());
                args.push(format!(
                    "PATH={}:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                    value
                ));
                continue;
            }
            args.push("-e".to_string());
            args.push(format!("{}={}", key, value));
        }
        for (host, container) in volumes {
            let host = Self::canonical_host(host);
            // msb distinguishes file mounts from directory mounts.
            let flag = if host.is_file() {
                "--mount-file"
            } else {
                "--mount-dir"
            };
            args.push(flag.to_string());
            args.push(format!(
                "{}:{}",
                host.to_string_lossy(),
                container.to_string_lossy()
            ));
        }
        args.push("-w".to_string());
        args.push(working_dir.to_string_lossy().to_string());

        // msb replaces the image CMD but keeps its entrypoint — a
        // `runs.entrypoint` override can only be approximated by running it
        // as the command. Correct for entrypoint-less images, warned about
        // otherwise.
        let mut command: Vec<String> = Vec::new();
        if let Some(ep) = entrypoint.filter(|s| !s.is_empty()) {
            wrkflw_logging::warning(&format!(
                "microsandbox cannot replace the image entrypoint — running '{}' as the command",
                ep
            ));
            command.push(ep.to_string());
        }
        command.extend(cmd.iter().map(|s| s.to_string()));
        if !command.is_empty() {
            args.push("--".to_string());
            args.extend(command);
        }

        wrkflw_logging::info(&format!(
            "microsandbox: booting microVM from '{}' ({} mounts)",
            image,
            volumes.len()
        ));
        // stdin is nulled: under the TUI the terminal writes mouse reports
        // to the app's stdin, and an inheriting child echoes them straight
        // into its captured output.
        let output = tokio::process::Command::new("msb")
            .args(&args)
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .map_err(|e| {
                ContainerError::ContainerStart(format!("Failed to execute msb: {}", e))
            })?;

        Ok(ContainerOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    async fn pull_image(&self, image: &str) -> Result<(), ContainerError> {
        let output = tokio::process::Command::new("msb")
            .args(["pull", image])
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .map_err(|e| ContainerError::ImagePull(format!("Failed to execute msb: {}", e)))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(ContainerError::ImagePull(format!(
                "msb pull {} failed: {}",
                image,
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    async fn build_image(
        &self,
        _dockerfile: &Path,
        tag: &str,
        _context_dir: &Path,
    ) -> Result<(), ContainerError> {
        Err(ContainerError::ImageBuild(format!(
            "the microsandbox runtime cannot build Dockerfiles (wanted '{}') — \
             it consumes OCI images from registries. Use --runtime docker for \
             workflows with Dockerfile-based actions.",
            tag
        )))
    }

    async fn prepare_language_environment(
        &self,
        language: &str,
        version: Option<&str>,
        _additional_packages: Option<Vec<String>>,
    ) -> Result<String, ContainerError> {
        // Same policy as emulation: pick a stock image, no custom builds.
        let base_image = match language {
            "python" => version.map_or("python:3.11-slim".to_string(), |v| format!("python:{}", v)),
            "node" => version.map_or("node:20-slim".to_string(), |v| format!("node:{}", v)),
            "java" => version.map_or("eclipse-temurin:17-jdk".to_string(), |v| {
                format!("eclipse-temurin:{}", v)
            }),
            "go" => version.map_or("golang:1.21-slim".to_string(), |v| format!("golang:{}", v)),
            "dotnet" => version.map_or("mcr.microsoft.com/dotnet/sdk:7.0".to_string(), |v| {
                format!("mcr.microsoft.com/dotnet/sdk:{}", v)
            }),
            "rust" => version.map_or("rust:latest".to_string(), |v| format!("rust:{}", v)),
            _ => {
                return Err(ContainerError::ContainerStart(format!(
                    "Unsupported language: {}",
                    language
                )))
            }
        };
        Ok(base_image)
    }

    async fn image_exists(&self, tag: &str) -> Result<bool, ContainerError> {
        let output = tokio::process::Command::new("msb")
            .args(["image", "ls"])
            .output()
            .await
            .map_err(|e| ContainerError::ImagePull(format!("Failed to execute msb: {}", e)))?;
        if !output.status.success() {
            return Ok(false);
        }
        let listing = String::from_utf8_lossy(&output.stdout);
        // Match on the repository[:tag] appearing in the listing; `msb pull`
        // is cheap on cache hits, so a false negative only costs a no-op.
        Ok(listing.contains(tag))
    }
}
