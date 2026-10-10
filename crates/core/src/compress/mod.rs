mod candidates;
pub mod pack;

pub use candidates::{
    CachedEstimate, CompressCandidate, ESTIMATE_TOP, MIN_FOLDER, MIN_SAVED, MIN_SAVED_PERCENT,
    QUIET_DAYS, candidates, estimate_top, proposable, saved_bytes, spawn_estimates,
};

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Result, bail};
use serde::Serialize;

use crate::evidence::identify::identify;

const SAMPLE_BYTES: u64 = 1 << 20;
const MAX_SAMPLES: usize = 16;
const MAX_WALK: usize = 200_000;
const CLUSTER: usize = 4096;
const LZX_CHUNK: usize = 32 << 10;
const LZNT1_CHUNK: usize = 4 << 10;
const LZNT1_UNIT: usize = 16 * CLUSTER;
// Real LZNT1 is a weaker LZ77 than deflate level 1.
const LZNT1_PENALTY: f64 = 1.12;
// LZMA2 runs about this many times slower than the zstd proxy used to size it.
const LZMA_SLOWDOWN: f64 = 2.5;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FormatEstimate {
    pub format: &'static str,
    pub in_place: bool,
    // Compressed size / original size over the compressible part; 1.0 = nothing saved.
    pub ratio: f64,
    pub compressed_bytes: u64,
    pub saved_bytes: u64,
    pub seconds: f64,
    // Every figure here is extrapolated from samples; WOF and LZNT1 are modelled, not run.
    pub estimate: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Estimate {
    pub path: String,
    pub is_dir: bool,
    pub logical_bytes: u64,
    pub sampled_bytes: u64,
    pub files_sampled: usize,
    pub already_compressed: bool,
    pub already_compressed_bytes: u64,
    pub note: Option<String>,
    pub formats: Vec<FormatEstimate>,
}

const FORMATS: [(&str, bool); 5] = [
    ("zip", false),
    ("7z", false),
    ("tar.zst", false),
    ("lzx", true),
    ("lznt1", true),
];

struct Probe {
    path: PathBuf,
    size: u64,
    compressed: bool,
}

fn walk(root: &Path) -> (Vec<(PathBuf, u64)>, u64) {
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = hoardini_win::list_dir(&dir) else {
            continue;
        };
        for e in entries {
            if e.is_dir() {
                if !e.is_reparse() {
                    stack.push(dir.join(&e.name));
                }
            } else if !e.is_link() && !e.is_placeholder() {
                total += e.logical;
                if files.len() < MAX_WALK {
                    files.push((dir.join(&e.name), e.logical));
                }
            }
        }
    }
    (files, total)
}

