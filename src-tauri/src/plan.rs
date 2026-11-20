use crate::{providers, sources, store::*};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read};

pub(crate) const API: &str = "https://api.openai.com/v1";
pub const MONTHLY_LIMIT: u64 = 1200;

pub fn selected(model: &Value) -> bool {
    model["runtime"] == "ChatGPT"
}

// Check provenance before callers reduce a Scroll entry to its text.
pub fn usable(model: &Value, row: &Value) -> bool {
    !selected(model) || cloud_usable(row)
}

pub fn cloud_usable(row: &Value) -> bool {
    let private_source =
        |value: &Value| matches!(value.as_str(), Some("messages" | "private-desk"));
    row["localOnly"] != true
        && row["section"] != "people"
        && !private_source(&row["source"])
        && !row["sources"]
            .as_array()
            .is_some_and(|items| items.iter().any(private_source))
}

pub(crate) fn clean(value: &Value) -> Res<Value> {
    match value {
        Value::String(text) if !text.is_empty() => {
            if !crate::local::sensitive(text) {
                if let Some(clean) = sources::audit(text) {
                    return Ok(json!(clean));
                }
            }
            Err("This passage can only be read on your Mac.".into())
        }
        Value::Array(a) => a.iter().map(clean).collect(),
        Value::Object(obj) => obj
            .iter()
            .map(|(k, v)| Ok((k.clone(), clean(v)?)))
            .collect(),
        value => Ok(value.clone()),
    }
}

fn session(s: &Store) -> Res<(String, String)> {
    #[cfg(test)]
    if let Some(port) = tests::TEST_PORT.get() {
        return Ok((
            format!("http://127.0.0.1:{port}"),
            "synthetic-session".into(),
        ));
    }
    Ok((API.to_owned(), providers::access_token(s)?))
}

pub fn think(
    s: &Store,
    model: &Value,
    job: &str,
    system: &str,
    input: &Value,
    limit: u32,
    schema: Option<&Value>,
) -> Res<String> {
    think_with(s, model, job, system, input, limit, schema, None)
}
pub fn think_with(
    s: &Store,
    model: &Value,
    job: &str,
    system: &str,
    input: &Value,
    limit: u32,
    schema: Option<&Value>,
    sink: Option<&mut dyn FnMut(&str)>,
) -> Res<String> {
    let mut fields = input.clone();
    let photos = fields
        .as_object_mut()
        .and_then(|f| f.remove("photo_inputs"))
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    if !selected(model)
        && !photos.is_empty()
        && model["runtime"] == "Ollama"
        && crate::models::vision(model)
    {
        return crate::models::photo_reply(model, system, &fields, limit, &photos, sink);
    }
    if !selected(model) {
        return crate::models::infer_reply(model, system, &fields.to_string(), limit, schema, sink);
    }
    let connected = s.get("model")?;
    if connected != *model
        && !(selected(&connected)
            && model["name"].is_string()
            && connected["fast"] == model["name"])
    {
        return Err("The notebook's thinking setting changed. Try again.".into());
    }
    let input = clean(&fields)?;
    let name = model["name"].as_str().ok_or("Connect ChatGPT again.")?;
    let (api, token) = session(s)?;
    reserve(s)?;
    let answer = complete_with(
        &api,
        &token,
        name,
        system,
        &input.to_string(),
        limit,
        schema,
        sink,
        &photos,
    )?;
    s.put(
        "route",
        &json!({"id":id(),"date":now(),"job":job,"provider":"ChatGPT","model":name,"items":1}),
    )?;
    Ok(answer)
}

pub fn one_question(
    s: &Store,
    model: &Value,
    preview: &str,
    sink: &mut dyn FnMut(&str),
) -> Res<String> {
    if !selected(model) || s.get("model")? != *model {
        return Err("The connected plan changed.".into());
    }
    let (api, token) = session(s)?;
    reserve(s)?;
    let name = model["name"].as_str().ok_or("Connect ChatGPT again.")?;
    let answer = complete_with(
        &api,
        &token,
        name,
        "Answer the one question directly. You have no notebook context.",
        preview,
        650,
        None,
        Some(sink),
        &[],
    )?;
    s.put("route",&json!({"id":id(),"date":now(),"job":"one approved question","provider":"ChatGPT","model":name,"items":1}))?;
    Ok(answer)
}

