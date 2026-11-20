use crate::{engine, local, models, plan, scroll, store::*};
use serde_json::{json, Value};
use std::{collections::HashSet, fs};

fn words(text: &str) -> HashSet<String> {
    let mut words = engine::tokens(text);
    words.retain(|w| {
        ![
            "the", "and", "for", "please", "could", "with", "this", "that", "from", "some",
            "these", "help", "into", "what", "would", "about", "again", "make", "want", "need",
            "give", "your", "you", "how", "should", "using",
        ]
        .contains(&w.as_str())
    });
    words
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
            rows.push(
                json!({"id":id,"title":title.trim_start_matches('#').trim(),"private":body.trim().starts_with("<!-- local -->"),"body":body.trim().trim_start_matches("<!-- local -->").trim()}),
            );
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
    atomic(
        &dir.join(format!("{id}.md")),
        format!(
            "# {title}\n\n{}{body}\n",
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
    q: &str,
    available: impl FnOnce() -> Option<Value>,
) -> Res<Value> {
    if !plan::selected(primary) || (!private && !routine(q)) {
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
fn repeated(s: &Store, q: &str) -> Res<bool> {
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
    ];
    let Some(action) = actions.iter().find(|a| terms.contains(**a)) else {
        return Ok(false);
    };
    let conn = s.db.lock().unwrap();
    let mut stmt=conn.prepare("SELECT json_extract(t.value,'$.question') FROM (SELECT v FROM docs WHERE kind='chat' ORDER BY rowid DESC LIMIT 32) c JOIN json_each(c.v,'$.turns') t WHERE t.key>=json_array_length(c.v,'$.turns')-12").msg()?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).msg()?;
    for row in rows {
        let other = words(&row.msg()?);
        if other.contains(*action) && terms.intersection(&other).count() >= 3 {
            return Ok(true);
        }
    }
    Ok(false)
}
fn finish_turn(s: &Store, chat: &mut Value, sheet: &str, question: &str) -> Res<()> {
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
    let terms = words(q);
    let available = skills(s)?;
    let explicit = a["skill"].as_str().filter(|k| !k.is_empty());
    let skill = if let Some(id) = explicit {
        Some(
            available
                .iter()
                .find(|v| v["id"] == id)
                .ok_or("This skill has been removed.")?,
        )
    } else {
        available
            .iter()
            .map(|v| {
                (
                    terms
                        .intersection(&words(v["title"].as_str().unwrap_or("")))
                        .count(),
                    v,
                )
            })
            .filter(|(n, _)| *n >= 2)
            .max_by_key(|(n, _)| *n)
            .map(|(_, v)| v)
    };
    if skill.is_some_and(|v| v["private"] == true) {
        private = true;
    }
    let model = choose_model(&primary, private, q, || local_model(s))?;
    let rules = engine::select_rules(s, &model, q, scroll::approved(s)?);
    let rules: Vec<_> = rules
        .iter()
        .filter_map(|r| r["text"].as_str())
        .filter(|t| !terms.is_disjoint(&words(t)))
        .take(4)
        .collect();
    let mut history:Vec<_>=turns.iter().rev().take(6).map(|v|json!({"user":v["question"].as_str().unwrap_or("").chars().take(850).collect::<String>(),"assistant":v["reply"].as_str().unwrap_or("").chars().take(1200).collect::<String>()})).collect();
    history.reverse();
    let payload = json!({"conversation":history,"approved_preferences":rules,"chosen_method":skill.map(|v|v["body"].clone()),"current_request":q});
    let answer=plan::think(s,&model,"conversation","You are Sumi, a thoughtful general-purpose conversational assistant. Answer the current request directly. Prefer a few clear sentences unless the person asks for detail. Use previous exchanges for follow-ups and honor relevant approved preferences and the chosen method. If the user requests a draft, write the draft. You can work with the supplied text. Never claim to have browsed, contacted anyone or changed files.",&payload,650,None)?;
    let repeated = repeated(s, q)?;
    let turn = json!({"id":key,"question":q,"reply":answer,"model":model["name"],"runtime":model["runtime"],"rules":rules,"skill":skill.map(|v|v["title"].clone()),"repeat":repeated && skill.is_none()});
    chat["turns"].as_array_mut().unwrap().push(turn);
    chat["private"] = json!(private);
    chat["pendingFile"] = json!(true);
    s.put("chat", &chat)?;
    finish_turn(s, &mut chat, sheet, q)?;
    Ok(Value::Null)
}

pub fn draft_skill(s: &Store, a: &Value) -> Res<Value> {
    let chat = s.doc("chat", ident(a["sheet"].as_str().unwrap_or(""))?)?;
    let turn = chat["turns"]
        .as_array()
        .and_then(|v| v.iter().find(|t| t["id"] == a["turn"]))
        .ok_or("Open the reply first.")?;
    let primary = s.get("model")?;
    let private =
        chat["private"] == true || s.doc("sheet", chat["id"].as_str().unwrap())?["private"] == true;
    let model = if private && plan::selected(&primary) {
        local_model(s).ok_or("Connect a local model to read this private sheet.")?
    } else {
        primary
    };
    let method = plan::think(s,&model,"skill","Extract a reusable method from this exchange. Give two to four short imperative steps. Keep the user's constraints. Omit the specific subject, names, and any preamble.",&json!({"request":turn["question"],"reply":turn["reply"]}),180,None)?;
    Ok(json!({"id":id(),"title":"","body":method,"private":private || !plan::selected(&model)}))
}

#[cfg(test)]
mod tests {
    use super::*;
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
            choose_model(&plan, false, "Plan a project", || panic!()).unwrap(),
            plan
        );
        assert_eq!(
            choose_model(&plan, false, "Please summarize this.", || Some(
                local.clone()
            ))
            .unwrap(),
            local
        );
        assert_eq!(
            choose_model(&plan, false, "Translate this note.", || None).unwrap(),
            plan
        );
        assert!(choose_model(&plan, true, "Plan a project", || None).is_err());
        assert_eq!(
            choose_model(&plan, true, "Plan a project", || Some(local.clone())).unwrap(),
            local
        );
        assert_eq!(
            choose_model(&local, false, "Plan a project", || panic!()).unwrap(),
            local
        );
    }
}
