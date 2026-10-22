//! What a project is built with, as files on disk show it (DEC-VA3).
//!
//! ⚠️ One list of Docker files. `validate` (its `localhost_in_docker` check)
//! and `doctor` (its `docker_config` check) each kept their own, and neither
//! knew `compose.yaml` — the name Docker Compose v2 looks for first — so a
//! project whose only Docker file was `compose.yaml` was "not a Docker project"
//! to both (I-257). `validate` also looked only in the current directory, so
//! `evnx validate --env packages/api/.env` from the repository root missed the
//! Dockerfile beside the file it was checking (I-046).
//!
//! `init`'s detector reads the compose files among them for their services
//! ([`compose_files`]); it kept its own list, without `compose.yaml`, until
//! 0.10.16 (I-257).

use std::path::{Path, PathBuf};

/// Files whose presence means the project runs in containers, in the order
/// they are reported.
pub const DOCKER_FILES: &[&str] = &[
    "compose.yaml",
    "compose.yml",
    "docker-compose.yml",
    "docker-compose.yaml",
    "Dockerfile",
    "Containerfile",
    ".dockerignore",
];

/// The compose files among [`DOCKER_FILES`], in the order Docker Compose
/// itself prefers them: what `init --detect` reads for services.
pub fn compose_files() -> &'static [&'static str] {
    &DOCKER_FILES[..4]
}

/// The [`DOCKER_FILES`] present in `dir`, in list order.
pub fn docker_files_in(dir: &Path) -> Vec<&'static str> {
    DOCKER_FILES
        .iter()
        .copied()
        .filter(|f| dir.join(f).is_file())
        .collect()
}

/// `start` and the folders above it, up to and including the first one that
/// holds `.git` — the project boundary `.evnx.toml` discovery uses too, so a
/// file in a parent repository is never taken for this project's.
///
/// Without a repository the walk reaches the filesystem root, as
/// `.evnx.toml` discovery does (and as `docker compose` itself searches).
pub fn upward(start: &Path) -> Vec<PathBuf> {
    // Absolute first: `Path::new(".").parent()` is `""`, which would end the
    // walk after one folder (see `core::config`'s walk for the same note).
    let start = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
    let mut out = Vec::new();
    let mut dir = Some(start.as_path());
    while let Some(d) = dir {
        out.push(d.to_path_buf());
        if d.join(".git").exists() {
            break;
        }
        dir = d.parent();
    }
    out
}

/// The nearest folder at or above `start`, within the project, that holds a
/// Docker file — the env file's own folder first, then each parent up to the
/// repository root (I-046).
pub fn docker_dir(start: &Path) -> Option<PathBuf> {
    upward(start)
        .into_iter()
        .find(|d| !docker_files_in(d).is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// I-257: the compose names, all four, and nothing else from the list.
    #[test]
    fn t_i257_compose_files_are_the_four_compose_names() {
        assert_eq!(
            compose_files(),
            [
                "compose.yaml",
                "compose.yml",
                "docker-compose.yml",
                "docker-compose.yaml"
            ]
        );
    }

    #[test]
    fn t_i257_compose_v2_names_are_docker_files() {
        for name in ["compose.yaml", "compose.yml"] {
            let d = TempDir::new().unwrap();
            fs::write(d.path().join(name), "services: {}\n").unwrap();
            assert_eq!(docker_files_in(d.path()), [name]);
        }
    }

    /// Doctor prints the list in this order; the files it knew before keep
    /// their relative order, so its output only changes where a new name is.
    #[test]
    fn the_older_names_keep_their_order() {
        let d = TempDir::new().unwrap();
        for f in ["Dockerfile", ".dockerignore", "docker-compose.yml"] {
            fs::write(d.path().join(f), "").unwrap();
        }
        assert_eq!(
            docker_files_in(d.path()),
            ["docker-compose.yml", "Dockerfile", ".dockerignore"]
        );
    }

    #[test]
    fn a_directory_with_a_docker_name_is_not_a_docker_file() {
        let d = TempDir::new().unwrap();
        fs::create_dir(d.path().join("Dockerfile")).unwrap();
        assert!(docker_files_in(d.path()).is_empty());
    }

    #[test]
    fn t_i046_the_walk_stops_at_the_repository_root() {
        let d = TempDir::new().unwrap();
        let repo = d.path().join("repo");
        let sub = repo.join("packages/api");
        fs::create_dir_all(&sub).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();
        // Above the repository: must not count.
        fs::write(d.path().join("Dockerfile"), "").unwrap();
        assert_eq!(docker_dir(&sub), None);

        fs::write(repo.join("compose.yaml"), "").unwrap();
        assert_eq!(docker_dir(&sub), Some(repo.clone()));

        // The env file's own folder wins over the root.
        fs::write(sub.join("Dockerfile"), "").unwrap();
        assert_eq!(docker_dir(&sub), Some(sub));
    }
}
