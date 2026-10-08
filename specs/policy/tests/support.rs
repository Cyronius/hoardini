// Shared fixture for the policy, act, and agent_approval test binaries (included with #[path]).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hoardini_core::actions::{Executor, Job};
use hoardini_core::destinations::{self, Destination, NewDestination};
use hoardini_core::index::{RefreshOptions, WinLister, refresh};
use hoardini_core::policy::{PolicyEnv, Verdict};
use hoardini_core::proposals::{self, NewItem, NewProposal, Proposal, ProposalItem, ProposalKind};
use hoardini_core::store::Db;
use hoardini_core::vault::DpapiVault;

pub struct Fx {
    pub tmp: tempfile::TempDir,
    pub profile: PathBuf,
    pub data: PathBuf,
    pub work: PathBuf,
    pub db: Db,
    pub env: PolicyEnv,
}

pub fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

// Deterministic, poorly compressible bytes.
pub fn data(n: usize, seed: u8) -> Vec<u8> {
    let mut x = 0x9E37_79B9_u32 ^ u32::from(seed);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8
        })
        .collect()
}

impl Fx {
    pub fn new() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let root = PathBuf::from(s(&root).trim_start_matches(r"\\?\"));
        let profile = root.join("profile");
        let data = root.join("data");
        let work = root.join("work");
        for d in [&profile, &data, &work] {
            std::fs::create_dir_all(d).unwrap();
        }
        let db = Db::open(data.join("hoardini.db")).unwrap();
        let env = PolicyEnv::with_paths(&profile, &data);
        Fx {
            tmp,
            profile,
            data,
            work,
            db,
            env,
        }
    }

    pub fn root(&self) -> PathBuf {
        self.work.parent().unwrap().to_path_buf()
    }

    pub fn file(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        write(&self.work.join(rel), bytes)
    }

    pub fn pfile(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        write(&self.profile.join(rel), bytes)
    }

    pub fn executor(&self) -> Executor {
        let vault = DpapiVault::open(&self.data.join("secrets.vault")).unwrap();
        Executor::with_policy(self.db.clone(), Arc::new(vault), self.env.clone())
    }

    pub fn request(&self, kind: ProposalKind, paths: &[&Path], dest: Option<&str>) -> NewProposal {
        NewProposal {
            conversation_id: Some("conv-1".into()),
            tool_call_id: Some("call-1".into()),
            kind,
            destination_id: dest.map(str::to_string),
            format: None,
            dedupe: None,
            summary: "test proposal".into(),
            items: paths
                .iter()
                .map(|p| NewItem {
                    path: s(p),
                    verdict: Some(Verdict::Safe),
                    reason: None,
                })
                .collect(),
        }
    }

    pub fn propose(
        &self,
        kind: ProposalKind,
        paths: &[&Path],
        dest: Option<&str>,
    ) -> Result<Proposal, String> {
        proposals::create_with(&self.db, &self.env, self.request(kind, paths, dest))
            .map_err(|e| e.to_string())
    }

    pub fn local_destination(&self, name: &str) -> (Destination, PathBuf) {
        let dir = self.root().join(format!("dest-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let d = destinations::create(
            &self.db.conn().unwrap(),
            NewDestination {
                name: name.into(),
                kind: "local".into(),
                config: serde_json::json!({ "path": s(&dir) }),
            },
        )
        .unwrap();
        (d, dir)
    }

    pub fn index_work(&self) {
        let opts = RefreshOptions {
            threshold: Some(0),
            hotspots_first: false,
            ..RefreshOptions::default()
        };
        let out = refresh(&self.db.conn().unwrap(), &WinLister, &s(&self.work), &opts).unwrap();
        assert!(out.completed);
    }

    pub fn get(&self, id: &str) -> Proposal {
        proposals::get(&self.db, id).unwrap().unwrap()
    }
}

pub fn write(path: &Path, bytes: &[u8]) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
    path.to_path_buf()
}

pub fn item<'a>(p: &'a Proposal, path: &Path) -> &'a ProposalItem {
    let want = s(path);
    p.items
        .iter()
        .find(|i| i.path.eq_ignore_ascii_case(&want))
        .unwrap_or_else(|| panic!("{want} not in proposal: {:#?}", p.items))
}

pub fn ids(p: &Proposal) -> Vec<i64> {
    p.items.iter().map(|i| i.id).collect()
}

pub fn wait(ex: &Executor, job_id: &str) -> Job {
    let start = Instant::now();
    loop {
        if let Some(j) = ex.jobs().into_iter().find(|j| j.id == job_id)
            && matches!(j.status.as_str(), "done" | "failed" | "cancelled")
        {
            return j;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "job {job_id} did not finish"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn count(db: &Db, sql: &str) -> i64 {
    db.conn().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
}
