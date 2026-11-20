use crate::{conversation::words, desk_tools, engine, store::*};
use chrono::{DateTime, Local};
use regex::Regex;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::LazyLock};

static HOME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:actually,? )?i (?:now live in|live in|moved to|am based in|am living in) ([\p{L}][\p{L} .'-]{0,48})$").unwrap()
});
static TASTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:(?:on|for) (?:my )?weekends?,? i (?:prefer|like|love)|i (?:prefer|like|love|enjoy))\s").unwrap()
});
static RELATIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(next (?:monday|tuesday|wednesday|thursday|friday|saturday|sunday)|tomorrow|today|yesterday|(?:[0-9]{1,2}|one|two|three|four) (?:days?|weeks?) ago)\b").unwrap()
});

fn anchor(day: chrono::NaiveDate, phrase: &str) -> Option<chrono::NaiveDate> {
    let lower = phrase.to_lowercase();
    if let Some(duration) = lower.strip_suffix(" ago") {
        let (n, unit) = duration.split_once(' ')?;
        let count = match n {
            "one" => 1,
            "two" => 2,
            "three" => 3,
            "four" => 4,
            _ => n.parse::<u64>().ok()?,
        };
        return day.checked_sub_days(chrono::Days::new(
            count * if unit.starts_with("week") { 7 } else { 1 },
        ));
    }
    desk_tools::calendar(day, phrase, 0)
}
pub fn observe(
    q: &str,
    turn: &str,
    at: DateTime<Local>,
    private: bool,
    restricted: bool,
) -> Vec<Value> {
    let mut notes = Vec::new();
    for (i, piece) in q.split_inclusive(['.', '!', '?', '\n']).enumerate() {
        if piece.ends_with('?') {
            continue;
        }
        let text = piece.trim().trim_end_matches(['.', '!']).trim();
        if text.is_empty() || text.len() > 320 {
            continue;
        }
        let mut valid_from = at;
        let mut assertion = text;
        // Permit a backdated correction without changing the date it was recorded.
        for prefix in [" since ", " as of ", " on "] {
            if let Some((fact, date)) = text.rsplit_once(prefix) {
                if let Ok(day) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
                    if let Some(when) = day
                        .and_hms_opt(0, 0, 0)
                        .and_then(|t| t.and_local_timezone(Local).earliest())
                    {
                        assertion = fact;
                        valid_from = when;
                    }
                }
            }
        }
        let lower = assertion.to_lowercase();
        let (topic, value) = if let Some(c) = HOME.captures(assertion) {
            if c[1].split_whitespace().count() > 5 {
                continue;
            }
            ("home".to_owned(), c[1].to_owned())
        } else if TASTE.is_match(assertion) {
            let scope = if lower.starts_with("on weekends") || lower.starts_with("for weekends") {
                "weekend preference".to_owned()
            } else {
                format!("preference/{}", &hash(&lower)[..16])
            };
            (scope, assertion.to_owned())
        } else if let Some((key, _)) = lower.strip_prefix("my ").and_then(|v| v.split_once(" is "))
        {
            (format!("my {key}"), assertion.to_owned())
        } else {
            continue;
        };
        let dates: Vec<_> = RELATIVE
            .find_iter(text)
            .filter_map(|m| {
                let date = anchor(at.date_naive(), m.as_str())?;
                Some(json!({"phrase":m.as_str(),"date":date.to_string()}))
            })
            .collect();
        notes.push(
            json!({"id":&hash(&format!("{turn}/{i}"))[..32],"topic":topic,"value":value,
            "text":text,"validFrom":valid_from.to_rfc3339(),"learnedAt":at.to_rfc3339(),
            "private":private,"restricted":restricted,"dates":dates}),
        );
    }
    notes
}

// Derive the end of each version from its successor, including retired successors.
// Closing a current version must never resurrect an older preference.
pub fn timeline(s: &Store) -> Res<Vec<Value>> {
    let mut notes = s.list("note")?;
    notes.sort_by_cached_key(|n| {
        std::cmp::Reverse((
            n["validFrom"].as_str().and_then(crate::scroll::instant),
            n["learnedAt"].as_str().and_then(crate::scroll::instant),
            n["id"].to_string(),
        ))
    });
    let private_sheets: std::collections::HashSet<_> = {
        let db = s.db.lock().unwrap();
        let mut st=db.prepare("SELECT id FROM docs WHERE (kind='sheet' AND json_extract(v,'$.private')=1) OR (kind='chat' AND json_extract(v,'$.networkClosed')=1)").msg()?;
        let rows = st.query_map([], |r| r.get::<_, String>(0)).msg()?;
        rows.collect::<rusqlite::Result<_>>().msg()?
    };
    let mut next = HashMap::new();
    for n in &mut notes {
        if n["sheet"]
            .as_str()
            .is_some_and(|k| private_sheets.contains(k))
        {
            n["restricted"] = json!(true);
            n["private"] = json!(true);
        }
        let successor = next.insert(n["topic"].to_string(), n["validFrom"].clone());
        if let Some(until) = successor {
            if n["validUntil"]
                .as_str()
                .is_none_or(|v| v > until.as_str().unwrap_or(""))
            {
                n["validUntil"] = until;
            }
        }
    }
    Ok(notes)
}

