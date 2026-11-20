use crate::{desk_tools, engine, local, memory, models, plan, scroll, store::*, web_search};
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
    for file in fs::read_dir(&dir).msg()? {
        let file = file.msg()?;
        let ty = file.file_type().msg()?;
        let path = file.path();
        let key = if ty.is_dir() {
            let Some(k) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.rsplit('-').next())
            else {
                continue;
            };
            k
        } else if ty.is_file() && path.extension().is_some_and(|x| x == "md") {
            path.file_stem().and_then(|n| n.to_str()).unwrap_or("")
        } else {
            continue;
        };
        let Ok(id) = ident(key) else {
            continue;
        };
        let source = if ty.is_dir() {
            path.join("SKILL.md")
        } else {
            path.clone()
        };
        if !source.symlink_metadata().is_ok_and(|m| m.is_file()) {
            continue;
        }
        let raw = fs::read_to_string(&source).msg()?;
        let value = raw
            .strip_prefix("---\n")
            .and_then(|v| v.split_once("\n---\n"))
            .map(|(_, v)| v)
            .unwrap_or(&raw);
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
            rows.push(json!({"id":id,"file":source.strip_prefix(&dir).msg()?.to_string_lossy(),"title":title.trim_start_matches('#').trim(),"private":private,"body":body,"learned":learned}));
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
    let previous = skills(s)?.into_iter().find(|v| v["id"] == id);
    let slug = title
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = slug.chars().take(30).collect::<String>();
    let slug = slug.trim_end_matches('-');
    let name = format!("{}-{id}", if slug.is_empty() { "method" } else { slug });
    let target = previous
        .as_ref()
        .and_then(|v| v["file"].as_str())
        .map(|f| dir.join(f))
        .unwrap_or_else(|| dir.join(name).join("SKILL.md"));
    let front = if target.file_name().is_some_and(|v| v == "SKILL.md") {
        let folder = target.parent().ok_or("Missing skill folder.")?;
        fs::create_dir_all(folder).msg()?;
        let machine = folder
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or("Invalid skill folder.")?;
        let description = body
            .split_whitespace()
            .take(40)
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "---\nname: {machine}\ndescription: {}\n---\n\n",
            json!(description)
        )
    } else {
        String::new()
    };
    atomic(
        &target,
        format!(
            "{front}# {title}\n\n{}{metadata}{body}\n",
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
        None => {
            let mut model = primary.clone();
            if let Some(fast) = primary["fast"].as_str() {
                model["name"] = json!(fast);
            }
            Ok(model)
        }
    }
}
pub fn publish(s: &Store, chat: &mut Value) -> Res<()> {
    mirror(s, chat)?;
    chat.as_object_mut().unwrap().remove("pendingFile");
    s.put("chat", chat)
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
        if let Some(image) = turn["image"].as_str() {
            text.push_str(&format!(
                "![Generated illustration](../images/{}.png)\n\n",
                ident(image)?
            ));
        }
        for source in turn["web"]["sources"].as_array().into_iter().flatten() {
            text.push_str(&format!(
                "- {} · {}\n",
                source["title"].as_str().unwrap_or("Source"),
                source["url"].as_str().unwrap_or("")
            ));
        }
        if let Some(n) = turn["usedMemory"].get("text").and_then(Value::as_str) {
            text.push_str(&format!("\nFrom your notebook: {n}\n\n"));
        }
        for passage in turn["timeline"].as_array().into_iter().flatten() {
            text.push_str(&format!(
                "> {} · {}{}\n\n",
                passage["when"].as_str().unwrap_or(""),
                if passage["role"] == "assistant" {
                    "Assistant: "
                } else {
                    ""
                },
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

// Answer direct name recall from the person's own sentence when it is present.
fn recalled_name(q: &str, turns: &[Value], recall: Option<&Value>) -> Option<String> {
    let lower = q.to_lowercase();
    if !lower.contains("name")
        || !["mentioned", "what did i", "remind me", "called", "named"]
            .iter()
            .any(|w| lower.contains(w))
    {
        return None;
    }
    static NAMED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r#"(?i:called|named)\s+["“']?([\p{Lu}][\p{L}'’-]*(?: [\p{Lu}][\p{L}'’-]*){0,5})"#,
        )
        .unwrap()
    });
    let terms = words(q);
    let earlier = turns.iter().rev().filter_map(|t| t["question"].as_str());
    let recalled = recall
        .and_then(|r| r["passages"].as_array())
        .into_iter()
        .flatten()
        .filter(|p| p["role"] != "assistant")
        .filter_map(|p| p["words"].as_str());
    earlier
        .chain(recalled)
        .flat_map(|text| text.split(['.', '!', '?']))
        .filter(|sentence| !terms.is_disjoint(&words(sentence)))
        .find_map(|sentence| {
            let name = NAMED.captures(sentence)?;
            Some(format!("You called it {}.", &name[1]))
        })
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
    let mut st=db.prepare("SELECT json_extract(t.value,'$.question'),coalesce(json_extract(t.value,'$.at'),json_extract(p.v,'$.date')) FROM docs c JOIN docs p ON p.kind='sheet' AND p.id=c.id JOIN json_each(c.v,'$.turns') t WHERE c.kind='chat' AND (?1=0 OR (coalesce(json_extract(c.v,'$.private'),0)=0 AND coalesce(json_extract(p.v,'$.private'),0)=0)) AND julianday(coalesce(json_extract(t.value,'$.at'),json_extract(p.v,'$.date')))>=julianday('now','-90 days') ORDER BY julianday(coalesce(json_extract(t.value,'$.at'),json_extract(p.v,'$.date'))) DESC LIMIT 256").msg()?;
    let mut rows = st.query([hosted]).msg()?;
    let mut days = std::collections::HashSet::from([chrono::Local::now().date_naive()]);
    let mut latest = None;
    while let Some(row) = rows.next().msg()? {
        let text: String = row.get(0).msg()?;
        let other = words(&text);
        if other.contains(*action)
            && terms.intersection(&other).count() >= 3
            && (!hosted || (!local::sensitive(&text) && crate::sources::audit(&text).is_some()))
        {
            let stamp: String = row.get(1).msg()?;
            if let Some(at) = scroll::instant(&stamp) {
                days.insert(at.with_timezone(&chrono::Local).date_naive());
            }
            latest.get_or_insert(text);
            if days.len() >= 3 {
                return Ok(latest);
            }
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
        &json!({"earlier_request":task["earlier"],"latest_request":task["cue"],"previous_method":task["previous_body"],"successful_result":task["successful_result"]}),
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

// The full text remains on disk when a learned method falls out of use.
fn retire_unused(s: &Store) -> Res<()> {
    let today = chrono::Local::now().date_naive().to_string();
    if s.get("skillsReviewedOn")? == today {
        return Ok(());
    }
    let mut seen = std::collections::HashSet::new();
    let mut active = skills(s)?;
    active.sort_by_cached_key(|v| {
        std::cmp::Reverse(v["learned"]["at"].as_str().unwrap_or("").to_owned())
    });
    for skill in active {
        let Some(created) = skill["learned"]["at"].as_str().and_then(scroll::instant) else {
            continue;
        };
        let body = skill["body"]
            .as_str()
            .unwrap_or("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let duplicate = !seen.insert((skill["private"] == true, body));
        if !duplicate && (chrono::Utc::now() - created).num_days() < 60 {
            continue;
        }
        let id = ident(skill["id"].as_str().unwrap_or(""))?;
        let used:Option<String>=s.db.lock().unwrap().query_row("SELECT max(json_extract(t.value,'$.at')) FROM docs c JOIN json_each(c.v,'$.turns') t WHERE c.kind='chat' AND json_extract(t.value,'$.methodId')=?", [id],|r|r.get(0)).msg()?;
        if duplicate
            || (chrono::Utc::now() - used.as_deref().and_then(scroll::instant).unwrap_or(created))
                .num_days()
                >= 60
        {
            let dir = s.root.join("scroll/skills");
            let file = skill["file"].as_str().ok_or("Missing skill file.")?;
            let target = dir.join("retired").join(file);
            fs::create_dir_all(target.parent().unwrap()).msg()?;
            fs::rename(dir.join(file), target).msg()?;
        }
    }
    s.set("skillsReviewedOn", &json!(today))
}
// If a reflection is interrupted, the idle worker can finish it on this Mac.
pub fn dream(s: &Store, early: bool) -> Res<()> {
    let primary = s.get("model")?;
    let quiet = || {
        if early {
            local::quiet_after_chat()
        } else {
            local::idle()
        }
    };
    memory::audit_note(s, &primary, quiet)?;
    memory::dream_transcript(s, &primary, quiet)?;
    memory::dream_core(s, &primary, quiet)?;
    memory::dream_proposal(s, &primary)?;
    let Ok(_edit) = s.edit.try_lock() else {
        return Ok(());
    };
    retire_unused(s)?;
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
    if let Some(notes) = chat.get("pendingNotes").and_then(Value::as_array) {
        for n in notes {
            if s.optional_doc("note", n["id"].as_str().unwrap())?.is_none() {
                s.put("note", n)?;
            }
        }
        memory::project(s)?;
        chat.as_object_mut().unwrap().remove("pendingNotes");
    }
    mirror(s, chat)?;
    if chat["preserveDraft"] != true && s.sheet(sheet)?.trim() == question {
        s.save_sheet(sheet, "")?;
    }
    chat.as_object_mut().unwrap().remove("preserveDraft");
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

fn requests_picture(lower: &str) -> bool {
    let request = lower.trim_start_matches("please ").trim_start();
    let request = request
        .strip_prefix("can you ")
        .or_else(|| request.strip_prefix("could you "))
        .unwrap_or(request);
    let drawing = ["draw ", "paint ", "illustrate "]
        .iter()
        .any(|prefix| request.starts_with(prefix));
    let structured = [
        "diagram",
        "chart",
        "graph",
        "timeline",
        "table",
        "comparison",
        "conclusion",
    ]
    .iter()
    .any(|kind| {
        request
            .split(|c: char| !c.is_alphabetic())
            .take(6)
            .any(|word| word == *kind)
    });
    (drawing && !structured)
        || [
            "generate an image",
            "make an image",
            "create an image",
            "generate a picture",
            "create a picture",
            "make a picture",
        ]
        .iter()
        .any(|prefix| request.starts_with(prefix))
}

pub fn private_request(q: &str) -> bool {
    let lower = q.to_lowercase();
    [
        "keep this private",
        "keep this offline",
        "keep this on my mac",
        "don't send this",
        "do not send this",
        "keep this local",
    ]
    .iter()
    .any(|t| lower.contains(t))
}

pub fn check_item(s: &Store, a: &Value) -> Res<Value> {
    let _guard = s.edit.lock().unwrap();
    let sheet = ident(a["sheet"].as_str().unwrap_or(""))?;
    let key = ident(a["turn"].as_str().unwrap_or(""))?;
    let item = a["item"]
        .as_str()
        .filter(|v| {
            v.len() < 32
                && v.split('-').count() == 3
                && v.split('-').all(|n| n.parse::<u16>().is_ok())
        })
        .ok_or("Unknown checklist item.")?;
    let value = a["value"].as_bool().ok_or("Missing selection.")?;
    let mut chat = s.doc("chat", sheet)?;
    let turn = chat["turns"]
        .as_array_mut()
        .and_then(|ts| ts.iter_mut().find(|v| v["id"] == key))
        .ok_or("Missing checklist.")?;
    turn["checks"][item] = json!(value);
    s.put("chat", &chat)?;
    Ok(Value::Null)
}
pub fn regenerate(s: &Store, a: &Value, sink: &mut dyn FnMut(&str)) -> Res<Value> {
    let _guard = s.edit.lock().unwrap();
    let sheet = ident(a["sheet"].as_str().unwrap_or(""))?;
    let turn = ident(a["turn"].as_str().unwrap_or(""))?;
    let attempt = ident(a["attempt"].as_str().unwrap_or(""))?;
    let mut chat = s.doc("chat", sheet)?;
    let history = chat["turns"].as_array().ok_or("Missing conversation.")?;
    let old = history
        .last()
        .filter(|t| t["id"] == turn)
        .ok_or("Only the current answer can be rewritten.")?;
    if old["attempt"] == attempt {
        mirror(s, &chat)?;
        return Ok(Value::Null);
    }
    if old["archiveLookup"] == true
        || !old["web"].is_null()
        || old["timeline"].as_array().is_some_and(|v| !v.is_empty())
    {
        return Err("Ask a follow-up to refine these sources.".into());
    }
    let primary = s.get("model")?;
    let private = chat["private"] == true || private_sheet(s, sheet)?;
    let model = choose_model(&primary, private, false, || local_model(s))?;
    let q = old["question"].as_str().ok_or("Missing question.")?;
    let notes = memory::relevant(s, q, false, plan::selected(&primary) && !private)?;
    let context: Vec<_> = notes
        .iter()
        .enumerate()
        .map(|(i, n)| json!({"number":i+1,"text":n["text"],"since":n["validFrom"]}))
        .collect();
    let files = history
        .iter()
        .rev()
        .find_map(|t| t["files"].as_array().filter(|v| !v.is_empty()))
        .cloned()
        .unwrap_or_default();
    if !files.is_empty() && plan::selected(&model) {
        return Err("Start a new reply to choose whether to share file excerpts.".into());
    }
    let earlier: Vec<_> = history
        .iter()
        .rev()
        .skip(1)
        .take(4)
        .rev()
        .map(|v| json!({"user":v["question"],"assistant":v["reply"]}))
        .collect();
    let payload = json!({"current_request":q,"conversation":earlier,"dated_notes":context,"tool_results":old["tools"],"attached_documents":crate::attachments::context(&files,q),"revision":"Give a fresh, direct answer."});
    let answer = plan::think_with(
        s,
        &model,
        "regeneration",
        include_str!("../../prompts/conversation.txt"),
        &payload,
        650,
        None,
        Some(sink),
    )?;
    let feedback =
        json!({"id":attempt,"at":now(),"request":q,"lane":old["lane"],"reason":"regeneration"});
    let last = chat["turns"].as_array_mut().unwrap().last_mut().unwrap();
    last["reply"] = json!(answer);
    last["checks"] = Value::Null;
    last["notes"] = json!(notes);
    last["rules"] = json!([]);
    last["skill"] = Value::Null;
    last["methodId"] = Value::Null;
    last["usedMemory"] = Value::Null;
    last["runtime"] = model["runtime"].clone();
    last["model"] = model["name"].clone();
    last["lane"] = json!(if private { "private" } else { "deep" });
    last["attempt"] = json!(attempt);
    last["regenerations"] = json!(last["regenerations"].as_u64().unwrap_or(0) + 1);
    chat["pendingFile"] = json!(true);
    chat["preserveDraft"] = json!(true);
    s.put("routeCorrection", &feedback)?;
    s.put("chat", &chat)?;
    finish_turn(s, &mut chat, sheet, "")?;
    Ok(Value::Null)
}
pub fn ask(s: &Store, a: &Value) -> Res<Value> {
    ask_with(s, a, &mut |_| {})
}
pub fn ask_with(s: &Store, a: &Value, sink: &mut dyn FnMut(&str)) -> Res<Value> {
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
    let added = a["files"].as_array().cloned().unwrap_or_default();
    if added.len() > 4
        || added.iter().any(|f| {
            f["name"].as_str().is_none_or(|s| s.len() > 255)
                || f["text"].as_str().is_none_or(|s| s.chars().count() > 60000)
        })
    {
        return Err("Choose up to four readable files.".into());
    }
    let files = if added.is_empty() {
        turns
            .iter()
            .rev()
            .find_map(|t| t["files"].as_array().filter(|files| !files.is_empty()))
            .cloned()
            .unwrap_or_default()
    } else {
        added.clone()
    };
    let has_files = !files.is_empty();
    let primary = s.get("model")?;
    let cues = words(q);
    let uses_private_memory = plan::selected(&primary)
        && memory::timeline(s)?.iter().any(|n| {
            memory::current(n)
                && n["restricted"] == true
                && cues
                    .intersection(&words(n["text"].as_str().unwrap_or("")))
                    .count()
                    >= 2
        });
    let privacy_request = uses_private_memory
        || private_request(q)
        || desk_tools::archive_question(q)
        || (has_files
            && (a["shareFiles"] != true
                || files.iter().any(|f| {
                    crate::local::sensitive(f["text"].as_str().unwrap_or(""))
                        || crate::sources::audit(f["text"].as_str().unwrap_or("")).is_none()
                })));
    let mut private = privacy_request
        || a["private"] == true
        || sheet_info["private"] == true
        || chat["private"] == true
        || !plan::selected(&primary)
        || local::sensitive(q)
        || crate::sources::audit(q).is_none();
    let at = chrono::Local::now();
    let mut restricted = privacy_request
        || a["private"] == true
        || sheet_info["private"] == true
        || chat["networkClosed"] == true
        || local::sensitive(q)
        || crate::sources::audit(q).is_none()
        || (chat["private"] == true && chat.get("networkClosed").is_none());
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
    // Searching for the original words is useful even on a Mac without local weights.
    // Keep the result and subsequent turns on this sheet out of hosted history.
    if local.is_none() && desk_tools::archive_question(q) && !has_files {
        let passages = desk_tools::recall(s, q, false, at.with_timezone(&chrono::Utc), q)?;
        let reply = if passages.is_empty() {
            "I couldn’t find a matching passage in your earlier conversations."
        } else {
            "Here are the matching words from your earlier conversations."
        };
        let turn = json!({"id":key,"at":at.to_rfc3339(),"question":q,"reply":reply,"lane":"private","runtime":"Sumi","model":"Notebook search","notes":[],"rules":[],"tools":[],"timeline":passages,"archiveLookup":true});
        chat["turns"].as_array_mut().unwrap().push(turn);
        chat["private"] = json!(true);
        chat["networkClosed"] = json!(true);
        chat["pendingFile"] = json!(true);
        s.put("chat", &chat)?;
        finish_turn(s, &mut chat, sheet, q)?;
        return Ok(Value::Null);
    }
    // A one-message exception can disclose only the exact preview the person accepted.
    // Mark the resulting turn private for all later retrieval, agents, and suggestions.
    if local.is_none()
        && plan::selected(&primary)
        && local::sensitive(q)
        && !privacy_request
        && a["private"] != true
        && sheet_info["private"] != true
        && chat["private"] != true
        && !has_files
        && !forced.or(matched).is_some_and(|v| v["private"] == true)
    {
        if let Some(preview) = crate::sources::audit(q) {
            if a["approvedQuestion"] != preview {
                return Ok(json!({"reviewQuestion":preview}));
            }
            let answer = plan::one_question(s, &primary, &preview, sink)?;
            chat["turns"].as_array_mut().unwrap().push(json!({"id":key,"at":at.to_rfc3339(),"question":q,"reply":answer,"lane":"deep","runtime":primary["runtime"],"model":primary["name"],"notes":[],"rules":[],"tools":[],"oneQuestion":true}));
            chat["private"] = json!(true);
            chat["networkClosed"] = json!(true);
            chat["pendingFile"] = json!(true);
            s.put("chat", &chat)?;
            finish_turn(s, &mut chat, sheet, q)?;
            return Ok(Value::Null);
        }
    }
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
    restricted |= skill.is_some_and(|v| v["private"] == true) && plan::selected(&primary);
    let corrected = s.list("routeCorrection")?.iter().take(64).any(|v| {
        let prior = words(v["request"].as_str().unwrap_or(""));
        let overlap = prior.intersection(&terms).count();
        overlap >= 2 && overlap * 2 >= prior.len().min(terms.len()) && v["lane"] == "quick"
    });
    let lower = q.to_lowercase();
    let image_request = !has_files && requests_picture(&lower);
    let drawable = image_request
        && !restricted
        && s.optional_doc("provider", "OpenRouter")?.is_some_and(|p| {
            p["imageModel"].as_str().is_some_and(|v| !v.is_empty())
                && p["cap"].as_u64().unwrap_or(0) > 0
        });
    let image_prompt = if drawable {
        crate::sources::audit(q)
    } else {
        None
    };
    let visual = image_request
        || has_files
        || [
            "draw a diagram",
            "as a diagram",
            "as a chart",
            "as a table",
            "make a chart",
            "plot ",
            "flowchart",
        ]
        .iter()
        .any(|v| q.to_lowercase().contains(v));
    let quick = !visual && !corrected && routing["quick"].as_bool().unwrap_or_else(|| routine(q));
    let model = choose_model(&primary, private, quick, || local.clone())?;
    let outing = web_search::outing(q, last);
    // Quick local replies can become history on a later hosted turn on the same sheet.
    let shareable_sheet = plan::selected(&primary) && !private;
    let mut notes = memory::relevant(s, q, outing, shareable_sheet)?;
    if !outing {
        for n in memory::core_notes(s, shareable_sheet)? {
            if !notes.iter().any(|v| v["id"] == n["id"]) {
                notes.push(n);
            }
        }
    }
    let web = if has_files || image_request {
        None
    } else {
        web_search::prepare(q, last, &routing, &notes, at.date_naive(), restricted)
    };
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
    let note_context:Vec<_>=notes.iter().enumerate().map(|(i,n)|json!({"number":i+1,"text":n["text"],"anchored_dates":n["dates"],"since":n["validFrom"]})).collect();
    let payload = json!({"photo_inputs":crate::attachments::photos(&files)?,"attached_documents":crate::attachments::context(&files,q),"dated_notes":note_context,"local_time":at.to_rfc3339(),"tool_results":tools,"conversation":history,"approved_preferences":rules,"chosen_method":skill.map(|v|v["body"].clone()),"current_request":q});
    let optional = if !private && !quick && !has_files && plan::selected(&model) {
        s.optional_doc("provider", "OpenRouter")?
            .filter(|p| p["conversation"] == true && p["cap"].as_u64().unwrap_or(0) > 0)
    } else {
        None
    };
    let mut responder = model.clone();
    let answer = if image_request {
        if image_prompt.is_some() {
            "Ready to draw this.".to_owned()
        } else if restricted {
            "Image generation needs an online sheet.".to_owned()
        } else {
            "Connect an image model in Settings to draw this.".to_owned()
        }
    } else if let Some(w) = &web {
        web_search::introduction(w).to_owned()
    } else if let Some(name) = recalled_name(q, turns, recall) {
        name
    } else if !timeline.is_empty() {
        "Here’s what you said, in order.".to_owned()
    } else if recall.is_some_and(|v| v["passages"].as_array().is_some_and(Vec::is_empty))
        && historical_only(q)
        && turns.is_empty()
    {
        "I couldn’t find an earlier passage about that in your notebook.".to_owned()
    } else if let Some(p) = optional {
        responder = json!({"runtime":"OpenRouter","name":p["model"]});
        crate::providers::conversation(
            s,
            &p,
            include_str!("../../prompts/conversation.txt"),
            &payload,
            sink,
        )?
    } else {
        plan::think_with(
            s,
            &model,
            "conversation",
            include_str!("../../prompts/conversation.txt"),
            &payload,
            650,
            None,
            Some(sink),
        )?
    };
    let earlier = if followup && last["question"].is_string() {
        last["question"].as_str().map(str::to_owned)
    } else {
        previous_method(s, q, !private)?
    };
    let revise = (!q.to_lowercase().starts_with("on weekends")
        && !q.to_lowercase().starts_with("for weekends"))
        && enduring(q)
        && !q.trim_end().ends_with('?')
        && q.len() <= 3200;
    let mut reflection = None;
    let one_off = [
        "just this time",
        "just for today",
        "for this one",
        "this time only",
    ]
    .iter()
    .any(|v| q.to_lowercase().contains(v));
    let success = [
        "that worked",
        "that's perfect",
        "that’s perfect",
        "exactly what i needed",
        "exactly what i wanted",
    ]
    .iter()
    .any(|v| q.trim_end_matches(['.', '!']).eq_ignore_ascii_case(v))
        && (last["tools"].as_array().is_some_and(|ts| ts.len() >= 2)
            || last["web"]["status"] == "done");
    if !one_off && (revise || success || (earlier.is_some() && skill.is_none())) {
        let updating = if revise { forced.or(matched) } else { None };
        let mut task = json!({"id":updating.map(|v|v["id"].clone()).unwrap_or_else(||json!(key)),"at":at.to_rfc3339(),"cue":q,"earlier":earlier,"explicit":revise,"private":private});
        if success {
            task["cue"] = last["question"].clone();
            task["successful_result"] = json!(last["reply"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(600)
                .collect::<String>());
        }
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
    let mut turn = json!({"lane":if visual {"visual"} else if private {"private"} else if quick {"quick"} else {"deep"},"notes":notes,"web":web,"id":key,"at":at.to_rfc3339(),"question":q,"reply":answer,"model":responder["name"],"runtime":responder["runtime"],"rules":rules,"skill":skill.map(|v|v["title"].clone()),"tools":tools,"timeline":timeline,"worksheet":worksheet,"selector":if routing["selector"]=="built-in"{Value::Null}else{selector["name"].clone()},"selectorRuntime":selector["runtime"],"methodId":skill.map(|v|v["id"].clone())});
    turn["files"] = json!(added);
    turn["imagePrompt"] = json!(image_prompt);
    if turn["web"]["status"] == "ready"
        && turn["web"]["outing"] == true
        && s.get("autoListings")? == true
        && !restricted
    {
        if let Err(e) = web_search::complete(s, &mut turn, &model, &notes) {
            turn["web"]["status"] = json!("retry");
            turn["reply"] = json!(e);
        }
    }
    let mut observed = memory::observe(q, key, at, private, restricted);
    for n in &mut observed {
        n["sheet"] = json!(sheet);
    }
    if !observed.is_empty() {
        chat["pendingNotes"] = json!(observed);
    }
    chat["networkClosed"] = json!(restricted);
    chat["turns"].as_array_mut().unwrap().push(turn);
    chat["private"] = json!(private);
    chat["pendingFile"] = json!(true);
    s.put("chat", &chat)?;
    if chat["turns"].as_array().is_some_and(|t| t.len() % 10 == 0) {
        s.set("longConversationDream", &json!(key))?;
    }
    if s.get("importOffer")?.is_null() && s.list("source")?.is_empty() {
        s.set("importOffer", &json!(sheet))?;
    }
    finish_turn(s, &mut chat, sheet, q)?;
    if let Some(job) = reflection {
        if reflect_method(s, &job["task"], selector).is_ok() {
            s.del("methodReflection", key)?;
        }
    }
    Ok(Value::Null)
}

pub fn confirm_search(s: &Store, a: &Value) -> Res<Value> {
    let _lock = s.edit.lock().unwrap();
    let sheet = ident(a["sheet"].as_str().unwrap_or(""))?;
    let mut chat = s.doc("chat", sheet)?;
    if chat["networkClosed"] == true || s.doc("sheet", sheet)?["private"] == true {
        return Err("This sheet is private.".into());
    }
    let local_only = chat["private"] == true;
    let turns = chat["turns"]
        .as_array_mut()
        .ok_or("Missing conversation.")?;
    let index = turns
        .iter()
        .position(|t| t["id"] == a["turn"])
        .ok_or("The question is no longer on this sheet.")?;
    let previous = index
        .checked_sub(1)
        .map(|i| turns[i].clone())
        .unwrap_or(Value::Null);
    let turn = &mut turns[index];
    if turn["web"]["status"] == "done" || turn["web"]["status"] == "empty" {
        return Ok(Value::Null);
    }
    if !matches!(turn["web"]["status"].as_str(), Some("ready" | "retry"))
        || turn["web"]["query"] != a["query"]
        || !a["query"].is_string()
    {
        return Err("Read the current query before searching.".into());
    }
    let primary = s.get("model")?;
    let model = choose_model(&primary, local_only, false, || local_model(s))?;
    let notes = memory::relevant(
        s,
        turn["question"].as_str().unwrap_or(""),
        turn["web"]["outing"] == true,
        plan::selected(&model),
    )?;
    if turn["web"]["outing"] == true {
        let updated = web_search::prepare(
            turn["question"].as_str().unwrap_or(""),
            &previous,
            &Value::Null,
            &notes,
            chrono::Local::now().date_naive(),
            false,
        );
        if let Some(new) = updated.filter(|w| w["query"] != turn["web"]["query"]) {
            turn["reply"] = json!(web_search::introduction(&new));
            turn["web"] = new;
            chat["pendingFile"] = json!(true);
            chat["preserveDraft"] = json!(true);
            s.put("chat", &chat)?;
            finish_turn(s, &mut chat, sheet, "")?;
            return Ok(Value::Null);
        }
    }
    if let Err(e) = web_search::complete(s, turn, &model, &notes) {
        turn["web"]["status"] = json!("retry");
        turn["reply"] = json!(e);
    }
    chat["pendingFile"] = json!(true);
    chat["preserveDraft"] = json!(true);
    s.put("chat", &chat)?;
    finish_turn(s, &mut chat, sheet, "")?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    #[test]
    fn pictures_and_structured_visuals_take_different_routes() {
        assert!(super::requests_picture("could you draw a maple tree?"));
        assert!(!super::requests_picture(
            "draw me a diagram of this system."
        ));
        assert!(!super::requests_picture(
            "draw a conclusion from our numbers."
        ));
    }

    use super::*;
    #[test]
    #[ignore = "Consolidates repeated user requests with an installed local model"]
    fn idle_learning_runs_on_user_evidence_and_survives_reopening() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::with_key(tmp.path().to_owned(), &[42; 32]).unwrap();
        crate::app::setup(&s).unwrap();
        s.set("model", &json!({"name":"llama3.2:3b","runtime":"Ollama"}))
            .unwrap();
        for (days, subject) in [(2, "garden"), (1, "library"), (0, "studio")] {
            let sheet = s.new_sheet().unwrap();
            let q=format!("Draft a meeting brief for the {subject} project. Put the decision first, followed by three bullets.");
            ask(&s, &json!({"sheet":sheet,"turn":id(),"text":q})).unwrap();
            if days > 0 {
                let mut chat = s.doc("chat", &sheet).unwrap();
                chat["turns"][0]["at"] =
                    json!((chrono::Local::now() - chrono::Duration::days(days)).to_rfc3339());
                s.put("chat", &chat).unwrap();
            }
        }
        assert!(skills(&s).unwrap().is_empty());
        assert_eq!(s.list("methodReflection").unwrap().len(), 1);
        drop(s);
        let s = Store::with_key(tmp.path().to_owned(), &[42; 32]).unwrap();
        crate::app::setup(&s).unwrap();
        dream(&s, false).unwrap();
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
