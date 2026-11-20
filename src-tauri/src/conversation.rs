use crate::{desk_tools, engine, local, models, plan, scroll, store::*};
use serde_json::{json, Value};
use std::{collections::HashSet, fs};

pub(crate) fn words(text: &str) -> HashSet<String> {
    static STOP: std::sync::LazyLock<HashSet<&'static str>> = std::sync::LazyLock::new(|| {
        "about again always and can could did each for from give had has help how into instead make mentioned much need next now originally per please prefer remember should some tell than that the these this time using usually want was were what with would you your".split_whitespace().collect()
    });
    let mut words = engine::tokens(text);
    words.retain(|w| !STOP.contains(w.as_str()));
    words
        .into_iter()
        .map(|w| {
            if w.len() > 4
                && w.ends_with('s')
                && !w.ends_with("ss")
                && !w.ends_with("us")
                && !w.ends_with("is")
            {
                w[..w.len() - 1].to_owned()
            } else {
                w
            }
        })
        .collect()
}

pub fn skills(s: &Store) -> Res<Vec<Value>> {
    let dir = s.root.join("scroll/skills");
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut rows = vec![];
    for file in fs::read_dir(dir).msg()? {
        let file = file.msg()?;
        if !file.file_type().msg()?.is_file() {
            continue;
        }
        let path = file.path();
        if path.extension().and_then(|v| v.to_str()) != Some("md") {
            continue;
        }
        let Some(id) = path
            .file_stem()
            .and_then(|v| v.to_str())
            .filter(|k| ident(k).is_ok())
        else {
            continue;
        };
        let value = fs::read_to_string(&path).msg()?;
        if let Some((title, body)) = value.trim().split_once('\n') {
            let body = body.trim();
            let private = body.starts_with("<!-- local -->");
            let mut body = body.trim_start_matches("<!-- local -->").trim();
            let mut learned = Value::Null;
            if let Some((metadata, rest)) = body
                .strip_prefix("<!-- sumi-method ")
                .and_then(|s| s.split_once(" -->\n"))
            {
                learned = serde_json::from_str(metadata).unwrap_or(Value::Null);
                body = rest.trim_start();
            }
            rows.push(json!({"id":id,"title":title.trim_start_matches('#').trim(),"private":private,"body":body,"learned":learned}));
        }
    }
    rows.sort_by_key(|v| v["title"].as_str().unwrap_or("").to_lowercase());
    Ok(rows)
}

pub fn save_skill(s: &Store, a: &Value) -> Res<Value> {
    let title = a["title"].as_str().unwrap_or("").trim();
    let body = a["body"].as_str().unwrap_or("").trim();
    if title.is_empty()
        || title.contains('\n')
        || title.len() > 120
        || body.is_empty()
        || body.len() > 3200
    {
        return Err("Give the skill a short name and a few steps.".into());
    }
    let id = ident(a["id"].as_str().unwrap_or(""))?;
    let dir = s.root.join("scroll/skills");
    fs::create_dir_all(&dir).msg()?;
    let metadata = if a["learned"].is_object() {
        format!(
            "<!-- sumi-method {} -->\n",
            a["learned"].to_string().replace("-->", "--\\u003e")
        )
    } else {
        String::new()
    };
    atomic(
        &dir.join(format!("{id}.md")),
        format!(
            "# {title}\n\n{}{metadata}{body}\n",
            if a["private"] == true {
                "<!-- local -->\n"
            } else {
                ""
            }
        )
        .as_bytes(),
    )?;
    Ok(json!(id))
}

fn routine(q: &str) -> bool {
    let text = q.trim().to_lowercase();
    [
        "summarize",
        "summarise",
        "rewrite",
        "shorten",
        "translate",
        "rephrase",
        "proofread",
        "tighten",
        "format",
    ]
    .iter()
    .any(|w| text.split_whitespace().take(5).any(|v| v == *w))
}
pub(crate) fn local_model(s: &Store) -> Option<Value> {
    #[cfg(test)]
    if crate::plan::simulated_hosted_account() {
        return None;
    }
    let last = s.get("previousLocalModel").ok()?;
    if last["runtime"] == "Sumi"
        && last["path"]
            .as_str()
            .is_some_and(|p| std::path::Path::new(p).is_file())
    {
        return Some(last);
    }
    let memory = local::fit(s)["memory"].as_u64().unwrap_or(0);
    let available = models::inventory();
    available
        .iter()
        .find(|m| m["runtime"] == last["runtime"] && m["name"] == last["name"])
        .cloned()
        .or_else(|| {
            available.into_iter().find(|m| {
                m["bytes"]
                    .as_u64()
                    .is_some_and(|n| n > 0 && n.saturating_mul(2) + 4_000_000_000 < memory)
            })
        })
}

