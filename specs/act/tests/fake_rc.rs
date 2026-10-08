// A stand-in for `rclone rcd`: real file copies into a temp "remote", plus a request log.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hoardini_core::destinations::rclone::Endpoint;
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub struct Settings {
    pub hashes: Vec<String>,
    pub about_free: Option<i64>,
    // Flip the last byte of every file uploaded, as a flaky remote would.
    pub tamper: bool,
    // A job reports finished only after this many status polls.
    pub slow_polls: usize,
    pub after_upload: Option<Box<dyn Fn() + Send + Sync>>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hashes: vec!["md5".into(), "sha256".into()],
            about_free: Some(5_000_000_000),
            tamper: false,
            slow_polls: 0,
            after_upload: None,
        }
    }
}

struct Job {
    group: String,
    total: i64,
    polls: usize,
    stopped: bool,
    output: Value,
}

#[derive(Default)]
struct State {
    jobs: HashMap<i64, Job>,
    next_job: i64,
    remotes: Vec<String>,
    log: Vec<(String, Value)>,
    bad_auth: usize,
    created: Vec<(String, String, Value)>,
}

pub struct FakeRc {
    pub root: PathBuf,
    pub ep: Endpoint,
    settings: Arc<Mutex<Settings>>,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
}

fn b64(input: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in input.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

impl FakeRc {
    pub fn start(root: &Path, settings: Settings) -> FakeRc {
        std::fs::create_dir_all(root).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let fake = FakeRc {
            root: root.to_path_buf(),
            ep: Endpoint {
                addr: format!("127.0.0.1:{port}"),
                user: "u-test".into(),
                pass: "p-test".into(),
            },
            settings: Arc::new(Mutex::new(settings)),
            state: Arc::new(Mutex::new(State::default())),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (st, se, stop, root, auth) = (
            fake.state.clone(),
            fake.settings.clone(),
            fake.stop.clone(),
            fake.root.clone(),
            format!("Basic {}", b64(b"u-test:p-test")),
        );
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((conn, _)) => {
                        let (st, se, root, auth) =
                            (st.clone(), se.clone(), root.clone(), auth.clone());
                        std::thread::spawn(move || {
                            let _ = serve(conn, &st, &se, &root, &auth);
                        });
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                }
            }
        });
        fake
    }

    pub fn calls(&self, method: &str) -> Vec<Value> {
        self.state
            .lock()
            .log
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, p)| p.clone())
            .collect()
    }

    pub fn methods(&self) -> Vec<String> {
        self.state
            .lock()
            .log
            .iter()
            .map(|(m, _)| m.clone())
            .collect()
    }

    pub fn bad_auth(&self) -> usize {
        self.state.lock().bad_auth
    }

    pub fn created(&self) -> Vec<(String, String, Value)> {
        self.state.lock().created.clone()
    }

    pub fn set_tamper(&self, on: bool) {
        self.settings.lock().tamper = on;
    }

    pub fn remote_files(&self) -> Vec<PathBuf> {
        files_under(&self.root)
    }
}

