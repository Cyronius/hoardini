use serde::{Deserialize, Serialize};

// Ordered least to most protective; a cap is the minimum protection a verdict may show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Safe,
    Redownload,
    Review,
    Keep,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Safe => "safe",
            Verdict::Redownload => "redownload",
            Verdict::Review => "review",
            Verdict::Keep => "keep",
        }
    }

    pub fn parse(s: &str) -> Option<Verdict> {
        match s {
            "safe" => Some(Verdict::Safe),
            "redownload" => Some(Verdict::Redownload),
            "review" => Some(Verdict::Review),
            "keep" => Some(Verdict::Keep),
            _ => None,
        }
    }

    pub fn capped(self, cap: Option<Verdict>) -> Verdict {
        match cap {
            Some(c) if c > self => c,
            _ => self,
        }
    }
}
