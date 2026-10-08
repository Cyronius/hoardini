use anyhow::{Context, Result, bail};
use hoardini_agent::provider::{ChatMessage, ChatRequest, ProviderConfig, ToolDef, build_body};
use serde_json::json;

fn tool() -> ToolDef {
    ToolDef {
        name: "get_free_space".into(),
        description: "Return free space in GB for a drive letter.".into(),
        parameters: json!({"type": "object",
            "properties": {"drive": {"type": "string"}}, "required": ["drive"]}),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        bail!(
            "usage: record_fixture <base_url> <model> <text|tool|parallel|unknown-model> <out.sse>"
        );
    }
    let (base, model, scenario, out) = (&args[1], &args[2], args[3].as_str(), &args[4]);
    let mut cfg = ProviderConfig::new("record", base);
    cfg.model = Some(model.clone());
    cfg.params.temperature = Some(0.0);
    cfg.params.max_tokens = Some(400);
    let mut req = ChatRequest {
        include_usage: true,
        ..Default::default()
    };
    let prompt = match scenario {
        "text" => "In one short sentence, what is a hard drive?",
        "tool" => "How much free space is on drive C? Use the tool.",
        "parallel" => {
            "Check the free space of drive C and of drive D. Call the tool once per drive."
        }
        "unknown-model" => {
            cfg.model = Some("this-model-does-not-exist-xyz".into());
            "Say hello."
        }
        other => bail!("unknown scenario {other}"),
    };
    req.messages.push(ChatMessage::text("user", prompt));
    if matches!(scenario, "tool" | "parallel") {
        req.tools.push(tool());
    }
    let body = build_body(&cfg, &req, true);
    let resp = reqwest::Client::new()
        .post(cfg.endpoint("chat/completions"))
        .json(&body)
        .send()
        .await
        .context("request failed")?;
    let status = resp.status();
    let raw = resp.bytes().await?;
    let header = format!(
        ": recorded from {base} model={} scenario={scenario} http={}\n",
        cfg.model.as_deref().unwrap_or(""),
        status.as_u16()
    );
    let mut bytes = header.into_bytes();
    bytes.extend_from_slice(&raw);
    std::fs::write(out, &bytes)?;
    println!("wrote {} bytes to {out} (HTTP {status})", bytes.len());
    Ok(())
}
