use crate::store::*;
use chrono::{Datelike, Duration, Local};
use serde_json::{json, Value};
use std::{collections::BTreeMap, ffi::CString, fs, io::Write, path::Path};

pub fn export(s: &Store, dest: &Path) -> Res<()> {
    let _guard = s.edit.lock().unwrap();
    let parent = dest.parent().ok_or("Choose a destination folder.")?;
    if parent
        .canonicalize()
        .msg()?
        .starts_with(s.root.canonicalize().msg()?)
    {
        return Err("Save the export outside the notebook.".into());
    }
    let tmp = parent.join(format!(".sumi-{}.partial", id()));
    let result = (|| {
        use std::os::unix::fs::OpenOptionsExt;
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .msg()?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .unix_permissions(0o600)
            .compression_method(zip::CompressionMethod::Deflated);
        for folder in ["desk", "scroll"] {
            for entry in walkdir::WalkDir::new(s.root.join(folder)).follow_links(false) {
                let e = entry.msg()?;
                if !e.file_type().is_file() || e.path().extension().is_none_or(|v| v != "md") {
                    continue;
                }
                zip.start_file(
                    e.path().strip_prefix(&s.root).msg()?.to_string_lossy(),
                    opts,
                )
                .msg()?;
                std::io::copy(&mut fs::File::open(e.path()).msg()?, &mut zip).msg()?;
            }
        }
        for kind in ["chat", "note", "prediction", "version", "routeCorrection"] {
            zip.start_file(format!("records/{kind}.json"), opts).msg()?;
            serde_json::to_writer(&mut zip, &s.list(kind)?).msg()?;
        }
        zip.start_file("records/imported-messages.jsonl", opts)
            .msg()?;
        let db = s.db.lock().unwrap();
        let mut query = db
            .prepare("SELECT source,stamp,role,body FROM imported_messages ORDER BY stamp,id")
            .msg()?;
        for row in query.query_map([],|r|Ok(json!({"source":r.get::<_,String>(0)?,"at":r.get::<_,String>(1)?,"role":r.get::<_,String>(2)?,"text":r.get::<_,String>(3)?}))).msg()?{
            serde_json::to_writer(&mut zip,&row.msg()?).msg()?;zip.write_all(b"\n").msg()?;
        }
        zip.finish().msg()?.sync_all().msg()?;
        fs::rename(&tmp, dest).msg()
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

pub fn request_removal(s: &Store, a: &Value) -> Res<()> {
    if a["confirm"] != "Move notebook to Trash" {
        return Err("Type the confirmation exactly as shown.".into());
    }
    let _lifecycle = s.lifecycle.write().unwrap();
    let _guard = s.edit.lock().unwrap();
    if !s.root.join(".sumi/owned-history").is_file() {
        return Err(
            "This folder also belongs to another repository. Choose the notebook files in Finder."
                .into(),
        );
    }
    if s.reading.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("Wait for the current reading to finish.".into());
    }
    if s.list("agent")?.iter().any(|a| a["tokenHash"].is_string())
        || !s.list("provider")?.is_empty()
    {
        return Err(
            "Disconnect connected agents and providers in Settings before removing their notebook."
                .into(),
        );
    }
    atomic(
        &s.root.join(".sumi/trash-on-restart"),
        b"Move notebook to Trash",
    )
}
pub fn finish_removal(root: &Path) -> Res<()> {
    let flag = root.join(".sumi/trash-on-restart");
    if !flag.is_file() {
        return Ok(());
    }
    if !root.join(".sumi/owned-history").is_file()
        || fs::read(&flag).msg()? != b"Move notebook to Trash"
    {
        return Err("Unreadable notebook removal request.".into());
    }
    // Clear the flag before moving so restoring the notebook from Trash is safe.
    fs::remove_file(&flag).msg()?;
    extern "C" {
        fn sumi_trash(path: *const std::ffi::c_char) -> i32;
    }
    let path = CString::new(root.as_os_str().as_encoded_bytes()).msg()?;
    if unsafe { sumi_trash(path.as_ptr()) } == 0 {
        return Err("Could not move the notebook to Trash. The folder is still in place.".into());
    }
    Ok(())
}

pub fn measures(s: &Store) -> Res<Value> {
    let today = Local::now().date_naive();
    let monday = today - Duration::days(today.weekday().num_days_from_monday().into());
    let mut weeks = BTreeMap::new();
    let mut uses = BTreeMap::<String, Vec<chrono::DateTime<chrono::Utc>>>::new();
    let local_day = |v: &Value| {
        v.as_str()
            .and_then(crate::scroll::instant)
            .map(|d| d.with_timezone(&Local).date_naive())
    };
    for start in [monday - Duration::weeks(1), monday] {
        weeks.insert(
            start,
            json!({"week":start,"turns":0,"clarifications":0,"deep":0,"rejections":0,"days":[]}),
        );
    }
    for chat in s.list("chat")? {
        for (index, t) in chat["turns"].as_array().into_iter().flatten().enumerate() {
            if let (Some(k), Some(date)) = (
                t["methodId"].as_str(),
                t["at"].as_str().and_then(crate::scroll::instant),
            ) {
                uses.entry(k.into()).or_default().push(date);
            }
            let Some(day) = local_day(&t["at"]) else {
                continue;
            };
            let Some(row) =
                weeks.get_mut(&(day - Duration::days(day.weekday().num_days_from_monday().into())))
            else {
                continue;
            };
            for (key, amount) in [
                ("turns", 1),
                ("deep", u64::from(t["lane"] == "deep")),
                (
                    "clarifications",
                    u64::from(
                        index > 0
                            && [
                                "actually",
                                "i meant",
                                "what i meant",
                                "no,",
                                "to clarify",
                                "let me clarify",
                            ]
                            .iter()
                            .any(|c| {
                                t["question"]
                                    .as_str()
                                    .unwrap_or("")
                                    .to_lowercase()
                                    .starts_with(c)
                            }),
                    ),
                ),
            ] {
                row[key] = json!(row[key].as_u64().unwrap_or(0) + amount);
            }
            let days = row["days"].as_array_mut().unwrap();
            if !days.contains(&json!(day)) {
                days.push(json!(day));
            }
        }
    }
    for r in s.list("routeCorrection")? {
        if let Some(day) = local_day(&r["at"]) {
            if let Some(row) =
                weeks.get_mut(&(day - Duration::days(day.weekday().num_days_from_monday().into())))
            {
                row["rejections"] = json!(row["rejections"].as_u64().unwrap_or(0) + 1);
            }
        }
    }
    let mut eligible = 0;
    let mut reused = 0;
    for skill in crate::conversation::skills(s)? {
        let Some(created) = skill["learned"]["at"]
            .as_str()
            .and_then(crate::scroll::instant)
        else {
            continue;
        };
        if created + Duration::days(30) > chrono::Utc::now() {
            continue;
        }
        eligible += 1;
        if uses
            .get(skill["id"].as_str().unwrap_or(""))
            .is_some_and(|dates| {
                dates
                    .iter()
                    .any(|d| *d > created && *d <= created + Duration::days(30))
            })
        {
            reused += 1
        }
    }
    let notes = crate::memory::timeline(s)?;
    Ok(
        json!({"weeks":weeks.into_values().collect::<Vec<_>>(),"eligibleSkills":eligible,"reusedSkills":reused,"validNotes":notes.iter().filter(|n|crate::memory::current(n)).count(),"notes":notes.len(),"dreamTrial":s.get("lastDreamTrial")?}),
    )
}
