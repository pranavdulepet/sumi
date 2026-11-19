use crate::{engine, models, scroll, sources, store::*};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub fn suggest(s: &Store, text: &str) -> Res<Value> {
    if text.trim().chars().count() < 12 {
        return Ok(Value::Null);
    }
    let Some(context) = sources::writing_context(text) else {
        return Ok(Value::Null);
    };
    let words: Vec<_> = context
        .split_whitespace()
        .rev()
        .take(3)
        .map(str::to_lowercase)
        .collect();
    if words.len() < 3 {
        return Ok(Value::Null);
    }
    let _guard = s.lifecycle.read().unwrap();
    let revision = s.get("sourceRevision")?;
    let candidate = |guess: &str, origins: BTreeSet<String>| json!({"id":id(),"guess":guess,"sources":origins,"sourceRevision":revision,"context":context});
    let mut votes = BTreeMap::<String, Vec<String>>::new();
    for moment in sources::moments(s, "train", 600)? {
        let sample: Vec<_> = moment["actual"]
            .as_str()
            .unwrap_or("")
            .split_whitespace()
            .collect();
        let Some(i) = sample.windows(3).rposition(|w| {
            w.iter()
                .rev()
                .zip(&words)
                .all(|(a, b)| a.to_lowercase() == *b)
        }) else {
            continue;
        };
        let tail = sample[i + 3..]
            .iter()
            .take(6)
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        if !tail.is_empty() {
            votes
                .entry(tail)
                .or_default()
                .push(moment["source"].as_str().unwrap_or("").to_owned());
        }
    }
    let total: usize = votes.values().map(Vec::len).sum();
    let required = s.get("ghostRepeat")?.as_u64().unwrap_or(2) as usize;
    if let Some((guess, origins)) = votes.into_iter().max_by_key(|(_, origins)| origins.len()) {
        if origins.len() >= required && origins.len() * 3 >= total * 2 {
            return Ok(candidate(&guess, origins.into_iter().collect()));
        }
        if total >= required {
            return Ok(Value::Null);
        }
    }
    let rules = engine::rank(
        scroll::approved(s)?
            .into_iter()
            .filter(|r| r["section"] == "writing" && r["confidence"].as_f64().unwrap_or(0.) >= 0.7)
            .collect(),
        &context,
    );
    let model = s.get("model")?;
    if rules.is_empty() || model.is_null() || required > 2 {
        return Ok(Value::Null);
    }
    let examples = sources::nearby(s, &context)?;
    let mut origins: BTreeSet<String> = examples
        .iter()
        .filter_map(|r| r["source"].as_str().map(str::to_owned))
        .collect();
    origins.extend(
        rules
            .iter()
            .flat_map(|r| r["sources"].as_array().into_iter().flatten())
            .filter_map(|v| v.as_str().map(str::to_owned)),
    );
    let guess = models::infer(
        &model,
        include_str!("../../prompts/ghost.txt"),
        &json!({"sheet":context,"examples":examples,"style":rules}).to_string(),
        14,
    )?;
    let guess = guess.trim();
    Ok(
        if guess.contains("NONE")
            || guess.split_whitespace().count() > 8
            || sources::audit(guess).as_deref() != Some(guess)
        {
            Value::Null
        } else {
            candidate(guess, origins)
        },
    )
}

pub fn shown(s: &Store, a: &Value) -> Res<Value> {
    let _guard = s.lifecycle.read().unwrap();
    let c = &a["candidate"];
    let id = ident(c["id"].as_str().unwrap_or(""))?;
    let sheet = ident(a["sheet"].as_str().unwrap_or(""))?;
    if c["sourceRevision"] != s.get("sourceRevision")? {
        return Ok(Value::Null);
    }
    let (Some(context), Some(guess)) = (
        c["context"].as_str().and_then(sources::audit),
        c["guess"].as_str().and_then(sources::audit),
    ) else {
        return Ok(Value::Null);
    };
    let record = json!({"id":id,"sheet":sheet,"question":context,"guess":guess,"sources":c["sources"],"status":"shown","date":now(),"category":"ghost","model":"local","scope":"live"});
    s.db.lock()
        .unwrap()
        .execute(
            "INSERT OR IGNORE INTO docs VALUES ('prediction',?,?)",
            rusqlite::params![id, record.to_string()],
        )
        .msg()?;
    Ok(Value::Null)
}

pub fn resolve(s: &Store, a: &Value) -> Res<Value> {
    let _guard = s.lifecycle.read().unwrap();
    let _edit = s.edit.lock().unwrap();
    let id = ident(a["id"].as_str().unwrap_or(""))?;
    let mut record = s.doc("prediction", id)?;
    if record["category"] != "ghost" || record["status"] != "shown" {
        return Ok(Value::Null);
    }
    let outcome = a["status"].as_str().unwrap_or("");
    match outcome {
        "accepted" => {
            record["actual"] = record["guess"].clone();
            record["grade"] = json!(3);
        }
        "edited" => {
            let Some(actual) = a["actual"].as_str().and_then(sources::audit) else {
                return Ok(Value::Null);
            };
            record["actual"] = json!(actual
                .split_whitespace()
                .take(8)
                .collect::<Vec<_>>()
                .join(" "));
        }
        "dismissed" => (),
        _ => return Err("Unknown writing outcome.".into()),
    }
    // A typed alternative is new first-person evidence. Keep a suggested phrase with its
    // original sources so that forgetting those sources removes the inference as well.
    if outcome == "edited"
        || outcome == "accepted"
            && record["sources"]
                .as_array()
                .is_some_and(|src| src.is_empty())
    {
        sources::add(
            s,
            "desk",
            record["sheet"].as_str().unwrap(),
            record["date"].as_str().unwrap(),
            record["question"].as_str().unwrap(),
            record["actual"].as_str().unwrap(),
        )?;
    }
    record["status"] = json!(outcome);
    s.put("prediction", &record)?;
    Ok(Value::Null)
}
