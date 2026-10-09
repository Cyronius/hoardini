use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use hoardini_core::history::{self, Since};
use hoardini_core::index::{self, IndexStatus, TopFilesQuery};
use hoardini_core::paths::humanize_bytes;
use hoardini_core::store::{Db, now_ms};

mod eval_cmd;

#[derive(Parser)]
#[command(name = "hoardini", version, about = "Hoardini: a Windows disk agent")]
struct Cli {
    #[arg(
        long,
        global = true,
        help = "SQLite database [default: %LOCALAPPDATA%\\hoardini\\hoardini.db]"
    )]
    db: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    #[command(
        about = "Run one index refresh; prints JSON lines (the app's child protocol)",
        disable_help_flag = true
    )]
    Index {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(about = "Largest folders or files in the stored index")]
    Top(TopArgs),
    #[command(about = "What grew, shrank, appeared, or vanished between refreshes")]
    Changes(ChangesArgs),
    #[command(about = "Serve the UI and API on loopback")]
    Serve(ServeArgs),
    #[command(about = "Score a model on the scripted agent questions over a fixture disk")]
    Eval(eval_cmd::EvalArgs),
}

#[derive(Args)]
struct ServeArgs {
    #[arg(long, default_value_t = 7043)]
    port: u16,
    #[arg(long, help = "Bearer token [default: random per launch]")]
    token: Option<String>,
    #[arg(
        long,
        help = "Serve the UI from this folder instead of the embedded build"
    )]
    ui_dir: Option<PathBuf>,
    #[arg(long, help = "Skip the background refresh at startup")]
    no_refresh: bool,
}

#[derive(Args)]
struct TopArgs {
    #[arg(long, help = "Only look under this folder")]
    under: Option<String>,
    #[arg(long, conflicts_with = "dirs", help = "Rank files")]
    files: bool,
    #[arg(long, help = "Rank folders (the default)")]
    dirs: bool,
    #[arg(long, default_value_t = 20)]
    limit: usize,
    #[arg(long, default_value_t = 1, help = "Folder levels below --under")]
    depth: u32,
    #[arg(long, default_value = "0", value_parser = parse_size, help = "Files: minimum size, e.g. 100M, 2G")]
    min_size: u64,
    #[arg(long, help = "Files: extensions, e.g. gguf,safetensors")]
    ext: Option<String>,
    #[arg(long, help = "Files: last used more than N days ago")]
    unused_days: Option<u32>,
    #[arg(long, help = "Print JSON")]
    json: bool,
}

#[derive(Args)]
struct ChangesArgs {
    #[arg(
        long,
        default_value = "previous",
        help = "Scan id, Nd for N days ago, or \"previous\""
    )]
    since: String,
    #[arg(long, help = "Volume root, e.g. C:\\ [default: the last refreshed]")]
    volume: Option<String>,
    #[arg(long, default_value_t = 15)]
    limit: usize,
    #[arg(long, help = "Print JSON")]
    json: bool,
}

fn parse_size(s: &str) -> Result<u64, String> {
    let t = s.trim().to_ascii_uppercase();
    let t = t.strip_suffix('B').unwrap_or(&t);
    let (num, mult) = match t.chars().last() {
        Some('K') => (&t[..t.len() - 1], 1u64 << 10),
        Some('M') => (&t[..t.len() - 1], 1 << 20),
        Some('G') => (&t[..t.len() - 1], 1 << 30),
        Some('T') => (&t[..t.len() - 1], 1 << 40),
        _ => (t, 1),
    };
    let v: f64 = num
        .trim()
        .parse()
        .map_err(|_| format!("not a size: {s:?}"))?;
    Ok((v * mult as f64) as u64)
}

fn open_db(db: Option<PathBuf>) -> Result<Db> {
    let path = db.unwrap_or_else(hoardini_core::paths::db_path);
    if !path.exists() {
        bail!("no index at {}; run `hoardini index` first", path.display());
    }
    Db::open(&path).with_context(|| format!("opening {}", path.display()))
}

fn age_line(st: &IndexStatus) -> String {
    let age = st
        .age_ms
        .map(index::humanize_age)
        .unwrap_or_else(|| "an unfinished first refresh".into());
    let mut line = format!(
        "Index of {} as of {age} (refresh {})",
        st.volume, st.generation
    );
    if st.refresh_incomplete {
        line.push_str(", a newer refresh has not finished");
    }
    line
}

fn top(db: Db, a: TopArgs) -> Result<()> {
    let conn = db.conn()?;
    let Some(st) = index::index_status(&conn, None)? else {
        bail!("the index is empty; run `hoardini index` first");
    };
    let header = age_line(&st);
    if a.files {
        let r = index::top_files(
            &conn,
            &TopFilesQuery {
                under: a.under,
                min_size: a.min_size,
                ext: a.ext,
                not_used_for_days: a.unused_days,
                limit: a.limit,
            },
        )?;
        if a.json {
            println!("{}", serde_json::to_string_pretty(&r)?);
            return Ok(());
        }
        println!("{header}");
        for f in &r.items {
            let used = f
                .last_used_ms
                .map(|t| index::humanize_age(now_ms() - t))
                .unwrap_or_else(|| "?".into());
            println!(
                "{:>10}  {:<16} {}{}",
                humanize_bytes(f.size),
                used,
                f.path,
                if f.placeholder { "  (cloud only)" } else { "" }
            );
        }
        if let Some(m) = r.more_line() {
            println!("{m}");
        }
    } else {
        let r = index::top_dirs(&conn, a.under.as_deref(), a.depth, a.limit)?;
        if a.json {
            println!("{}", serde_json::to_string_pretty(&r)?);
            return Ok(());
        }
        println!("{header}");
        for d in &r.items {
            println!(
                "{:>10}  {:>10} files  {}{}",
                humanize_bytes(d.size),
                d.files,
                d.path,
                if d.denied { "  (access denied)" } else { "" }
            );
        }
        if let Some(m) = r.more_line() {
            println!("{m}");
        }
    }
    Ok(())
}

