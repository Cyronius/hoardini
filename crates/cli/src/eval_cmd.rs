use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
use hoardini_agent::eval::{self, Options, RamProbe, Report};
use hoardini_agent::provider::ProviderConfig;
use hoardini_core::runtime::{self, Pinned};
use hoardini_core::store::{Db, now_ms};
use hoardini_core::vault::{DpapiVault, SecretStore};
use hoardini_server::builtin::{self, Builtin, LlamaLauncher};

#[derive(Args)]
pub struct EvalArgs {
    #[arg(
        long,
        help = "OpenAI-compatible base URL, e.g. http://localhost:1234/v1"
    )]
    base_url: Option<String>,
    #[arg(
        long,
        help = "Model id (with --provider-id builtin: a pinned model id)"
    )]
    model: Option<String>,
    #[arg(
        long,
        help = "Use the small-model profile (reduced toolset, short prompt)"
    )]
    small: bool,
    #[arg(
        long,
        help = "\"builtin\" runs a pinned model in the llama-server child; another id evaluates that saved provider"
    )]
    provider_id: Option<String>,
    #[arg(long, help = "Write the full results as JSON")]
    out: Option<PathBuf>,
    #[arg(long, value_delimiter = ',', help = "Only these case ids")]
    cases: Vec<String>,
    #[arg(long, default_value_t = 1, help = "Run every case this many times")]
    repeat: u32,
    #[arg(long, help = "Name in the results table [default: the model id]")]
    label: Option<String>,
    #[arg(long, default_value_t = 600, help = "Seconds one chat turn may take")]
    turn_timeout: u64,
    #[arg(long, num_args = 1.., help = "Print one table for these saved results and exit")]
    compare: Vec<PathBuf>,
    #[arg(
        long,
        num_args = 1..,
        help = "Score these saved results again with the current rules (rewrites the files) and exit"
    )]
    rescore: Vec<PathBuf>,
    #[arg(long, help = "Keep the fixture folder and database afterwards")]
    keep: bool,
    #[arg(long, hide = true)]
    fixture_root: Option<String>,
    #[arg(long, hide = true)]
    work_dir: Option<PathBuf>,
}

const PROFILE_REL: &str = r"Users\ana";

fn load(p: &Path) -> Result<Report> {
    let text = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", p.display()))
}

