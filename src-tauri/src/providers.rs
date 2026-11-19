use crate::{sources, store::*};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
const KINDS: [&str; 7] = [
    "ChatGPT",
    "OpenRouter",
    "OpenAI",
    "Anthropic",
    "Gemini",
    "Jev",
    "Compatible",
];
pub(crate) fn http() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(65))
        .build()
}
fn account(s: &Store, k: &str) -> String {
    format!("{}-{k}", &hash(&s.root.to_string_lossy())[..24])
}
fn vault(s: &Store, k: &str, v: &Value) -> Res<()> {
    security_framework::passwords::set_generic_password(
        "studio.sumi.providers",
        &account(s, k),
        v.to_string().as_bytes(),
    )
    .msg()
}
fn credential(s: &Store, k: &str) -> Res<Value> {
    serde_json::from_slice(&keychain("studio.sumi.providers", &account(s, k))?).msg()
}
fn response(r: Result<ureq::Response, ureq::Error>) -> Res<Value> {
    let res = r.map_err(|e| match e {
        ureq::Error::Status(code, _) => format!("Provider returned HTTP {code}."),
        _ => "Provider connection unavailable.".into(),
    })?;
    res.into_json().msg()
}
pub fn configure(s: &Store, a: &Value) -> Res<()> {
    let k = a["provider"].as_str().unwrap_or("");
    if !KINDS.contains(&k) {
        return Err("Unknown provider.".into());
    }
    let model = a["model"].as_str().unwrap_or("").trim();
    if k != "Jev" && (model.is_empty() || model.len() > 160) {
        return Err("Enter a model ID available to your account.".into());
    }
    let cap = a["cap"].as_u64().unwrap_or(0).min(10000);
    let endpoint = if k == "Compatible" {
        let u = url::Url::parse(a["endpoint"].as_str().unwrap_or("")).msg()?;
        if u.scheme() != "https"
            || !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
        {
            return Err("Use an HTTPS base URL without credentials or query parameters.".into());
        }
        u.to_string()
    } else {
        String::new()
    };
    if let Some(key) = a["key"].as_str().filter(|v| !v.is_empty()) {
        if k == "ChatGPT" {
            return Err("Use Continue with ChatGPT.".into());
        }
        vault(s, k, &json!({"key":key}))?
    } else {
        credential(s, k).map_err(|_| "Connect an account or enter its API key first.")?;
    }
    s.put(
        "provider",
        &json!({"id":k,"model":model,"cap":cap,"endpoint":endpoint,"enabled":a["enabled"]==true,"quiz":k=="Jev" && a["quiz"]==true,"backtest":k!="Jev" && a["backtest"]==true}),
    )?;
    Ok(())
}
pub fn disconnect(s: &Store, k: &str) -> Res<()> {
    if !KINDS.contains(&k) {
        return Err("Unknown provider.".into());
    }
    let _lock = if k == "ChatGPT" {
        Some(plan_lock(s)?)
    } else {
        None
    };
    if k == "ChatGPT" && crate::plan::selected(&s.get("model")?) {
        s.set("model", &s.get("previousLocalModel")?)?;
    }
    let revocation = if k == "ChatGPT" {
        credential(s, k).ok().and_then(|c| {
            let token = c["refresh_token"].as_str()?;
            let client = c["client"].as_str()?;
            Some(revoke_plan(token, client))
        })
    } else {
        None
    };
    match security_framework::passwords::delete_generic_password(
        "studio.sumi.providers",
        &account(s, k),
    ) {
        Err(e) if e.code() != -25300 => return Err(e.to_string()),
        _ => {}
    }
    s.del("provider", k)?;
    if revocation.is_some_and(|r| r.is_err()) {
        return Err(
            "Disconnected on this Mac. Open ChatGPT Settings to confirm you removed Sumi’s access."
                .into(),
        );
    }
    Ok(())
}
fn revoke_plan(token: &str, client: &str) -> Res<()> {
    let h = http();
    let meta = response(
        h.get("https://auth.openai.com/.well-known/openid-configuration")
            .call(),
    )?;
    let endpoint = meta["revocation_endpoint"]
        .as_str()
        .filter(|s| s.starts_with("https://auth.openai.com/"))
        .ok_or("No revocation endpoint.")?;
    for attempt in 0..2 {
        match h.post(endpoint).send_form(&[
            ("token", token),
            ("client_id", client),
            ("token_type_hint", "refresh_token"),
        ]) {
            Ok(_) => return Ok(()),
            Err(ureq::Error::Status(c, _)) if c < 500 => break,
            Err(_) if attempt == 0 => std::thread::sleep(Duration::from_millis(300)),
            _ => break,
        }
    }
    Err("Revocation could not be confirmed.".into())
}
pub(crate) fn reserve(s: &Store, p: &Value) -> Res<()> {
    let month = chrono::Utc::now().format("%Y-%m").to_string();
    let key = format!("budget/{month}/{}", p["id"].as_str().unwrap_or(""));
    let mut c = s.db.lock().unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .msg()?;
    use rusqlite::OptionalExtension;
    let spent = tx
        .query_row("SELECT v FROM kv WHERE k=?", [&key], |r| {
            r.get::<_, String>(0)
        })
        .optional()
        .msg()?
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    if p["enabled"] != true || spent >= p["cap"].as_u64().unwrap_or(0) {
        return Err("Monthly request cap reached or provider switched off.".into());
    }
    tx.execute(
        "INSERT OR REPLACE INTO kv VALUES(?,?)",
        rusqlite::params![key, (spent + 1).to_string()],
    )
    .msg()?;
    tx.commit().msg()
}
pub fn reflect(s: &Store, sys: &str, batch: &[Value]) -> Res<Option<String>> {
    if !crate::local::idle() {
        return Ok(None);
    }
    let Some(p) = s
        .list("provider")?
        .into_iter()
        .find(|v| v["enabled"] == true && v["id"] != "Jev")
    else {
        return Ok(None);
    };
    // Only the narrow audited snippets reach this boundary. Messages and sensitive inferences are local.
    let mut clean = vec![];
    for v in batch {
        if v["source"] == "messages" {
            return Ok(None);
        }
        let mut item = json!({"id":v["id"],"date":v["date"]});
        for k in ["situation", "reply"] {
            let t = v[k].as_str().unwrap_or("");
            if crate::local::sensitive(t) {
                return Ok(None);
            }
            let Some(safe) = sources::audit(t) else {
                return Ok(None);
            };
            item[k] = json!(safe)
        }
        clean.push(item)
    }
    send_text(
        s,
        &p,
        sys,
        &json!(clean).to_string(),
        "reflection",
        clean.len(),
    )
    .map(Some)
}
fn send_text(s: &Store, p: &Value, sys: &str, input: &str, job: &str, items: usize) -> Res<String> {
    let k = p["id"].as_str().unwrap();
    let model = p["model"].as_str().unwrap();
    let h = http();
    let cred = credential(s, k)?;
    let key = cred["key"].as_str().unwrap_or("");
    reserve(s, p)?;
    let limit = match job {
        "reflection" => 600,
        "grading" => 160,
        _ => 96,
    };
    let answer = if k == "ChatGPT" {
        crate::plan::complete(
            crate::plan::API,
            &access_token(s)?,
            model,
            sys,
            input,
            limit,
            None,
        )?
    } else {
        let messages = json!([{"role":"system","content":sys},{"role":"user","content":input}]);
        let (request, body) = match k {
            "Anthropic" => (
                h.post("https://api.anthropic.com/v1/messages")
                    .set("x-api-key", key)
                    .set("anthropic-version", "2023-06-01"),
                json!({"model":model,"max_tokens":limit,"system":sys,"messages":[{"role":"user","content":input}]}),
            ),
            "Gemini" => {
                if !model
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
                {
                    return Err("Invalid Gemini model identifier.".into());
                }
                (h.post(&format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"))
                    .set("x-goog-api-key",key),
                    json!({"systemInstruction":{"parts":[{"text":sys}]},"contents":[{"parts":[{"text":input}]}],"generationConfig":{"maxOutputTokens":limit}}))
            }
            "OpenRouter" | "Compatible" => {
                let base = if k == "OpenRouter" {
                    "https://openrouter.ai/api/v1"
                } else {
                    p["endpoint"].as_str().ok_or("Missing endpoint")?
                };
                (
                    h.post(&format!("{}/chat/completions", base.trim_end_matches('/')))
                        .set("Authorization", &format!("Bearer {key}")),
                    json!({"model":model,"messages":messages,"max_tokens":limit}),
                )
            }
            "OpenAI" => (
                h.post("https://api.openai.com/v1/responses")
                    .set("Authorization", &format!("Bearer {key}")),
                json!({"model":model,"instructions":sys,"input":input,"store":false,"max_output_tokens":limit}),
            ),
            _ => return Err("Unknown text provider.".into()),
        };
        let v = response(request.send_json(body))?;
        match k {
            "Anthropic" => v.pointer("/content/0/text"),
            "Gemini" => v.pointer("/candidates/0/content/parts/0/text"),
            "OpenAI" => v["output"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|x| x["type"] == "message")
                .and_then(|m| m.pointer("/content/0/text")),
            _ => v.pointer("/choices/0/message/content"),
        }
        .and_then(Value::as_str)
        .ok_or("Provider returned no usable text.")?
        .to_owned()
    };
    s.put(
        "route",
        &json!({"id":id(),"date":now(),"job":job,"provider":k,"model":model,"items":items}),
    )?;
    Ok(answer)
}
pub fn backtest_provider(s: &Store) -> Res<Option<Value>> {
    Ok(s.list("provider")?.into_iter().find(|p| {
        p["enabled"] == true
            && p["backtest"] == true
            && p["cap"].as_u64().unwrap_or(0) > 0
            && p["id"] != "Jev"
    }))
}
pub fn blind_job(
    s: &Store,
    p: &Value,
    job: &str,
    prompt: &str,
    fields: &[(&str, &str)],
    rules: &[Value],
) -> Res<Option<String>> {
    if !crate::local::idle() {
        return Ok(None);
    }
    let mut input = json!({});
    let clean = |t: &str| -> Option<String> {
        if crate::local::sensitive(t) {
            None
        } else {
            sources::audit(t).map(|v| v.chars().take(750).collect())
        }
    };
    for (name, value) in fields {
        let Some(t) = clean(value) else {
            return Ok(None);
        };
        input[*name] = json!(t);
    }
    let mut snippets = vec![];
    for rule in rules.iter().take(8) {
        if rule["section"] == "people"
            || rule["sources"]
                .as_array()
                .is_some_and(|ss| ss.iter().any(|s| s == "messages"))
        {
            return Ok(None);
        }
        let Some(t) = rule["text"].as_str().and_then(clean) else {
            return Ok(None);
        };
        snippets.push(t);
    }
    input["rules"] = json!(snippets);
    send_text(
        s,
        p,
        prompt,
        &input.to_string(),
        job,
        fields.len() + snippets.len(),
    )
    .map(Some)
}
fn plan_lock(s: &Store) -> Res<std::fs::File> {
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(s.root.join(".sumi/plan.lock"))
        .msg()?;
    f.lock().msg()?;
    Ok(f)
}
pub(crate) fn access_token(s: &Store) -> Res<String> {
    let _lock = plan_lock(s)?;
    plan_token(s, credential(s, "ChatGPT")?)
}
fn plan_token(s: &Store, mut c: Value) -> Res<String> {
    if c["expires"].as_i64().unwrap_or(0) <= chrono::Utc::now().timestamp() + 60 {
        let t = response(
            http()
                .post("https://auth.openai.com/api/accounts/oauth/token")
                .send_form(&[
                    ("grant_type", "refresh_token"),
                    ("client_id", c["client"].as_str().unwrap_or("")),
                    ("refresh_token", c["refresh_token"].as_str().unwrap_or("")),
                    ("resource", "https://api.openai.com/v1"),
                ]),
        )?;
        if !t["access_token"].is_string() {
            return Err("Sign in to ChatGPT again.".into());
        }
        if let Some(scopes) = t["scope"].as_str() {
            c["scope"] = json!(scopes);
        }
        c["access_token"] = t["access_token"].clone();
        if t["refresh_token"].is_string() {
            c["refresh_token"] = t["refresh_token"].clone()
        }
        c["expires"] =
            json!(chrono::Utc::now().timestamp() + t["expires_in"].as_i64().unwrap_or(0));
        vault(s, "ChatGPT", &c)?;
    }
    if !c["scope"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .any(|sc| sc == "chatgpt.tokens.use.direct")
    {
        return Err("Sign in to allow ChatGPT plan use for Sumi.".into());
    }
    c["access_token"]
        .as_str()
        .map(str::to_owned)
        .ok_or("Sign in with ChatGPT again.".into())
}
fn verify(jwt: &str, client: &str, nonce: &str) -> Res<Value> {
    let parts: Vec<_> = jwt.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid identity token.".into());
    }
    let head: Value = serde_json::from_slice(&B64.decode(parts[0]).msg()?).msg()?;
    if head["alg"] != "RS256" {
        return Err("Unrecognized signing algorithm.".into());
    }
    let h = http();
    let meta = response(
        h.get("https://auth.openai.com/.well-known/openid-configuration")
            .call(),
    )?;
    let uri = meta["jwks_uri"]
        .as_str()
        .ok_or("Missing identity key endpoint.")?;
    if !uri.starts_with("https://auth.openai.com/") {
        return Err("Unexpected identity key endpoint.".into());
    }
    let keys = response(h.get(uri).call())?;
    let key = keys["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|k| k["kid"] == head["kid"] && k["kty"] == "RSA")
        .ok_or("Signing key unavailable.")?;
    let n = B64.decode(key["n"].as_str().unwrap_or("")).msg()?;
    let e = B64.decode(key["e"].as_str().unwrap_or("")).msg()?;
    ring::signature::RsaPublicKeyComponents { n: &n, e: &e }
        .verify(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            format!("{}.{}", parts[0], parts[1]).as_bytes(),
            &B64.decode(parts[2]).msg()?,
        )
        .map_err(|_| "Identity signature did not verify.")?;
    let claims: Value = serde_json::from_slice(&B64.decode(parts[1]).msg()?).msg()?;
    if claims["iss"] != "https://auth.openai.com"
        || claims["aud"] != client
        || claims["nonce"] != nonce
        || claims["exp"].as_i64().unwrap_or(0) <= chrono::Utc::now().timestamp()
        || !claims["sub"].is_string()
    {
        return Err("The account identity could not be verified.".into());
    }
    Ok(claims)
}
pub fn oauth(s: &Store, kind: &str, primary: bool) -> Res<Value> {
    let plan = kind == "ChatGPT";
    if !plan && kind != "OpenRouter" {
        return Err("Unsupported sign-in.".into());
    }
    let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| e.to_string())?;
    let addr = server.server_addr().to_ip().unwrap();
    let redirect = format!("http://{addr}/auth/callback");
    let state = id();
    let nonce = id();
    let verifier = B64.encode(rand::random::<[u8; 40]>());
    let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
    let mut host = s.get("hostId")?;
    if host.is_null() {
        host = json!(format!("urn:uuid:{}", uuid::Uuid::new_v4()));
        s.set("hostId", &host)?;
    }
    let previous = credential(s, kind).ok();
    let original_model = s.get("model")?;
    let registered = previous.as_ref().and_then(|p| p["client"].as_str());
    let mut url = url::Url::parse(if plan {
        "https://auth.openai.com/api/accounts/authorize"
    } else {
        "https://openrouter.ai/auth"
    })
    .msg()?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("state", &state)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", &challenge);
        if plan {
            qp.append_pair("client_id", registered.unwrap_or("dynamic_agent_client"))
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &redirect)
                .append_pair("ext_agent_host_id", host.as_str().unwrap())
                .append_pair(
                    "scope",
                    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
                )
                .append_pair("resource", "https://api.openai.com/v1")
                .append_pair("nonce", &nonce);
            if registered.is_none() {
                qp.append_pair("agent_name_hint", "Sumi");
            }
        } else {
            qp.append_pair("callback_url", &redirect)
                .append_pair("key_label", "Sumi");
        }
    }
    std::process::Command::new("/usr/bin/open")
        .arg(url.as_str())
        .spawn()
        .msg()?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(180) {
        let Some(req) = server.recv_timeout(Duration::from_secs(1)).msg()? else {
            continue;
        };
        let parsed = url::Url::parse(&format!("http://{addr}{}", req.url())).msg()?;
        let fields: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        if parsed.path() != "/auth/callback" || fields.get("state") != Some(&state) {
            let _ = req.respond(tiny_http::Response::empty(400));
            continue;
        }
        let _ = req.respond(tiny_http::Response::from_string(
            "Return to Sumi to finish connecting.",
        ));
        if fields.contains_key("error") {
            return Err("Sign-in was cancelled or plan access was declined.".into());
        }
        let code = fields.get("code").ok_or("Sign-in was cancelled.")?;
        let h = http();
        if plan {
            let client = fields
                .get("client_id")
                .map(String::as_str)
                .or(registered)
                .ok_or("Missing client registration.")?;
            if client == "dynamic_agent_client" || registered.is_some_and(|v| v != client) {
                return Err("Client registration mismatch.".into());
            }
            let mut t = response(
                h.post("https://auth.openai.com/api/accounts/oauth/token")
                    .send_form(&[
                        ("grant_type", "authorization_code"),
                        ("client_id", client),
                        ("code", code),
                        ("code_verifier", &verifier),
                        ("redirect_uri", &redirect),
                        ("resource", "https://api.openai.com/v1"),
                    ]),
            )?;
            let claims = verify(t["id_token"].as_str().unwrap_or(""), client, &nonce)?;
            if registered.is_some()
                && previous
                    .as_ref()
                    .is_some_and(|p| p["subject"] != claims["sub"])
            {
                return Err("The account does not match this registration.".into());
            }
            if !t["scope"]
                .as_str()
                .unwrap_or("")
                .split_whitespace()
                .any(|s| s == "chatgpt.tokens.use.direct")
            {
                return Err("Plan usage was not enabled for Sumi.".into());
            }
            t["subject"] = claims["sub"].clone();
            t["client"] = json!(client);
            t["expires"] =
                json!(chrono::Utc::now().timestamp() + t["expires_in"].as_i64().unwrap_or(0));
            let _lock = plan_lock(s)?;
            let model = if primary {
                Some(crate::plan::verify_connection(
                    s,
                    t["access_token"]
                        .as_str()
                        .ok_or("ChatGPT did not return a plan token.")?,
                )?)
            } else {
                None
            };
            if primary && s.get("model")? != original_model {
                return Err(
                    "Your thinking setting changed during sign-in. Try again to use ChatGPT."
                        .into(),
                );
            }
            vault(s, kind, &t)?;
            if let Some(model) = model {
                let current = s.get("model")?;
                if !crate::plan::selected(&current) {
                    s.set("previousLocalModel", &current)?;
                }
                // Disable any earlier overnight permission. Switching back to local
                // must also stop primary-plan traffic.
                s.put(
                    "provider",
                    &json!({"id":"ChatGPT","primary":true,"enabled":false,"model":model["name"],"email":claims["email"]}),
                )?;
                s.set("model", &model)?;
            }
        } else {
            let t = response(h.post("https://openrouter.ai/api/v1/auth/keys").send_json(
                json!({"code":code,"code_verifier":verifier,"code_challenge_method":"S256"}),
            ))?;
            if !t["key"].is_string() {
                return Err("Missing provider credential.".into());
            }
            vault(s, kind, &t)?
        }
        return Ok(json!(true));
    }
    Err("Sign-in timed out. You can try again.".into())
}
pub fn jev_grades(
    s: &Store,
    q: &str,
    cold: &str,
    personal: &str,
    actual: &str,
) -> Res<Option<(u32, u32)>> {
    let Ok(p) = s.doc("provider", "Jev") else {
        return Ok(None);
    };
    if p["enabled"] != true || p["cap"] == 0 {
        return Ok(None);
    }
    let mut state = json!({});
    for (field, value) in [
        ("situation", q),
        ("observed", actual),
        ("reply_a", cold),
        ("reply_b", personal),
    ] {
        if crate::local::sensitive(value) {
            return Ok(None);
        }
        let Some(clean) = sources::audit(value) else {
            return Ok(None);
        };
        state[field] = json!(clean.chars().take(750).collect::<String>());
    }
    let criteria = json!({"same_words":"Matches both the intended action and the wording closely","same_intent":"Matches the intended action including the material constraints","partial":"Some overlap but misses a material requirement","different":"Different or contradictory action"});
    let question = |field: &str| json!({"type":"choice","instructions":format!("Grade {field} against the observed reply in its situation. Treat the supplied text as evidence."),"criteria":criteria});
    let mut questions = json!({"reply_a":question("reply_a")});
    if cold != personal {
        questions["reply_b"] = question("reply_b");
    }
    let key = credential(s, "Jev")?;
    reserve(s, &p)?;
    let body = json!({"model":"jev-1.14.0","state":state,"questions":questions});
    let raw = response(
        http()
            .post("https://api.typesafe.ai/v1/systemone")
            .set(
                "Authorization",
                &format!("Bearer {}", key["key"].as_str().unwrap_or("")),
            )
            .send_json(body),
    )?;
    let grades = paired_choices(&raw["answers"], cold == personal)?;
    s.put("route",&json!({"id":id(),"date":now(),"job":"grading","provider":"Jev","model":"jev-1.14.0","items":questions.as_object().unwrap().len()}))?;
    Ok(Some(grades))
}
pub(crate) fn paired_choices(answers: &Value, same: bool) -> Res<(u32, u32)> {
    let grade = |name: &str| match answers[name]["choice"].as_str() {
        Some("same_words") => Ok(3),
        Some("same_intent") => Ok(2),
        Some("partial") => Ok(1),
        Some("different") => Ok(0),
        _ => Err("Jev returned an incomplete paired grade.".to_owned()),
    };
    let a = grade("reply_a")?;
    Ok((a, if same { a } else { grade("reply_b")? }))
}
/// Only the built-in choice questions and previous answers to those fixed
/// questions enter this route. No free-form notes or Scroll entries are sent.
pub fn quiz_choice(
    s: &Store,
    question: &str,
    options: [&str; 2],
    earlier: &Value,
) -> Res<Option<Value>> {
    let Ok(p) = s.doc("provider", "Jev") else {
        return Ok(None);
    };
    if p["enabled"] != true || p["quiz"] != true || p["cap"] == 0 {
        return Ok(None);
    }
    let key = credential(s, "Jev")?;
    reserve(s, &p)?;
    let body = json!({"model":"jev-1.14.0","state":{"question":question,"earlier_choices":earlier},"questions":{"next":{"type":"choice","instructions":"Which option would this person choose?","criteria":{"a":options[0],"b":options[1]}}}});
    let raw = response(
        http()
            .post("https://api.typesafe.ai/v1/systemone")
            .set(
                "Authorization",
                &format!("Bearer {}", key["key"].as_str().unwrap_or("")),
            )
            .send_json(body),
    )?;
    let item = &raw["answers"]["next"];
    let label = item["choice"].as_str().ok_or("Jev returned no choice.")?;
    let guess = match label {
        "a" => options[0],
        "b" => options[1],
        _ => return Err("Jev returned an unknown option.".into()),
    };
    let probability = item["probabilities"][label]
        .as_f64()
        .filter(|n| (0.0..=1.0).contains(n))
        .ok_or("Jev returned no choice probability.")?;
    s.put("route",&json!({"id":id(),"date":now(),"job":"choice prediction","provider":"Jev","model":raw["model"],"items":1}))?;
    Ok(Some(
        json!({"guess":guess,"probability":probability,"model":raw["model"]}),
    ))
}
