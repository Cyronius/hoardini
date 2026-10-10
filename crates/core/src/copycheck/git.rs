use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::json;

use super::{Confidence, CopyCheck, CopyResult, Cx, Find, Found, Subject, with_note};

pub struct GitCheck;

impl GitCheck {
    pub const NAME: &'static str = "git";
}

const MAX_DEPTH: usize = 64;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// `.git` is a file, not a folder, in worktrees and submodules.
pub fn work_tree(path: &Path) -> Option<PathBuf> {
    let mut cur = Some(path);
    for _ in 0..MAX_DEPTH {
        let p = cur?;
        if p.file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case(".git"))
        {
            return None;
        }
        if p.join(".git").exists() {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}

struct Git<'a> {
    exe: &'a str,
    root: &'a Path,
}

impl Git<'_> {
    // Read-only commands, and fsmonitor off: a repo's config could otherwise run a program on `status`.
    fn run(&self, args: &[&str]) -> anyhow::Result<(bool, String)> {
        let out = Command::new(self.exe)
            .arg("--no-optional-locks")
            .arg("-C")
            .arg(self.root)
            .args(["-c", "core.fsmonitor=false", "-c", "core.quotepath=off"])
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| anyhow::anyhow!("git could not run: {e}"))?;
        Ok((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ))
    }

    fn line(&self, args: &[&str]) -> Option<String> {
        match self.run(args) {
            Ok((true, s)) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
            _ => None,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub untracked: usize,
    pub ignored: usize,
    pub changed: usize,
}

pub fn parse_status(out: &str) -> Status {
    let mut s = Status::default();
    let mut it = out.split('\0').filter(|e| !e.is_empty());
    while let Some(e) = it.next() {
        let code = e.get(..2).unwrap_or("");
        match code {
            "??" => s.untracked += 1,
            "!!" => s.ignored += 1,
            _ => {
                s.changed += 1;
                // A rename carries its old path as the next entry.
                if code.starts_with('R') || code.starts_with('C') {
                    it.next();
                }
            }
        }
    }
    s
}

// With core.ignorecase, `status` says nothing about a path whose letter case differs from its index entry.
pub fn index_trouble(listed: &str, rel: &str, dir: bool) -> Option<&'static str> {
    let entries: Vec<(char, &str)> = listed
        .split('\0')
        .filter_map(|e| {
            let (tag, name) = e.split_once(' ')?;
            Some((tag.chars().next()?, name))
        })
        .collect();
    let prefix = format!("{rel}/");
    let exact = |n: &str| {
        if dir {
            rel == "." || n.starts_with(&prefix)
        } else {
            n == rel
        }
    };
    if entries.is_empty() {
        Some("not tracked by git")
    } else if entries.iter().any(|(_, n)| !exact(n)) {
        Some("tracked by git under another letter case")
    } else if entries.iter().any(|(t, _)| *t != 'H') {
        Some("git is told to skip changes to it (assume-unchanged or skip-worktree)")
    } else {
        None
    }
}

// Credentials never leave the URL: `https://user:token@host/x.git` becomes `host/x`.
pub fn clean_remote(url: &str) -> String {
    let u = url.trim();
    let no_scheme = u.split_once("://").map_or(u, |(_, r)| r);
    let no_user = no_scheme.rsplit_once('@').map_or(no_scheme, |(_, r)| r);
    let drive = no_user.as_bytes().get(1) == Some(&b':');
    let host_path = if !u.contains("://") && no_user.contains(':') && !drive {
        no_user.replacen(':', "/", 1)
    } else {
        no_user.to_string()
    };
    host_path
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_string()
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .map(|r| r.to_string_lossy().replace('\\', "/"))
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| ".".into())
}

impl CopyCheck for GitCheck {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn applies(&self, _cx: &Cx<'_>, s: &Subject) -> bool {
        work_tree(Path::new(&s.path)).is_some()
    }

    fn covers_dir(&self, _cx: &Cx<'_>, path: &str) -> bool {
        Path::new(path).join(".git").exists()
    }

    // A push, a commit or a fetch changes the answer without touching the file, so it is asked every pass.
    fn still_valid(&self, _cx: &Cx<'_>, _s: &Subject, _r: &CopyResult) -> bool {
        false
    }

