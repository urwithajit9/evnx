//! `evnx scan hook install | uninstall | status` (C9; S11, S12).
//!
//! A managed block in the repository's `pre-commit` hook runs
//! `evnx scan --staged`; with `--pre-push`, one in `pre-push` runs
//! `evnx scan --history` over the commits being pushed (SCAN-PREPUSH). Lines
//! the hook already had are kept, a reinstall replaces the block in place, and
//! `uninstall` removes only the block.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

const BEGIN: &str = "# >>> evnx scan (managed) >>>";
const END: &str = "# <<< evnx scan <<<";

/// What the hook runs. ⚠️ A missing evnx blocks (S12), and so does a scan that
/// could not run (exit 2): a hook that quietly passes is how a check vanishes.
const BLOCK: &str = r#"# >>> evnx scan (managed) >>>
# Added by `evnx scan hook install`; `evnx scan hook uninstall` removes it.
if [ "$(git config --bool hooks.evnx-scan)" = "false" ]; then
  echo "evnx scan: skipped (git config hooks.evnx-scan is false)" >&2
elif ! command -v evnx >/dev/null 2>&1; then
  echo "evnx scan: evnx is not on PATH, so this commit was not checked and is blocked." >&2
  echo "  Install it (https://evnx.dev/install.sh), or commit with git commit --no-verify" >&2
  exit 1
else
  evnx scan --staged
  evnx_status=$?
  if [ "$evnx_status" -ne 0 ]; then
    if [ "$evnx_status" -eq 1 ]; then
      echo "evnx scan: commit blocked, secrets are staged (see above)." >&2
    else
      echo "evnx scan: commit blocked, the scan could not run (see above)." >&2
    fi
    echo "  To commit anyway: git commit --no-verify. To turn the hook off: git config hooks.evnx-scan false" >&2
    exit 1
  fi
fi
# <<< evnx scan <<<
"#;

/// The `pre-push` block (SCAN-PREPUSH). Git writes one line per ref being
/// pushed to the hook's stdin: `<local ref> <local sha> <remote ref> <remote
/// sha>`. A delete (local sha all zeros) is skipped; a new branch, or a remote
/// sha this clone does not have, is scanned as `<local sha> --not --remotes`
/// (everything no remote has); any other push, as `<remote sha>..<local sha>`.
/// With nothing on stdin (the hook run by hand) it is `@{u}..HEAD`, or
/// `HEAD --not --remotes` with no upstream.
///
/// ⚠️ The bypass (D11) is `EVNX_SCAN_BYPASS="<reason>"`. A reason is required:
/// set but empty, or only blanks, blocks the push. It is logged, with the time,
/// the author's email, the ranges and the reason, to `.git/evnx/bypass.log` by
/// this script, so it works with no evnx on `PATH`. The reason is flattened to
/// one line first, so it cannot forge a second log entry. `git push
/// --no-verify` skips the hook itself and cannot be logged.
const PUSH_BLOCK: &str = r#"# >>> evnx scan (managed) >>>
# Added by `evnx scan hook install --pre-push`; `evnx scan hook uninstall --pre-push` removes it.
evnx_ranges=""
evnx_lines=0
if [ ! -t 0 ]; then
  while read -r evnx_lref evnx_lsha evnx_rref evnx_rsha; do
    [ -n "$evnx_lsha" ] || continue
    evnx_lines=$((evnx_lines + 1))
    case "$evnx_lsha" in *[!0]*) ;; *) continue ;; esac
    evnx_known=0
    case "$evnx_rsha" in *[!0]*)
      if git cat-file -e "$evnx_rsha^{commit}" 2>/dev/null; then evnx_known=1; fi ;;
    esac
    if [ "$evnx_known" -eq 1 ]; then
      evnx_ranges="$evnx_ranges$evnx_rsha..$evnx_lsha
"
    else
      evnx_ranges="$evnx_ranges$evnx_lsha --not --remotes
"
    fi
  done
fi
if [ "$evnx_lines" -eq 0 ]; then
  if git rev-parse --verify -q '@{u}' >/dev/null 2>&1; then
    evnx_ranges='@{u}..HEAD
'
  else
    evnx_ranges='HEAD --not --remotes
'
  fi
fi
if [ -z "$evnx_ranges" ]; then
  :
elif [ "$(git config --bool hooks.evnx-scan)" = "false" ]; then
  echo "evnx scan: skipped (git config hooks.evnx-scan is false)" >&2