fn reserve(s: &Store) -> Res<()> {
    providers::reserve(
        s,
        &json!({"id":"ChatGPT","enabled":true,"cap":MONTHLY_LIMIT}),
    )
}

pub fn verify_connection(s: &Store, token: &str) -> Res<Value> {
    reserve(s)?;
    verify_at(API, token)
}

fn verify_at(api: &str, token: &str) -> Res<Value> {
    let list: Value = providers::http()
        .get(&format!("{api}/models"))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(failure)?
        .into_json()
        .msg()?;
    let name = list["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"] == "list")
        .filter_map(|m| m["slug"].as_str())
        .find(|s| !s.is_empty())
        .ok_or("Your ChatGPT plan has no available model. Try thinking on this Mac.")?;
    let answer = complete(
        api,
        token,
        name,
        "Reply with only the word Ready.",
        "Connection check.",
        32,
        None,
    )?;
    if !answer
        .trim_matches(|c: char| !c.is_alphabetic())
        .eq_ignore_ascii_case("ready")
    {
        return Err("ChatGPT didn't finish the connection check. Try again.".into());
    }
    let fast = list["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"] == "list")
        .filter_map(|m| m["slug"].as_str())
        .find(|id| id != &name && ["mini", "instant", "spark"].iter().any(|k| id.contains(k)));
    let mut model = json!({"name":name,"runtime":"ChatGPT"});
    if let Some(fast) = fast {
        model["fast"] = json!(fast);
    }
    Ok(model)
}

fn failure(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(401, _) => "Sign in to ChatGPT again.",
        ureq::Error::Status(403, _) => "Your account hasn't allowed ChatGPT plan use for Sumi.",
        ureq::Error::Status(429, _) => {
            "Your ChatGPT plan limit was reached. Try later or think on this Mac."
        }
        _ => "Sumi couldn't reach ChatGPT. Try again or think on this Mac.",
    }
    .into()
}

pub(crate) fn complete(
    api: &str,
    token: &str,
    model: &str,
    system: &str,
    input: &str,
    limit: u32,
    schema: Option<&Value>,
) -> Res<String> {
    complete_with(api, token, model, system, input, limit, schema, None, &[])
}
pub(crate) fn complete_with(
    api: &str,
    token: &str,
    model: &str,
    system: &str,
    input: &str,
    limit: u32,
    schema: Option<&Value>,
    sink: Option<&mut dyn FnMut(&str)>,
    photos: &[Value],
) -> Res<String> {
    // Plan usage rejects max_output_tokens. Give the model a length instruction
    // and independently bound the output our client accepts.
    let mut body = json!({"model":model,"instructions":format!("{system}\nKeep your answer within {limit} tokens."),
        "input":[{"role":"user","content":input}],"stream":true,"store":false});
    if !photos.is_empty() {
        let mut parts = vec![json!({"type":"input_text","text":input})];
        for photo in photos.iter().filter_map(Value::as_str) {
            parts.push(
                json!({"type":"input_image","image_url":format!("data:image/jpeg;base64,{photo}")}),
            );
        }
        body["input"][0]["content"] = json!(parts);
    }
    if let Some(schema) = schema {
        body["text"] =
            json!({"format":{"type":"json_schema","name":"result","schema":schema,"strict":true}});
    }
    let response = providers::http()
        .post(&format!("{api}/responses"))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(body)
        .map_err(failure)?;
    consume_with(response.into_reader(), (limit as usize * 12).max(256), sink)
}

