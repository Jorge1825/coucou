// What a finished Claude Code run changed, from git: files, lines added and
// removed, new files. Read-only (`git diff --numstat`, `git ls-files`), run
// directly — never through a shell — in the session's own folder.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// File names sent to the island; the counts cover all of them.
const MAX_NAMES: usize = 8;

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitSummary {
    pub files: u32,
    pub insertions: u32,
    pub deletions: u32,
    /// New files not yet added to git.
    pub untracked: u32,
    pub names: Vec<String>,
}

fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let exe = crate::find_on_path("git")?;
    let out = Command::new(exe)
        .arg("-C")
        .arg(cwd)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `None` when the folder is not a git repository (or git is not installed).
pub fn summary(cwd: &str) -> Option<GitSummary> {
    let dir = Path::new(cwd);
    if cwd.is_empty() || !dir.is_dir() {
        return None;
    }
    // Against HEAD covers staged and unstaged changes; a repository without any
    // commit yet has no HEAD, so fall back to the working tree.
    let numstat = git(dir, &["diff", "HEAD", "--numstat", "--no-ext-diff", "--no-color"])
        .or_else(|| git(dir, &["diff", "--numstat", "--no-ext-diff", "--no-color"]))?;
    let mut s = parse_numstat(&numstat);
    if let Some(untracked) = git(dir, &["ls-files", "--others", "--exclude-standard"]) {
        for name in untracked.lines().filter(|l| !l.is_empty()) {
            s.untracked += 1;
            if s.names.len() < MAX_NAMES {
                s.names.push(name.to_string());
            }
        }
    }
    Some(s)
}

fn parse_numstat(text: &str) -> GitSummary {
    let mut s = GitSummary::default();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(add), Some(del), Some(name)) = (parts.next(), parts.next(), parts.next()) else { continue };
        s.files += 1;
        // Binary files show "-" for both counts.
        s.insertions += add.parse::<u32>().unwrap_or(0);
        s.deletions += del.parse::<u32>().unwrap_or(0);
        if s.names.len() < MAX_NAMES {
            s.names.push(name.to_string());
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numstat_counts_lines_and_binaries() {
        let s = parse_numstat("12\t3\tsrc/a.rs\n-\t-\tlogo.png\n0\t7\tREADME.md\n");
        assert_eq!((s.files, s.insertions, s.deletions), (3, 12, 10));
        assert_eq!(s.names, vec!["src/a.rs", "logo.png", "README.md"]);
    }
}