pub fn current(n: &Value) -> bool {
    let now = chrono::Utc::now();
    let from = n["validFrom"].as_str().and_then(crate::scroll::instant);
    let until = n["validUntil"].as_str().and_then(crate::scroll::instant);
    from.is_some_and(|t| t <= now) && until.is_none_or(|t| now < t)
}
pub fn relevant(s: &Store, q: &str, outing: bool, hosted: bool) -> Res<Vec<Value>> {
    let terms = words(q);
    let mut notes: Vec<_> = timeline(s)?
        .into_iter()
        .filter(|n| {
            current(n)
                && (!hosted || n["private"] != true)
                && ((outing
                    && (n["topic"] == "home"
                        || n["topic"]
                            .as_str()
                            .is_some_and(|t| t.contains("preference"))))
                    || !terms.is_disjoint(&words(n["text"].as_str().unwrap_or(""))))
        })
        .collect();
    for entry in crate::scroll::approved(s)? {
        let text = entry["text"].as_str().unwrap_or("");
        if hosted && (!crate::plan::cloud_usable(&entry) || crate::local::sensitive(text)) {
            continue;
        }
        let taste = outing
            && [
                "weekend",
                "museum",
                "galler",
                "workshop",
                "outdoor",
                "hiking",
                "concert",
                "printmaking",
            ]
            .iter()
            .any(|t| text.to_lowercase().contains(t));
        if taste || !terms.is_disjoint(&words(text)) {
            notes.push(json!({"id":entry["id"],"topic":"approved preference","text":text,"validFrom":entry["updated"],"kind":"entry","restricted":true,"importance":entry["importance"]}));
        }
    }
    notes.sort_by_cached_key(|n| {
        let overlap = terms
            .intersection(&words(n["text"].as_str().unwrap_or("")))
            .count();
        let age = n["validFrom"]
            .as_str()
            .and_then(crate::scroll::instant)
            .map(|at| (chrono::Utc::now() - at).num_days().max(0))
            .unwrap_or(0);
        let score = 4. * overlap as f64
            + 2. * n["importance"].as_f64().unwrap_or(0.5)
            + 0.5f64.powf(age as f64 / 120.);
        std::cmp::Reverse(((outing && n["topic"] == "home"), (100. * score) as i64))
    });
    notes.truncate(8);
    Ok(notes)
}

pub fn project(s: &Store) -> Res<()> {
    let mut body = String::from("# What Sumi knows\n");
    for n in timeline(s)? {
        body.push_str(&format!(
            "\n- {} · since {}{}\n",
            n["text"].as_str().unwrap_or(""),
            n["validFrom"].as_str().unwrap_or(""),
            n["validUntil"]
                .as_str()
                .map(|t| format!(" · until {t}"))
                .unwrap_or_default()
        ));
        for anchor in n["dates"].as_array().into_iter().flatten() {
            body.push_str(&format!("  - {} = {}\n", anchor["phrase"], anchor["date"]));
        }
    }
    atomic(&s.root.join("desk/memories.md"), body.as_bytes())?;
    let core = core_notes(s, false)?
        .iter()
        .filter_map(|n| Some(format!("- {}", n["text"].as_str()?)))
        .collect::<Vec<_>>()
        .join("\n");
    atomic(
        &s.root.join("desk/core.md"),
        format!("# Core\n\n{core}\n").as_bytes(),
    )
}

pub fn close(s: &Store, id: &str) -> Res<Value> {
    let _lock = s.edit.lock().unwrap();
    let mut n = s.doc("note", ident(id)?)?;
    if n["validUntil"].is_null() {
        n["validUntil"] = json!(now());
        s.put("note", &n)?;
    }
    project(s)?;
    Ok(Value::Null)
}