fn choose_model(
    primary: &Value,
    private: bool,
    quick: bool,
    available: impl FnOnce() -> Option<Value>,
) -> Res<Value> {
    if !plan::selected(primary) || (!private && !quick) {
        return Ok(primary.clone());
    }
    match available() {
        Some(model) => Ok(model),
        None if private => Err("This sheet needs a local model. Connect one in Settings.".into()),
        None => Ok(primary.clone()),
    }
}
fn mirror(s: &Store, chat: &Value) -> Res<()> {
    let dir = s.root.join("desk/conversations");
    fs::create_dir_all(&dir).msg()?;
    let mut text = String::new();
    for turn in chat["turns"].as_array().into_iter().flatten() {
        text.push_str(&format!(
            "## You\n\n{}\n\n## Sumi\n\n{}\n\n",
            turn["question"].as_str().unwrap_or(""),
            turn["reply"].as_str().unwrap_or("")
        ));
        for passage in turn["timeline"].as_array().into_iter().flatten() {
            text.push_str(&format!(
                "> {} · {}\n\n",
                passage["when"].as_str().unwrap_or(""),
                passage["words"].as_str().unwrap_or("")
            ));
        }
        if turn["worksheet"] == true {
            for tool in turn["tools"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|v| v["date"].is_string() || v["value"].is_number())
            {
                text.push_str(&format!("- {}\n", tool["display"].as_str().unwrap_or("")));
            }
            text.push('\n');
        }
    }
    atomic(
        &dir.join(format!("{}.md", ident(chat["id"].as_str().unwrap_or(""))?)),
        text.as_bytes(),
    )
}
pub fn titles(s: &Store) -> Res<std::collections::HashMap<String, String>> {
    let conn = s.db.lock().unwrap();
    let mut stmt=conn.prepare("SELECT id,substr(json_extract(v,'$.turns[0].question'),1,60) FROM docs WHERE kind='chat'").msg()?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).msg()?;
    rows.collect::<rusqlite::Result<_>>().msg()
}
pub(crate) fn enduring(q: &str) -> bool {
    let q = q.to_lowercase();
    [
        "always ",
        "whenever ",
        "from now on",
        "going forward",
        "next time",
        "in the future",
        "each time",
        "every time",
        "by default",
        "in future",
        "remember to",
        "make it a habit",
        "i usually",
        "i prefer",
    ]
    .iter()
    .any(|cue| q.contains(cue))
}

fn historical_only(q: &str) -> bool {
    let q = q.to_lowercase();
    q.starts_with("what did i") || q.starts_with("what was my") || desk_tools::wants_timeline(&q)
}

fn related(q: &str, skill: &Value) -> bool {
    let topic = format!(
        "{} {}",
        skill["title"].as_str().unwrap_or(""),
        skill["learned"]["cue"].as_str().unwrap_or("")
    );
    if !words(q).is_disjoint(&words(&topic)) {
        return true;
    }
    let a = local::embedding(q);
    let b = local::embedding(&topic);
    let similarity: f32 = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(x, y)| {
            f32::from_le_bytes(x.try_into().unwrap()) * f32::from_le_bytes(y.try_into().unwrap())
        })
        .sum();
    similarity > 0.72
}
fn matching_skill<'a>(q: &str, available: &'a [Value]) -> Option<&'a Value> {
    let terms = words(q);
    available
        .iter()
        .filter_map(|v| {
            let title = words(v["title"].as_str().unwrap_or(""));
            let evidence = words(v["learned"]["cue"].as_str().unwrap_or(""));
            let overlap = terms.intersection(&title).count();
            let shared = terms.intersection(&evidence).count();
            let qualifies = overlap >= 2 || (title.len() == 1 && overlap == 1) || shared >= 2;
            let modified = v["learned"]["at"]
                .as_str()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.timestamp())
                .unwrap_or(0);
            qualifies.then_some(((overlap * 3 + shared, modified), v))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, v)| v)
}

