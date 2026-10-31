//! How a file is named in output (RM-32, I-080): `./.env`, `./sub/.env.prod`.
//!
//! One form in every command, so `scan` and `cloud pull` (which joined onto
//! `.`) and `diff`, `validate`, `doctor` and `restore` (which printed the path
//! as typed) no longer disagree about the same file. The `./` also keeps a name
//! starting with `-` from reading as a flag when it is copied into a shell.

use std::path::Path;

/// `path` as output names it. A relative path gets `./` unless it already
/// starts with `.` or `..` as a component. An absolute path, and `-` or
/// `<stdin>` for standard input, are left alone.
///
/// ⚠️ `./` on Windows too, never `.\`: `validate --format json` carries this,
/// and a report must not change with the machine that wrote it (`scan` made
/// the same choice for its machine-readable output). Windows opens `./.env`.
///
/// shortcut: an absolute path inside the current folder stays absolute; make it
/// relative if a command starts receiving absolute paths by default.
pub fn label(path: &str) -> String {
    if path.is_empty() || path == "-" || path.starts_with('<') {
        return path.to_string();
    }
    let p = Path::new(path);
    if p.is_absolute()
        || matches!(
            p.components().next(),
            Some(std::path::Component::CurDir | std::path::Component::ParentDir)
        )
    {
        return path.to_string();
    }
    format!("./{path}")
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn rm32_one_form_for_a_file() {
        assert_eq!(label(".env"), "./.env");
        assert_eq!(label("sub/.env.prod"), "./sub/.env.prod");
        assert_eq!(label("-flagish"), "./-flagish");
        for kept in ["./.env", "../.env", ".", "-", "<stdin>", ""] {
            assert_eq!(label(kept), kept);
        }
        #[cfg(unix)]
        assert_eq!(label("/etc/app/.env"), "/etc/app/.env");
    }
}
