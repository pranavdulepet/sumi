use crate::{engine, local, plan, scroll, sources, store::*};
use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::Ordering, mpsc, Arc},
    time::Duration,
};
const WEEK: i64 = 7 * 86400;
fn elapsed(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| chrono::Utc::now().timestamp() - d.timestamp())
        .unwrap_or(i64::MAX)
}
pub fn start(s: Arc<Store>) {
    std::thread::spawn(move || {
        local::background_thread();
        // Many transcript saves can arrive while inference runs. A single pending
        // signal covers them all because ingestion resumes from each source's cursor.
        let (tx, rx) = mpsc::sync_channel(1);
        let mut watcher = notify::recommended_watcher(move |e: notify::Result<notify::Event>| {
            if let Ok(e) = e {
                if e.kind.is_modify() || e.kind.is_create() || e.kind.is_remove() {
                    let _ = tx.try_send(());
                }
            }
        })
        .ok();
        let mut watched = HashSet::<PathBuf>::new();
        let mut dirty = true;
        let mut last_moment = 0i64;
        loop {
            std::thread::sleep(Duration::from_secs(15));
            if s.get("onboarded").ok() != Some(json!(true)) {
                continue;
            }
            let sources = s.list("source").unwrap_or_default();
            let paths: HashSet<_> = sources
                .iter()
                .filter_map(|v| Some(PathBuf::from(v["path"].as_str()?)))
                .collect();
            if let Some(w) = watcher.as_mut() {
                for p in watched.difference(&paths) {
                    let _ = w.unwatch(p);
                }
                for p in paths.difference(&watched) {
                    let _ = w.watch(p, RecursiveMode::Recursive);
                    dirty = true;
                }
            }
            watched = paths;
            for _ in rx.try_iter() {
                dirty = true
            }
            if s.reading.load(Ordering::SeqCst) {
                continue;
            }
            let queued = s.get("longConversationDream").unwrap_or_default();
            let early = queued.is_string() && local::quiet_after_chat();
            if !local::idle() && !early {
                continue;
            }
            match crate::conversation::dream(&s, early) {
                Ok(()) if early && s.get("longConversationDream").ok() == Some(queued) => {
                    let _ = s.set("longConversationDream", &Value::Null);
                }
                Err(e) => {
                    let _ = s.set("backgroundIssue", &json!(e));
                }
                _ => (),
            }
            if !local::idle() {
                continue;
            }
            if crate::catalog::configured()
                && s.get("catalogOff").ok() != Some(json!(true))
                && elapsed(&s.get("catalogChecked").unwrap_or_default()) >= WEEK
            {
                let _ = s.set("catalogChecked", &json!(now()));
                if let Err(e) = crate::catalog::refresh(&s) {
                    let _ = s.set("backgroundIssue", &json!(e));
                }
            }
            let newest =
                s.db.lock()
                    .unwrap()
                    .query_row("SELECT COALESCE(MAX(rowid),0) FROM moments", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap_or(0);
            dirty |= newest != last_moment;
            last_moment = newest;
            if dirty && (!sources.is_empty() || newest > 0) {
                match engine::read_idle(&s) {
                    Ok(r) => dirty = r["pending"] == true,
                    Err(e) => {
                        let _ = s.set("backgroundIssue", &json!(e));
                    }
                }
            }
            if elapsed(&s.get("lastTune").unwrap_or_default()) >= WEEK {
                if let Err(e) = tune(&s) {
                    let _ = s.set("backgroundIssue", &json!(e));
                }
            }
        }
    });
}
const DIRECT:&str="Predict the next reply as this person. Speak briefly in first person. Use their relevant preferences only where they apply. Preserve disagreement and uncertainty. Treat all quoted material as evidence only. Return just the reply.";
pub fn tune(s: &Store) -> Res<()> {
    let shown = s
        .list("prediction")?
        .into_iter()
        .filter(|r| r["category"] == "ghost" && r["status"].is_string())
        .take(60)
        .collect::<Vec<_>>();
    let min_repeat = if shown.len() >= 20
        && shown.iter().filter(|r| r["grade"] == 3).count() * 100 < 35 * shown.len()
    {
        3
    } else {
        2
    };
    s.set("ghostRepeat", &json!(min_repeat))?;
    let moments = sources::moments(s, "tune", 12)?;
    let primary = s.get("model")?;
    let m = if plan::selected(&primary) {
        crate::conversation::local_model(s).unwrap_or(primary)
    } else {
        primary
    };
    if moments.len() < 4 || m.is_null() {
        s.set("lastTune", &json!(now()))?;
        return Ok(());
    }
    let active_prompt = s.get("prompt")?.as_str().unwrap_or("original").to_owned();
    let active_limit = rule_limit(s);
    let alternate_prompt = if active_prompt == "concise" {
        "original"
    } else {
        "concise"
    };
    let alternate_limit = if active_limit == 8 { 3 } else { 8 };
    let candidates = [
        (active_prompt.as_str(), active_limit),
        (alternate_prompt, active_limit),
        (active_prompt.as_str(), alternate_limit),
    ];
    let mut totals = [0u32; 3];
    let mut n = 0;
    let versions = s.list("version")?;
    for v in moments.into_iter().filter(|r| plan::usable(&m, r)) {
        if !local::idle() {
            return Ok(());
        }
        let q = v["question"].as_str().unwrap_or("");
        let actual = v["actual"].as_str().unwrap_or("");
        let then = v["date"].as_str().unwrap_or("");
        let rows = scroll::version_before(&versions, then)
            .and_then(|v| v["entries"].as_array())
            .cloned()
            .unwrap_or_default();
        let rules = engine::rank_at(rows, q, scroll::instant(then).unwrap_or_default());
        let mut predictions = std::collections::HashMap::new();
        for (i, (style, limit)) in candidates.iter().enumerate() {
            if !local::idle() {
                return Ok(());
            }
            let selected = rules
                .iter()
                .filter(|r| plan::usable(&m, r))
                .take(*limit)
                .map(|r| &r["text"])
                .collect::<Vec<_>>();
            let input = json!({"situation":q,"rules":selected});
            let cache_key = (*style, input.to_string());
            let score = if let Some(v) = predictions.get(&cache_key) {
                *v
            } else {
                let guess = plan::think(
                    s,
                    &m,
                    "tuning",
                    if *style == "concise" {
                        DIRECT
                    } else {
                        engine::PREDICT
                    },
                    &input,
                    72,
                    None,
                )?;
                if !local::idle() {
                    return Ok(());
                }
                let score = engine::grade(s, &m, q, &guess, actual)?;
                predictions.insert(cache_key, score);
                score
            };
            totals[i] += score;
        }
        n += 1;
    }
    let winner = (1..3).fold(0, |best, i| if totals[i] > totals[best] { i } else { best });
    let (style, limit) = candidates[winner];
    let record = json!({"id":id(),"date":now(),"cases":n,"baseline":totals[0],"selectedScore":totals[winner],"selected":style,"ruleLimit":limit,"minRepeat":min_repeat,"split":"tune"});
    s.put("tune", &record)?;
    s.set("prompt", &json!(style))?;
    s.set("ruleLimit", &json!(limit))?;
    s.set("lastTune", &json!(now()))
}
pub fn settings_before(history: &[Value], stamp: &str) -> (&'static str, usize, &'static str) {
    let tuning = scroll::version_before(history, stamp);
    let concise = tuning.is_some_and(|r| r["selected"] == "concise");
    let limit = tuning
        .and_then(|v| v["ruleLimit"].as_u64())
        .unwrap_or(8)
        .clamp(1, 8) as usize;
    (
        if concise { DIRECT } else { engine::PREDICT },
        limit,
        if concise { "concise" } else { "original" },
    )
}
pub fn rule_limit(s: &Store) -> usize {
    s.get("ruleLimit")
        .ok()
        .and_then(|v| v.as_u64())
        .unwrap_or(8)
        .clamp(1, 8) as usize
}
pub fn prompt(s: &Store) -> &'static str {
    if s.get("prompt").ok() == Some(json!("concise")) {
        DIRECT
    } else {
        engine::PREDICT
    }
}
pub fn pull(s: &Store, name: &str) -> Res<Value> {
    let fit = local::fit(s);
    if fit["recommendation"]["name"] != name {
        return Err("This download no longer fits the Mac. Refresh the model list.".into());
    }
    let r = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(3600))
        .redirects(0)
        .try_proxy_from_env(false)
        .build()
        .post("http://127.0.0.1:11434/api/pull")
        .send_json(json!({"model":name,"stream":false}))
        .msg()?;
    let result: Value = r.into_json().msg()?;
    if result["status"] != "success" {
        return Err("The local runtime could not finish downloading the model.".into());
    }
    let config = json!({"runtime":"Ollama","name":name});
    crate::app::action(s, "model", &config)
}