// Only observations that still agree across distinct local dates can enter the
// compact core. Its members stay linked to the original, correctable notes.
fn core_candidates(notes: &[Value]) -> Vec<Value> {
    let mut days: HashMap<(String, String), std::collections::HashSet<String>> = HashMap::new();
    for n in notes {
        if let (Some(day), Some(text)) = (
            n["learnedAt"].as_str().and_then(|v| v.get(..10)),
            n["value"].as_str(),
        ) {
            days.entry((n["topic"].to_string(), text.to_lowercase()))
                .or_default()
                .insert(day.to_owned());
        }
    }
    notes
        .iter()
        .filter(|n| {
            current(n)
                && days
                    .get(&(
                        n["topic"].to_string(),
                        n["value"].as_str().unwrap_or("").to_lowercase(),
                    ))
                    .is_some_and(|d| d.len() >= 3)
        })
        .take(16)
        .cloned()
        .collect()
}
pub fn core_notes(s: &Store, hosted: bool) -> Res<Vec<Value>> {
    let ids = s.get("coreNotes")?;
    Ok(timeline(s)?
        .into_iter()
        .filter(|n| {
            current(n)
                && (!hosted || n["private"] != true)
                && ids.as_array().is_some_and(|v| v.contains(&n["id"]))
        })
        .collect())
}
fn holdout(s: &Store, after: &str) -> Res<Vec<Value>> {
    let conn = s.db.lock().unwrap();
    let mut stmt=conn.prepare("SELECT json_extract(old.value,'$.reply'),json_extract(t.value,'$.question'),json_extract(t.value,'$.id') FROM docs c JOIN json_each(c.v,'$.turns') t JOIN json_each(c.v,'$.turns') old ON old.key=t.key-1 WHERE c.kind='chat' AND julianday(json_extract(old.value,'$.at'))>julianday(?) AND julianday(json_extract(t.value,'$.at'))>julianday('now','-60 days') ORDER BY julianday(json_extract(t.value,'$.at')) DESC LIMIT 3").msg()?;
    let it=stmt.query_map([after],|r|Ok(json!({"situation":r.get::<_,String>(0)?,"actual":r.get::<_,String>(1)?,"id":r.get::<_,String>(2)?}))).msg()?;
    it.collect::<rusqlite::Result<_>>().msg()
}
fn replay(
    s: &Store,
    primary: &Value,
    baseline: &[Value],
    candidate: &[Value],
    cases: &[Value],
    idle: impl Fn() -> bool,
) -> Res<Option<[u32; 2]>> {
    let local;
    let model = if crate::plan::selected(primary) {
        let Some(found) = crate::conversation::local_model(s) else {
            return Ok(None);
        };
        local = found;
        &local
    } else {
        primary
    };
    let mut scores = [0u32; 2];
    for case in cases {
        let situation = case["situation"].as_str().unwrap_or("");
        let actual = case["actual"].as_str().unwrap_or("");
        for (i, rules) in [baseline, candidate].into_iter().enumerate() {
            if !idle() {
                return Ok(None);
            }
            let guess = engine::predict_ranked(s, model, situation, rules, engine::PREDICT)?;
            scores[i] += engine::grade(s, model, situation, &guess, actual)?;
        }
    }
    Ok(Some(scores))
}

