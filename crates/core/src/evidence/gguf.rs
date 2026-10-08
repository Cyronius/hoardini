use std::collections::BTreeMap;
use std::io::{BufReader, Read, Seek, SeekFrom};

use serde::{Deserialize, Serialize};

pub const MAX_KV_REGION: u64 = 32 * 1024 * 1024;
pub const MAX_HEADER: u64 = 64 * 1024 * 1024;
const MAX_STRING: u64 = 16 * 1024 * 1024;
const MAX_TENSORS: u64 = 1_000_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GgufInfo {
    pub version: u32,
    pub tensor_count: u64,
    pub kv_count: u64,
    pub name: Option<String>,
    pub architecture: Option<String>,
    pub file_type: Option<u32>,
    pub quant: Option<String>,
    pub size_label: Option<String>,
    pub context_length: Option<u64>,
    pub parameters: Option<u64>,
    pub truncated: bool,
}

fn ftype_name(v: u32) -> Option<&'static str> {
    Some(match v {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "Q2_K_S",
        22 => "IQ3_XS",
        23 => "IQ3_XXS",
        24 => "IQ1_S",
        25 => "IQ4_NL",
        26 => "IQ3_S",
        27 => "IQ3_M",
        28 => "IQ2_S",
        29 => "IQ2_M",
        30 => "IQ4_XS",
        31 => "IQ1_M",
        32 => "BF16",
        36 => "TQ1_0",
        37 => "TQ2_0",
        38 => "MXFP4_MOE",
        _ => return None,
    })
}

fn ggml_type_name(t: u32) -> Option<&'static str> {
    Some(match t {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        6 => "Q5_0",
        7 => "Q5_1",
        8 => "Q8_0",
        9 => "Q8_1",
        10 => "Q2_K",
        11 => "Q3_K",
        12 => "Q4_K",
        13 => "Q5_K",
        14 => "Q6_K",
        15 => "Q8_K",
        16 => "IQ2_XXS",
        17 => "IQ2_XS",
        18 => "IQ3_XXS",
        19 => "IQ1_S",
        20 => "IQ4_NL",
        21 => "IQ3_S",
        22 => "IQ2_S",
        23 => "IQ4_XS",
        24 => "I8",
        25 => "I16",
        26 => "I32",
        27 => "I64",
        28 => "F64",
        29 => "IQ1_M",
        30 => "BF16",
        34 => "TQ1_0",
        35 => "TQ2_0",
        39 => "MXFP4",
        _ => return None,
    })
}

pub fn is_float_quant(q: &str) -> bool {
    matches!(q, "F32" | "F16" | "BF16" | "F64")
}

enum Val {
    U(u64),
    I(i64),
    S(String),
    Other,
}

impl Val {
    fn as_u64(&self) -> Option<u64> {
        match self {
            Val::U(v) => Some(*v),
            Val::I(v) if *v >= 0 => Some(*v as u64),
            _ => None,
        }
    }
}

struct Cur<R: Read + Seek> {
    r: BufReader<R>,
}

type Res<T> = std::io::Result<T>;

fn eof() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "gguf: malformed or over budget",
    )
}

impl<R: Read + Seek> Cur<R> {
    fn pos(&mut self) -> Res<u64> {
        self.r.stream_position()
    }

