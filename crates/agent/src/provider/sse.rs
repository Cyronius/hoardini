#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    data: Vec<String>,
    event: Option<String>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(bytes);
        let buf = std::mem::take(&mut self.buf);
        let mut out = Vec::new();
        let mut pos = 0;
        while let Some(rel) = buf[pos..].iter().position(|&b| b == b'\n' || b == b'\r') {
            let i = pos + rel;
            let next = if buf[i] == b'\r' {
                // A trailing \r may be half of \r\n; wait for the next chunk.
                match buf.get(i + 1) {
                    None => break,
                    Some(b'\n') => i + 2,
                    Some(_) => i + 1,
                }
            } else {
                i + 1
            };
            self.line(&buf[pos..i], &mut out);
            pos = next;
        }
        self.buf = buf[pos..].to_vec();
        out
    }

    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut out = Vec::new();
        let rest = std::mem::take(&mut self.buf);
        if !rest.is_empty() {
            self.line(rest.strip_suffix(b"\r").unwrap_or(&rest), &mut out);
        }
        self.dispatch(&mut out);
        out
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        let event = self.event.take();
        if !self.data.is_empty() {
            out.push(SseEvent {
                event,
                data: self.data.drain(..).collect::<Vec<_>>().join("\n"),
            });
        }
    }

    fn line(&mut self, raw: &[u8], out: &mut Vec<SseEvent>) {
        let line = String::from_utf8_lossy(raw);
        if line.is_empty() {
            return self.dispatch(out);
        }
        if line.starts_with(':') {
            return;
        }
        // Some servers put a bare JSON error body in a stream; surface it as data.
        if self.data.is_empty() && line.starts_with('{') {
            self.data.push(line.into_owned());
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line.as_ref(), ""),
        };
        match field {
            "data" => self.data.push(value.to_string()),
            "event" => self.event = Some(value.to_string()),
            _ => {}
        }
    }
}