fn previous_method(s: &Store, q: &str, hosted: bool) -> Res<Option<String>> {
    let terms = words(q);
    let actions = [
        "plan",
        "prep",
        "draft",
        "write",
        "rewrite",
        "summarize",
        "prepare",
        "format",
        "review",
        "translate",
        "tighten",
        "split",
        "compare",
        "organize",
    ];
    let Some(action) = actions.iter().find(|v| terms.contains(**v)) else {
        return Ok(None);
    };
    let db = s.db.lock().unwrap();
    let mut st=db.prepare("SELECT json_extract(t.value,'$.question') FROM docs c JOIN docs p ON p.kind='sheet' AND p.id=c.id JOIN json_each(c.v,'$.turns') t WHERE c.kind='chat' AND (?1=0 OR (coalesce(json_extract(c.v,'$.private'),0)=0 AND coalesce(json_extract(p.v,'$.private'),0)=0)) AND julianday(coalesce(json_extract(t.value,'$.at'),json_extract(p.v,'$.date')))>=julianday('now','-90 days') ORDER BY julianday(coalesce(json_extract(t.value,'$.at'),json_extract(p.v,'$.date'))) DESC LIMIT 256").msg()?;
    let mut rows = st.query([hosted]).msg()?;
    while let Some(row) = rows.next().msg()? {
        let text: String = row.get(0).msg()?;
        let other = words(&text);
        if other.contains(*action)
            && terms.intersection(&other).count() >= 3
            && (!hosted || (!local::sensitive(&text) && crate::sources::audit(&text).is_some()))
        {
            return Ok(Some(text));
        }
    }
    Ok(None)
}

fn reflect_method(s: &Store, task: &Value, model: &Value) -> Res<()> {
    let schema = json!({"type":"object","properties":{"name":{"type":"string"},"method":{"type":"string"}},"required":["name","method"],"additionalProperties":false});
    let target = task["id"].as_str().ok_or("Missing method identity.")?;
    let all = skills(s)?;
    let existing = all.iter().find(|v| v["id"] == target);
    if let (Some(old), Some(new)) = (
        existing
            .and_then(|v| v["learned"]["at"].as_str())
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()),
        task["at"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()),
    ) {
        if new < old {
            return Ok(());
        }
    }

    if existing.is_none()
        && task["explicit"] != true
        && matching_skill(task["cue"].as_str().unwrap_or(""), &all).is_some()
    {
        return Ok(());
    }
    // An edit or deletion made while reflection was pending takes precedence.
    if existing.as_ref().map(|v| &v["body"]) != task.get("previous_body") {
        return Ok(());
    }
    let response = plan::think(
        s,
        model,
        "method reflection",
        include_str!("../../prompts/method.txt"),
        &json!({"earlier_request":task["earlier"],"latest_request":task["cue"],"previous_method":task["previous_body"]}),
        200,
        Some(&schema),
    )?;
    let reading: Value = serde_json::from_str(response.trim()).msg()?;
    let name = existing
        .and_then(|v| v["title"].as_str())
        .or_else(|| reading["name"].as_str())
        .unwrap_or("")
        .trim();
    let body = if task["explicit"] == true {
        task["cue"].as_str().unwrap_or("").trim()
    } else {
        reading["method"].as_str().unwrap_or("").trim()
    };
    if name.is_empty()
        || body.is_empty()
        || reading["method"].as_str().unwrap_or("").trim().is_empty()
    {
        return Ok(());
    }
    let skill = json!({"id":target,"title":name,"body":body,"private":task["private"],"learned":{"at":task["at"],"cue":task["cue"]}});
    save_skill(s, &skill)?;
    Ok(())
}

