use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use hoardini_core::destinations::rclone::{self, authorize, daemon};
use hoardini_core::runtime::RCLONE;
use hoardini_core::vault::is_ref;
use serde::Deserialize;
use serde_json::json;

use crate::state::{ApiError, ApiResult, AppState, blocking, parse};

const AUTHORIZE_TIMEOUT: Duration = Duration::from_secs(300);

pub async fn status() -> ApiResult {
    blocking(|| {
        Ok(Json(json!({
            "installed": rclone::is_installed(),
            "version": RCLONE.version,
            "size": RCLONE.size,
            "license_url": RCLONE.license_url,
        })))
    })
    .await
}

pub async fn install(State(s): State<AppState>) -> ApiResult {
    blocking(move || {
        let job = s
            .exec
            .install_runtime(&RCLONE, hoardini_core::paths::data_dir())?;
        Ok(Json(json!(job)))
    })
    .await
}

#[derive(Deserialize)]
struct AuthorizeBody {
    #[serde(rename = "type")]
    kind: String,
    client_id_ref: Option<String>,
    client_secret_ref: Option<String>,
}

fn resolve(s: &AppState, reference: Option<&str>, what: &str) -> Result<Option<String>, ApiError> {
    use hoardini_core::vault::ExposeSecret;
    let Some(r) = reference.filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    if !is_ref(r) {
        return Err(ApiError::bad(format!(
            "{what} must be a secret:// reference"
        )));
    }
    match s.secrets.get(r)? {
        Some(v) => Ok(Some(v.expose_secret().to_string())),
        None => Err(ApiError::bad(format!(
            "{what} is not in the vault; save it first"
        ))),
    }
}

pub async fn authorize_remote(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: AuthorizeBody = parse(&body)?;
    let st = s.clone();
    let token_ref = blocking(move || {
        let id = resolve(&st, b.client_id_ref.as_deref(), "client_id_ref")?;
        let secret = resolve(&st, b.client_secret_ref.as_deref(), "client_secret_ref")?;
        if b.kind == "drive" && (id.is_none() || secret.is_none()) {
            return Err(ApiError::bad(
                "Google Drive needs your own OAuth client ID and secret",
            ));
        }
        let exe = daemon().verified_exe()?;
        let token = authorize(
            &exe,
            &b.kind,
            id.as_deref(),
            secret.as_deref(),
            AUTHORIZE_TIMEOUT,
        )?;
        let token_ref = format!("secret://rclone/token/{}", uuid::Uuid::now_v7().simple());
        st.secrets.set(&token_ref, token)?;
        Ok(token_ref)
    })
    .await?;
    s.bump();
    Ok(Json(json!({ "token_ref": token_ref })))
}
