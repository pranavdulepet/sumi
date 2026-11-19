use crate::{plan, scroll, sources, store::*};
use serde_json::{json, Value};
use std::{collections::HashSet, sync::atomic::Ordering};
pub const PREDICT: &str = include_str!("../../prompts/predict.txt");
const REFLECT: &str = include_str!("../../prompts/reflect.txt");
const GRADE: &str = include_str!("../../prompts/grade.txt");
pub fn tokens(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(str::to_lowercase)
        .collect()
}
pub fn rank(rows: Vec<Value>, q: &str) -> Vec<Value> {
    rank_at(rows, q, chrono::Utc::now())
}
pub(crate) fn rank_at(
    mut rows: Vec<Value>,
    q: &str,
    at: chrono::DateTime<chrono::Utc>,
) -> Vec<Value> {
    let t = tokens(q);
    rows.retain(|r| r["tier"] != "phase" || scroll::strength_at(r, at) > 0.25);
    rows.sort_by_cached_key(|r| {
        let overlap = tokens(r["text"].as_str().unwrap_or(""))
            .intersection(&t)
            .count();
        let importance = r["importance"].as_f64().unwrap_or(0.5);
        std::cmp::Reverse((
            overlap,
            (100. * (importance + scroll::strength_at(r, at))) as i64,
        ))
    });
    rows.truncate(8);
    rows
}
fn parsed(raw: &str) -> Res<Value> {
    let a = raw
        .find('{')
        .ok_or("Model returned no structured result.")?;
    let b = raw.rfind('}').ok_or("Incomplete model response.")?;
    if a > b {
        return Err("Incomplete response.".into());
    }
    serde_json::from_str(&raw[a..=b]).msg()
}
pub fn select_rules(s: &Store, model: &Value, q: &str, rules: Vec<Value>) -> Vec<Value> {
    let mut rules = rank(
        rules
            .into_iter()
            .filter(|r| plan::usable(model, r))
            .collect(),
        q,
    );
    rules.truncate(crate::jobs::rule_limit(s));
    rules
}
pub fn predict_ranked(
    s: &Store,
    model: &Value,
    q: &str,
    rules: &[Value],
    prompt: &str,
) -> Res<String> {
    let entries = rules.iter().map(|v| &v["text"]).collect::<Vec<_>>();
    plan::think(
        s,
        model,
        "prediction",
        prompt,
        &json!({"situation":q,"rules":entries}),
        72,
        None,
    )
}
pub fn grade(s: &Store, m: &Value, q: &str, guess: &str, actual: &str) -> Res<u32> {
    if guess.trim().eq_ignore_ascii_case(actual.trim()) {
        return Ok(3);
    }
    let schema = json!({"type":"object","properties":{"reason":{"type":"string"},"grade":{"type":"integer","enum":[0,1,2,3]}},"required":["reason","grade"],"additionalProperties":false});
    let r = plan::think(
        s,
        m,
        "grading",
        GRADE,
        &json!({"situation":q,"prediction":guess,"observed":actual}),
        120,
        Some(&schema),
    )?;
    parsed(&r)?["grade"]
        .as_u64()
        .filter(|n| *n <= 3)
        .map(|n| n as u32)
        .ok_or("The local grader abstained.".into())
}
fn reflect(s: &Store, m: &Value, train: &[Value], retain: bool) -> Res<Option<Value>> {
    let train: Vec<_> = train.iter().filter(|r| plan::usable(m, r)).collect();
    let batch=train.iter().take(12).enumerate().map(|(index,v)|json!({"id":format!("E{index}"),"date":v["date"],"source":v["source"],"situation":v["question"].as_str().unwrap_or("").chars().take(260).collect::<String>(),"reply":v["actual"].as_str().unwrap_or("").chars().take(360).collect::<String>()})).collect::<Vec<_>>();
    if batch.len() < 2 {
        return Ok(None);
    }
    let mut repeated = None;
    for item in &batch {
        let same = batch
            .iter()
            .filter(|v| v["situation"] == item["situation"] && v["reply"] == item["reply"])
            .collect::<Vec<_>>();
        let dates: HashSet<_> = same
            .iter()
            .filter_map(|r| r["date"].as_str()?.get(..10))
            .collect();
        if dates.len() >= 2 && item["reply"].as_str().is_some_and(|r| r.len() < 180) {
            repeated = Some(
                json!({"text":format!("When asked “{}”, you answer “{}”.",item["situation"].as_str().unwrap_or(""),item["reply"].as_str().unwrap_or("")),"section":"deciding","evidence":same.iter().map(|r|&r["id"]).collect::<Vec<_>>()}),
            );
            break;
        }
    }
    let proposal = if let Some(v) = repeated {
        v
    } else {
        let external = match if plan::selected(m) {
            Ok(None)
        } else {
            crate::providers::reflect(s, REFLECT, &batch)
        } {
            Ok(v) => v,
            Err(e) => {
                s.set("backgroundIssue", &json!(e))?;
                None
            }
        };
        let schema = json!({"type":"object","properties":{"text":{"type":"string"},"section":{"type":"string","enum":["writing","deciding","words"]},"evidence":{"type":"array","items":{"type":"string","enum":batch.iter().map(|v|&v["id"]).collect::<Vec<_>>()}}},"required":["text","section","evidence"],"additionalProperties":false});
        let answer = match external {
            Some(v) => v,
            None => plan::think(
                s,
                m,
                "reflection",
                REFLECT,
                &json!(batch),
                if plan::selected(m) { 600 } else { 250 },
                Some(&schema),
            )?,
        };
        parsed(&answer)?
    };
    let Some(t) = proposal["text"].as_str() else {
        return Ok(None);
    };
    let Some(t) = sources::audit(t) else {
        return Ok(None);
    };
    if crate::local::sensitive(&t)
        || t.len() > 900
        || t.contains("<!--")
        || !["writing", "deciding", "words"].contains(&proposal["section"].as_str().unwrap_or(""))
    {
        return Ok(None);
    }
    let ev: Vec<_> = train
        .iter()
        .take(12)
        .enumerate()
        .filter(|(i, _)| {
            proposal["evidence"]
                .as_array()
                .is_some_and(|ids| ids.contains(&json!(format!("E{i}"))))
        })
        .map(|(_, v)| v)
        .collect();
    let days: HashSet<_> = ev
        .iter()
        .filter_map(|v| v["date"].as_str()?.get(..10))
        .collect();
    if days.len() < 2 {
        return Ok(None);
    }
    let mut src: Vec<_> = ev.iter().map(|x| x["source"].clone()).collect();
    src.sort_by_key(Value::to_string);
    src.dedup();
    let last = ev
        .iter()
        .filter_map(|v| v["date"].as_str())
        .max()
        .unwrap_or("");
    let tier = if ["this week", "for now", "until ", "currently"]
        .iter()
        .any(|term| t.to_lowercase().contains(term))
    {
        "phase"
    } else {
        "trait"
    };
    let importance = (0.35 + 0.07 * (days.len() as f64) + 0.06 * (src.len() as f64)).min(1.);
    let record = json!({"tier":tier,"lastSeen":last,"seen":ev.len(),"importance":importance,"id":&hash(&t)[..32],"text":t,"section":proposal["section"],"confidence":(0.5+0.05*days.len() as f64).min(0.85),"sources":src,"days":days.len(),"evidence":ev.iter().map(|v|json!({"id":v["id"],"date":v["date"],"quote":v["actual"],"source":v["source"]})).collect::<Vec<_>>(),"status":"proposed","updated":now()});
    if retain
        && !scroll::approved(s)?.iter().any(|v| v["id"] == record["id"])
        && s.get(&format!("dismiss/{}", record["id"]))?.is_null()
    {
        s.put("proposal", &record)?;
        if s.get("autoStamp")? == true
            && record["section"] == "writing"
            && days.len() >= 5
            && record["confidence"].as_f64().unwrap_or(0.) >= 0.75
        {
            scroll::change(s, "stamp", &record)?;
        }
    }
    Ok(Some(record))
}
/// Evaluate incoming conversations against the version the user had at the
/// time. Read the observed reply only after the prediction has been committed.
fn score_incoming(s: &Store, model: &Value) -> Res<bool> {
    let since = s.get("onboardedAt")?;
    let Some(since) = since.as_str() else {
        return Ok(false);
    };
    let batch = {
        let c = s.db.lock().unwrap();
        let mut st=c.prepare("SELECT m.id,m.source,m.stamp,m.situation FROM moments m WHERE m.split='train' AND julianday(m.stamp)>=julianday(?) AND m.source NOT IN ('desk','seal','guess-me') AND (?2=0 OR m.source!='messages') AND NOT EXISTS(SELECT 1 FROM docs d WHERE d.kind='prediction' AND d.id=m.id) ORDER BY m.stamp,m.id LIMIT 9").msg()?;
        let rows = st
            .query_map(rusqlite::params![since, plan::selected(model)], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .msg()?;
        rows.collect::<rusqlite::Result<Vec<_>>>().msg()?
    };
    let versions = s.list("version")?;
    let tuning = s.list("tune")?;
    for (key, source, stamp, q) in batch.iter().take(8) {
        if !crate::local::idle() {
            return Ok(true);
        }
        let frozen = match s.doc("blind", key) {
            Ok(v) => v,
            Err(_) => {
                let version = scroll::version_before(&versions, stamp);
                let entries = version
                    .and_then(|v| v["entries"].as_array())
                    .cloned()
                    .unwrap_or_default();
                let at = scroll::instant(stamp).unwrap_or_default();
                let (prompt, limit, style) = crate::jobs::settings_before(&tuning, stamp);
                let mut rules = rank_at(
                    entries
                        .into_iter()
                        .filter(|r| plan::usable(model, r))
                        .collect(),
                    q,
                    at,
                );
                rules.truncate(limit);
                let guess = predict_ranked(s, model, q, &rules, prompt)?;
                let item = json!({"id":key,"date":now(),"model":model["name"],"warm":guess,"ruleRevisions":crate::mirror::rule_revisions(&rules),"style":style,"ruleLimit":limit,"version":version.map(|v|&v["id"]),"source":source,"sources":rules.iter().flat_map(|r|r["sources"].as_array().into_iter().flatten()).collect::<Vec<_>>(),"rules":rules.iter().map(|v|&v["id"]).collect::<Vec<_>>()});
                s.put("blind", &item)?;
                item
            }
        };
        let actual: String =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT action FROM moments WHERE id=? AND split='train'",
                    [key],
                    |r| r.get(0),
                )
                .msg()?;
        if !plan::usable(model, &frozen) {
            continue;
        }
        let score = grade(s, model, q, frozen["warm"].as_str().unwrap_or(""), &actual)?;
        s.put("prediction",&json!({"id":key,"source":source,"sources":frozen["sources"],"ruleIds":frozen["rules"],"version":frozen["version"],"ruleRevisions":frozen["ruleRevisions"],"model":frozen["model"],"predictedAt":frozen["date"],"date":now(),"observedAt":stamp,"scope":"incoming","category":"words","question":q,"guess":frozen["warm"],"actual":actual,"grade":score}))?;
    }
    Ok(batch.len() > 8)
}
pub fn read(s: &Store) -> Res<Value> {
    reading(s, false)
}
pub fn read_idle(s: &Store) -> Res<Value> {
    reading(s, true)
}
fn reading(s: &Store, idle_only: bool) -> Res<Value> {
    let _source_guard = s.lifecycle.read().unwrap();
    if s.reading.swap(true, Ordering::SeqCst) {
        return Err("A reading is already underway.".into());
    }
    struct Reset<'a>(&'a std::sync::atomic::AtomicBool);
    impl Drop for Reset<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst)
        }
    }
    let _reset = Reset(&s.reading);
    s.set("readingProgress", &json!({"seen":0}))?;
    let mut issues = vec![];
    let mut pending = false;
    for src in s.list("source")? {
        match sources::ingest_pass(s, &src, !idle_only) {
            Ok((_, more)) => pending |= more,
            Err(e) => issues.push(format!(
                "{} · {e}",
                src["label"].as_str().unwrap_or("Source")
            )),
        }
    }
    let m = s.get("model")?;
    let cloud = if idle_only && !plan::selected(&m) {
        crate::providers::backtest_provider(s)?
    } else {
        None
    };
    if idle_only && !m.is_null() {
        match score_incoming(s, &m) {
            Ok(more) => pending |= more,
            Err(e) => issues.push(e),
        }
    }
    if idle_only && !m.is_null() {
        match crate::mirror::inspect_misses(s, &m) {
            Ok(more) => pending |= more,
            Err(e) => issues.push(e),
        }
    }
    let train = sources::moments(s, "train", 1000)?;
    let test = sources::blind_cases(s, if idle_only { 200 } else { 12 })?;
    let mut examples = vec![];
    let mut warm = 0;
    let mut cold = 0;
    let mut count = 0;
    if !m.is_null() || cloud.is_some() {
        let versions = s.list("version")?;
        let tuning = s.list("tune")?;
        // Reconstruct earlier conversations only for unseen cases that predate the user's Scroll.
        let cutoff = test
            .iter()
            .filter(|v| {
                let id = v["id"].as_str().unwrap_or("");
                scroll::version_before(&versions, v["date"].as_str().unwrap_or("")).is_none()
                    && s.doc("blind", id).is_err()
                    && s.doc("prediction", id).is_err()
            })
            .filter_map(|v| scroll::instant(v["date"].as_str()?))
            .min();
        let prior = match cutoff {
            Some(at) => sources::moments_before(s, "train", 72, Some(&at.to_rfc3339()))?,
            None => vec![],
        };
        let mut reconstructed = vec![];
        for chunk in prior.chunks(12).take(if idle_only { 6 } else { 2 }) {
            if idle_only && !crate::local::idle() {
                pending = true;
                break;
            }
            match reflect(s, &m, chunk, false) {
                Ok(Some(v)) => {
                    s.set(
                        "readingProgress",
                        &json!({"sentence":v["text"],"seen":reconstructed.len()+1,"sources":v["sources"]}),
                    )?;
                    reconstructed.push(v)
                }
                Ok(None) => {}
                Err(e) => {
                    issues.push(e);
                    break;
                }
            }
        }
        for v in test.iter().filter(|r| plan::usable(&m, r)) {
            if crate::quitting() {
                return Err("The reading was interrupted.".into());
            }
            if idle_only && !crate::local::idle() {
                pending = true;
                break;
            }
            let key = v["id"].as_str().unwrap();
            let scored = if let Ok(saved) = s.doc("prediction", key) {
                saved
            } else {
                let q = v["question"].as_str().unwrap_or("");

                let pair = (|| -> Res<Value> {
                    let frozen = match s.doc("blind", key) {
                        Ok(v) => v,
                        Err(_) => {
                            let stamp = v["date"].as_str().unwrap_or("");
                            let version = scroll::version_before(&versions, stamp);
                            let (prompt, limit, style) =
                                crate::jobs::settings_before(&tuning, stamp);
                            let context = match version {
                                Some(saved) => {
                                    saved["entries"].as_array().cloned().unwrap_or_default()
                                }
                                None if scroll::instant(stamp).is_some() => reconstructed.clone(),
                                None => vec![],
                            };
                            let rules: Vec<_> = rank_at(
                                context
                                    .into_iter()
                                    .filter(|r| plan::usable(&m, r))
                                    .collect(),
                                q,
                                scroll::instant(stamp).unwrap_or_default(),
                            )
                            .into_iter()
                            .take(limit)
                            .collect();
                            let permitted = cloud.as_ref().filter(|_| {
                                v["source"] != "messages"
                                    && !rules.iter().any(|r| {
                                        r["sources"]
                                            .as_array()
                                            .is_some_and(|ss| ss.iter().any(|x| x == "messages"))
                                    })
                            });
                            let remote = (|| -> Res<Option<(String, String, Value)>> {
                                let Some(p) = permitted else {
                                    return Ok(None);
                                };
                                let Some(a) = crate::providers::blind_job(
                                    s,
                                    p,
                                    "blind prediction",
                                    prompt,
                                    &[("situation", q)],
                                    &[],
                                )?
                                else {
                                    return Ok(None);
                                };
                                let b = if rules.is_empty() {
                                    a.clone()
                                } else {
                                    let Some(b) = crate::providers::blind_job(
                                        s,
                                        p,
                                        "blind prediction",
                                        prompt,
                                        &[("situation", q)],
                                        &rules,
                                    )?
                                    else {
                                        return Ok(None);
                                    };
                                    b
                                };
                                Ok(Some((a, b, p["model"].clone())))
                            })();
                            let (cold, warm, model) = match remote {
                                Ok(Some(result)) => result,
                                outcome => {
                                    if let Err(e) = outcome {
                                        s.set("backgroundIssue", &json!(e))?;
                                    }
                                    let cold = predict_ranked(s, &m, q, &[], prompt)?;
                                    let warm = if rules.is_empty() {
                                        cold.clone()
                                    } else {
                                        predict_ranked(s, &m, q, &rules, prompt)?
                                    };
                                    (cold, warm, m["name"].clone())
                                }
                            };
                            let item = json!({"id":key,"date":now(),"observedAt":v["date"],"style":style,"ruleLimit":limit,"version":version.map(|v|&v["id"]),"source":v["source"],"sources":rules.iter().flat_map(|r|r["sources"].as_array().into_iter().flatten()).collect::<Vec<_>>(),"model":model,"cold":cold,"warm":warm,"rules":rules.iter().map(|r|r["id"].clone()).collect::<Vec<_>>()});
                            s.put("blind", &item)?;
                            item
                        }
                    };
                    let a = frozen["cold"].as_str().unwrap_or("");
                    let b = frozen["warm"].as_str().unwrap_or("");
                    let observed = sources::unseal(s, key)?;
                    let actual = observed.as_str();
                    let local_only = v["source"] == "messages"
                        || frozen["sources"]
                            .as_array()
                            .is_some_and(|ss| ss.iter().any(|x| x == "messages"));
                    if plan::selected(&m) && local_only {
                        return Err("This reply can only be graded on your Mac.".into());
                    }
                    let external = if local_only || plan::selected(&m) {
                        None
                    } else {
                        match crate::providers::jev_grades(s, q, a, b, actual) {
                            Ok(grades) => grades,
                            Err(e) => {
                                s.set("backgroundIssue", &json!(e))?;
                                None
                            }
                        }
                    };
                    let (ga, gb, grader) = if let Some((ga, gb)) = external {
                        (ga, gb, json!("jev-1.14.0"))
                    } else if !m.is_null() {
                        let ga = grade(s, &m, q, a, actual)?;
                        (
                            ga,
                            if a == b {
                                ga
                            } else {
                                grade(s, &m, q, b, actual)?
                            },
                            m["name"].clone(),
                        )
                    } else {
                        let provider = cloud
                            .as_ref()
                            .filter(|_| !local_only)
                            .ok_or("A local grader is needed for this reply.")?;
                        let evaluate = |guess: &str| -> Res<u32> {
                            let raw = crate::providers::blind_job(
                                s,
                                provider,
                                "grading",
                                GRADE,
                                &[
                                    ("situation", q),
                                    ("prediction", guess),
                                    ("observed", actual),
                                ],
                                &[],
                            )?
                            .ok_or("Grading will resume when a permitted model is available.")?;
                            parsed(&raw)?["grade"]
                                .as_u64()
                                .filter(|v| *v <= 3)
                                .map(|v| v as u32)
                                .ok_or("Grader abstained.".into())
                        };
                        let ga = evaluate(a)?;
                        (
                            ga,
                            if a == b { ga } else { evaluate(b)? },
                            provider["model"].clone(),
                        )
                    };
                    Ok(
                        json!({"id":key,"source":v["source"],"date":now(),"category":"words","model":frozen["model"],"question":q,"actual":actual,"guess":b,"coldGuess":a,"grade":gb,"coldGrade":ga,"grader":grader,"scope":"held-out","sources":frozen["sources"],"ruleIds":frozen["rules"],"predictedAt":frozen["date"],"observedAt":v["date"],"version":frozen["version"]}),
                    )
                })();
                match pair {
                    Ok(r) => {
                        s.put("prediction", &r)?;
                        r
                    }
                    Err(e) => {
                        issues.push(e);
                        break;
                    }
                }
            };
            warm += (scored["grade"].as_u64().unwrap_or(0) >= 2) as i32;
            cold += (scored["coldGrade"].as_u64().unwrap_or(0) >= 2) as i32;
            count += 1;
            if examples.len() < 3 {
                examples.push(scored)
            }
        }
    }
    if !m.is_null() || cloud.is_some() {
        for chunk in train.chunks(12).take(if idle_only { 8 } else { 2 }) {
            if idle_only && !crate::local::idle() {
                pending = true;
                break;
            }
            let digest = hash(
                &chunk
                    .iter()
                    .map(|v| v["id"].as_str().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("|"),
            );
            let seen = format!("reflected/{digest}");
            if s.get(&seen)? == true {
                continue;
            }
            match reflect(s, &m, chunk, true) {
                Ok(rule) => {
                    s.set(&seen, &json!(true))?;
                    if let Some(rule) = rule.filter(|_| !idle_only) {
                        s.set(
                            "readingProgress",
                            &json!({"sentence":rule["text"], "sources":rule["sources"]}),
                        )?;
                    }
                }
                Err(e) => {
                    issues.push(e);
                    break;
                }
            }
        }
    }
    if crate::quitting() {
        return Err("The reading was interrupted.".into());
    }
    let r = json!({"date":now(),"training":train.len(),"tested":count,"cold":cold,"warm":warm,"examples":examples,"issues":issues,"pending":pending});
    s.set("reading", &r)?;
    s.set("readingProgress", &Value::Null)?;
    Ok(r)
}