// Imported writing habits remain proposals until a later, independent sample
// supports automatically adding one to the approved memory.
pub fn dream_proposal(s: &Store, primary: &Value) -> Res<()> {
    if primary.is_null() || s.get("autoStamp")? != true {
        return Ok(());
    }
    let current = crate::scroll::approved(s)?;
    for proposal in s.list("proposal")? {
        if proposal["section"] != "writing"
            || proposal["days"].as_u64().unwrap_or(0) < 5
            || proposal["confidence"].as_f64().unwrap_or(0.) < 0.75
        {
            continue;
        }
        let after = current
            .iter()
            .filter_map(|r| r["updated"].as_str())
            .chain(proposal["lastSeen"].as_str())
            .max()
            .unwrap_or("");
        let cases = holdout(s, after)?;
        if cases.len() < 2 {
            continue;
        }
        let signature = hash(&json!([current, proposal, cases]).to_string());
        let key = format!("trial/{}", proposal["id"]);
        if s.get(&key)? == signature {
            continue;
        }
        let mut candidate = current.clone();
        candidate.push(proposal.clone());
        let Some(scores) = replay(s, primary, &current, &candidate, &cases, crate::local::idle)?
        else {
            return Ok(());
        };
        let Ok(_edit) = s.edit.try_lock() else {
            return Ok(());
        };
        if s.get("autoStamp")? == true
            && crate::scroll::approved(s)? == current
            && s.optional_doc("proposal", proposal["id"].as_str().unwrap_or(""))?
                .as_ref()
                == Some(&proposal)
        {
            if scores[1] >= scores[0] {
                crate::scroll::change_locked(s, "stamp", &proposal)?;
            }
            s.set(&key, &json!(signature))?;
        }
        break;
    }
    Ok(())
}
pub fn dream_core(s: &Store, primary: &Value, idle: impl Fn() -> bool) -> Res<()> {
    if primary.is_null() {
        return Ok(());
    }
    let baseline = core_notes(s, false)?;
    let candidate = core_candidates(&timeline(s)?);
    let ids = |notes: &[Value]| notes.iter().map(|v| v["id"].clone()).collect::<Vec<_>>();
    if ids(&baseline) == ids(&candidate) || candidate.is_empty() {
        return Ok(());
    }
    let cutoff = baseline
        .iter()
        .chain(&candidate)
        .filter_map(|v| v["learnedAt"].as_str())
        .max()
        .unwrap_or("");
    let cases = holdout(s, cutoff)?;
    if cases.len() < 2 {
        return Ok(());
    }
    let stamp = hash(&json!([ids(&baseline), ids(&candidate), ids(&cases)]).to_string());
    if s.get("lastDreamTrial")?["stamp"] == stamp {
        return Ok(());
    }
    let Some(scores) = replay(s, primary, &baseline, &candidate, &cases, idle)? else {
        return Ok(());
    };
    let Ok(_edit) = s.edit.try_lock() else {
        return Ok(());
    };
    if ids(&core_notes(s, false)?) != ids(&baseline)
        || ids(&core_candidates(&timeline(s)?)) != ids(&candidate)
    {
        return Ok(());
    }
    if scores[1] >= scores[0] {
        s.set("coreNotes", &json!(ids(&candidate)))?;
        project(s)?;
    }
    s.set("lastDreamTrial",&json!({"stamp":stamp,"cases":cases.len(),"baseline":scores[0],"candidate":scores[1],"at":now()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    #[test]
    fn corrections_close_the_old_fact_and_only_distinct_days_promote() {
        let folder = tempfile::tempdir().unwrap();
        let s = Store::with_key(folder.path().into(), &[8; 32]).unwrap();
        crate::scroll::init(&s).unwrap();
        let mut id = 0;
        let mut say = |day, q: &str| {
            id += 1;
            for n in observe(
                q,
                &format!("turn{id}"),
                Local.with_ymd_and_hms(2026, 11, day, 12, id, 0).unwrap(),
                true,
                false,
            ) {
                s.put("note", &n).unwrap();
            }
        };
        say(1, "I live in Boston.");
        say(2, "I moved to Salem.");
        say(2, "I live in Salem.");
        say(3, "I live in Salem.");
        assert!(core_candidates(&timeline(&s).unwrap()).is_empty());
        say(4, "I live in Salem.");
        let core = core_candidates(&timeline(&s).unwrap());
        assert_eq!(core.len(), 1);
        assert_eq!(core[0]["value"], "Salem");
        close(&s, core[0]["id"].as_str().unwrap()).unwrap();
        assert!(relevant(&s, "weekend", true, false).unwrap().is_empty());
        assert!(timeline(&s)
            .unwrap()
            .iter()
            .all(|v| v["validUntil"].is_string()));
    }
}

#[cfg(test)]
mod integration {
    use super::*;
    #[test]
    #[ignore = "Uses the installed local model to replay held-out follow-ups against two memories"]
    fn compares_candidate_with_baseline_before_installing_core() {
        let folder = tempfile::tempdir().unwrap();
        let s = Store::with_key(folder.path().into(), &[18; 32]).unwrap();
        let model = json!({"name":"llama3.2:3b","runtime":"Ollama"});
        let start = Local::now() - chrono::Duration::days(6);
        for day in 0..3 {
            for n in observe(
                "On weekends I love making woodblock prints.",
                "evidence",
                (start + chrono::Duration::days(day)),
                true,
                false,
            ) {
                let mut n = n;
                n["id"] = json!(id());
                s.put("note", &n).unwrap();
            }
        }
        dream_core(&s, &model, || true).unwrap();
        assert!(s.get("coreNotes").unwrap().is_null());
        for (i, prompt) in [
            "Would you rather go dancing or print woodblocks?",
            "How would you spend a free Saturday?",
        ]
        .iter()
        .enumerate()
        {
            let at = (start + chrono::Duration::days(4)).to_rfc3339();
            s.put("chat",&json!({"id":id(),"turns":[{"id":id(),"at":at,"reply":prompt},{"id":format!("held-{i}"),"at":at,"question":"I would make woodblock prints."}]})).unwrap();
        }
        dream_core(&s, &model, || true).unwrap();
        let trial = s.get("lastDreamTrial").unwrap();
        assert_eq!(trial["cases"], 2);
        assert_eq!(
            !core_notes(&s, false).unwrap().is_empty(),
            trial["candidate"].as_i64() >= trial["baseline"].as_i64()
        );
        assert!(core_notes(&s, true).unwrap().is_empty());
    }
}