elif [ "${EVNX_SCAN_BYPASS+set}" = "set" ]; then
  evnx_reason=$(printf '%s' "$EVNX_SCAN_BYPASS" | tr '
	' '   ')
  if [ -z "$(printf '%s' "$evnx_reason" | tr -d '[:space:]')" ]; then
    echo "evnx scan: EVNX_SCAN_BYPASS needs a reason, so this push is blocked." >&2
    echo "  EVNX_SCAN_BYPASS=\"why this is safe\" git push" >&2
    exit 1
  fi
  evnx_log=$(git rev-parse --git-path evnx/bypass.log)
  mkdir -p "$(dirname "$evnx_log")"
  printf '%s\t%s\t%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(git config user.email)" \
    "$(printf '%s' "$evnx_ranges" | tr '\n' ' ' | sed 's/ *$//')" "$evnx_reason" >> "$evnx_log"
  echo "evnx scan: push not scanned, bypassed with a reason: $evnx_reason" >&2
  echo "  logged to $evnx_log" >&2
elif ! command -v evnx >/dev/null 2>&1; then
  echo "evnx scan: evnx is not on PATH, so this push was not checked and is blocked." >&2
  echo "  Install it (INSTALL_URL), or push with git push --no-verify" >&2
  exit 1
else
  evnx_status=0
  evnx_ifs=$IFS
  IFS='
'
  for evnx_range in $evnx_ranges; do
    IFS=$evnx_ifs
    evnx scan --history "$evnx_range"
    evnx_one=$?
    if [ "$evnx_one" -ne 0 ] && [ "$evnx_status" -ne 2 ]; then evnx_status=$evnx_one; fi
    IFS='
'
  done
  IFS=$evnx_ifs
  if [ "$evnx_status" -ne 0 ]; then
    if [ "$evnx_status" -eq 1 ]; then
      echo "evnx scan: push blocked, secrets are in the commits being pushed (see above)." >&2
    else
      echo "evnx scan: push blocked, the scan could not run (see above)." >&2
    fi
    echo "  Rotate the key, then rewrite the commits. To push anyway, with a reason that is logged:" >&2
    echo "  EVNX_SCAN_BYPASS=\"why\" git push. To turn the hook off: git config hooks.evnx-scan false" >&2
    exit 1
  fi
fi
# <<< evnx scan <<<
"#;

/// Which hook a command is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    PreCommit,
    PrePush,
}

impl Kind {
    pub fn from_flag(pre_push: bool) -> Kind {
        if pre_push {
            Kind::PrePush
        } else {
            Kind::PreCommit
        }
    }

    /// The hook's file name in the hooks folder.
    fn file(self) -> &'static str {
        match self {
            Kind::PreCommit => "pre-commit",
            Kind::PrePush => "pre-push",
        }
    }

    /// The managed block, ready to write.
    fn block(self) -> String {
        match self {
            Kind::PreCommit => BLOCK.to_string(),
            Kind::PrePush => PUSH_BLOCK.replace("INSTALL_URL", crate::core::build::INSTALL_URL),
        }
    }

    /// What the installed hook does, for the message.
    fn does(self) -> &'static str {
        match self {
            Kind::PreCommit => "`evnx scan --staged` before every commit",
            Kind::PrePush => "`evnx scan --history` on the commits of every push",
        }
    }

    /// The id of the hook for a manager's config.
    fn manager_hook(self) -> &'static str {
        match self {
            Kind::PreCommit => "evnx-scan-installed",
            Kind::PrePush => "evnx-scan-push",
        }
    }

    /// What a manager's `install` needs to write this hook.
    fn manager_install(self) -> &'static str {
        match self {
            Kind::PreCommit => "",
            Kind::PrePush => " --hook-type pre-push",
        }
    }

    /// The flag that names this hook.
    fn flag(self) -> &'static str {
        match self {
            Kind::PreCommit => "",
            Kind::PrePush => " --pre-push",
        }
    }
}

/// Hook managers that own the file outright, by the line they generate.
const MANAGERS: [(&str, &str); 2] = [
    ("# File generated by pre-commit", "pre-commit"),
    ("# File generated by prek", "prek"),
];

/// `.git/hooks/<hook>`, or wherever `core.hooksPath` and worktrees put it.
fn hook_path(cwd: &Path, kind: Kind) -> Result<PathBuf> {
    let root = super::git_blobs::toplevel(cwd, "scan hook")?;
    let out = Command::new("git")
        .args(["rev-parse", "--git-path", "hooks"])
        .current_dir(&root)
        .output()
        .context("running git")?;
    if !out.status.success() {
        bail!("git could not name the hooks folder");
    }
    let dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim_end());
    // Relative answers are relative to where git ran: the top folder.
    Ok(root.join(dir).join(kind.file()))
}

/// `text` with the managed block removed, and whether there was one.
fn without_block(text: &str) -> (String, bool) {
    let (Some(start), Some(end)) = (text.find(BEGIN), text.find(END)) else {
        return (text.to_string(), false);
    };
    let end = text[end..].find('\n').map_or(text.len(), |n| end + n + 1);
    (format!("{}{}", &text[..start], &text[end..]), true)
}