// Size facts come from the fixture index, so a throwaway copy is built to read them.
fn scratch_sizes() -> Result<eval::Sizes> {
    let dir = std::env::temp_dir().join(format!(
        "hoardini-rescore-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let root = dir.join("root");
    std::fs::create_dir_all(&root)?;
    let db = Db::open(dir.join("eval.db"))?;
    let built = eval::build_fixture(&root.to_string_lossy(), &db);
    let sizes = built.and_then(|b| eval::fixture_sizes(&db, &b.fixture));
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
    sizes
}

pub fn run(db: Option<PathBuf>, a: EvalArgs) -> Result<ExitCode> {
    if !a.rescore.is_empty() {
        let sizes = scratch_sizes()?;
        let mut reports = Vec::new();
        for p in &a.rescore {
            let r = eval::rescore(load(p)?, &sizes);
            std::fs::write(p, serde_json::to_vec_pretty(&r)?)?;
            reports.push(r);
        }
        print!("{}", eval::summary_table(&reports));
        return Ok(ExitCode::SUCCESS);
    }
    if !a.compare.is_empty() {
        let reports = a
            .compare
            .iter()
            .map(|p| load(p))
            .collect::<Result<Vec<_>>>()?;
        print!("{}", eval::summary_table(&reports));
        return Ok(ExitCode::SUCCESS);
    }
    match a.fixture_root.clone() {
        Some(root) => child(db, a, &root),
        None => parent(a),
    }
}

// A stale substitution (from a killed run) can point at a deleted folder, so ask subst too.
fn map_drive(target: &Path) -> Result<String> {
    let listed = Command::new("subst")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_uppercase())
        .unwrap_or_default();
    for c in ('H'..='Z').rev() {
        let drive = format!("{c}:");
        if listed.contains(&format!("{drive}\\:")) || Path::new(&format!("{drive}\\")).exists() {
            continue;
        }
        if subst(&[&drive, &target.to_string_lossy()]).is_ok() {
            return Ok(drive);
        }
    }
    bail!("no free drive letter for the fixture")
}

fn subst(args: &[&str]) -> Result<()> {
    let st = Command::new("subst").args(args).status()?;
    if !st.success() {
        bail!("subst {} failed ({st})", args.join(" "));
    }
    Ok(())
}

async fn install(pin: &Pinned, root: &Path) -> Result<()> {
    if runtime::is_installed(pin, root) {
        return Ok(());
    }
    eprintln!(
        "verifying or downloading {} {} ({} MB)",
        pin.id,
        pin.version,
        pin.size >> 20
    );
    let last = std::sync::Mutex::new(0u64);
    let progress = |done: u64, total: u64| {
        let pct = done * 100 / total.max(1);
        let mut l = last.lock().unwrap_or_else(|e| e.into_inner());
        if pct >= *l + 10 {
            *l = pct;
            eprintln!("  {pct}%");
        }
    };
    runtime::ensure(pin, root, &progress, &AtomicBool::new(false)).await?;
    Ok(())
}

// The fixture lives on a substituted drive so tool paths read like a real disk (X:\Users\ana\...).
fn parent(a: EvalArgs) -> Result<ExitCode> {
    let data = hoardini_core::paths::data_dir();
    if a.provider_id.as_deref() == Some(builtin::PROVIDER_ID) {
        let model = a
            .model
            .as_deref()
            .context("--provider-id builtin needs --model <pinned model id>")?;
        let pin = builtin::model_pin(model)
            .with_context(|| format!("{model} is not a pinned built-in model"))?;
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            install(builtin::runtime_for(model), &data).await?;
            install(pin, &data).await
        })?;
    } else if a.provider_id.is_none() && a.base_url.is_none() {
        bail!("pass --base-url (and --model), or --provider-id");
    }
    let work =
        std::env::temp_dir().join(format!("hoardini-eval-{}-{}", std::process::id(), now_ms()));
    let fixture = work.join("root");
    std::fs::create_dir_all(&fixture)?;
    let drive = match map_drive(&fixture) {
        Ok(d) => d,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&work);
            return Err(e);
        }
    };
    let profile = format!(r"{drive}\{PROFILE_REL}");
    let exe = std::env::current_exe()?;
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    args.extend([
        "--fixture-root".into(),
        format!("{drive}\\"),
        "--work-dir".into(),
        work.to_string_lossy().into_owned(),
    ]);
    // The catalog and policy read these; SystemRoot stays real because Winsock needs it.
    let status = Command::new(exe)
        .args(&args)
        .env("USERPROFILE", &profile)
        .env("HOMEDRIVE", &drive)
        .env("HOMEPATH", format!(r"\{PROFILE_REL}"))
        .env("SystemDrive", &drive)
        .env("ProgramData", format!(r"{drive}\ProgramData"))
        .env("ProgramFiles", format!(r"{drive}\Program Files"))
        .env("ProgramW6432", format!(r"{drive}\Program Files"))
        .env("ProgramFiles(x86)", format!(r"{drive}\Program Files (x86)"))
        .env("LOCALAPPDATA", format!(r"{profile}\AppData\Local"))
        .env("APPDATA", format!(r"{profile}\AppData\Roaming"))
        .env("HOARDINI_DATA_DIR", &data)
        .status();
    let undo = subst(&[&drive, "/D"]);
    if !a.keep {
        let _ = std::fs::remove_dir_all(&work);
    } else {
        eprintln!("kept {}", work.display());
    }
    undo?;
    let status = status?;
    Ok(ExitCode::from(
        status.code().unwrap_or(1).clamp(0, 255) as u8
    ))
}

