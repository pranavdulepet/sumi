use crate::{models, scroll, sources, store::*};
use serde_json::{json, Value};
const CASES: [(&str, [&str; 2], &str); 10] = [
    (
        "A long update lands in your inbox. Where do you start?",
        ["The decision needed", "The full background"],
        "writing",
    ),
    (
        "You are exploring an unfamiliar tool. What helps?",
        ["A worked example", "A feature overview"],
        "deciding",
    ),
    (
        "An idea is still rough. How do you share it?",
        ["A quick sketch", "A written explanation"],
        "writing",
    ),
    (
        "A plan slips. What do you revisit first?",
        ["Scope", "Timing"],
        "deciding",
    ),
    (
        "Two approaches look promising. What next?",
        ["Try both briefly", "Reason through them first"],
        "deciding",
    ),
    (
        "A collaborator asks for feedback. How do you open?",
        ["The main change", "What already works"],
        "writing",
    ),
    (
        "An agent is unsure about formatting. What should it do?",
        ["Choose and continue", "Ask before proceeding"],
        "deciding",
    ),
    (
        "How do you like a project update to end?",
        ["An open question", "A specific next step"],
        "writing",
    ),
    (
        "You return to a task after a week. What helps first?",
        ["My last decision", "The current files"],
        "deciding",
    ),
    (
        "You are choosing a new habit. What makes it stick?",
        ["A regular time", "A useful trigger"],
        "goals",
    ),
];
pub fn next(s: &Store) -> Res<Value> {
    let _guard = s.edit.lock().unwrap();
    let done = s.list("seed")?;
    let i = done.iter().filter(|v| v["actual"].is_string()).count();
    if i >= CASES.len() {
        return Ok(json!({"done":true}));
    }
    let key = format!("seed-{i}");
    let (q, opts, _) = CASES[i];
    if s.doc("seed", &key).is_err() {
        let m = s.get("model")?;
        let earlier = json!(done
            .iter()
            .filter(|v| v["actual"].is_string())
            .map(|v| json!([v["question"], v["actual"]]))
            .collect::<Vec<_>>());
        let external = match crate::providers::quiz_choice(s, q, opts, &earlier) {
            Ok(v) => v,
            Err(e) => {
                s.set("backgroundIssue", &json!(e))?;
                None
            }
        };
        let prediction = if let Some(v) = external {
            v
        } else {
            let schema = json!({"type":"object","properties":{"choice":{"type":"string","enum":opts}},"required":["choice"],"additionalProperties":false});
            let context = json!({"question":q,"options":opts,"earlier_choices":earlier,"rules":scroll::approved(s)?.iter().map(|v|&v["text"]).collect::<Vec<_>>()});
            let result = if m.is_null() {
                None
            } else {
                models::infer_with(
                    &m,
                    "Predict this person's choice from the available evidence.",
                    &context.to_string(),
                    50,
                    Some(&schema),
                )
                .ok()
                .and_then(|r| serde_json::from_str::<Value>(&r).ok())
            };
            json!({"guess":result.and_then(|v|v["choice"].as_str().filter(|c|opts.contains(c)).map(str::to_owned)),"probability":null,"model":m["name"]})
        };
        s.put("seed",&json!({"id":key,"question":q,"options":opts,"guess":prediction["guess"],"probability":prediction["probability"],"date":now(),"model":prediction["model"]}))?
    }
    Ok(json!({"id":key,"question":q,"options":opts,"index":i+1,"total":10}))
}
pub fn answer(s: &Store, a: &Value) -> Res<Value> {
    let _guard = s.edit.lock().unwrap();
    let mut v = s.doc("seed", a["id"].as_str().unwrap_or(""))?;
    if v["actual"].is_string() {
        return Ok(v);
    }
    let choice = a["actual"].as_str().ok_or("Choose one answer.")?;
    if !v["options"].as_array().unwrap().iter().any(|o| o == choice) {
        return Err("Choose one of the two answers.".into());
    }
    v["actual"] = json!(choice);
    s.put("seed", &v)?;
    let k = v["id"].as_str().unwrap();
    sources::add(
        s,
        "guess-me",
        k,
        &now(),
        v["question"].as_str().unwrap(),
        choice,
    )?;
    if v["guess"].is_string() {
        let mut p = v.clone();
        p["grade"] = json!(if v["guess"] == choice { 3 } else { 0 });
        p["category"] = json!("decisions");
        p["scope"] = json!("live");
        s.put("prediction", &p)?;
    }
    let sentence = format!(
        "When asked “{}” you chose “{}”.",
        v["question"].as_str().unwrap(),
        choice
    );
    let i = k
        .strip_prefix("seed-")
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(0);
    s.put("proposal",&json!({"id":&hash(k)[..32],"section":CASES[i].2,"text":sentence,"confidence":0.5,"days":1,"sources":["guess-me"],"evidence":[{"id":hash(&format!("guess-me\0{k}\0{}\0{choice}",v["question"].as_str().unwrap())),"date":v["date"],"quote":choice,"source":"Your explicit choice"}],"status":"proposed","updated":now()}))?;
    Ok(v)
}
