use crate::store::*;
use serde_json::{json, Value};
use std::time::Duration;
const OLLAMA: &str = "http://127.0.0.1:11434";
const STUDIO: &str = "http://127.0.0.1:1234";
fn agent(t: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .try_proxy_from_env(false)
        .timeout_connect(Duration::from_millis(850))
        .timeout_read(Duration::from_secs(t))
        .build()
}
pub fn inventory() -> Vec<Value> {
    let mut out = vec![];
    for (kind, base, url, field, name) in [
        ("Ollama", OLLAMA, "/api/tags", "models", "name"),
        ("LM Studio", STUDIO, "/v1/models", "data", "id"),
    ] {
        if let Ok(resp) = agent(3).get(&format!("{base}{url}")).call() {
            if let Ok(v) = resp.into_json::<Value>() {
                for m in v[field].as_array().into_iter().flatten() {
                    if let Some(n) = m[name].as_str() {
                        if !n.contains(":cloud") && !n.ends_with("-cloud") {
                            out.push(json!({"runtime":kind,"name":n,"bytes":m["size"].as_u64().unwrap_or(0)}))
                        }
                    }
                }
            }
        }
    }
    out.sort_by_key(|v| v["bytes"].as_u64().unwrap_or(0));
    out
}
pub fn infer(config: &Value, system: &str, input: &str, limit: u32) -> Res<String> {
    infer_with(config, system, input, limit, None)
}
pub fn infer_with(
    config: &Value,
    system: &str,
    input: &str,
    limit: u32,
    schema: Option<&Value>,
) -> Res<String> {
    let name = config["name"]
        .as_str()
        .ok_or("Choose a local model first.")?;
    let messages = json!([{"role":"system","content":system},{"role":"user","content":input}]);
    if config["runtime"] == "Sumi" {
        return crate::bundled::infer(config, messages, limit, schema);
    }
    let (url, mut body) = match config["runtime"].as_str() {
        Some("Ollama") => (
            format!("{OLLAMA}/api/chat"),
            json!({"model":name,"stream":false,"think":false,"keep_alive":"4m","messages":messages,"options":{"temperature":0,"num_predict":limit,"num_ctx":4096}}),
        ),
        Some("LM Studio") => (
            format!("{STUDIO}/v1/chat/completions"),
            json!({"model":name,"messages":messages,"temperature":0,"max_tokens":limit}),
        ),
        _ => return Err("That local runtime is unavailable.".into()),
    };
    if let Some(schema) = schema {
        if config["runtime"] == "Ollama" {
            body["format"] = schema.clone()
        } else {
            body["response_format"] = json!({"type":"json_schema","json_schema":{"name":"result","strict":true,"schema":schema}})
        }
    }
    let v: Value = agent(120)
        .post(&url)
        .send_json(body)
        .map_err(|e| format!("Local model unavailable: {e}"))?
        .into_json()
        .msg()?;
    let reply = v
        .pointer("/message/content")
        .or_else(|| v.pointer("/choices/0/message/content"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if reply.is_empty() {
        Err("The local model returned an empty answer.".into())
    } else {
        Ok(reply.to_owned())
    }
}
pub fn start_ollama() -> Res<()> {
    let binary = [
        "/Applications/Ollama.app/Contents/Resources/ollama",
        "/opt/homebrew/bin/ollama",
        "/usr/local/bin/ollama",
    ]
    .into_iter()
    .find(|s| std::path::Path::new(s).exists())
    .ok_or("Install Ollama or start LM Studio's local server, then refresh.")?;
    std::process::Command::new(binary)
        .arg("serve")
        .env("OLLAMA_HOST", "127.0.0.1:11434")
        .env("OLLAMA_NO_CLOUD", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .msg()?;
    Ok(())
}