    fn u32(&mut self) -> Res<u32> {
        let mut b = [0u8; 4];
        self.r.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn u64(&mut self) -> Res<u64> {
        let mut b = [0u8; 8];
        self.r.read_exact(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    fn string(&mut self) -> Res<String> {
        let n = self.u64()?;
        if n > MAX_STRING {
            return Err(eof());
        }
        let mut buf = vec![0u8; n as usize];
        self.r.read_exact(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    fn skip(&mut self, n: u64) -> Res<()> {
        self.r.seek_relative(i64::try_from(n).map_err(|_| eof())?)
    }

    fn skip_string(&mut self) -> Res<()> {
        let n = self.u64()?;
        self.skip(n)
    }

    fn scalar(&mut self, t: u32) -> Res<Val> {
        Ok(match t {
            0 => Val::U(self.fixed(1)?),
            1 => Val::I(self.fixed(1)? as u8 as i8 as i64),
            2 => Val::U(self.fixed(2)?),
            3 => Val::I(self.fixed(2)? as u16 as i16 as i64),
            4 => Val::U(self.fixed(4)?),
            5 => Val::I(self.fixed(4)? as u32 as i32 as i64),
            6 | 7 => {
                self.skip(if t == 6 { 4 } else { 1 })?;
                Val::Other
            }
            8 => Val::S(self.string()?),
            10 => Val::U(self.fixed(8)?),
            11 => Val::I(self.fixed(8)? as i64),
            12 => {
                self.skip(8)?;
                Val::Other
            }
            _ => return Err(eof()),
        })
    }

    fn fixed(&mut self, n: usize) -> Res<u64> {
        let mut b = [0u8; 8];
        self.r.read_exact(&mut b[..n])?;
        Ok(u64::from_le_bytes(b))
    }

    fn value(&mut self, t: u32, keep: bool) -> Res<Val> {
        if t != 9 {
            if !keep && t == 8 {
                self.skip_string()?;
                return Ok(Val::Other);
            }
            return self.scalar(t);
        }
        let et = self.u32()?;
        let n = self.u64()?;
        match et {
            0 | 1 | 7 => self.skip(n)?,
            2 | 3 => self.skip(n.checked_mul(2).ok_or_else(eof)?)?,
            4..=6 => self.skip(n.checked_mul(4).ok_or_else(eof)?)?,
            10..=12 => self.skip(n.checked_mul(8).ok_or_else(eof)?)?,
            8 => {
                for _ in 0..n {
                    self.skip_string()?;
                    if self.pos()? > MAX_KV_REGION {
                        return Err(eof());
                    }
                }
            }
            _ => return Err(eof()),
        }
        Ok(Val::Other)
    }
}

pub fn parse<R: Read + Seek>(mut reader: R, file_len: u64) -> std::io::Result<GgufInfo> {
    reader.seek(SeekFrom::Start(0))?;
    let mut cur = Cur {
        r: BufReader::with_capacity(64 * 1024, reader),
    };
    let mut magic = [0u8; 4];
    cur.r.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Err(eof());
    }
    let version = cur.u32()?;
    if !(2..=3).contains(&version) {
        return Err(eof());
    }
    let tensor_count = cur.u64()?;
    let kv_count = cur.u64()?;
    if kv_count > 1_000_000 || tensor_count > MAX_TENSORS {
        return Err(eof());
    }
    let mut info = GgufInfo {
        version,
        tensor_count,
        kv_count,
        ..Default::default()
    };
    let mut ints: BTreeMap<String, u64> = BTreeMap::new();

    for _ in 0..kv_count {
        if cur.pos()? > MAX_KV_REGION.min(file_len) {
            info.truncated = true;
            return Ok(finish(info, &ints, None));
        }
        let key = match cur.string() {
            Ok(k) => k,
            Err(_) => {
                info.truncated = true;
                return Ok(finish(info, &ints, None));
            }
        };
        let t = cur.u32()?;
        let wanted = key.starts_with("general.") || key.ends_with(".context_length");
        let v = match cur.value(t, wanted) {
            Ok(v) => v,
            Err(_) => {
                info.truncated = true;
                return Ok(finish(info, &ints, None));
            }
        };
        match (key.as_str(), &v) {
            ("general.name", Val::S(s)) => info.name = Some(s.clone()),
            ("general.architecture", Val::S(s)) => info.architecture = Some(s.clone()),
            ("general.size_label", Val::S(s)) => info.size_label = Some(s.clone()),
            ("general.file_type", v) => info.file_type = v.as_u64().map(|x| x as u32),
            (k, v) if k.ends_with(".context_length") => {
                if let Some(x) = v.as_u64() {
                    ints.insert(k.to_string(), x);
                }
            }
            _ => {}
        }
    }

    let mut params: u64 = 0;
    let mut by_type: BTreeMap<u32, u64> = BTreeMap::new();
    let mut complete = true;
    for _ in 0..tensor_count {
        let ok = (|| -> Res<()> {
            if cur.pos()? > MAX_HEADER {
                return Err(eof());
            }
            cur.skip_string()?;
            let nd = cur.u32()?;
            if nd > 8 {
                return Err(eof());
            }
            let mut elems: u64 = 1;
            for _ in 0..nd {
                elems = elems.saturating_mul(cur.u64()?);
            }
            let ty = cur.u32()?;
            cur.skip(8)?;
            params = params.saturating_add(elems);
            *by_type.entry(ty).or_default() += elems;
            Ok(())
        })();
        if ok.is_err() {
            complete = false;
            break;
        }
    }
    info.truncated = !complete;
    let dominant = by_type
        .iter()
        .max_by_key(|(_, n)| **n)
        .and_then(|(t, _)| ggml_type_name(*t));
    let mut out = finish(info, &ints, dominant);
    out.parameters = complete.then_some(params);
    Ok(out)
}

fn finish(mut info: GgufInfo, ints: &BTreeMap<String, u64>, dominant: Option<&str>) -> GgufInfo {
    info.context_length = info
        .architecture
        .as_ref()
        .and_then(|a| ints.get(&format!("{a}.context_length")))
        .or_else(|| ints.values().next())
        .copied();
    info.quant = info
        .file_type
        .and_then(ftype_name)
        .or(dominant)
        .map(str::to_string);
    info
}