impl Drop for FakeRc {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn resolve(root: &Path, fs: &str) -> (bool, PathBuf) {
    match fs.strip_prefix("fake:") {
        Some(rest) => (true, root.join(rest)),
        None => (false, PathBuf::from(fs)),
    }
}

fn read_request(conn: &mut TcpStream) -> std::io::Result<(String, String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        let n = conn.read(&mut chunk)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let len = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    while buf.len() < header_end + len {
        let n = conn.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let path = head
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .trim_start_matches('/')
        .to_string();
    let auth = head
        .lines()
        .find_map(|l| {
            l.strip_prefix("authorization: ")
                .or_else(|| l.strip_prefix("Authorization: "))
        })
        .unwrap_or("")
        .trim()
        .to_string();
    Ok((path, auth, buf[header_end..].to_vec()))
}

fn serve(
    mut conn: TcpStream,
    st: &Mutex<State>,
    se: &Mutex<Settings>,
    root: &Path,
    auth: &str,
) -> std::io::Result<()> {
    conn.set_nonblocking(false)?;
    let (method, got_auth, body) = read_request(&mut conn)?;
    let params: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let (code, out) = if got_auth != auth {
        st.lock().bad_auth += 1;
        (401, json!({"error": "unauthorized"}))
    } else {
        st.lock().log.push((method.clone(), params.clone()));
        handle(&method, &params, st, se, root)
    };
    let text = out.to_string();
    write!(
        conn,
        "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    )?;
    conn.flush()
}

fn copy_into(src: &Path, dst: &Path, tamper: bool, total: &mut i64) {
    if src.is_dir() {
        std::fs::create_dir_all(dst).unwrap();
        for e in std::fs::read_dir(src).unwrap().flatten() {
            copy_into(&e.path(), &dst.join(e.file_name()), tamper, total);
        }
        return;
    }
    std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
    let mut bytes = std::fs::read(src).unwrap();
    *total += bytes.len() as i64;
    if tamper && let Some(b) = bytes.last_mut() {
        *b ^= 0xFF;
    }
    std::fs::write(dst, bytes).unwrap();
}

fn finish_or_queue(p: &Value, st: &Mutex<State>, total: i64, output: Value) -> (u16, Value) {
    if p["_async"].as_bool() != Some(true) {
        return (200, output);
    }
    let mut g = st.lock();
    g.next_job += 1;
    let id = g.next_job;
    let group = p["_group"].as_str().unwrap_or("").to_string();
    g.jobs.insert(
        id,
        Job {
            group,
            total,
            polls: 0,
            stopped: false,
            output,
        },
    );
    (200, json!({"jobid": id}))
}

fn handle(
    method: &str,
    p: &Value,
    st: &Mutex<State>,
    se: &Mutex<Settings>,
    root: &Path,
) -> (u16, Value) {
    let s = |k: &str| p[k].as_str().unwrap_or("").to_string();
    match method {
        "core/version" => (200, json!({"version": "fake"})),
        "config/listremotes" => (200, json!({"remotes": st.lock().remotes.clone()})),
        "config/create" => {
            let name = s("name");
            let mut g = st.lock();
            g.remotes.push(name.clone());
            g.created.push((name, s("type"), p["parameters"].clone()));
            (200, json!({}))
        }
        "config/delete" => (200, json!({})),
        "operations/about" => match se.lock().about_free {
            Some(f) => (200, json!({"total": f * 2, "used": f, "free": f})),
            None => (500, json!({"error": "about not supported"})),
        },
        "operations/fsinfo" => (200, json!({"Hashes": se.lock().hashes.clone()})),
        "operations/list" => {
            let (_, dir) = resolve(root, &s("fs"));
            let recurse = p["opt"]["recurse"].as_bool().unwrap_or(false);
            let want_hash = p["opt"]["showHash"].as_bool().unwrap_or(false)
                && se.lock().hashes.iter().any(|h| h == "sha256");
            let mut list = Vec::new();
            let files = if recurse {
                files_under(&dir)
            } else {
                std::fs::read_dir(&dir)
                    .map(|rd| {
                        rd.flatten()
                            .map(|e| e.path())
                            .filter(|p| p.is_file())
                            .collect()
                    })
                    .unwrap_or_default()
            };
            for f in files {
                let rel = f
                    .strip_prefix(&dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = std::fs::read(&f).unwrap();
                let mut e = json!({"Path": rel, "Name": f.file_name().unwrap().to_string_lossy(), "Size": bytes.len(), "IsDir": false});
                if want_hash {
                    e["Hashes"] = json!({"sha256": format!("{:x}", Sha256::digest(&bytes))});
                }
                list.push(e);
            }
            (200, json!({"list": list}))
        }
        "operations/copyfile" | "sync/copy" => {
            let (src_remote, dst_remote) = (
                s("srcFs").starts_with("fake:"),
                s("dstFs").starts_with("fake:"),
            );
            let (_, mut src) = resolve(root, &s("srcFs"));
            let (_, mut dst) = resolve(root, &s("dstFs"));
            if method == "operations/copyfile" {
                src = src.join(s("srcRemote"));
                dst = dst.join(s("dstRemote"));
            }
            let _ = src_remote;
            let mut total = 0;
            copy_into(&src, &dst, se.lock().tamper && dst_remote, &mut total);
            if dst_remote && let Some(hook) = &se.lock().after_upload {
                hook();
            }
            finish_or_queue(p, st, total, json!({}))
        }
        "operations/hashsum" => {
            let (_, path) = resolve(root, &s("fs"));
            let files = if path.is_file() {
                vec![path.clone()]
            } else {
                files_under(&path)
            };
            let mut total = 0;
            let lines: Vec<String> = files
                .iter()
                .map(|f| {
                    let bytes = std::fs::read(f).unwrap();
                    total += bytes.len() as i64;
                    let rel = if path.is_file() {
                        f.file_name().unwrap().to_string_lossy().into_owned()
                    } else {
                        f.strip_prefix(&path)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/")
                    };
                    format!("{}  {rel}", blake3::hash(&bytes).to_hex())
                })
                .collect();
            finish_or_queue(
                p,
                st,
                total,
                json!({"hashsum": lines, "hashType": "blake3"}),
            )
        }
        "job/status" => {
            let slow = se.lock().slow_polls;
            let mut g = st.lock();
            let Some(j) = g.jobs.get_mut(&p["jobid"].as_i64().unwrap_or(-1)) else {
                return (500, json!({"error": "job not found"}));
            };
            j.polls += 1;
            if j.stopped {
                return (
                    200,
                    json!({"finished": true, "success": false, "error": "context canceled"}),
                );
            }
            (
                200,
                json!({"finished": j.polls > slow, "success": true, "error": "", "output": j.output.clone()}),
            )
        }
        "core/stats" => {
            let slow = se.lock().slow_polls.max(1) as i64;
            let g = st.lock();
            let bytes = g
                .jobs
                .values()
                .filter(|j| j.group == s("group"))
                .map(|j| j.total * (j.polls as i64).min(slow) / slow)
                .sum::<i64>();
            (200, json!({"bytes": bytes}))
        }
        "core/stats-delete" => (200, json!({})),
        "job/stop" => {
            if let Some(j) = st.lock().jobs.get_mut(&p["jobid"].as_i64().unwrap_or(-1)) {
                j.stopped = true;
            }
            (200, json!({}))
        }
        "operations/deletefile" => {
            let (_, dir) = resolve(root, &s("fs"));
            let _ = std::fs::remove_file(dir.join(s("remote")));
            (200, json!({}))
        }
        "operations/purge" => {
            let (_, dir) = resolve(root, &s("fs"));
            let _ = std::fs::remove_dir_all(dir);
            (200, json!({}))
        }
        other => (404, json!({"error": format!("unknown method {other}")})),
    }
}
