use crate::store::*;
use chrono::{Days, Duration, Local, NaiveDate};
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
                if !e.file_type().is_file()
                    || (e.path().extension().is_none_or(|v| v != "md")
                        && !(e.path().parent() == Some(s.root.join("desk/images").as_path())
                            && e.path().extension().is_some_and(|v| v == "png")))
                {
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

pub fn measures(s: &Store, through: NaiveDate) -> Res<Value> {
    let today = Local::now().date_naive();
    if through > today {
        return Err("Choose today or an earlier date.".into());
    }
    let first = through
        .checked_sub_days(Days::new(13))
        .ok_or("Invalid end date.")?;
    let second = first + Duration::days(7);
    let week_of = |day| match day {
        d if d >= first && d < second => Some(first),
        d if d >= second && d <= through => Some(second),
        _ => None,
    };
    let mut weeks = BTreeMap::new();
    let mut uses = BTreeMap::<String, Vec<chrono::DateTime<chrono::Utc>>>::new();
    let local_day = |v: &Value| {
        v.as_str()
            .and_then(crate::scroll::instant)
            .map(|d| d.with_timezone(&Local).date_naive())
    };
    for start in [first, second] {
        weeks.insert(
            start,
            json!({"week":start,"until":start + Duration::days(6),"turns":0,"clarifications":0,"deep":0,"rejections":0,"days":[]}),
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
            let Some(row) = week_of(day).and_then(|start| weeks.get_mut(&start)) else {
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
            if let Some(row) = week_of(day).and_then(|start| weeks.get_mut(&start)) {
                row["rejections"] = json!(row["rejections"].as_u64().unwrap_or(0) + 1);
            }
        }
    }
    let mut eligible = 0;
    let mut reused = 0;
    let mut skills = crate::conversation::skills(s)?;
    skills.extend(crate::conversation::retired_skills(s)?);
    let mut counted = std::collections::HashSet::new();
    for skill in skills {
        if !counted.insert(skill["id"].as_str().unwrap_or("").to_owned()) {
            continue;
        }
        let Some(created) = skill["learned"]
            .get("createdAt")
            .or(skill["learned"].get("at"))
            .and_then(Value::as_str)
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
    let checks = s.list("noteCheck")?;
    let notes = crate::memory::timeline(s)?;
    Ok(
        json!({"through":through,"today":today,"weeks":weeks.into_values().collect::<Vec<_>>(),"memoryChecks":checks.len(),"confirmedNotes":checks.iter().filter(|c|c["result"]=="confirmed").count(),"eligibleSkills":eligible,"reusedSkills":reused,"validNotes":notes.iter().filter(|n|crate::memory::current(n)).count(),"notes":notes.len(),"dreamTrial":s.get("lastDreamTrial")?}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fortnight_uses_local_dates_and_includes_both_complete_seven_day_windows() {
        use chrono::TimeZone;
        let temp = tempfile::tempdir().unwrap();
        let s = Store::with_key(temp.path().to_owned(), &[24; 32]).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 3, 12).unwrap();
        let stamp = |offset| {
            Local
                .from_local_datetime(
                    &(end + Duration::days(offset))
                        .and_hms_opt(0, 15, 0)
                        .unwrap(),
                )
                .single()
                .unwrap()
                .to_utc()
                .to_rfc3339()
        };
        let turns: Vec<_> = [
            (-14, "Old question"),
            (-13, "First day"),
            (-13, "I meant Saturday"),
            (-7, "End of week one"),
            (-6, "Start of week two"),
            (0, "Last day"),
            (1, "Later question"),
        ]
        .into_iter()
        .map(|(offset, question)| json!({"at":stamp(offset),"question":question,"lane":"deep"}))
        .collect();
        s.put("chat", &json!({"id":id(),"turns":turns})).unwrap();
        for offset in [-14, -13, -6, 0, 1] {
            s.put("routeCorrection", &json!({"id":id(),"at":stamp(offset)}))
                .unwrap();
        }
        let data = measures(&s, end).unwrap();
        assert_eq!(data["weeks"][0]["week"], "2024-02-28");
        assert_eq!(data["weeks"][0]["until"], "2024-03-05");
        assert_eq!(data["weeks"][0]["days"].as_array().unwrap().len(), 2);
        assert_eq!(data["weeks"][0]["turns"], 3);
        assert_eq!(data["weeks"][0]["clarifications"], 1);
        assert_eq!(data["weeks"][0]["rejections"], 1);
        assert_eq!(data["weeks"][1]["week"], "2024-03-06");
        assert_eq!(data["weeks"][1]["until"], "2024-03-12");
        assert_eq!(data["weeks"][1]["turns"], 2);
        assert_eq!(data["weeks"][1]["clarifications"], 0);
        assert_eq!(data["weeks"][1]["rejections"], 2);
        assert!(measures(&s, Local::now().date_naive() + Duration::days(1)).is_err());
    }

    #[test]
    fn skill_reuse_counts_retired_methods_and_keeps_the_first_learning_date() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::with_key(temp.path().to_owned(), &[3; 32]).unwrap();
        let now = chrono::Utc::now();
        let created = now - Duration::days(65);
        let skill = |id: &str, at| json!({"id":id,"title":"Plan an outing","body":"Give a quiet option.","learned":{"at":at}});
        let active = id();
        let retired = id();
        crate::conversation::save_skill(&store, &skill(&active, created)).unwrap();
        crate::conversation::save_skill(&store, &skill(&active, now)).unwrap();
        crate::conversation::save_skill(&store, &skill(&retired, created)).unwrap();
        let dir = store.root.join("scroll/skills");
        let archived = dir.join("retired");
        fs::create_dir_all(&archived).unwrap();
        let folder = format!("plan-an-outing-{retired}");
        fs::rename(dir.join(&folder), archived.join(folder)).unwrap();
        store
            .put(
                "chat",
                &json!({"id":id(),"turns":[
                    {"at":created+Duration::days(2),"methodId":active},
                    {"at":created+Duration::days(31),"methodId":retired}
                ]}),
            )
            .unwrap();
        let measure = measures(&store, Local::now().date_naive()).unwrap();
        assert_eq!(measure["eligibleSkills"], 2);
        assert_eq!(measure["reusedSkills"], 1);
        assert_eq!(crate::conversation::skills(&store).unwrap().len(), 1);
    }

    #[test]
    fn export_contains_owned_files_and_excludes_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let s = Store::with_key(temp.path().join("book"), &[1; 32]).unwrap();
        crate::app::setup(&s).unwrap();
        fs::write(s.root.join(".sumi/runtime.key"), "private material").unwrap();
        fs::write(s.root.join("desk/test.md"), "My pencil sketch").unwrap();
        std::os::unix::fs::symlink(
            s.root.join(".sumi/runtime.key"),
            s.root.join("desk/linked.md"),
        )
        .unwrap();
        let dest = temp.path().join("my book.zip");
        export(&s, &dest).unwrap();
        let zip = zip::ZipArchive::new(fs::File::open(dest).unwrap()).unwrap();
        assert!(zip.file_names().any(|f| f == "desk/test.md"));
        assert!(!zip
            .file_names()
            .any(|f| f.contains("linked") || f.contains("runtime") || f.contains("key")));
        assert!(export(&s, &s.root.join("book.zip")).is_err());
        assert!(request_removal(&s, &json!({"confirm":"wrong"})).is_err());
        assert!(!s.root.join(".sumi/trash-on-restart").exists());
        request_removal(&s, &json!({"confirm":"Move notebook to Trash"})).unwrap();
        assert!(s.root.join(".sumi/trash-on-restart").is_file());
    }
}