#[cfg(test)]
fn consume(reader: impl Read, max: usize) -> Res<String> {
    consume_with(reader, max, None)
}
fn consume_with(
    reader: impl Read,
    max: usize,
    mut sink: Option<&mut dyn FnMut(&str)>,
) -> Res<String> {
    let mut answer = String::new();
    // Include headroom for lifecycle events and a repeated copy of the final output.
    let stream = BufReader::new(reader.take(2 * 1024 * 1024));
    let mut data = Vec::new();
    for line in stream.lines() {
        let line = line.msg()?;
        if let Some(payload) = line.strip_prefix("data:") {
            data.push(payload.trim_start().to_owned());
        } else if line.is_empty() && !data.is_empty() {
            let payload = data.join("\n");
            data.clear();
            if payload == "[DONE]" {
                break;
            }
            let event: Value = serde_json::from_str(&payload)
                .map_err(|_| "ChatGPT returned an unreadable reply.")?;
            match event["type"].as_str() {
                Some("response.output_text.delta") => {
                    let delta = event["delta"].as_str().unwrap_or("");
                    answer.push_str(delta);
                    if answer.len() > max {
                        return Err("The reply was too long for this part of the notebook.".into());
                    }
                    if let Some(f) = sink.as_mut() {
                        f(delta)
                    }
                }
                Some("response.completed") if !answer.trim().is_empty() => {
                    return Ok(answer.trim().to_owned())
                }
                Some("response.failed" | "error") => {
                    let code = event
                        .pointer("/response/error/code")
                        .or(event.get("code"))
                        .and_then(Value::as_str);
                    return Err(match code {
                        Some("subscription_sharing_usage_limit_exceeded") => {
                            "Your ChatGPT plan limit was reached. Try later or think on this Mac."
                        }
                        Some("subscription_sharing_usage_unavailable") => {
                            "ChatGPT plan usage is unavailable for this account."
                        }
                        _ => "ChatGPT couldn't finish this reply.",
                    }
                    .into());
                }
                Some("response.incomplete") => break,
                _ => {}
            }
        }
    }
    Err("ChatGPT stopped before the reply was complete. Try again.".into())
}

