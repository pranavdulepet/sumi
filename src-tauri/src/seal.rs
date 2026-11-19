use crate::{engine, scroll, sources, store::*};
use serde_json::{json, Value};
use std::{
    io::{BufRead, Write},
    path::Path,
};
pub const AGENTS: [&str; 4] = ["Claude Code", "Codex", "Claude Desktop", "Cursor"];
pub(crate) fn token_hash(token: &str) -> String {
    hash(&format!("sumi-seal/{token}"))
}
fn locate(s: &Store, token: &str) -> Res<Value> {
    s.list("agent")?
        .into_iter()
        .find(|a| a["linkState"].is_null() && a["tokenHash"] == token_hash(token))
        .ok_or("This agent is not connected.".into())
}
pub fn authorized(s: &Store, token: &str) -> bool {
    locate(s, token).is_ok()
}
fn scoped(s: &Store, a: &Value) -> Res<Vec<Value>> {
    let mut items = scroll::approved(s)?;
    items.retain(|v| {
        a["sections"]
            .as_array()
            .is_some_and(|ss| ss.contains(&v["section"]))
    });
    Ok(items)
}
fn shared_rules(items: Vec<Value>) -> Vec<Value> {
    items
        .into_iter()
        .map(|v| json!({"id":v["id"],"text":v["text"],"section":v["section"]}))
        .collect()
}
pub(crate) fn config_path(home: &Path, name: &str) -> Res<std::path::PathBuf> {
    Ok(home.join(match name {
        "Claude Code" => ".claude.json",
        "Codex" => ".codex/config.toml",
        "Claude Desktop" => "Library/Application Support/Claude/claude_desktop_config.json",
        "Cursor" => ".cursor/mcp.json",
        _ => return Err("Unknown agent.".into()),
    }))
}
pub fn detected() -> Vec<&'static str> {
    let home = sources::home();
    AGENTS
        .into_iter()
        .filter(|n| {
            config_path(&home, n).is_ok_and(|p| p.exists())
                || match *n {
                    "Codex" => home.join(".codex").is_dir(),
                    "Claude Code" => home.join(".claude").is_dir(),
                    "Claude Desktop" => Path::new("/Applications/Claude.app").exists(),
                    "Cursor" => Path::new("/Applications/Cursor.app").exists(),
                    _ => false,
                }
        })
        .collect()
}
pub use crate::agent_link::{connect, disconnect};
pub fn export(s: &Store) -> Res<()> {
    let mut agents = s.list("agent")?;
    agents.retain(|a| a["linkState"].is_null());
    for a in &mut agents {
        a.as_object_mut().unwrap().remove("tokenHash");
        a.as_object_mut().unwrap().remove("link");
    }
    let path = s.root.join("seal.yaml");
    let data = serde_json::to_vec_pretty(&json!({"agents":agents})).msg()?;
    if std::fs::read(&path).ok().as_deref() != Some(data.as_slice()) {
        atomic(&path, &data)?;
    }
    if s.get("sealExportPending")? == true {
        s.set("sealExportPending", &json!(false))?;
    }
    Ok(())
}
pub fn policy(s: &Store, a: &Value) -> Res<()> {
    let _lock = crate::agent_link::notebook_lock(s)?;
    let mut record = s.doc("agent", a["id"].as_str().unwrap_or(""))?;
    if !record["linkState"].is_null() {
        return Err("Finish changing this connection first.".into());
    }
    if !["Read", "Ask", "Answer for me"].contains(&a["permission"].as_str().unwrap_or("")) {
        return Err("Unknown permission.".into());
    }
    if !a["sections"].as_array().is_some_and(|v| {
        v.iter()
            .all(|x| scroll::SECTIONS.contains(&x.as_str().unwrap_or("")))
    }) {
        return Err("Invalid section.".into());
    }
    record["sections"] = a["sections"].clone();
    record["permission"] = a["permission"].clone();
    record["threshold"] = json!(a["threshold"].as_f64().unwrap_or(0.96).clamp(0.8, 1.0));
    s.set("sealExportPending", &json!(true))?;
    s.put("agent", &record)?;
    export(s)
}
pub fn call(s: &Store, token: &str, name: &str, args: &Value) -> Res<Value> {
    // A queued request and revocation must agree on whether the ticket can still be answered.
    let _ticket_lock = if name == "sumi_answer" {
        Some(crate::agent_link::notebook_lock(s)?)
    } else {
        None
    };
    let a = locate(s, token)?;
    let p = a["permission"].as_str().unwrap_or("Read");
    let who = a["id"].as_str().unwrap();
    let q = args["question"]
        .as_str()
        .or(args["task"].as_str())
        .or(args["event"].as_str())
        .unwrap_or("");
    let q = if name == "sumi_answer" && args["ticket"].is_string() {
        "Check a queued question".into()
    } else {
        sources::audit(q).ok_or("This request cannot be retained.")?
    };
    let mut log = json!({"id":id(),"agent":who,"name":a["name"],"date":now(),"tool":name,"question":q,"status":"served"});
    let result = match name {
        "sumi_context" => json!({"entries":shared_rules(engine::rank(scoped(s,&a)?,&q))}),
        "sumi_predict" if p != "Read" => {
            let entries = shared_rules(engine::select_rules(s, &q, scoped(s, &a)?));
            let model = s.get("model")?;
            let guess = if args["options"].is_null() {
                engine::predict_ranked(&model, &q, &entries, crate::jobs::prompt(s))?
            } else {
                let choices = args["options"]
                    .as_array()
                    .filter(|o| (2..=16).contains(&o.len()))
                    .ok_or("Provide two to sixteen choices.")?;
                for option in choices {
                    let label = option
                        .as_str()
                        .filter(|v| !v.trim().is_empty() && v.len() <= 240)
                        .ok_or("Use short text options.")?;
                    if sources::audit(label).as_deref() != Some(label) {
                        return Err("These option labels include private identifiers.".into());
                    }
                }
                let schema = json!({"type":"object","properties":{"choice":{"type":"string","enum":choices}},"required":["choice"],"additionalProperties":false});
                let raw = crate::models::infer_with(
                    &model,
                    crate::jobs::prompt(s),
                    &json!({"situation":q,"rules":entries,"options":choices}).to_string(),
                    120,
                    Some(&schema),
                )?;
                let answer: Value = serde_json::from_str(&raw).msg()?;
                if !choices.contains(&answer["choice"]) {
                    return Err("The local model abstained.".into());
                }
                answer["choice"].as_str().unwrap().to_owned()
            };
            json!({"guess":guess,"confidence":null,"rules":entries,"advisory":true})
        }
        "sumi_answer" if p == "Answer for me" => {
            if let Some(ticket) = args["ticket"].as_str() {
                let old = s.doc("log", ticket)?;
                if old["agent"] != who
                    || old["tool"] != "sumi_answer"
                    || !matches!(
                        old["status"].as_str(),
                        Some("waiting" | "answered" | "reviewed")
                    )
                {
                    return Err("Unknown question for this agent.".into());
                }
                let result = if old["human"].is_string() {
                    json!({"answer":old["human"],"decidedBy":"user"})
                } else if old["status"] == "answered" {
                    old["shared"].clone()
                } else {
                    json!({"queued":true,"id":ticket})
                };
                log["ticket"] = json!(ticket);
                log["status"] = json!("checked");
                log["shared"] = result.clone();
                s.put("log", &log)?;
                return Ok(result);
            }

            // Only these closed preference questions are eligible for automatic replies. Other wording always escalates.
            let options = args["options"].as_array().cloned().unwrap_or_default();
            let safe = match q.as_str() {
                "Which response length do you prefer?" => {
                    options == vec![json!("Brief"), json!("Detailed")]
                }
                "Which format do you prefer?" => options == vec![json!("Bullets"), json!("Prose")],
                _ => false,
            };
            let hist: Vec<Value> = s
                .list("log")?
                .into_iter()
                .filter(|l| {
                    l["agent"] == who
                        && l["question"] == q
                        && l["options"] == json!(options)
                        && l["human"].is_string()
                })
                .collect();
            let predicted = hist.first().and_then(|l| l["human"].as_str());
            let checked: Vec<_> = hist.iter().filter(|l| l["shadow"].is_string()).collect();
            let confidence = (1.
                + checked.iter().filter(|l| l["shadow"] == l["human"]).count() as f64)
                / (2. + checked.len() as f64);
            log["shadow"] = json!(predicted);
            log["confidence"] = json!(confidence);
            log["options"] = json!(options);
            if safe
                && checked.len() >= 20
                && confidence >= a["threshold"].as_f64().unwrap_or(1.)
                && options.iter().any(|v| v.as_str() == predicted)
            {
                log["status"] = json!("answered");
                json!({"id":log["id"],"answer":predicted,"confidence":confidence})
            } else {
                log["status"] = json!("waiting");
                json!({"queued":true,"id":log["id"]})
            }
        }
        "sumi_observe" => {
            log["status"] = json!("observed");
            sources::add(
                s,
                &format!("agent-{who}"),
                &id(),
                &now(),
                "Agent-reported observation",
                &q,
            )?;
            json!({"recorded":true})
        }
        _ => return Err("This permission does not allow that tool.".into()),
    };
    if name == "sumi_predict" {
        let current = locate(s, token)?;
        if current["permission"] != a["permission"] || current["sections"] != a["sections"] {
            return Err("The agent permissions changed during this prediction.".into());
        }
    }
    log["shared"] = result.clone();
    s.put("log", &log)?;
    Ok(result)
}
fn tools_list(permission: &str) -> Value {
    let mut out = vec![];
    for (name, arg, detail) in [
        (
            "sumi_context",
            "task",
            "Read relevant permitted Scroll entries.",
        ),
        ("sumi_predict", "question", "Ask for an advisory guess."),
        (
            "sumi_answer",
            "question",
            "Ask for a low-stakes preference or queue it for the person.",
        ),
        ("sumi_observe", "event", "Record an observed correction."),
    ] {
        if (name == "sumi_answer" && permission != "Answer for me")
            || (name == "sumi_predict" && permission == "Read")
        {
            continue;
        }
        let mut tool = json!({"name":name,"description":detail,"inputSchema":{"type":"object","properties":{(arg):{"type":"string"},"options":{"type":"array","items":{"type":"string"}},"ticket":{"type":"string","description":"ID of a previously queued question."}},"required":[arg]}});
        if name == "sumi_answer" {
            tool["inputSchema"]
                .as_object_mut()
                .unwrap()
                .remove("required");
            tool["inputSchema"]["anyOf"] =
                json!([{"required":["question"]},{"required":["ticket"]}]);
        }
        out.push(tool)
    }
    json!({"tools":out})
}
pub fn rpc(s: &Store, token: &str, req: &Value) -> Option<Value> {
    if req.get("id").is_none() {
        return None;
    }
    let result = (|| -> Res<Value> {
        let a = locate(s, token)?;
        match req["method"].as_str().unwrap_or("") {
            "initialize" => Ok(
                json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{},"resources":{}},"serverInfo":{"name":"Sumi","version":"1.0.0"}}),
            ),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(tools_list(a["permission"].as_str().unwrap_or("Read"))),
            "resources/list" => Ok(
                json!({"resources":a["sections"].as_array().into_iter().flatten().map(|v|json!({"uri":format!("sumi://scroll/{}",v.as_str().unwrap_or("")),"name":v,"mimeType":"text/plain"})).collect::<Vec<_>>()}),
            ),
            "resources/read" => {
                let uri = req["params"]["uri"].as_str().unwrap_or("");
                let sec = uri
                    .strip_prefix("sumi://scroll/")
                    .ok_or("Unknown resource")?;
                if !a["sections"].as_array().unwrap().iter().any(|v| v == sec) {
                    return Err("Section is outside this agent's scope.".into());
                }
                let scoped_rows: Vec<_> = scoped(s, &a)?
                    .into_iter()
                    .filter(|v| v["section"] == sec)
                    .collect();
                let rule_ids = scoped_rows
                    .iter()
                    .map(|v| v["id"].clone())
                    .collect::<Vec<_>>();
                let entries = scoped_rows
                    .iter()
                    .map(|v| v["text"].as_str().unwrap_or("").to_owned())
                    .collect::<Vec<_>>();
                s.put("log",&json!({"id":id(),"date":now(),"agent":a["id"],"name":a["name"],"tool":"resources/read","question":uri,"ruleIds":rule_ids,"status":"served","shared":entries}))?;
                Ok(
                    json!({"contents":[{"uri":uri,"mimeType":"text/plain","text":entries.join("\n")}]}),
                )
            }
            "tools/call" => {
                let r = call(
                    s,
                    token,
                    req["params"]["name"].as_str().unwrap_or(""),
                    &req["params"]["arguments"],
                );
                Ok(match r {
                    Ok(v) => json!({"content":[{"type":"text","text":v.to_string()}]}),
                    Err(e) => json!({"isError":true,"content":[{"type":"text","text":e}]}),
                })
            }
            _ => Err("Unsupported method.".into()),
        }
    })();
    Some(match result {
        Ok(v) => json!({"jsonrpc":"2.0","id":req["id"],"result":v}),
        Err(e) => json!({"jsonrpc":"2.0","id":req["id"],"error":{"code":-32000,"message":e}}),
    })
}
pub fn serve(s: &Store, token: &str) -> Res<()> {
    for line in std::io::stdin().lock().lines() {
        let line = line.msg()?;
        let Ok(v) = serde_json::from_str(&line) else {
            continue;
        };
        if let Some(out) = rpc(s, token, &v) {
            writeln!(std::io::stdout().lock(), "{out}").msg()?
        }
    }
    Ok(())
}