    fn find(&self, cx: &Cx<'_>, s: &Subject) -> anyhow::Result<Find> {
        let path = Path::new(&s.path);
        let Some(root) = work_tree(path) else {
            return Ok(Find::Skip);
        };
        let git = Git {
            exe: &cx.env.git,
            root: &root,
        };
        let rel = rel(&root, path);
        let spec = format!(":(literal){rel}");
        let (ok, out) = git.run(&[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
            "--",
            &spec,
        ])?;
        if !ok {
            anyhow::bail!("git status failed in {}", root.display());
        }
        let st = parse_status(&out);
        let root_s = root.to_string_lossy().into_owned();
        if !s.dir {
            if st.ignored > 0 {
                return Ok(Find::Skip);
            }
            if st.untracked > 0 {
                return Ok(Find::NoCopy(with_note(
                    json!({"root": root_s}),
                    "not tracked by git",
                )));
            }
            if st.changed > 0 {
                return Ok(Find::NoCopy(with_note(
                    json!({"root": root_s}),
                    "changed since the last commit",
                )));
            }
        } else if st != Status::default() {
            let note = format!(
                "{} untracked, {} ignored and {} changed path(s) are not in git",
                st.untracked, st.ignored, st.changed
            );
            return Ok(Find::NoCopy(with_note(
                json!({"root": root_s, "untracked": st.untracked, "ignored": st.ignored, "changed": st.changed}),
                note,
            )));
        }
        let icase = format!(":(literal,icase){rel}");
        let (ok, listed) = git.run(&["ls-files", "-z", "-v", "--", &icase])?;
        if !ok {
            anyhow::bail!("git ls-files failed in {}", root.display());
        }
        if let Some(note) = index_trouble(&listed, &rel, s.dir) {
            return Ok(Find::NoCopy(with_note(json!({"root": root_s}), note)));
        }
        let Some(upstream) =
            git.line(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        else {
            return Ok(Find::NoCopy(with_note(
                json!({"root": root_s}),
                "the branch has no upstream, so nothing is pushed",
            )));
        };
        let ahead: u64 = git
            .line(&["rev-list", "--count", "@{u}..HEAD"])
            .and_then(|n| n.parse().ok())
            .unwrap_or(u64::MAX);
        if ahead > 0 {
            let (same, _) = git.run(&[
                "diff",
                "--quiet",
                "--no-ext-diff",
                "--no-textconv",
                "@{u}",
                "HEAD",
                "--",
                &spec,
            ])?;
            if !same {
                let note = format!("changed in {ahead} commit(s) not pushed to {upstream}");
                return Ok(Find::NoCopy(with_note(
                    json!({"root": root_s, "upstream": upstream, "ahead": ahead}),
                    note,
                )));
            }
        }
        let remote = upstream.split('/').next().unwrap_or("origin").to_string();
        let url = git
            .line(&["config", "--get", &format!("remote.{remote}.url")])
            .map(|u| clean_remote(&u));
        Ok(Find::Copy(Found {
            locator: upstream.clone(),
            detail: json!({"root": root_s, "upstream": upstream, "url": url, "ahead": ahead}),
        }))
    }

    fn verify(&self, _cx: &Cx<'_>, s: &Subject, f: Found) -> anyhow::Result<CopyResult> {
        let what = if s.dir {
            "everything tracked is pushed to"
        } else {
            "tracked and pushed to"
        };
        let note = match f.detail["url"].as_str() {
            Some(u) => format!("{what} {} ({u})", f.locator),
            None => format!("{what} {}", f.locator),
        };
        Ok(CopyResult::new(
            Self::NAME,
            Some(f.locator),
            Confidence::Identical,
            with_note(f.detail, note),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Traces: EVID-COPY-GIT
    #[test]
    fn status_entries_are_counted_by_kind() {
        let s = parse_status("?? a.bin\0!! target/\0 M src/x.rs\0R  new.rs\0old.rs\0");
        assert_eq!(
            s,
            Status {
                untracked: 1,
                ignored: 1,
                changed: 2
            }
        );
        assert_eq!(parse_status(""), Status::default());
    }

    // Traces: EVID-COPY-GIT
    #[test]
    fn index_entries_must_match_case_and_carry_no_skip_flag() {
        assert_eq!(index_trouble("H a/b.bin\0", "a/b.bin", false), None);
        assert_eq!(
            index_trouble("H a/b.bin\0", "A/b.bin", false),
            Some("tracked by git under another letter case")
        );
        assert_eq!(index_trouble("H sub/x\0H sub/y\0", "sub", true), None);
        assert!(index_trouble("H sub/x\0H Sub/y\0", "sub", true).is_some());
        assert!(index_trouble("h sub/x\0", "sub", true).is_some());
        assert!(index_trouble("S x\0", "x", false).is_some());
        assert_eq!(index_trouble("", "x", false), Some("not tracked by git"));
        assert_eq!(index_trouble("H x\0H y/z\0", ".", true), None);
    }

    // Traces: EVID-COPY-GIT
    #[test]
    fn remote_urls_lose_credentials_and_suffixes() {
        assert_eq!(
            clean_remote("https://user:ghp_secret@github.com/org/repo.git"),
            "github.com/org/repo"
        );
        assert_eq!(
            clean_remote("git@github.com:org/repo.git"),
            "github.com/org/repo"
        );
        assert_eq!(clean_remote(r"C:\remotes\repo.git"), r"C:\remotes\repo");
        assert_eq!(clean_remote("C:/remotes/repo.git"), "C:/remotes/repo");
    }
}