// If a reflection is interrupted, the idle worker can finish it on this Mac.
pub fn dream(s: &Store) -> Res<()> {
    let Ok(_edit) = s.edit.try_lock() else {
        return Ok(());
    };
    let job = {
        let db = s.db.lock().unwrap();
        use rusqlite::OptionalExtension;
        let raw:Option<String>=db.query_row("SELECT v FROM docs WHERE kind='methodReflection' AND (json_extract(v,'$.retry') IS NULL OR julianday(json_extract(v,'$.retry')) < julianday('now','-1 hour')) ORDER BY json_extract(v,'$.at') LIMIT 1",[],|r|r.get(0)).optional().msg()?;
        raw.map(|v| serde_json::from_str::<Value>(&v))
            .transpose()
            .msg()?
    };
    let Some(mut job) = job else { return Ok(()) };
    let primary = s.get("model")?;
    let model = if plan::selected(&primary) {
        local_model(s)
    } else {
        Some(primary)
    };
    job["retry"] = json!(now());
    s.put("methodReflection", &job)?;
    if let Some(model) = model {
        reflect_method(s, &job["task"], &model)?;
        s.del("methodReflection", job["id"].as_str().unwrap())?;
    }
    Ok(())
}
fn finish_turn(s: &Store, chat: &mut Value, sheet: &str, question: &str) -> Res<()> {
    if let Some(job) = chat.get("pendingReflection") {
        s.put("methodReflection", job)?;
        chat.as_object_mut().unwrap().remove("pendingReflection");
    }
    mirror(s, chat)?;
    if s.sheet(sheet)?.trim() == question {
        s.save_sheet(sheet, "")?;
    }
    chat.as_object_mut().unwrap().remove("pendingFile");
    s.put("chat", chat)
}
pub fn recover(s: &Store) -> Res<()> {
    let pending: Vec<String> = {
        let db = s.db.lock().unwrap();
        let mut q = db
            .prepare("SELECT id FROM docs WHERE kind='chat' AND json_extract(v,'$.pendingFile')=1")
            .msg()?;
        let rows = q.query_map([], |r| r.get(0)).msg()?;
        rows.collect::<rusqlite::Result<_>>().msg()?
    };
    for id in pending {
        let mut chat = s.doc("chat", &id)?;
        let last = chat["turns"]
            .as_array()
            .and_then(|ts| ts.last())
            .and_then(|t| t["question"].as_str())
            .unwrap_or("")
            .to_owned();
        finish_turn(s, &mut chat, &id, &last)?;
    }
    Ok(())
}
pub fn private_sheet(s: &Store, sheet: &str) -> Res<bool> {
    Ok(s.doc("sheet", sheet)?["private"] == true
        || s.optional_doc("chat", sheet)?
            .is_some_and(|chat| chat["private"] == true))
}