fn signed(delta: i64) -> String {
    let sign = if delta < 0 { "-" } else { "+" };
    format!("{sign}{}", humanize_bytes(delta.unsigned_abs()))
}

fn changes(db: Db, a: ChangesArgs) -> Result<()> {
    let conn = db.conn()?;
    let since: Since = a.since.parse()?;
    let volume_id = match a.volume.as_deref() {
        Some(v) => Some(
            index::index_status(&conn, Some(v))?
                .with_context(|| format!("{v} has no index"))?
                .volume_id,
        ),
        None => None,
    };
    let Some(ch) = history::changes(&conn, volume_id, since, a.limit)? else {
        println!("Not enough history yet: changes need two completed refreshes.");
        return Ok(());
    };
    if a.json {
        println!("{}", serde_json::to_string_pretty(&ch)?);
        return Ok(());
    }
    print!(
        "{}: scan #{} ({}) → #{} ({})",
        ch.volume,
        ch.from.id,
        history::fmt_date(ch.from.finished_ms),
        ch.to.id,
        history::fmt_date(ch.to.finished_ms)
    );
    if let Some(fd) = ch.free_delta {
        print!(", free {}", signed(fd));
    }
    println!(", indexed {}", signed(ch.indexed_delta));
    if ch.grew.is_empty() && ch.new.is_empty() && ch.shrank.is_empty() && ch.removed.is_empty() {
        println!("No folder or file of 100 MB or more changed by 1 MB or more.");
        return Ok(());
    }
    let sections = [
        ("Grew", &ch.grew, ch.omitted.grew),
        ("New", &ch.new, ch.omitted.new),
        ("Shrank", &ch.shrank, ch.omitted.shrank),
        ("Removed", &ch.removed, ch.omitted.removed),
    ];
    for (title, list, omitted) in sections {
        if list.is_empty() {
            continue;
        }
        println!("{title}:");
        for c in list {
            let tail = match (c.before, c.after) {
                (Some(b), Some(a)) => format!("  ({} → {})", humanize_bytes(b), humanize_bytes(a)),
                _ => String::new(),
            };
            println!("{:>11}  {:<4} {}{tail}", signed(c.delta), c.kind, c.path);
        }
        if omitted > 0 {
            println!("  +{omitted} more");
        }
    }
    Ok(())
}

fn serve(db: Option<PathBuf>, a: ServeArgs) -> Result<()> {
    let _log = hoardini_server::init_logging(&hoardini_server::default_log_dir(), true)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let handle = hoardini_server::start(hoardini_server::ServerOptions {
            port: a.port,
            token: a.token,
            db_path: db.unwrap_or_else(hoardini_core::paths::db_path),
            ui_dir: a.ui_dir,
            dialogs: None,
            refresh_on_start: !a.no_refresh,
            indexer_exe: None,
        })
        .await?;
        println!("{}", handle.url());
        tokio::signal::ctrl_c().await?;
        let _ = handle.shutdown.send(());
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        Ok(())
    })
}

fn stderr_logging() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    // The app spawns `<exe> index ...`; keep that path free of clap's global-flag parsing.
    if argv.get(1).is_some_and(|a| a == "index") {
        stderr_logging();
        return ExitCode::from(index::run_child(&argv[2..]).clamp(0, 255) as u8);
    }
    // The executor relaunches this exe elevated for system_cleanup jobs (ACT-WINDOWS-CLEANUP).
    if argv.get(1).is_some_and(|a| a == "system-cleanup") {
        let code = hoardini_core::syscleanup::run_helper(&argv[2..]);
        return ExitCode::from(code.clamp(0, 255) as u8);
    }
    let cli = Cli::parse();
    // `serve` installs the file logger itself, also when a global flag precedes it (`--db x serve`).
    if !matches!(cli.cmd, Cmd::Serve(_)) {
        stderr_logging();
    }
    let r = match cli.cmd {
        Cmd::Index { mut args } => {
            if let Some(db) = cli.db {
                args.push("--db".into());
                args.push(db.to_string_lossy().into_owned());
            }
            return ExitCode::from(index::run_child(&args).clamp(0, 255) as u8);
        }
        Cmd::Top(a) => open_db(cli.db).and_then(|db| top(db, a)),
        Cmd::Changes(a) => open_db(cli.db).and_then(|db| changes(db, a)),
        Cmd::Serve(a) => serve(cli.db, a),
        Cmd::Eval(a) => match eval_cmd::run(cli.db, a) {
            Ok(code) => return code,
            Err(e) => Err(e),
        },
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size("100M").unwrap(), 100 << 20);
        assert_eq!(parse_size("1.5g").unwrap(), 3 << 29);
        assert_eq!(parse_size("2KB").unwrap(), 2048);
        assert_eq!(parse_size("4096").unwrap(), 4096);
        assert!(parse_size("lots").is_err());
    }
}
