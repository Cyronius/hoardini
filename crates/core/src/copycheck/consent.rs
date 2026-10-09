use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::store::{get_meta, set_meta};

const KEY: &str = "online_checks";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    Huggingface,
    Winget,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Services {
    #[serde(default)]
    pub huggingface: bool,
    #[serde(default)]
    pub winget: bool,
}

// Online checks stay off until the user has answered; offline checks never consult this.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnlineConsent {
    #[serde(default)]
    pub asked: bool,
    #[serde(default)]
    pub services: Services,
}

impl OnlineConsent {
    pub fn allows(&self, s: Service) -> bool {
        match s {
            Service::Huggingface => self.services.huggingface,
            Service::Winget => self.services.winget,
        }
    }

    pub fn all() -> OnlineConsent {
        OnlineConsent {
            asked: true,
            services: Services {
                huggingface: true,
                winget: true,
            },
        }
    }
}

pub fn load(conn: &Connection) -> OnlineConsent {
    get_meta(conn, KEY)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

pub fn save(conn: &Connection, c: &OnlineConsent) -> anyhow::Result<()> {
    set_meta(conn, KEY, &serde_json::to_string(c)?)
}

// Applies a partial `{asked?, services: {huggingface?, winget?}}`; anything else is an error and nothing changes.
pub fn patch(current: OnlineConsent, v: &serde_json::Value) -> Result<OnlineConsent, String> {
    let o = v
        .as_object()
        .ok_or("online_checks must be an object like {\"asked\": true, \"services\": {...}}")?;
    let mut next = current;
    for (k, val) in o {
        match k.as_str() {
            "asked" => {
                next.asked = val
                    .as_bool()
                    .ok_or("online_checks.asked must be true or false")?
            }
            "services" => {
                let s = val
                    .as_object()
                    .ok_or("online_checks.services must be an object")?;
                for (name, on) in s {
                    let on = on.as_bool().ok_or_else(|| {
                        format!("online_checks.services.{name} must be true or false")
                    })?;
                    match name.as_str() {
                        "huggingface" => next.services.huggingface = on,
                        "winget" => next.services.winget = on,
                        other => return Err(format!("unknown online check service {other:?}")),
                    }
                }
            }
            other => return Err(format!("unknown online_checks field {other:?}")),
        }
    }
    Ok(next)
}
