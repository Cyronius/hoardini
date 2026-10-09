mod cases;
mod fixture;
mod meter;
mod score;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use hoardini_core::paths::humanize_bytes;
use hoardini_core::store::{Db, now_ms};
use hoardini_core::vault::SecretStore;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::agui::{AgMessage, Event, RunAgentInput};
use crate::provider::{Client, ProviderConfig};
use crate::run::{AgentEnv, IndexControl, new_id, run_agent};
use crate::thread;
use crate::tools::util::redact_all;

pub use cases::{Sizes, cases, size_re};
pub use fixture::{Fixture, build as build_fixture, reset_memories};
pub use meter::{EvalMeter, RequestStat};
pub use score::{
    Call, CaseScore, CaseSpec, GATE, Gate, Totals, Turn, arg_text, forbid, gate_failures, group,
    leaked_calls, median, percentile, rule, score, totals, turns_from,
};

// (working set, peak working set, private bytes) of the model server, when the eval owns it.
pub type RamProbe = Arc<dyn Fn() -> Option<(u64, u64, u64)> + Send + Sync>;

pub struct Options {
    pub provider: ProviderConfig,
    pub label: String,
    pub root: String,
    pub db_path: PathBuf,
    pub only: Vec<String>,
    pub repeat: u32,
    pub ram: Option<RamProbe>,
    pub secrets: Option<Arc<dyn SecretStore>>,
    pub turn_timeout: Duration,
    pub verbose: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseRun {
    pub repeat: u32,
    pub wall_s: f64,
    pub score: CaseScore,
    pub requests: Vec<RequestStat>,
    pub transcript: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub label: String,
    pub model: Option<String>,
    pub base_url: String,
    pub small: bool,
    pub started_ms: i64,
    pub finished_ms: i64,
    pub repeat: u32,
    pub totals: Totals,
    pub gate: Gate,
    pub failures: Vec<String>,
    pub passed: bool,
    pub cases: Vec<CaseRun>,
}

struct NoSecrets;

impl SecretStore for NoSecrets {
    fn get(&self, _r: &str) -> Result<Option<SecretString>> {
        Ok(None)
    }
    fn set(&self, _r: &str, _v: SecretString) -> Result<()> {
        anyhow::bail!("the eval has no vault")
    }
    fn delete(&self, _r: &str) -> Result<()> {
        Ok(())
    }
    fn refs(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
}

struct Idle;

impl IndexControl for Idle {
    fn status(&self) -> Value {
        json!({"state": "idle"})
    }
    fn refresh(&self, _mode: &str) -> Result<bool> {
        Ok(false)
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push('…');
    out
}

fn transcript_json(turns: &[Turn], profile: &str) -> Vec<Value> {
    turns
        .iter()
        .map(|t| {
            let calls: Vec<Value> = t
                .calls
                .iter()
                .map(|c| {
                    json!({
                        "name": c.name,
                        "args": redact_all(&c.args, profile),
                        "result": c.result.as_deref().map(|r| clip(&redact_all(r, profile), 300)),
                    })
                })
                .collect();
            json!({"text": redact_all(&t.text, profile), "calls": calls})
        })
        .collect()
}

#[derive(Default)]
struct RamPeak {
    working: u64,
    private: u64,
}

async fn run_turn(
    env: &Arc<AgentEnv>,
    provider: &ProviderConfig,
    thread_id: &str,
    text: &str,
    limit: Duration,
) -> Option<String> {
    let input = RunAgentInput {
        thread_id: thread_id.to_string(),
        run_id: new_id(),
        messages: vec![AgMessage {
            id: new_id(),
            role: "user".into(),
            content: Some(text.to_string()),
            tool_calls: None,
            tool_call_id: None,
            data: None,
        }],
        ..Default::default()
    };
    let (tx, mut rx) = mpsc::channel(1024);
    let task = tokio::spawn(run_agent(env.clone(), provider.clone(), input, tx));
    let mut error = None;
    let collect = async {
        while let Some(ev) = rx.recv().await {
            if let Event::RunError { message, .. } = ev {
                error = Some(message);
            }
        }
    };
    if tokio::time::timeout(limit, collect).await.is_err() {
        task.abort();
        return Some(format!("timed out after {} s", limit.as_secs()));
    }
    match task.await {
        Ok(Err(e)) => Some(format!("{e:#}")),
        Err(e) => Some(format!("agent task failed: {e}")),
        Ok(Ok(())) => error,
    }
}

pub async fn run(opts: Options) -> Result<Report> {
    let started_ms = now_ms();
    let db = Db::open(&opts.db_path)?;
    let root = opts.root.clone();
    let db2 = db.clone();
    let built = tokio::task::spawn_blocking(move || build_fixture(&root, &db2)).await??;
    let fx = built.fixture.clone();
    let sizes = fixture_sizes(&db, &fx)?;
    let mut all = cases(&sizes);
    if !opts.only.is_empty() {
        all.retain(|c| opts.only.iter().any(|o| o == &c.id));
    }
    let meter = Arc::new(EvalMeter::default());
    let mut env = AgentEnv::new(
        db.clone(),
        Client::with_meter(meter.clone()),
        opts.secrets.clone().unwrap_or_else(|| Arc::new(NoSecrets)),
        Arc::new(Idle),
    );
    env.redact_profile = Some(fx.profile.clone());
    let env = Arc::new(env);

    let peak = Arc::new(Mutex::new(RamPeak::default()));
    let sampler = opts.ram.clone().map(|probe| {
        let peak = peak.clone();
        tokio::spawn(async move {
            loop {
                if let Some((ws, _, private)) = probe() {
                    let mut p = peak.lock().unwrap_or_else(|e| e.into_inner());
                    p.working = p.working.max(ws);
                    p.private = p.private.max(private);
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
    });

    let mut runs = Vec::new();
    for rep in 0..opts.repeat.max(1) {
        for case in &all {
            let thread_id = new_id();
            let _ = meter.take();
            let t0 = Instant::now();
            let mut run_error = None;
            for turn in &case.turns {
                let turn = fx.question(turn);
                run_error =
                    run_turn(&env, &opts.provider, &thread_id, &turn, opts.turn_timeout).await;
                if run_error.is_some() {
                    break;
                }
            }
            let wall_s = t0.elapsed().as_secs_f64();
            let requests = meter.take();
            let msgs: Vec<_> = thread::load(&db.conn()?, &thread_id)?
                .into_iter()
                .map(|s| s.msg)
                .collect();
            let turns = turns_from(&msgs);
            let sc = score(case, &turns, run_error.as_deref(), &fx.profile);
            reset_memories(&db, built.memories_baseline)?;
            if opts.verbose {
                eprintln!("{}", case_line(&sc, wall_s));
            }
            runs.push(CaseRun {
                repeat: rep,
                wall_s,
                score: sc,
                requests,
                transcript: transcript_json(&turns, &fx.profile),
            });
        }
    }
    if let Some(s) = sampler {
        s.abort();
    }

    let mut t = run_totals(&runs);
    if let Some(probe) = &opts.ram {
        let p = peak.lock().unwrap_or_else(|e| e.into_inner());
        let os_peak = probe().map(|(_, peak_ws, _)| peak_ws).unwrap_or(0);
        t.peak_ram_bytes = Some(p.working.max(os_peak));
        t.peak_private_bytes = Some(p.private);
    }
    let failures = gate_failures(&t, &GATE);
    Ok(Report {
        label: opts.label,
        model: opts.provider.model.clone(),
        base_url: opts.provider.base_url.clone(),
        small: opts.provider.small,
        started_ms,
        finished_ms: now_ms(),
        repeat: opts.repeat.max(1),
        totals: t,
        gate: GATE,
        passed: failures.is_empty(),
        failures,
        cases: runs,
    })
}

fn run_totals(runs: &[CaseRun]) -> Totals {
    let scores: Vec<CaseScore> = runs.iter().map(|r| r.score.clone()).collect();
    let mut t = totals(&scores);
    let walls: Vec<f64> = runs.iter().map(|r| r.wall_s).collect();
    t.median_case_s = median(&walls);
    let reqs: Vec<&RequestStat> = runs.iter().flat_map(|r| &r.requests).collect();
    let ttfts: Vec<f64> = reqs.iter().filter_map(|r| r.ttft_ms).collect();
    t.median_ttft_ms = median(&ttfts);
    t.p90_ttft_ms = percentile(&ttfts, 0.9);
    // Server-reported timings beat wall-clock estimates; mix them only when no request has them.
    let timed = reqs.iter().any(|r| r.server_timed);
    let (tok, ms) = reqs
        .iter()
        .filter(|r| r.server_timed || !timed)
        .filter_map(|r| Some((r.completion_tokens? as f64, r.decode_ms?)))
        .filter(|(_, ms)| *ms > 0.0)
        .fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
    t.decode_tps = (ms > 0.0).then(|| tok / (ms / 1000.0));
    let (ptok, pms) = reqs
        .iter()
        .filter_map(|r| Some((r.prompt_tokens? as f64, r.prompt_ms?)))
        .filter(|(_, ms)| *ms > 0.0)
        .fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
    t.prompt_tps = (pms > 0.0).then(|| ptok / (pms / 1000.0));
    t
}

pub fn fixture_sizes(db: &Db, fx: &Fixture) -> Result<Sizes> {
    Ok(Sizes {
        lmstudio: fx.size_of(db, r"~\.lmstudio")?,
        coder: fx.size_of(
            db,
            r"~\.lmstudio\models\unsloth\Qwen3-Coder-30B-A3B-Instruct-GGUF\Qwen3-Coder-30B-A3B-Instruct-Q4_K_M.gguf",
        )?,
        pip: fx.size_of(db, r"~\.cache\pip")?,
        nuget: fx.size_of(db, r"~\.nuget")?,
        target_growth: fx.size_of(db, r"code\hoardini\target")?,
    })
}

fn turns_from_json(transcript: &[Value]) -> Vec<Turn> {
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    transcript
        .iter()
        .map(|t| Turn {
            text: s(t, "text").unwrap_or_default(),
            calls: t
                .get("calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|c| Call {
                    name: s(c, "name").unwrap_or_default(),
                    args: s(c, "args").unwrap_or_default(),
                    result: s(c, "result"),
                })
                .collect(),
        })
        .collect()
}

// Scores saved transcripts again with the current rules; timings and RAM are kept as measured.
pub fn rescore(mut r: Report, sizes: &Sizes) -> Report {
    let all = cases(sizes);
    for run in &mut r.cases {
        if let Some(case) = all.iter().find(|c| c.id == run.score.id) {
            let turns = turns_from_json(&run.transcript);
            run.score = score(case, &turns, run.score.run_error.as_deref(), "");
        }
    }
    let (ram, private) = (r.totals.peak_ram_bytes, r.totals.peak_private_bytes);
    r.totals = run_totals(&r.cases);
    r.totals.peak_ram_bytes = ram;
    r.totals.peak_private_bytes = private;
    r.gate = GATE;
    r.failures = gate_failures(&r.totals, &GATE);
    r.passed = r.failures.is_empty();
    r
}

fn pct1(v: f64) -> String {
    format!("{:.1}%", v * 100.0)
}

fn pct(v: f64) -> String {
    format!("{:.0}%", v * 100.0)
}

fn opt<T>(v: Option<T>, f: impl Fn(T) -> String) -> String {
    v.map(f).unwrap_or_else(|| "n/a".into())
}

pub fn case_line(s: &CaseScore, wall_s: f64) -> String {
    let mut flags = Vec::new();
    if !s.missing.is_empty() {
        flags.push(format!("missing {}", s.missing.join(", ")));
    }
    if !s.misused.is_empty() {
        flags.push(format!("unasked {}", s.misused.join(", ")));
    }
    if !s.facts_missed.is_empty() {
        flags.push(format!("no fact: {}", s.facts_missed.join("; ")));
    }
    if !s.violations.is_empty() {
        flags.push(format!("VIOLATION: {}", s.violations.join("; ")));
    }
    if let Some(e) = &s.run_error {
        flags.push(format!("error: {}", clip(e, 80)));
    }
    format!(
        "{:<18} tool {:>4}  answer {:>4}  calls {}/{}  {:>5.1} s  [{}]  {}",
        s.id,
        pct(s.tool_score),
        opt(s.answer_score, pct),
        s.calls_parsed,
        s.calls_attempted,
        wall_s,
        s.tools_used.join(","),
        flags.join(" | ")
    )
}

pub fn case_table(r: &Report) -> String {
    let mut out = String::new();
    for c in &r.cases {
        let _ = writeln!(out, "{}", case_line(&c.score, c.wall_s));
    }
    out
}

pub fn summary_table(reports: &[Report]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "| model | parse | tool | answer | viol. | errors | median q | TTFT med / p90 | decode tok/s | prompt tok/s | peak RAM | gate |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---|---|---|");
    for r in reports {
        let t = &r.totals;
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | {} / {} | {} | {} | {} | {} |",
            r.label,
            opt(t.parse_rate, pct1),
            pct1(t.tool_choice),
            pct1(t.answer),
            t.violations,
            t.run_errors,
            opt(t.median_case_s, |s| format!("{s:.0} s")),
            opt(t.median_ttft_ms, |m| format!("{:.1} s", m / 1000.0)),
            opt(t.p90_ttft_ms, |m| format!("{:.1} s", m / 1000.0)),
            opt(t.decode_tps, |v| format!("{v:.1}")),
            opt(t.prompt_tps, |v| format!("{v:.0}")),
            opt(t.peak_ram_bytes, humanize_bytes),
            if r.passed {
                "pass".to_string()
            } else {
                format!("fail: {}", r.failures.join("; "))
            }
        );
    }
    out
}