fn manager(text: &str) -> Option<&'static str> {
    MANAGERS
        .iter()
        .find(|(marker, _)| text.contains(marker))
        .map(|(_, name)| *name)
}

pub fn install(cwd: &Path, kind: Kind) -> Result<PathBuf> {
    let path = hook_path(cwd, kind)?;
    let existing = match std::fs::read_to_string(&path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if let Some(name) = existing.as_deref().and_then(manager) {
        bail!(
            "{name} manages {}; two managers on one hook fight. Add evnx to \
             .pre-commit-config.yaml instead (prek reads the same file):\n\n\
             - repo: https://github.com/urwithajit9/evnx\n  \
             rev: v{}\n  \
             hooks:\n    \
             - id: {}\n\n\
             then run `{name} install{}`.",
            path.display(),
            env!("CARGO_PKG_VERSION"),
            kind.manager_hook(),
            kind.manager_install(),
        );
    }
    let text = match &existing {
        Some(t) => {
            let (mut rest, _) = without_block(t);
            if !rest.is_empty() && !rest.ends_with('\n') {
                rest.push('\n');
            }
            rest + &kind.block()
        }
        None => format!("#!/bin/sh\n{}", kind.block()),
    };
    if let Some(dir) = path.parent() {
        crate::core::safe_write::create_dirs(dir)?;
    }
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    if existing.is_none() {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    crate::utils::ui::success(format!("{} runs {}", path.display(), kind.does()));
    Ok(path)
}

pub fn uninstall(cwd: &Path, kind: Kind) -> Result<()> {
    let path = hook_path(cwd, kind)?;
    let Ok(text) = std::fs::read_to_string(&path) else {
        crate::utils::ui::info(format!("no {}; nothing to remove", path.display()));
        return Ok(());
    };
    let (rest, had) = without_block(&text);
    if !had {
        crate::utils::ui::info(format!("{} has no evnx block", path.display()));
        return Ok(());
    }
    // Only what install itself would have written is left: the file was ours.
    if rest.trim().is_empty() || rest.trim() == "#!/bin/sh" {
        std::fs::remove_file(&path)?;
    } else {
        std::fs::write(&path, rest)?;
    }
    crate::utils::ui::success(format!("removed the evnx block from {}", path.display()));
    Ok(())
}

pub fn status(cwd: &Path, kind: Kind) -> Result<()> {
    let path = hook_path(cwd, kind)?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if without_block(&text).1 {
        println!("installed: {}", path.display());
    } else if let Some(name) = manager(&text) {
        println!(
            "not installed: {name} manages {}; add the {} hook there",
            path.display(),
            kind.manager_hook()
        );
    } else {
        println!(
            "not installed: {} (run `evnx scan hook install{}`)",
            path.display(),
            kind.flag()
        );
    }
    let off = Command::new("git")
        .args(["config", "--bool", "hooks.evnx-scan"])
        .current_dir(cwd)
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "false");
    if off {
        println!("turned off: git config hooks.evnx-scan is false");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WP-16: the message in the hook names the one install URL.
    #[test]
    fn wp16_hook_message_uses_the_install_url() {
        assert!(BLOCK.contains(&format!("({})", crate::core::build::INSTALL_URL)));
        assert!(!BLOCK.contains("www.evnx.dev"));
    }

    /// SCAN-PREPUSH: the block is the same managed block, in the same markers,
    /// so install, reinstall and uninstall treat both hooks alike.
    #[test]
    fn the_push_block_uses_the_same_markers_and_the_install_url() {
        let block = Kind::PrePush.block();
        assert!(block.starts_with(BEGIN) && block.trim_end().ends_with(END));
        assert!(block.contains(&format!("({})", crate::core::build::INSTALL_URL)));
        assert!(!block.contains("INSTALL_URL"));
        let text = format!("#!/bin/sh\necho mine\n{block}echo after\n");
        assert_eq!(
            without_block(&text),
            ("#!/bin/sh\necho mine\necho after\n".into(), true)
        );
    }

    #[test]
    fn the_block_comes_out_whole_and_nothing_else_does() {
        let text = format!("#!/bin/sh\necho mine\n{BLOCK}echo after\n");
        let (rest, had) = without_block(&text);
        assert!(had);
        assert_eq!(rest, "#!/bin/sh\necho mine\necho after\n");
        assert_eq!(without_block("echo mine\n"), ("echo mine\n".into(), false));
    }

    #[test]
    fn managers_are_known_by_their_generated_line() {
        assert_eq!(
            manager("#!/bin/sh\n# File generated by prek\n"),
            Some("prek")
        );
        assert_eq!(
            manager("#!/usr/bin/env bash\n# File generated by pre-commit: x\n"),
            Some("pre-commit")
        );
        assert_eq!(manager("#!/bin/sh\necho mine\n"), None);
    }
}