#[cfg(test)]
pub fn simulated_hosted_account() -> bool {
    tests::TEST_PORT.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    thread_local! {pub(super) static TEST_PORT: std::cell::Cell<Option<u16>> = const {std::cell::Cell::new(None)};}
    fn event(v: Value) -> String {
        format!("data: {v}\r\n\r\n")
    }
    fn delta() -> String {
        event(json!({"type":"response.output_text.delta","delta":"Ready"}))
    }
    #[test]
    fn a_partial_or_overlong_reply_never_confirms_a_connection() {
        for terminal in ["response.failed", "response.incomplete", "error"] {
            let body = delta()
                + &event(
                    json!({"type":terminal,"response":{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}}),
                );
            assert!(consume(body.as_bytes(), 32).is_err());
        }
        assert!(consume(delta().as_bytes(), 32).is_err());
        let ok = delta() + &event(json!({"type":"response.completed"}));
        assert_eq!(consume(ok.as_bytes(), 32).unwrap(), "Ready");
        assert!(consume(ok.as_bytes(), 4).is_err());
    }
    #[test]
    fn messages_and_people_do_not_leave_with_the_plan() {
        let plan = json!({"runtime":"ChatGPT"});
        for private in [
            json!({"source":"messages"}),
            json!({"sources":["desk","messages"]}),
            json!({"section":"people"}),
        ] {
            assert!(!usable(&plan, &private));
            assert!(usable(&json!({"runtime":"Sumi"}), &private));
        }
        assert!(clean(&json!({"examples":[{"reply":"password=private-example"}]})).is_err());
        assert!(!clean(&json!({"text":"Send it to sample@example.org"}))
            .unwrap()
            .to_string()
            .contains("example.org"));
    }
    #[test]
    #[ignore = "Exercises the official ChatGPT plan HTTP contract on loopback"]
    fn verifies_an_account_model_and_consumes_the_completed_stream() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let worker = std::thread::spawn(move || {
            for path in ["/models", "/responses", "/responses"] {
                let mut req = server
                    .recv_timeout(std::time::Duration::from_secs(8))
                    .unwrap()
                    .expect("missing request");
                assert_eq!(req.url(), path);
                assert!(req
                    .headers()
                    .iter()
                    .any(|h| h.field.equiv("Authorization")
                        && h.value.as_str() == "Bearer test-session"));
                let body = if path == "/models" {
                    json!({"models":[{"slug":"hidden","visibility":"hide"},{"slug":"eligible","visibility":"list"}]}).to_string()
                } else {
                    let v: Value = serde_json::from_reader(req.as_reader()).unwrap();
                    assert_eq!(v["model"], "eligible");
                    assert_eq!(v["stream"], true);
                    assert_eq!(v["store"], false);
                    assert!(v["input"].is_array() && v.get("max_output_tokens").is_none());
                    if v["input"][0]["content"] != "Connection check." {
                        assert_eq!(v.pointer("/text/format/name"), Some(&json!("result")));
                        assert!(v["instructions"].as_str().unwrap().contains("160 tokens"));
                    }
                    delta() + &event(json!({"type":"response.completed"}))
                };
                req.respond(tiny_http::Response::from_string(body)).unwrap();
            }
        });
        assert_eq!(
            verify_at(&url, "test-session").unwrap(),
            json!({"runtime":"ChatGPT","name":"eligible"})
        );
        complete(
            &url,
            "test-session",
            "eligible",
            "Grade the reply.",
            "test",
            160,
            Some(&json!({"type":"object"})),
        )
        .unwrap();
        worker.join().unwrap();
    }
    #[test]
    #[ignore = "Exercises notebook readings and live jobs through a synthetic streaming plan on loopback"]
    fn a_plan_notebook_reads_scores_and_predicts_without_a_local_model() {
        use crate::{app, engine, ghost, scroll};
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        TEST_PORT.set(Some(server.server_addr().to_ip().unwrap().port()));
        let running = Arc::new(AtomicBool::new(true));
        let live = running.clone();
        let worker = std::thread::spawn(move || {
            while live.load(Ordering::SeqCst) {
                let Some(mut req) = server
                    .recv_timeout(std::time::Duration::from_millis(100))
                    .unwrap()
                else {
                    continue;
                };
                let v: Value = serde_json::from_reader(req.as_reader()).unwrap();
                let instructions = v["instructions"].as_str().unwrap();
                let input = v["input"][0]["content"].as_str().unwrap();
                assert!(!input.contains("privatecanary"));
                let grade = instructions.starts_with(include_str!("../../prompts/grade.txt"));
                if !grade {
                    assert!(!input.contains("unseenanswer"));
                }
                let reply = if instructions.starts_with("Read a person's correction.") {
                    assert!(input.contains("margins for notes"));
                    "I prefer wide margins for handwritten notes."
                } else if instructions.starts_with("Answer the situation") {
                    let payload: Value = serde_json::from_str(input).unwrap();
                    assert_eq!(
                        payload["rules"],
                        json!(["Leave wide margins on workshop handouts."])
                    );
                    assert!(!input.contains("narrow maybe"));
                    "Leave wide margins."
                } else if instructions.starts_with("Select tools") {
                    r#"{"math":"","date":"","days":0,"recall":"","skill":0,"quick":false}"#
                } else if instructions.starts_with("You are Sumi") {
                    let payload: Value = serde_json::from_str(input).unwrap();
                    assert!(payload["current_request"]
                        .as_str()
                        .unwrap()
                        .contains("sketch"));
                    assert!(payload["approved_preferences"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("Sketch the idea first.")));
                    assert_eq!(payload["conversation"], json!([]));
                    "Begin with a small sketch."
                } else if grade {
                    r#"{"grade":2,"reason":"Same action."}"#
                } else if instructions.starts_with(include_str!("../../prompts/reflect.txt")) {
                    r#"{"text":"Start with a sketch.","section":"writing","evidence":["E0","E1"]}"#
                } else if instructions.starts_with(include_str!("../../prompts/ghost.txt")) {
                    "my next sketch"
                } else {
                    "I will sketch it."
                };
                let stream = event(json!({"type":"response.output_text.delta","delta":reply}))
                    + &event(json!({"type":"response.completed"}));
                req.respond(tiny_http::Response::from_string(stream))
                    .unwrap();
            }
        });
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::with_key(tmp.path().join("notebook"), &[72; 32]).unwrap();
        app::setup(&s).unwrap();
        let model = json!({"runtime":"ChatGPT","name":"eligible"});
        s.set("model", &model).unwrap();
        let group = |training: bool| {
            (0..1000)
                .map(|i| format!("chat-{i}"))
                .find(|g| {
                    let n = u8::from_str_radix(&hash(g)[..2], 16).unwrap();
                    (training && n >= 52) || (!training && n < 26)
                })
                .unwrap()
        };
        let train = group(true);
        let test = group(false);
        for (day, q, a) in [
            (
                "2026-04-01",
                "Where do you start?",
                "I draw a small picture.",
            ),
            (
                "2026-04-02",
                "How would you explain an idea?",
                "Sketch it first.",
            ),
        ] {
            sources::add(&s, "notes", &train, day, q, a).unwrap();
        }
        sources::add(
            &s,
            "notes",
            &test,
            "2026-04-06",
            "How should we plan this?",
            "unseenanswer sketch it",
        )
        .unwrap();
        sources::add(
            &s,
            "messages",
            &train,
            "2026-04-03",
            "privatecanary",
            "privatecanary",
        )
        .unwrap();
        sources::add(
            &s,
            "messages",
            &test,
            "2026-04-07",
            "privatecanary case",
            "privatecanary",
        )
        .unwrap();
        let report = engine::read(&s).unwrap();
        assert_eq!(report["tested"], 1, "{report}");
        let pred = s.list("prediction").unwrap();
        assert_eq!(pred[0]["grader"], "eligible");
        assert_eq!(pred[0]["model"], "eligible");
        scroll::change(
            &s,
            "create",
            &json!({"section":"writing","text":"Sketch the idea first."}),
        )
        .unwrap();
        assert_eq!(
            ghost::suggest(&s, "My draft starts with").unwrap()["guess"],
            "my next sketch"
        );
        let conversation = s.new_sheet().unwrap();
        app::action(
            &s,
            "askSheet",
            &json!({"sheet":conversation,"turn":id(),"text":"How should I sketch an idea?"}),
        )
        .unwrap();
        let turn = s.doc("chat", &conversation).unwrap()["turns"][0].clone();
        assert_eq!(turn["reply"], "Begin with a small sketch.");
        assert_eq!(turn["runtime"], "ChatGPT");
        s.new_sheet().unwrap();
        let sheet = s.get("current").unwrap();
        s.save_sheet(sheet.as_str().unwrap(), "Should I draw a draft now?")
            .unwrap();
        app::action(&s, "sealGuess", &json!({"id":sheet})).unwrap();
        assert_eq!(s.list("sealed").unwrap()[0]["model"], "eligible");
        let lesson = app::action(
            &s,
            "readCorrection",
            &json!({"text":"Please give me wider margins for notes."}),
        )
        .unwrap();
        let approval = json!({"id":lesson["id"],"text":"Leave wide margins on workshop handouts."});
        app::action(&s, "keepCorrection", &approval).unwrap();
        app::action(&s, "keepCorrection", &approval).unwrap();
        let decision =
            json!({"id":lesson["id"],"question":"How should I lay out the next workshop handout?"});
        let seal = app::action(&s, "tryCorrection", &decision).unwrap();
        assert_eq!(app::action(&s, "tryCorrection", &decision).unwrap(), seal);
        assert!(app::state(&s).unwrap()["firstCorrection"]["sealed"]["guess"].is_null());
        let opened = app::action(
            &s,
            "breakSeal",
            &json!({"id":seal["id"],"actual":"narrow maybe"}),
        )
        .unwrap();
        assert_eq!(opened["guess"], "Leave wide margins.");
        app::action(&s, "verdict", &json!({"id":seal["id"],"right":false})).unwrap();
        assert_eq!(
            s.doc("prediction", seal["id"].as_str().unwrap()).unwrap()["grade"],
            0
        );
        let routes = s.list("route").unwrap();
        for job in [
            "prediction",
            "grading",
            "reflection",
            "ghost ink",
            "conversation",
            "correction",
        ] {
            assert!(
                routes
                    .iter()
                    .any(|r| r["job"] == job && r["provider"] == "ChatGPT"),
                "{job}: {routes:?}"
            );
        }
        let budget = format!("budget/{}/ChatGPT", chrono::Utc::now().format("%Y-%m"));
        s.set(&budget, &json!(MONTHLY_LIMIT)).unwrap();
        assert!(
            think(&s, &model, "prediction", "Reply", &json!("text"), 12, None)
                .unwrap_err()
                .contains("cap reached")
        );
        s.set("model", &Value::Null).unwrap();
        assert!(think(&s, &model, "prediction", "Reply", &json!("text"), 12, None).is_err());
        running.store(false, Ordering::SeqCst);
        worker.join().unwrap();
        TEST_PORT.set(None);
    }
}
