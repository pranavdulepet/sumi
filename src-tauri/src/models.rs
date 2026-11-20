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
    infer_reply(config, system, input, limit, schema, None)
}
pub fn infer_reply(
    config: &Value,
    system: &str,
    input: &str,
    limit: u32,
    schema: Option<&Value>,
    sink: Option<&mut dyn FnMut(&str)>,
) -> Res<String> {
    let name = config["name"]
        .as_str()
        .ok_or("Choose a local model first.")?;
    let messages = json!([{"role":"system","content":system},{"role":"user","content":input}]);
    if config["runtime"] == "Sumi" {
        return crate::bundled::infer(config, messages, limit, schema, sink);
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
    body["stream"] = json!(sink.is_some());
    let response = agent(120)
        .post(&url)
        .send_json(body)
        .map_err(|e| format!("Local model unavailable: {e}"))?;
    if let Some(sink) = sink {
        return consume_chat(
            response.into_reader(),
            config["runtime"] == "Ollama",
            limit,
            sink,
        );
    }
    let v: Value = response.into_json().msg()?;
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

pub fn consume_chat(
    reader: impl std::io::Read,
    ollama: bool,
    limit: u32,
    sink: &mut dyn FnMut(&str),
) -> Res<String> {
    use std::io::{BufRead, BufReader};
    let mut answer = String::new();
    let mut complete = false;
    for line in BufReader::new(reader.take(2_097_152)).lines() {
        let line = line.msg()?;
        let raw = if ollama {
            line.as_str()
        } else if let Some(data) = line.strip_prefix("data:") {
            data.trim()
        } else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }
        if raw == "[DONE]" {
            complete = true;
            break;
        }
        let event: Value =
            serde_json::from_str(raw).map_err(|_| "The model returned an unreadable response.")?;
        if !event["error"].is_null() {
            return Err("The model stopped before finishing.".into());
        }
        let delta = if ollama {
            event.pointer("/message/content")
        } else {
            event.pointer("/choices/0/delta/content")
        }
        .and_then(Value::as_str)
        .unwrap_or("");
        if answer.len() + delta.len() > limit as usize * 16 {
            return Err("The answer exceeded the notebook limit.".into());
        }
        if !delta.is_empty() {
            answer.push_str(delta);
            sink(delta);
        }
        if ollama && event["done"] == true {
            complete = true;
            break;
        }
    }
    if complete && !answer.trim().is_empty() {
        Ok(answer.trim().to_owned())
    } else {
        Err("The model stopped before finishing. Try again.".into())
    }
}

#[cfg(test)]
mod streaming {
    use super::*;
    #[test]
    fn both_local_protocols_deliver_fragments_and_reject_disconnects() {
        for (ollama,body,end) in [
            (true,"{\"message\":{\"content\":\"One\"},\"done\":false}\n{\"message\":{\"content\":\" two\"},\"done\":false}\n","{\"done\":true}\n"),
            (false,"data: {\"choices\":[{\"delta\":{\"content\":\"One\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\" two\"}}]}\n\n","data: [DONE]\n\n")
        ]{
            let mut pieces=vec![];
            assert!(consume_chat(body.as_bytes(),ollama,40,&mut |_|{}).is_err());
            assert_eq!(consume_chat(format!("{body}{end}").as_bytes(),ollama,40,&mut |d|pieces.push(d.to_owned())).unwrap(),"One two");
            assert_eq!(pieces,["One"," two"]);
        }
    }
}

pub fn vision(model: &Value) -> bool {
    agent(3)
        .post(&format!("{OLLAMA}/api/show"))
        .send_json(json!({"model":model["name"]}))
        .ok()
        .and_then(|r| r.into_json::<Value>().ok())
        .is_some_and(|v| {
            v["capabilities"]
                .as_array()
                .is_some_and(|c| c.contains(&json!("vision")))
        })
}
pub fn photo_reply(
    model: &Value,
    system: &str,
    input: &Value,
    limit: u32,
    photos: &[Value],
    sink: Option<&mut dyn FnMut(&str)>,
) -> Res<String> {
    let response=agent(120).post(&format!("{OLLAMA}/api/chat")).send_json(json!({"model":model["name"],"stream":true,"think":false,"options":{"temperature":0,"num_predict":limit},"messages":[{"role":"system","content":system},{"role":"user","content":input.to_string(),"images":photos}]})).msg()?;
    consume_chat(
        response.into_reader(),
        true,
        limit,
        sink.unwrap_or(&mut |_| {}),
    )
}
