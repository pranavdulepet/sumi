use crate::{conversation::words, desk_tools, engine, store::*};
use chrono::{DateTime, Local};
use regex::Regex;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::LazyLock};

static HOME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:actually,? )?i (?:now live in|live in|moved to|am based in|am living in) ([\p{L}][\p{L} .'-]{0,48})$").unwrap()
});
static PERSON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:My |my )?(?:cofounder|partner|colleague|manager|friend|sister|brother) ([A-Z][\p{L}'-]+) (?:is|owns|leads|likes|works)\b|^([A-Z][\p{L}'-]+) (?:is my|owns|leads|is responsible for)\b").unwrap()
});
static PROJECT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:i(?: am|'m|’m)|we(?: are|'re|’re)) (?:currently )?(?:working on|focused on|deep in|preparing for) (.{2,80})$").unwrap()
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
        } else if let Some(c) = PERSON.captures(assertion) {
            (
                format!("person/{}", c.get(1).or(c.get(2)).unwrap().as_str()),
                assertion.to_owned(),
            )
        } else if let Some(c) = PROJECT.captures(assertion) {
            (format!("project/{}", c[1].trim()), assertion.to_owned())
        } else if lower.starts_with("today i ")
            || lower.starts_with("tomorrow i ")
            || lower.starts_with("yesterday i ")
        {
            (format!("moment/{turn}/{i}"), assertion.to_owned())
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
        let scale = if topic.starts_with("moment/") {
            "moment"
        } else if topic.starts_with("project/") {
            "phase"
        } else {
            "trait"
        };
        let expiry = match scale {
            "phase" => Some(valid_from + chrono::Duration::days(35)),
            "moment" => Some(valid_from + chrono::Duration::days(2)),
            _ => None,
        };
        notes.push(
            json!({"id":&hash(&format!("{turn}/{i}"))[..32],"topic":topic,"value":value,
            "text":text,"scale":scale,"importance":if topic.starts_with("person/"){0.85}else{0.7},"validUntil":expiry.map(|t|t.to_rfc3339()),"validFrom":valid_from.to_rfc3339(),"learnedAt":at.to_rfc3339(),
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
    let all = timeline(s)?;
    let mut evidence = HashMap::<(String, String), std::collections::HashSet<String>>::new();
    for n in &all {
        if let Some(day) = n["learnedAt"].as_str().and_then(|v| v.get(..10)) {
            evidence
                .entry((n["topic"].to_string(), n["value"].to_string()))
                .or_default()
                .insert(day.into());
        }
    }
    let mut notes: Vec<_> = all
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
        let repeats = evidence
            .get(&(n["topic"].to_string(), n["value"].to_string()))
            .map_or(0, |v| v.len().saturating_sub(1).min(6));
        let score = 4. * overlap as f64
            + (repeats as f64).ln_1p()
            + 2. * n["importance"].as_f64().unwrap_or(0.5)
            + 0.5f64.powf(
                age as f64
                    / match n["scale"].as_str() {
                        Some("phase") => 21.,
                        Some("moment") => 2.,
                        _ => 120.,
                    },
            );
        std::cmp::Reverse(((outing && n["topic"] == "home"), (100. * score) as i64))
    });
    notes.truncate(8);
    Ok(notes)
}

pub fn project(s: &Store) -> Res<()> {
    let mut body = String::from("# What Sumi knows\n");
    let mut pages = std::collections::BTreeMap::<String, Vec<String>>::new();
    let directory = s.root.join("desk/pages");
    std::fs::create_dir_all(&directory).msg()?;
    for n in timeline(s)? {
        let topic = n["topic"].as_str().unwrap_or("Timeline");
        let title = if let Some(v) = topic.strip_prefix("person/") {
            format!("People · {v}")
        } else if let Some(v) = topic.strip_prefix("project/") {
            format!("Projects · {v}")
        } else if topic.contains("preference") {
            "Preferences".to_owned()
        } else {
            "Timeline".to_owned()
        };
        pages.entry(title).or_default().push(format!(
            "- {} · {}{}",
            n["text"].as_str().unwrap_or(""),
            n["validFrom"].as_str().unwrap_or(""),
            n["validUntil"]
                .as_str()
                .map(|u| format!(" → {u}"))
                .unwrap_or_default()
        ));
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
    let mut ids = vec![];
    for (title, lines) in pages {
        let key = hash(&title)[..32].to_owned();
        atomic(
            &directory.join(format!("{key}.md")),
            format!("# {title}\n\n{}\n", lines.join("\n")).as_bytes(),
        )?;
        body.push_str(&format!(
            "\n- [{}](pages/{key}.md)",
            title.replace(['[', ']'], "")
        ));
        ids.push(key);
    }
    for old in s
        .get("projectedPages")?
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if ident(old).is_ok() && !ids.iter().any(|k| k == old) {
            let path = directory.join(format!("{old}.md"));
            if path.exists() {
                std::fs::remove_file(path).msg()?;
            }
        }
    }
    s.set("projectedPages", &json!(ids))?;
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
    if current(&n) {
        n["validUntil"] = json!(now());
        s.put("note", &n)?;
    }
    project(s)?;
    Ok(Value::Null)
}

pub fn revise(s: &Store, a: &Value, remove: bool) -> Res<Value> {
    let _lock = s.edit.lock().unwrap();
    let key = ident(a["id"].as_str().unwrap_or(""))?;
    let note = s.doc("note", key)?;
    // Materialise other validity windows before changing this version.
    for n in timeline(s)?
        .into_iter()
        .filter(|n| n["topic"] == note["topic"] && n["id"] != key && n["validUntil"].is_string())
    {
        s.put("note", &n)?;
    }
    if remove {
        s.set(
            &format!("forgotNote/{}", hash(note["text"].as_str().unwrap_or(""))),
            &json!(true),
        )?;
        s.del("noteCheck", key)?;
        s.del("note", key)?;
    } else {
        let text = a["text"]
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty() && v.chars().count() <= 320)
            .ok_or("Write a correction of up to 320 characters.")?;
        let mut replacement = observe(
            text,
            &id(),
            Local::now(),
            note["private"] == true,
            note["restricted"] == true,
        )
        .into_iter()
        .next()
        .unwrap_or_else(|| note.clone());
        let now = now();
        replacement["id"] = json!(id());
        if note["topic"]
            .as_str()
            .is_some_and(|t| t.starts_with("project/") || t.starts_with("person/"))
            && replacement["topic"] != note["topic"]
        {
            let mut previous = note.clone();
            previous["validUntil"] = json!(now);
            s.put("note", &previous)?;
        } else {
            replacement["topic"] = note["topic"].clone();
        }
        replacement["text"] = json!(text);
        if replacement["value"] == note["value"] {
            replacement["value"] = json!(text);
        }
        replacement["learnedAt"] = json!(now);
        replacement["validFrom"] = json!(now);
        replacement["sheet"] = note["sheet"].clone();
        replacement["importance"] = json!(1.0);
        replacement["validUntil"] = if replacement["scale"] == "phase" {
            json!((Local::now() + chrono::Duration::days(35)).to_rfc3339())
        } else if replacement["scale"] == "moment" {
            json!((Local::now() + chrono::Duration::days(2)).to_rfc3339())
        } else {
            Value::Null
        };
        s.put("note", &replacement)?;
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
                && n["scale"] != "moment"
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
// An idle observation must quote a real user turn and survive a later, disjoint
// prediction trial. The model cannot write instructions into the core.
pub fn dream_transcript(s: &Store, primary: &Value, quiet: impl Fn() -> bool + Copy) -> Res<()> {
    if primary.is_null() || !quiet() {
        return Ok(());
    }
    let local = if crate::plan::selected(primary) {
        crate::conversation::local_model(s)
    } else {
        Some(primary.clone())
    };
    let Some(model) = local else { return Ok(()) };
    let sources: Vec<(String, String, String, String, bool)> = {
        let db = s.db.lock().unwrap();
        let mut st=db.prepare("SELECT c.id,json_extract(t.value,'$.id'),json_extract(t.value,'$.question'),json_extract(t.value,'$.at'),coalesce(json_extract(c.v,'$.networkClosed'),0) FROM docs c JOIN json_each(c.v,'$.turns') t WHERE c.kind='chat' AND json_extract(t.value,'$.at') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM kv WHERE k='examined/'||json_extract(t.value,'$.id')) AND julianday(json_extract(t.value,'$.at')) < julianday('now','-1 hour') ORDER BY julianday(json_extract(t.value,'$.at')) LIMIT 5").msg()?;
        let rows = st
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .msg()?;
        rows.collect::<rusqlite::Result<_>>().msg()?
    };
    for (sheet, turn, text, at, restricted) in sources {
        if !quiet() {
            return Ok(());
        }
        let key = format!("examined/{turn}");
        if text.trim_end().ends_with('?')
            || crate::sources::audit(&text).is_none()
            || timeline(s)?
                .iter()
                .any(|n| text.contains(n["text"].as_str().unwrap_or("")))
        {
            s.set(&key, &json!(true))?;
            continue;
        }
        let cases = holdout(s, &at)?;
        if cases.len() < 2 {
            continue;
        }
        let schema = json!({"type":"object","properties":{"quote":{"type":"string"},"topic":{"type":"string"},"scope":{"type":"string","enum":["person","project","preference","none"]}},"required":["quote","topic","scope"],"additionalProperties":false});
        let raw=crate::plan::think(s,&model,"local dream","Find one explicit, lasting statement about this person's life or preferences. Quote their exact words, without interpretation. Return none if uncertain, hypothetical, or a one-off. Topic is a short subject name.",&json!({"words":text}),160,Some(&schema))?;
        let result: Value = serde_json::from_str(&raw).msg()?;
        let quote = result["quote"].as_str().unwrap_or("").trim();
        let topic = result["topic"].as_str().unwrap_or("").trim();
        let scope = result["scope"].as_str().unwrap_or("");
        if !matches!(scope, "person" | "project" | "preference")
            || quote.chars().count() < 8
            || quote.chars().count() > 320
            || !text.contains(quote)
            || topic.is_empty()
            || topic.len() > 60
            || s.get(&format!("forgotNote/{}", hash(quote)))? == true
        {
            s.set(&key, &json!(true))?;
            continue;
        }
        let baseline = core_notes(s, false)?;
        let until = if scope == "project" {
            crate::scroll::instant(&at).map(|d| (d + chrono::Duration::days(35)).to_rfc3339())
        } else {
            None
        };
        let note = json!({"id":&hash(&turn)[..32],"sheet":sheet,"topic":format!("{scope}/{topic}"),"value":quote,"text":quote,"learnedAt":now(),"validFrom":at,"validUntil":until,"private":true,"restricted":restricted,"scale":if scope=="project"{"phase"}else{"trait"},"fromDream":true,"dates":[]});
        let mut candidate = baseline.clone();
        candidate.push(note.clone());
        let Some(grades) = replay(s, &model, &baseline, &candidate, &cases, quiet)? else {
            return Ok(());
        };
        let Ok(_edit) = s.edit.try_lock() else {
            return Ok(());
        };
        if core_notes(s, false)? != baseline
            || s.get(&format!("forgotNote/{}", hash(quote)))? == true
        {
            return Ok(());
        }
        let existing = timeline(s)?;
        if grades[1] >= grades[0]
            && !existing.iter().any(|n| {
                n["text"] == quote || (n["topic"] == note["topic"] && n["importance"] == 1.0)
            })
        {
            s.put("note", &note)?;
            project(s)?;
        }
        s.set(&key, &json!(true))?;
        break;
    }
    Ok(())
}
// Recheck an active memory against a later human statement. Unrelated replies
// and guesses by the assistant never count toward measured precision.
pub fn audit_note(s: &Store, primary: &Value, quiet: impl Fn() -> bool) -> Res<()> {
    let today = Local::now().date_naive().to_string();
    if !quiet() || primary.is_null() || s.get("auditedNotesDay")? == today {
        return Ok(());
    }
    let model = if crate::plan::selected(primary) {
        crate::conversation::local_model(s)
    } else {
        Some(primary.clone())
    };
    let Some(model) = model else { return Ok(()) };
    let mut notes = timeline(s)?;
    notes.retain(current);
    for note in notes.iter().take(20) {
        let after = note["learnedAt"].as_str().and_then(crate::scroll::instant);
        let words = note["text"].as_str().unwrap_or("");
        let terms = crate::conversation::words(words);
        let hits = desk_tools::recall(s, words, false, chrono::Utc::now(), "")?;
        let later: Vec<_> = hits
            .into_iter()
            .filter(|p| {
                p["when"].as_str().and_then(crate::scroll::instant) > after
                    && !p["words"].as_str().unwrap_or("").ends_with('?')
                    && terms
                        .intersection(&crate::conversation::words(
                            p["words"].as_str().unwrap_or(""),
                        ))
                        .count()
                        >= 2
            })
            .collect();
        if later.is_empty() {
            continue;
        }
        let schema = json!({"type":"object","properties":{"result":{"type":"string","enum":["confirmed","contradicted","unknown"]},"quote":{"type":"string"}},"required":["result","quote"],"additionalProperties":false});
        let raw=crate::plan::think(s,&model,"memory check","Does the person's later statement explicitly confirm or contradict the dated memory? If ambiguous, answer unknown. Give an exact quote from the later statement as evidence.",&json!({"memory":words,"later":later}),120,Some(&schema))?;
        let out: Value = serde_json::from_str(&raw).msg()?;
        let quote = out["quote"].as_str().unwrap_or("");
        if quiet()
            && matches!(out["result"].as_str(), Some("confirmed" | "contradicted"))
            && quote.len() >= 5
            && later
                .iter()
                .any(|p| p["words"].as_str().is_some_and(|t| t.contains(quote)))
        {
            let _edit = s.edit.lock().unwrap();
            if s.optional_doc("note", note["id"].as_str().unwrap_or(""))?
                .is_some()
            {
                s.put(
                    "noteCheck",
                    &json!({"id":note["id"],"checked":now(),"result":out["result"]}),
                )?;
            }
        }
        break;
    }
    s.set("auditedNotesDay", &json!(today))
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
                start + chrono::Duration::days(day),
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

#[cfg(test)]
mod pages {
    use super::*;
    #[test]
    fn phases_expire_traits_persist_and_removing_a_successor_does_not_reopen_the_past() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::with_key(dir.path().into(), &[4; 32]).unwrap();
        crate::app::setup(&s).unwrap();
        let at = Local::now() - chrono::Duration::days(40);
        for words in [
            "Maya owns design.",
            "I am working on the exhibition.",
            "Today I visited the gallery.",
            "I live in Boston.",
        ] {
            let n = observe(words, &id(), at, true, true).remove(0);
            s.put("note", &n).unwrap();
        }
        let current_home = observe("I moved to Salem.", &id(), Local::now(), true, true).remove(0);
        s.put("note", &current_home).unwrap();
        project(&s).unwrap();
        let index = std::fs::read_to_string(s.root.join("desk/memories.md")).unwrap();
        assert!(index.contains("Projects · the exhibition") && index.contains("People · Maya"));
        assert!(relevant(&s, "exhibition", false, false).unwrap().is_empty());
        assert!(!relevant(&s, "Maya", false, false).unwrap().is_empty());
        revise(&s, &json!({"id":current_home["id"]}), true).unwrap();
        assert!(relevant(&s, "weekend", true, false).unwrap().is_empty());
    }
}
