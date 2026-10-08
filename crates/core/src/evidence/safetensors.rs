use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};

use serde::{Deserialize, Serialize};

pub const MAX_HEADER: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SafetensorsInfo {
    pub header_len: u64,
    pub tensor_count: u64,
    pub parameters: u64,
    pub dtypes: BTreeMap<String, u64>,
    pub metadata: BTreeMap<String, String>,
}

impl SafetensorsInfo {
    pub fn dominant_dtype(&self) -> Option<&str> {
        self.dtypes
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(d, _)| d.as_str())
    }
}

pub fn header_len_if_plausible(first_bytes: &[u8], file_len: u64) -> Option<u64> {
    if first_bytes.len() < 9 || first_bytes[8] != b'{' {
        return None;
    }
    let n = u64::from_le_bytes(first_bytes[..8].try_into().ok()?);
    ((2..=MAX_HEADER).contains(&n) && n.checked_add(8)? <= file_len).then_some(n)
}

pub fn parse<R: Read + Seek>(mut r: R, file_len: u64) -> std::io::Result<SafetensorsInfo> {
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_string());
    r.seek(SeekFrom::Start(0))?;
    let mut head = [0u8; 9];
    r.read_exact(&mut head)?;
    let n = header_len_if_plausible(&head, file_len).ok_or_else(|| bad("not safetensors"))?;
    let mut buf = vec![0u8; n as usize];
    buf[0] = b'{';
    r.read_exact(&mut buf[1..])?;
    let json: serde_json::Value = serde_json::from_slice(&buf).map_err(|e| bad(&e.to_string()))?;
    let obj = json
        .as_object()
        .ok_or_else(|| bad("header is not an object"))?;

    let mut info = SafetensorsInfo {
        header_len: n,
        ..Default::default()
    };
    for (name, v) in obj {
        if name == "__metadata__" {
            if let Some(m) = v.as_object() {
                for (k, val) in m {
                    let s = val.as_str().map_or_else(|| val.to_string(), str::to_string);
                    info.metadata.insert(k.clone(), s);
                }
            }
            continue;
        }
        let Some(dtype) = v.get("dtype").and_then(|d| d.as_str()) else {
            continue;
        };
        let elems = v
            .get("shape")
            .and_then(|s| s.as_array())
            .map(|dims| {
                dims.iter()
                    .map(|d| d.as_u64().unwrap_or(0))
                    .fold(1u64, u64::saturating_mul)
            })
            .unwrap_or(0);
        info.tensor_count += 1;
        info.parameters = info.parameters.saturating_add(elems);
        *info.dtypes.entry(dtype.to_string()).or_default() += elems;
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_metadata_dtypes_and_params() {
        let json = br#"{"__metadata__":{"format":"pt"},"a":{"dtype":"BF16","shape":[4,8],"data_offsets":[0,64]},"b":{"dtype":"F32","shape":[2],"data_offsets":[64,72]}}"#;
        let mut file = (json.len() as u64).to_le_bytes().to_vec();
        file.extend_from_slice(json);
        file.extend_from_slice(&[0u8; 72]);
        let len = file.len() as u64;
        let info = parse(std::io::Cursor::new(file), len).unwrap();
        assert_eq!(info.parameters, 34);
        assert_eq!(info.tensor_count, 2);
        assert_eq!(info.dominant_dtype(), Some("BF16"));
        assert_eq!(info.metadata["format"], "pt");
    }
}