fn eval_profile(base_url: &str, model: Option<String>, small: bool) -> ProviderConfig {
    // Same knobs as the Built-in profile so a reference server is measured like a candidate.
    let mut p = builtin::profile(model.as_deref().unwrap_or(""));
    p.id = None;
    p.name = "eval".into();
    p.base_url = base_url.into();
    p.model = model.filter(|m| !m.trim().is_empty());
    p.small = small;
    p
}

fn child(db: Option<PathBuf>, a: EvalArgs, root: &str) -> Result<ExitCode> {
    let work = a
        .work_dir
        .clone()
        .context("--work-dir is required with --fixture-root")?;
    let data = hoardini_core::paths::data_dir();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let mut secrets: Option<Arc<dyn SecretStore>> = None;
        let mut ram: Option<RamProbe> = None;
        let mut owned: Option<(Arc<Builtin>, builtin::Lease)> = None;
        let (provider, label) = match a.provider_id.as_deref() {
            Some(builtin::PROVIDER_ID) => {
                let model = a.model.clone().context("--model is required")?;
                let b = Builtin::new(
                    Arc::new(LlamaLauncher {
                        root: data.clone(),
                        ctx: builtin::CONTEXT,
                    }),
                    Arc::new(|| 60),
                    Some(data.clone()),
                );
                eprintln!(
                    "starting llama-server with {model} ({} threads)",
                    builtin::threads()
                );
                let lease = b.acquire(&model).await?;
                let mut p = builtin::profile(&model);
                p.base_url = lease.base_url.clone();
                let (h, v) = lease.auth_header();
                p.headers.insert(h, v);
                if let Some(pid) = b.pid() {
                    ram = Some(Arc::new(move || {
                        hoardini_win::process_memory(pid)
                            .ok()
                            .map(|m| (m.working_set, m.peak_working_set, m.private_bytes))
                    }));
                }
                owned = Some((b, lease));
                let backend = if builtin::needs_gpu(&model) {
                    "GPU"
                } else {
                    "CPU"
                };
                (p, format!("{model} (built-in, {backend})"))
            }
            Some(id) => {
                let path = db.unwrap_or_else(hoardini_core::paths::db_path);
                let conn = Db::open(&path)?.conn()?;
                let mut p = hoardini_server::config_by_id(&conn, id)?
                    .with_context(|| format!("no provider {id} in {}", path.display()))?;
                if p.api_key.is_some() {
                    secrets = Some(Arc::new(DpapiVault::open(&data.join("secrets.vault"))?));
                }
                p.id = None;
                if a.small {
                    p.small = true;
                }
                let label = p.model.clone().unwrap_or_else(|| p.name.clone());
                (p, label)
            }
            None => {
                let base = a.base_url.clone().context("--base-url is required")?;
                let p = eval_profile(&base, a.model.clone(), a.small);
                (p, a.model.clone().unwrap_or_else(|| base.clone()))
            }
        };
        let opts = Options {
            label: a.label.clone().unwrap_or(label),
            provider,
            root: root.to_string(),
            db_path: work.join("eval.db"),
            only: a.cases.clone(),
            repeat: a.repeat,
            ram,
            secrets,
            turn_timeout: Duration::from_secs(a.turn_timeout),
            verbose: true,
        };
        let result = eval::run(opts).await;
        if let Some((b, lease)) = owned {
            drop(lease);
            b.stop().await;
        }
        let report = result?;
        println!();
        print!("{}", eval::case_table(&report));
        println!();
        print!("{}", eval::summary_table(std::slice::from_ref(&report)));
        if let Some(out) = &a.out {
            std::fs::write(out, serde_json::to_vec_pretty(&report)?)?;
            eprintln!("wrote {}", out.display());
        }
        Ok(ExitCode::SUCCESS)
    })
}
