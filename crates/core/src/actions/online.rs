use std::path::Path;
use std::sync::Arc;

use super::handler::{Handler, ItemCx, JobCx, Outcome, Runner};
use crate::cloud::{Cloud, root_for};
use crate::policy::ItemKind;

pub(crate) struct OnlineOnlyHandler;

impl Handler for OnlineOnlyHandler {
    fn begin(&self, job: &JobCx<'_>) -> anyhow::Result<Box<dyn Runner>> {
        Ok(Box::new(OnlineOnlyRunner {
            cloud: job.env.cloud().clone(),
        }))
    }
}

struct OnlineOnlyRunner {
    cloud: Arc<dyn Cloud>,
}

#[derive(Default)]
struct Tally {
    freed: u64,
    files: usize,
    already: usize,
    not_in_sync: usize,
    failed: usize,
    last_error: Option<String>,
}

impl OnlineOnlyRunner {
    fn one(&self, cx: &mut ItemCx<'_>, p: &Path, t: &mut Tally) {
        match self.cloud.state(p) {
            Ok(Some(st)) if st.in_sync => {
                if st.local_bytes == 0 {
                    t.already += 1;
                    return;
                }
                match self.cloud.free(p) {
                    Ok(n) => {
                        t.freed += n;
                        t.files += 1;
                        (cx.progress)(crate::index::to_i64(n));
                    }
                    Err(e) => {
                        t.failed += 1;
                        t.last_error = Some(format!("{}: {e}", p.display()));
                    }
                }
            }
            _ => t.not_in_sync += 1,
        }
    }

    fn walk(&self, cx: &mut ItemCx<'_>, dir: &Path, t: &mut Tally) -> anyhow::Result<()> {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in hoardini_win::list_dir(&d)? {
                cx.check_cancel()?;
                let p = d.join(&e.name);
                if e.is_link() || crate::cloud::never_synced(&e.name) {
                    continue;
                }
                if e.is_dir() {
                    stack.push(p);
                } else {
                    self.one(cx, &p, t);
                }
            }
        }
        Ok(())
    }
}

impl Runner for OnlineOnlyRunner {
    fn run(&mut self, cx: &mut ItemCx<'_>) -> anyhow::Result<Outcome> {
        cx.check_cancel()?;
        let roots = self.cloud.roots();
        let Some(root) = root_for(&roots, &cx.item.path) else {
            anyhow::bail!("not in a cloud-synced folder; nothing was changed");
        };
        let provider = root.provider.clone();
        let path = Path::new(&cx.item.path);
        let mut t = Tally::default();
        let walked = match cx.item.kind {
            ItemKind::File => {
                self.one(cx, path, &mut t);
                Ok(())
            }
            ItemKind::Dir => self.walk(cx, path, &mut t),
            ItemKind::Link => anyhow::bail!("a link holds no data"),
        };
        cx.freed = crate::index::to_i64(t.freed);
        cx.rescan = true;
        walked?;
        if t.files == 0 && t.already == 0 {
            if let Some(e) = t.last_error {
                anyhow::bail!("could not make it online-only: {e}");
            }
            anyhow::bail!("nothing here is in sync with {provider} yet; nothing was changed");
        }
        let mut detail = format!("{} file(s) now online-only in {provider}", t.files);
        if t.not_in_sync > 0 {
            detail.push_str(&format!(
                "; {} not in sync were left on this PC",
                t.not_in_sync
            ));
        }
        if t.failed > 0 {
            detail.push_str(&format!("; {} could not be freed", t.failed));
        }
        Ok(Outcome {
            undo: serde_json::json!({
                "kind": "pin",
                "provider": provider,
                "how": "Right-click it and choose Always keep on this device",
            }),
            detail: Some(detail),
        })
    }
}