fn read_sample(f: &mut std::fs::File, off: u64, len: usize) -> std::io::Result<Vec<u8>> {
    f.seek(SeekFrom::Start(off))?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

// Evenly spread 1 MB reads (the whole file when it is small).
fn file_samples(path: &Path, size: u64, max: usize) -> std::io::Result<Vec<Vec<u8>>> {
    let mut f = hoardini_win::open_read_preserving_atime(path)?;
    if size <= SAMPLE_BYTES {
        return Ok(vec![read_sample(&mut f, 0, size as usize)?]);
    }
    let n = size.div_ceil(SAMPLE_BYTES).min(max as u64).max(1);
    let span = size - SAMPLE_BYTES;
    (0..n)
        .map(|i| {
            let off = if n == 1 { 0 } else { span * i / (n - 1) };
            read_sample(&mut f, off, SAMPLE_BYTES as usize)
        })
        .collect()
}

fn identify_file(path: &Path, size: u64) -> bool {
    let Ok(f) = hoardini_win::open_read_preserving_atime(path) else {
        return false;
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    identify(f, size, &name).already_compressed
}

fn deflate_len(data: &[u8], level: u32) -> usize {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(level));
    if e.write_all(data).is_err() {
        return data.len();
    }
    e.finish().map_or(data.len(), |v| v.len())
}

fn zstd_len(data: &[u8], level: i32) -> usize {
    zstd::bulk::compress(data, level).map_or(data.len(), |v| v.len())
}

fn lzx_len(data: &[u8]) -> usize {
    let sum: usize = data
        .chunks(LZX_CHUNK)
        .map(|c| zstd_len(c, 9).min(c.len()))
        .sum();
    sum.div_ceil(CLUSTER) * CLUSTER
}

// NTFS keeps a 64 KB unit compressed only when that frees at least one cluster.
fn lznt1_len(data: &[u8]) -> usize {
    data.chunks(LZNT1_UNIT)
        .map(|unit| {
            let packed: f64 = unit
                .chunks(LZNT1_CHUNK)
                .map(|c| ((deflate_len(c, 1) as f64 * LZNT1_PENALTY) as usize).min(c.len()) as f64)
                .sum();
            let stored = (packed as usize).div_ceil(CLUSTER) * CLUSTER;
            if stored + CLUSTER <= unit.len().div_ceil(CLUSTER) * CLUSTER {
                stored
            } else {
                unit.len()
            }
        })
        .sum()
}

#[derive(Default, Clone, Copy)]
struct Tally {
    out: u64,
    secs: f64,
}

pub fn estimate(path: &Path) -> Result<Estimate> {
    let md = std::fs::symlink_metadata(path)?;
    if md.file_type().is_symlink() {
        bail!(
            "{} is a link; point at the real file or folder",
            path.display()
        );
    }
    let is_dir = md.is_dir();
    let (probes, logical): (Vec<Probe>, u64) = if is_dir {
        let (mut files, total) = walk(path);
        files.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
        files.truncate(MAX_SAMPLES);
        let probes = files
            .into_iter()
            .filter(|(_, s)| *s > 0)
            .map(|(p, s)| Probe {
                compressed: identify_file(&p, s),
                path: p,
                size: s,
            })
            .collect();
        (probes, total)
    } else {
        let size = md.len();
        let probe = Probe {
            compressed: identify_file(path, size),
            path: path.to_path_buf(),
            size,
        };
        (vec![probe], size)
    };

    let mut est = Estimate {
        path: path.to_string_lossy().into_owned(),
        is_dir,
        logical_bytes: logical,
        sampled_bytes: 0,
        files_sampled: 0,
        already_compressed: false,
        already_compressed_bytes: 0,
        note: None,
        formats: Vec::new(),
    };
    let flagged: u64 = probes.iter().filter(|p| p.compressed).map(|p| p.size).sum();
    est.already_compressed_bytes = flagged;
    let workable: Vec<&Probe> = probes
        .iter()
        .filter(|p| !p.compressed && p.size > 0)
        .collect();
    est.already_compressed = logical > 0 && (workable.is_empty() || flagged >= logical);
    let rest_bytes = logical.saturating_sub(flagged);

    let per_file_max = if is_dir { 1 } else { MAX_SAMPLES };
    let mut tallies = [Tally::default(); 5];
    let mut in_bytes = 0u64;
    let mut read_secs = 0.0;
    if !est.already_compressed {
        for p in &workable {
            let t = Instant::now();
            let samples = match file_samples(&p.path, p.size, per_file_max) {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(path = %p.path.display(), error = %e, "estimate: unreadable");
                    continue;
                }
            };
            read_secs += t.elapsed().as_secs_f64();
            est.files_sampled += 1;
            for s in &samples {
                in_bytes += s.len() as u64;
                run_codecs(s, &mut tallies);
            }
        }
        est.sampled_bytes = in_bytes;
        if in_bytes == 0 {
            est.note = Some("nothing readable to sample".into());
        }
    } else {
        est.note = Some("already compressed data (media, archives, quantized weights); recompressing gains about nothing".into());
    }

    let read_bps = if read_secs > 0.0 {
        in_bytes as f64 / read_secs
    } else {
        f64::INFINITY
    };
    for (i, (format, in_place)) in FORMATS.iter().enumerate() {
        let (ratio, secs_per_byte) = if in_bytes == 0 {
            (1.0, 0.0)
        } else {
            let t = tallies[i];
            let mut spb = t.secs / in_bytes as f64 + 1.0 / read_bps;
            if *format == "7z" {
                spb = t.secs * LZMA_SLOWDOWN / in_bytes as f64 + 1.0 / read_bps;
            }
            ((t.out as f64 / in_bytes as f64).min(1.0), spb)
        };
        let compressed = (rest_bytes as f64 * ratio) as u64 + flagged;
        est.formats.push(FormatEstimate {
            format,
            in_place: *in_place,
            ratio,
            compressed_bytes: compressed,
            saved_bytes: logical.saturating_sub(compressed),
            seconds: rest_bytes as f64 * secs_per_byte,
            estimate: true,
        });
    }
    Ok(est)
}

fn run_codecs(data: &[u8], t: &mut [Tally; 5]) {
    let time = |f: &dyn Fn() -> usize| {
        let s = Instant::now();
        let n = f();
        (n.min(data.len()) as u64, s.elapsed().as_secs_f64())
    };
    let runs = [
        time(&|| deflate_len(data, 6)),
        // zstd at a high level stands in for LZMA, which is too slow to sample with.
        time(&|| zstd_len(data, 15)),
        time(&|| zstd_len(data, 6)),
        time(&|| lzx_len(data)),
        time(&|| lznt1_len(data)),
    ];
    for (tally, (out, secs)) in t.iter_mut().zip(runs) {
        tally.out += out;
        tally.secs += secs;
    }
}

#[cfg(test)]
mod tests;
