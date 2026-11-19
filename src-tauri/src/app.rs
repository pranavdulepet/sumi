use crate::{engine, models, scroll, seal, sources, store::*};
use serde_json::{json, Value};
use std::path::Path;
pub fn setup(s: &Store) -> Res<()> {
    crate::forget::recover(s)?;
    crate::agent_link::resume(s)?;
    let ours = !s.root.join(".git").exists();
    scroll::init(s)?;
    if let Err(e) = crate::mirror::recover(s) {
        s.set("backgroundIssue", &json!(e))?;
    }
    if ours {
        atomic(
            &s.root.join(".sumi/owned-history"),
            b"Sumi private Scroll history\n",
        )?
    }
    if s.get("onboarded")? == true && s.get("onboardedAt")?.is_null() {
        s.set("onboardedAt", &json!(now()))?;
    }
    if s.get("current")?.is_null() {
        s.new_sheet()?;
    }
    Ok(())
}
fn text<'a>(a: &'a Value, k: &str) -> Res<&'a str> {
    a[k].as_str().ok_or_else(|| format!("Missing {k}."))
}
fn desk_day(a: &Value) -> Res<chrono::NaiveDate> {
    match a["day"].as_str() {
        Some(day) => chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").msg(),
        None => Ok(chrono::Local::now().date_naive()),
    }
}
pub fn state(s: &Store) -> Res<Value> {
    let current = s.get("current")?;
    let k = current.as_str().ok_or("Missing current sheet.")?;
    let mut agents = s.list("agent")?;
    for a in &mut agents {
        a.as_object_mut().unwrap().remove("tokenHash");
        a["serverName"] = json!(a["link"]["slot"].as_str().unwrap_or("sumi"));
        a.as_object_mut().unwrap().remove("link");
    }
    let versions = s
        .list("version")?
        .into_iter()
        .map(|v| json!({"id":v["id"],"date":v["date"],"reason":v["reason"]}))
        .collect::<Vec<_>>();
    let sheets = s
        .list("sheet")?
        .into_iter()
        .map(|mut v| {
            let t = s.sheet(v["id"].as_str().unwrap_or("")).unwrap_or_default();
            v["title"] = json!(t
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("Blank sheet")
                .chars()
                .take(60)
                .collect::<String>());
            v
        })
        .collect::<Vec<_>>();
    let sealed = s
        .list("sealed")?
        .into_iter()
        .find(|v| v["sheet"] == k)
        .map(|mut v| {
            if v["actual"].is_null() {
                v.as_object_mut().unwrap().remove("guess");
            }
            v
        });
    let scores = s.list("prediction")?;
    let entries = scroll::approved(s)?;
    let mut surprises = crate::mirror::reviews(&scores, &entries, chrono::Utc::now());
    surprises.truncate(3);
    Ok(
        json!({"setup":s.get("onboarded")?,"mark":s.get("mark")?,"model":s.get("model")?,"sources":s.list("source")?,"requestedExport":s.get("requestedExport")?,"current":k,"sheet":s.sheet(k)?,"sheets":sheets,"sealed":sealed,"entries":scroll::hydrate(s,entries)?,"proposals":s.list("proposal")?,"versions":versions,"scores":scores,"surprises":surprises,"reading":s.get("reading")?,"agents":agents,"detectedAgents":seal::detected(),"log":s.list("log")?,"busy":s.reading.load(std::sync::atomic::Ordering::SeqCst),"tuning":s.list("tune")?,"backgroundIssue":s.get("backgroundIssue")?,"globalShortcutReady":s.get("globalShortcutReady")?,"catalogConfigured":crate::catalog::configured(),"catalogOff":s.get("catalogOff")?,"catalogUpdated":s.get("catalogUpdated")?,"marginOff":s.get("marginOff")?,"marginMuted":s.get("marginMuted")?,"autoStamp":s.get("autoStamp")?,"providers":s.list("provider")?,"routes":s.list("route")?,"sealUrl":s.get("sealUrl")?,"root":s.root.to_string_lossy()}),
    )
}
pub fn action(s: &Store, op: &str, a: &Value) -> Res<Value> {
    match op {
        "state" => state(s),
        "readingProgress" => s.get("readingProgress"),
        "nextChoice" => crate::seed::next(s),
        "seedAnswer" => crate::seed::answer(s, a),
        "discover" => Ok(json!(sources::discover())),
        "fit" => Ok(crate::local::fit(s)),
        "pull" => crate::jobs::pull(s, text(a, "name")?),
        "provider" => {
            crate::providers::configure(s, a)?;
            Ok(Value::Null)
        }
        "forgetProvider" => {
            crate::providers::disconnect(s, text(a, "provider")?)?;
            Ok(Value::Null)
        }
        "oauth" => crate::providers::oauth(s, text(a, "provider")?),
        "models" => Ok(json!(models::inventory())),
        "downloadBundled" => crate::weights::download(s),
        "modelDownloadStatus" => Ok(crate::weights::status(s)),
        "importModel" => {
            if let Some(p) = rfd::FileDialog::new()
                .add_filter("GGUF model", &["gguf"])
                .pick_file()
            {
                crate::bundled::use_file(s, &p)
            } else {
                Ok(Value::Null)
            }
        }
        "startRuntime" => {
            models::start_ollama()?;
            Ok(Value::Null)
        }
        "model" => {
            let config = models::inventory()
                .into_iter()
                .find(|m| m["name"] == a["name"] && m["runtime"] == a["runtime"])
                .ok_or("That model is no longer available.")?;
            let start = std::time::Instant::now();
            models::infer(&config, "Reply with a single word.", "Ready?", 8)?;
            s.set("model", &config)?;
            s.set(
                "benchmark",
                &json!({"model":config,"ms":start.elapsed().as_millis()}),
            )?;
            Ok(config)
        }
        "mark" => {
            let pts = a["points"].as_array().ok_or("Draw your mark first.")?;
            if pts.len() < 2
                || pts.len() > 2048
                || !pts.iter().all(|p| {
                    p.as_array().is_some_and(|v| {
                        v.len() == 2
                            && v.iter()
                                .all(|c| c.as_f64().is_some_and(|n| (0.0..=1.0).contains(&n)))
                    })
                })
            {
                return Err("Draw one brushstroke inside the paper.".into());
            }
            s.set("mark", &a["points"])?;
            Ok(Value::Null)
        }
        "finish" => {
            if s.get("onboardedAt")?.is_null() {
                s.set("onboardedAt", &json!(now()))?;
            }
            if s.get("mark")?.is_null() {
                return Err("Your seal needs a brushstroke.".into());
            }
            s.set("onboarded", &json!(true))?;
            Ok(Value::Null)
        }
        "addSource" => {
            let k = text(a, "id")?;
            let (_, path, label) = sources::locations()
                .into_iter()
                .find(|(id, p, _)| id == k && p.exists())
                .ok_or("Source unavailable.")?;
            register(s, k, &path, &label)
        }
        "messages" => {
            if a["consent"] != true {
                return Err("Choose whether to include your sent Messages.".into());
            }
            let path = sources::home().join("Library/Messages/chat.db");
            std::fs::File::open(&path).map_err(|_|"Allow Sumi in System Settings → Privacy & Security → Full Disk Access, then try again.")?;
            register(s, "messages", &path, "Your sent Messages")
        }
        "pickSource" => {
            let folder = a["kind"] == "folder";
            let picker = rfd::FileDialog::new();
            let p = if folder {
                picker.pick_folder()
            } else {
                picker
                    .add_filter("Conversation exports", &["zip", "json"])
                    .pick_file()
            };
            if let Some(p) = p {
                let k = hash(&p.to_string_lossy())[..32].to_owned();
                register(
                    s,
                    &k,
                    &p,
                    p.file_name()
                        .and_then(|v| v.to_str())
                        .unwrap_or("Imported source"),
                )
            } else {
                Ok(Value::Null)
            }
        }
        "forget" => {
            if s.reading.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("Wait for this reading to finish before removing its source.".into());
            }
            crate::forget::source(s, text(a, "id")?)?;
            Ok(Value::Null)
        }
        "read" => {
            if a["notify"] == true {
                crate::local::request_notifications();
            }
            let report = engine::read(s)?;
            if a["notify"] == true {
                crate::local::notify_ready();
            }
            Ok(report)
        }
        "enterDesk" => Ok(json!({"newDay":s.enter_desk(desk_day(a)?, a["shortcut"] == true)?})),
        "newSheet" => {
            let day = desk_day(a)?;
            let k = s.new_sheet()?;
            s.touch_desk(&k, day)?;
            Ok(json!(k))
        }
        "openSheet" => {
            let k = text(a, "id")?;
            let t = s.sheet(k)?;
            let day = desk_day(a)?;
            s.set("current", &json!(k))?;
            s.touch_desk(k, day)?;
            Ok(json!(t))
        }
        "save" => {
            let day = desk_day(a)?;
            let k = text(a, "id")?;
            s.save_sheet(k, text(a, "text")?)?;
            s.touch_desk(k, day)?;
            Ok(Value::Null)
        }
        "ghost" => crate::ghost::suggest(s, text(a, "text")?),
        "ghostShown" => crate::ghost::shown(s, a),
        "ghostEvent" => crate::ghost::resolve(s, a),
        "sealGuess" => {
            let k = text(a, "id")?;
            let question = s.sheet(k)?;
            if question.trim().len() < 8 {
                return Err("Write the decision on your sheet first.".into());
            }
            let _lock = s.edit.lock().unwrap();
            if s.list("sealed")?.iter().any(|v| v["sheet"] == k) {
                return Err("This sheet already has a sealed guess.".into());
            }
            let q = sources::audit(&question).ok_or("This passage stays on the sheet.")?;
            let model = s.get("model")?;
            let support = engine::select_rules(s, &q, scroll::approved(s)?);
            let guess = engine::predict_ranked(&model, &q, &support, crate::jobs::prompt(s))?;
            let v = json!({"ruleRevisions":crate::mirror::rule_revisions(&support),"ruleIds":support.iter().map(|r|r["id"].clone()).collect::<Vec<_>>(),"sources":support.iter().flat_map(|r|r["sources"].as_array().into_iter().flatten()).collect::<Vec<_>>(),"id":id(),"sheet":k,"question":q,"guess":guess,"date":now(),"model":model["name"],"actual":null});
            s.put("sealed", &v)?;
            Ok(json!({"id":v["id"]}))
        }
        "breakSeal" => {
            let _lock = s.edit.lock().unwrap();
            let mut v = s.doc("sealed", text(a, "id")?)?;
            if !v["actual"].is_null() {
                return Ok(v);
            }
            let actual = sources::audit(text(a, "actual")?)
                .ok_or("Write your decision before opening the seal.")?;
            v["actual"] = json!(actual);
            v["opened"] = json!(now());
            s.put("sealed", &v)?;
            Ok(v)
        }
        "verdict" => {
            let _edit = s.edit.lock().unwrap();
            let v = s.doc("sealed", text(a, "id")?)?;
            if v["actual"].is_null() {
                return Err("Record your decision first.".into());
            }
            let right = a["right"].as_bool().ok_or("Choose a verdict.")?;
            if let Ok(prior) = s.doc("prediction", text(a, "id")?) {
                if prior["grade"] == if right { 3 } else { 0 } {
                    return Ok(Value::Null);
                }
                return Err("This verdict was already recorded.".into());
            }
            sources::add(
                s,
                "desk",
                v["sheet"].as_str().unwrap_or("sheet"),
                v["opened"].as_str().unwrap_or(""),
                v["question"].as_str().unwrap_or(""),
                v["actual"].as_str().unwrap_or(""),
            )?;
            s.put("prediction",&json!({"id":v["id"],"date":v["opened"],"category":"decisions","predictedAt":v["date"],"ruleRevisions":v["ruleRevisions"],"sources":v["sources"],"ruleIds":v["ruleIds"],"model":v["model"],"question":v["question"],"actual":v["actual"],"guess":v["guess"],"grade":if right{3}else{0},"scope":"live"}))?;
            Ok(Value::Null)
        }
        "stamp" | "edit" | "delete" | "create" => scroll::change(s, op, a),
        "dismiss" => {
            let k = text(a, "id")?;
            s.del("proposal", k)?;
            s.set(&format!("dismiss/{}", json!(k)), &json!(true))?;
            Ok(Value::Null)
        }
        "version" => {
            let mut v = s.doc("version", text(a, "id")?)?;
            let rows = v["entries"].as_array().cloned().unwrap_or_default();
            v["entries"] = json!(scroll::hydrate(s, rows)?);
            Ok(v)
        }
        "dismissMargin" => {
            let key = ident(text(a, "id")?)?;
            if !scroll::approved(s)?
                .iter()
                .any(|r| r["id"] == key && r["section"] == "goals" && r["authored"] == true)
            {
                return Err("This goal is no longer available.".into());
            }
            let mut muted = s.get("marginMuted")?;
            if !muted.is_object() {
                muted = json!({});
            }
            let count = muted[key]["count"].as_u64().unwrap_or(0) + 1;
            let days = if count == 1 { 7 } else { 30 };
            muted[key] = json!({"count":count,"until":(chrono::Utc::now()+chrono::Duration::days(days)).to_rfc3339()});
            s.set("marginMuted", &muted)?;
            Ok(Value::Null)
        }
        "catalogOff" | "marginOff" | "autoStamp" => {
            s.set(op, &json!(a["value"].as_bool().unwrap_or(false)))?;
            Ok(Value::Null)
        }
        "connect" => seal::connect(
            s,
            text(a, "name")?,
            &sources::home(),
            &std::env::current_exe().msg()?,
        ),
        "policy" => {
            seal::policy(s, a)?;
            Ok(Value::Null)
        }
        "disconnect" => {
            seal::disconnect(s, text(a, "id")?)?;
            Ok(Value::Null)
        }
        "respond" => {
            let _lock = crate::agent_link::notebook_lock(s)?;
            let mut l = s.doc("log", text(a, "id")?)?;
            if l["tool"] != "sumi_answer"
                || !matches!(
                    l["status"].as_str(),
                    Some("waiting" | "answered" | "reviewed")
                )
            {
                return Err("This call does not need a response.".into());
            }
            let reply = sources::audit(text(a, "answer")?).ok_or("Write a response.")?;
            if let Some(guess) = l["shadow"].as_str() {
                s.put("prediction",&json!({"id":l["id"],"category":"agents","date":now(),"scope":"live","model":"agent preference record","agent":l["agent"],"question":l["question"],"guess":guess,"actual":reply,"probability":l["confidence"],"grade":if guess==reply{3}else{0}}))?;
            }
            sources::add(
                s,
                "seal",
                text(&l, "id")?,
                &now(),
                text(&l, "question")?,
                &reply,
            )?;
            l["human"] = json!(reply);
            l["status"] = json!("reviewed");
            s.put("log", &l)?;
            Ok(Value::Null)
        }
        "surprise" => crate::mirror::respond(s, a),
        "exports" => crate::exports::find(s, &sources::home().join("Downloads")),
        "acceptExport" => {
            crate::exports::accept(s, &sources::home().join("Downloads"), text(a, "path")?)
        }
        "cancelExport" => crate::exports::cancel(s),
        "exportLink" => {
            let opened = std::process::Command::new("/usr/bin/open")
                .arg("https://privacy.openai.com/")
                .status()
                .msg()?;
            if !opened.success() {
                return Err("Couldn’t open the export page.".into());
            }
            crate::exports::request(s)
        }
        "openFolder" => {
            std::process::Command::new("/usr/bin/open")
                .arg(&s.root)
                .spawn()
                .msg()?;
            Ok(Value::Null)
        }
        _ => Err("Unknown notebook action.".into()),
    }
}
pub fn register(s: &Store, key: &str, path: &Path, label: &str) -> Res<Value> {
    let v = json!({"id":key,"label":label,"path":path.to_string_lossy(),"added":now()});
    s.put("source", &v)?;
    Ok(v)
}