pub fn ask(s: &Store, a: &Value) -> Res<Value> {
    let sheet = ident(a["sheet"].as_str().unwrap_or(""))?;
    let key = ident(a["turn"].as_str().unwrap_or(""))?;
    let q = a["text"].as_str().unwrap_or("").trim();
    if q.is_empty() || q.chars().count() > 3500 {
        return Err("Use up to 3,500 characters on this part of the sheet.".into());
    }
    let _lock = s.edit.lock().unwrap();
    let sheet_info = s.doc("sheet", sheet)?;
    let mut chat = match s.optional_doc("chat", sheet)? {
        Some(v) => v,
        None => json!({"id":sheet,"turns":[]}),
    };
    let turns = chat["turns"]
        .as_array()
        .ok_or("This conversation could not be read.")?;
    if turns.iter().any(|v| v["id"] == key) {
        finish_turn(s, &mut chat, sheet, q)?;
        return Ok(Value::Null);
    }
    let primary = s.get("model")?;
    let mut private = a["private"] == true
        || sheet_info["private"] == true
        || chat["private"] == true
        || !plan::selected(&primary)
        || local::sensitive(q)
        || crate::sources::audit(q).is_none();
    let at = chrono::Local::now();
    let terms = words(q);
    let available = skills(s)?;
    let last = turns.last().unwrap_or(&Value::Null);
    let followup = enduring(q)
        && (q.to_lowercase().starts_with("actually") || q.to_lowercase().contains("next time"));
    let prior_skill = available
        .iter()
        .find(|v| v["id"] == last["methodId"] || v["id"] == last["id"]);
    let matched =
        matching_skill(q, &available).or_else(|| if followup { prior_skill } else { None });
    let explicit = a["skill"].as_str().filter(|k| !k.is_empty());
    let forced = explicit
        .map(|id| {
            available
                .iter()
                .find(|v| v["id"] == id)
                .ok_or("This skill has been removed.")
        })
        .transpose()?;
    private |= forced.or(matched).is_some_and(|v| v["private"] == true);
    let local = if plan::selected(&primary) {
        local_model(s)
    } else {
        Some(primary.clone())
    };
    let selector = if private {
        local
            .as_ref()
            .ok_or("This sheet needs a local model. Connect one in Settings.")?
    } else {
        local.as_ref().unwrap_or(&primary)
    };
    let mut menu: Vec<_> = available
        .iter()
        .filter(|v| !plan::selected(selector) || v["private"] != true)
        .cloned()
        .collect();
    menu.sort_by_cached_key(|v| {
        std::cmp::Reverse(
            terms
                .intersection(&words(v["title"].as_str().unwrap_or("")))
                .count(),
        )
    });
    menu.truncate(24);
    let routing = desk_tools::select(
        s,
        selector,
        q,
        turns.last().unwrap_or(&Value::Null),
        &menu,
        at,
    )?;
    let named = routing["skill"]
        .as_u64()
        .and_then(|v| v.checked_sub(1))
        .and_then(|v| menu.get(v as usize))
        .filter(|skill| related(q, skill));
    let skill = forced.or(matched).or(named);
    private |= skill.is_some_and(|v| v["private"] == true);
    let quick = routing["quick"].as_bool().unwrap_or_else(|| routine(q));
    let model = choose_model(&primary, private, quick, || local.clone())?;
    let tools = desk_tools::run(s, &routing, selector, private, at, q)?;
    let recall = tools.iter().find(|t| t["tool"] == "Notebook");
    let timeline: Vec<_> = recall
        .filter(|_| desk_tools::wants_timeline(q))
        .and_then(|r| r["passages"].as_array())
        .into_iter()
        .flatten()
        .filter(|p| !p["words"].as_str().unwrap_or("").trim_end().ends_with('?'))
        .cloned()
        .collect();
    let skill = if desk_tools::wants_timeline(q) {
        None
    } else {
        skill
    };
    let worksheet = tools
        .iter()
        .any(|v| v["value"].is_number() || v["date"].is_string())
        && [
            "show the calculation",
            "show the math",
            "breakdown",
            "break it down",
            "as a table",
            "show your work",
        ]
        .iter()
        .any(|cue| {
            q.to_lowercase().contains(cue)
                || skill
                    .and_then(|v| v["body"].as_str())
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(cue)
        });

    let allowed = scroll::approved(s)?
        .into_iter()
        .filter(|r| private || plan::cloud_usable(r))
        .collect();
    let rules = engine::select_rules(s, &model, q, allowed);
    let rules: Vec<_> = rules
        .iter()
        .filter_map(|r| r["text"].as_str())
        .filter(|t| !terms.is_disjoint(&words(t)))
        .take(4)
        .collect();
    let mut history:Vec<_>=turns.iter().rev().take(6).map(|v|json!({"user":v["question"].as_str().unwrap_or("").chars().take(850).collect::<String>(),"assistant":v["reply"].as_str().unwrap_or("").chars().take(1200).collect::<String>()})).collect();
    history.reverse();
    let payload = json!({"local_time":at.to_rfc3339(),"tool_results":tools,"conversation":history,"approved_preferences":rules,"chosen_method":skill.map(|v|v["body"].clone()),"current_request":q});
    let answer = if !timeline.is_empty() {
        "Here’s what you said, in order.".to_owned()
    } else if recall.is_some_and(|v| v["passages"].as_array().is_some_and(Vec::is_empty))
        && historical_only(q)
        && turns.is_empty()
    {
        "I couldn’t find an earlier passage about that in your notebook.".to_owned()
    } else {
        plan::think(
            s,
            &model,
            "conversation",
            include_str!("../../prompts/conversation.txt"),
            &payload,
            650,
            None,
        )?
    };
    let earlier = if followup && last["question"].is_string() {
        last["question"].as_str().map(str::to_owned)
    } else {
        previous_method(s, q, !private)?
    };
    let revise = enduring(q) && !q.trim_end().ends_with('?') && q.len() <= 3200;
    let mut reflection = None;
    let one_off = [
        "just this time",
        "just for today",
        "for this one",
        "this time only",
    ]
    .iter()
    .any(|v| q.to_lowercase().contains(v));
    if !one_off && (revise || (earlier.is_some() && skill.is_none())) {
        let updating = if revise { forced.or(matched) } else { None };
        let mut task = json!({"id":updating.map(|v|v["id"].clone()).unwrap_or_else(||json!(key)),"at":at.to_rfc3339(),"cue":q,"earlier":earlier,"explicit":revise,"private":private});
        if let Some(v) = updating {
            task["previous_body"] = v["body"].clone();
        }
        let job = json!({"id":key,"at":at.to_rfc3339(),"task":task});
        chat["pendingReflection"] = job.clone();
        // Explicit instructions take effect immediately. Repeated behavior is consolidated at idle.
        if revise || local.is_none() {
            reflection = Some(job);
        }
    }
    let turn = json!({"id":key,"at":at.to_rfc3339(),"question":q,"reply":answer,"model":model["name"],"runtime":model["runtime"],"rules":rules,"skill":skill.map(|v|v["title"].clone()),"tools":tools,"timeline":timeline,"worksheet":worksheet,"selector":selector["name"],"selectorRuntime":selector["runtime"],"methodId":skill.map(|v|v["id"].clone())});
    chat["turns"].as_array_mut().unwrap().push(turn);
    chat["private"] = json!(private);
    chat["pendingFile"] = json!(true);
    s.put("chat", &chat)?;
    finish_turn(s, &mut chat, sheet, q)?;
    if let Some(job) = reflection {
        if reflect_method(s, &job["task"], selector).is_ok() {
            s.del("methodReflection", key)?;
        }
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Consolidates repeated user requests with an installed local model"]
    fn idle_learning_runs_on_user_evidence_and_survives_reopening() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::with_key(tmp.path().to_owned(), &[42; 32]).unwrap();
        crate::app::setup(&s).unwrap();
        s.set("model", &json!({"name":"llama3.2:3b","runtime":"Ollama"}))
            .unwrap();
        for subject in ["garden", "library"] {
            let sheet = s.new_sheet().unwrap();
            let q=format!("Draft a meeting brief for the {subject} project. Put the decision first, followed by three bullets.");
            ask(&s, &json!({"sheet":sheet,"turn":id(),"text":q})).unwrap();
        }
        assert!(skills(&s).unwrap().is_empty());
        assert_eq!(s.list("methodReflection").unwrap().len(), 1);
        drop(s);
        let s = Store::with_key(tmp.path().to_owned(), &[42; 32]).unwrap();
        crate::app::setup(&s).unwrap();
        dream(&s).unwrap();
        let learned = skills(&s).unwrap();
        assert_eq!(learned.len(), 1);
        assert!(!learned[0]["title"].as_str().unwrap().is_empty());
        assert!(s.list("methodReflection").unwrap().is_empty());
        let sheet = s.new_sheet().unwrap();
        ask(&s,&json!({"sheet":sheet,"turn":id(),"text":"Draft a meeting brief for the playhouse project."})).unwrap();
        assert_eq!(
            s.doc("chat", &sheet).unwrap()["turns"][0]["skill"],
            learned[0]["title"]
        );
    }
    #[test]
    fn private_verdict_stays_local_in_future_readings() {
        use crate::app;
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::with_key(tmp.path().to_owned(), &[63; 32]).unwrap();
        app::setup(&s).unwrap();
        let sheet = s.get("current").unwrap();
        let plan_model = json!({"runtime":"ChatGPT","name":"eligible"});
        s.set("model", &plan_model).unwrap();
        app::action(&s, "sheetPrivacy", &json!({"id":sheet,"value":true})).unwrap();
        assert!(app::action(
            &s,
            "ghost",
            &json!({"sheet":sheet,"text":"Please write my private sketch note."})
        )
        .unwrap()
        .is_null());
        let key = id();
        s.put("sealed", &json!({"id":key,"sheet":sheet,"question":"Which picture?","actual":"The sketch.","guess":"The sketch.","opened":now()})).unwrap();
        app::action(&s, "verdict", &json!({"id":key,"right":true})).unwrap();
        assert!(!plan::cloud_usable(&s.doc("prediction", &key).unwrap()));
        let source: String =
            s.db.lock()
                .unwrap()
                .query_row("SELECT source FROM moments LIMIT 1", [], |r| r.get(0))
                .unwrap();
        assert!(!plan::cloud_usable(&json!({"source":source})));
        assert!(!plan::cloud_usable(&json!({"sources":[source]})));
    }
    #[test]
    fn routing_honors_sheet_privacy_and_local_only_mode() {
        let plan = json!({"runtime":"ChatGPT","name":"eligible"});
        let local = json!({"runtime":"Ollama","name":"installed"});
        assert_eq!(
            choose_model(&plan, false, false, || panic!()).unwrap(),
            plan
        );
        assert_eq!(
            choose_model(&plan, false, true, || Some(local.clone())).unwrap(),
            local
        );
        assert_eq!(choose_model(&plan, false, true, || None).unwrap(), plan);
        assert!(choose_model(&plan, true, false, || None).is_err());
        assert_eq!(
            choose_model(&plan, true, false, || Some(local.clone())).unwrap(),
            local
        );
        assert_eq!(
            choose_model(&local, false, false, || panic!()).unwrap(),
            local
        );
    }
}
