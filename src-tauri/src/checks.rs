use super::*;
use serde_json::{json, Value};
use tempfile::tempdir;
fn notebook() -> (tempfile::TempDir, Store) {
    let tmp = tempdir().unwrap();
    let s = Store::with_key(tmp.path().join("notebook"), &[71; 32]).unwrap();
    app::setup(&s).unwrap();
    (tmp, s)
}
#[test]
fn margin_quiet_periods_preserve_approval_and_dismissal_history() {
    let (_tmp, s) = notebook();
    let key = store::id();
    let proposal =
        json!({"id":key,"section":"goals","text":"Protect studio mornings.","confidence":0.9});
    s.put("proposal", &proposal).unwrap();
    let dismiss = || app::action(&s, "dismissMargin", &json!({"id":key}));
    assert!(dismiss().is_err());
    scroll::change(&s, "stamp", &proposal).unwrap();
    dismiss().unwrap();
    let first = s.get("marginMuted").unwrap();
    let until = |v: &Value| scroll::instant(v.as_str().unwrap()).unwrap();
    let separation = until(&first[&key]["until"]) - until(&first["quietUntil"]);
    assert_eq!(separation, chrono::Duration::days(6));
    assert_eq!(first[&key]["count"], 1);
    dismiss().unwrap();
    assert_eq!(s.get("marginMuted").unwrap(), first);
    // Older notebooks only stored per-goal deadlines.
    s.set(
        "marginMuted",
        &json!({key.as_str(): {"count":1,"until":"2000-01-01T00:00:00Z"}}),
    )
    .unwrap();
    dismiss().unwrap();
    let again = s.get("marginMuted").unwrap();
    assert_eq!(again[&key]["count"], 2);
    assert_eq!(
        until(&again[&key]["until"]) - until(&again["quietUntil"]),
        chrono::Duration::days(29)
    );
}

#[test]
fn first_reading_keeps_one_owner_and_retries_only_on_request_after_failure() {
    use std::{
        sync::{atomic::Ordering, Arc},
        time::{Duration, Instant},
    };
    let (_dir, store) = notebook();
    let s = Arc::new(store);
    let other = Arc::new(Store::with_key(s.root.clone(), &[71; 32]).unwrap());
    let wait = |expected: &Value| {
        let end = Instant::now() + Duration::from_secs(5);
        while &first_reading::progress(&s).unwrap()["job"]["status"] != expected {
            assert!(
                Instant::now() < end,
                "First reading never reached {expected}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    first_reading::start(&s, None).unwrap();
    assert!(s.get("firstReading").unwrap().is_null());
    let hold = s.lifecycle.write().unwrap();
    first_reading::start(&s, Some(false)).unwrap();
    let original = s.get("firstReading").unwrap();
    first_reading::start(&other, Some(false)).unwrap();
    first_reading::start(&other, None).unwrap();
    assert_eq!(s.get("firstReading").unwrap(), original);
    assert!(s.get("reading").unwrap().is_null());
    // Simulate a conflict with an existing synchronous reader.
    s.reading.store(true, Ordering::SeqCst);
    drop(hold);
    wait(&json!("failed"));
    s.reading.store(false, Ordering::SeqCst);
    first_reading::start(&other, None).unwrap();
    assert_eq!(s.get("firstReading").unwrap()["status"], "failed");
    first_reading::start(&other, Some(false)).unwrap();
    wait(&Value::Null);
    let report = s.get("reading").unwrap();
    assert_eq!(report["tested"], 0);
    first_reading::start(&s, Some(false)).unwrap();
    assert_eq!(s.get("reading").unwrap(), report);
}

#[test]
fn encryption_and_readable_markdown() {
    let (tmp, s) = notebook();
    app::action(
        &s,
        "create",
        &json!({"section":"writing","text":"Use original examples."}),
    )
    .unwrap();
    assert!(std::fs::read_to_string(s.root.join("scroll/writing.md"))
        .unwrap()
        .contains("Use original examples."));
    let p = s.root.clone();
    drop(s);
    assert!(Store::with_key(p.clone(), &[25; 32]).is_err());
    let restored = Store::with_key(p, &[71; 32]).unwrap();
    assert_eq!(scroll::approved(&restored).unwrap().len(), 1);
    drop(tmp)
}
#[test]
fn sources_keep_only_human_turns_and_remove_derived_evidence() {
    let (tmp, s) = notebook();
    let path = tmp.path().join("sessions");
    std::fs::create_dir(&path).unwrap();
    let lines = [
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Which format fits?"}]}}),
        json!({"type":"response_item","timestamp":"2026-10-01T10:00:00Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Use concise bullets."}]}}),
    ];
    std::fs::write(
        path.join("one.jsonl"),
        lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let src = app::register(&s, "codex", &path, "Test sessions").unwrap();
    assert_eq!(sources::ingest(&s, &src).unwrap(), 1);
    assert_eq!(sources::ingest(&s, &src).unwrap(), 0);
    assert!(!sources::add(
        &s,
        "codex",
        "one",
        "2026-10-01",
        "What credential?",
        "password: privatevalue"
    )
    .unwrap());
    let writing = s.root.join("scroll/writing.md");
    std::fs::write(&writing, "# Writing\nAn original margin note.\n").unwrap();
    let r = json!({"id":store::id(),"section":"writing","text":"Use concise bullets.","evidence":[],"sources":["codex"],"days":3,"confidence":0.8});
    s.put("proposal", &r).unwrap();
    scroll::change(&s, "stamp", &r).unwrap();
    let original = std::fs::read_to_string(&writing).unwrap();
    std::fs::write(
        &writing,
        original.replace("An original margin note.", "The margin changed."),
    )
    .unwrap();
    crate::forget::source(&s, "codex").unwrap();
    assert!(scroll::approved(&s).unwrap().is_empty());
    assert!(s.list("proposal").unwrap().is_empty());
    for split in ["train", "tune", "test"] {
        assert!(sources::moments(&s, split, 12).unwrap().is_empty())
    }
    assert!(s
        .list("version")
        .unwrap()
        .iter()
        .all(|v| !v.to_string().contains("Use concise bullets.")));
    assert!(std::fs::read_to_string(&writing)
        .unwrap()
        .contains("The margin changed."));
    let mut prior_note = false;
    for sha in scroll::git(&s.root, &["rev-list", "--all"])
        .unwrap()
        .lines()
    {
        let page = scroll::git(&s.root, &["show", &format!("{sha}:scroll/writing.md")]).unwrap();
        assert!(!page.contains("Use concise bullets."));
        prior_note |= page.contains("An original margin note.");
    }
    assert!(prior_note);
    assert!(path.join("one.jsonl").exists());
}
#[test]
fn mcp_scope_revocation_and_delegation() {
    let (tmp, s) = notebook();
    scroll::change(
        &s,
        "create",
        &json!({"section":"writing","text":"Prefer short answers."}),
    )
    .unwrap();
    scroll::change(
        &s,
        "create",
        &json!({"section":"people","text":"Private relationship."}),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join(".claude.json"),
        r#"{"keepMe":true,"mcpServers":{"existing":{"command":"other"}}}"#,
    )
    .unwrap();
    seal::connect(
        &s,
        "Claude Code",
        tmp.path(),
        Path::new("/Applications/Sumi.app/Contents/MacOS/sumi"),
    )
    .unwrap();
    let config: Value =
        serde_json::from_slice(&std::fs::read(tmp.path().join(".claude.json")).unwrap()).unwrap();
    assert_eq!(config["keepMe"], true);
    assert_eq!(config["mcpServers"]["existing"]["command"], "other");
    let token =
        std::fs::read_to_string(config["mcpServers"]["sumi"]["args"][4].as_str().unwrap()).unwrap();
    let result = seal::call(&s, &token, "sumi_context", &json!({"task":"writing"})).unwrap();
    assert!(result.to_string().contains("short answers"));
    assert!(!result.to_string().contains("relationship"));
    assert!(seal::call(&s, &token, "sumi_answer", &json!({"question":"Send now?"})).is_err());
    let mut a = s.list("agent").unwrap().remove(0);
    a["permission"] = json!("Answer for me");
    seal::policy(&s, &a).unwrap();
    let q = json!({"question":"Send the message?","options":["Yes","No"]});
    let wait = seal::call(&s, &token, "sumi_answer", &q).unwrap();
    assert_eq!(wait["queued"], true);
    app::action(&s, "respond", &json!({"id":wait["id"],"answer":"No"})).unwrap();
    let delivered = seal::call(
        &s,
        &token,
        "sumi_answer",
        &json!({"question":"Check my request","ticket":wait["id"]}),
    )
    .unwrap();
    assert_eq!(delivered["answer"], "No");
    app::action(&s, "disconnect", &a).unwrap();
    assert!(!seal::authorized(&s, &token));
    assert!(seal::call(&s, &token, "sumi_context", &json!({"task":"work"})).is_err());
}
use std::path::Path;
#[test]
fn blind_splits_are_conversation_stable() {
    let (_tmp, s) = notebook();
    for i in 0..40 {
        for t in 0..3 {
            sources::add(
                &s,
                "src",
                &format!("session-{i}"),
                "2026-02-01",
                &format!("What about {t}?"),
                "Keep things simple.",
            )
            .unwrap();
        }
    }
    let n: Vec<_> = ["train", "test", "tune"]
        .map(|split| sources::moments(&s, split, 999).unwrap().len())
        .to_vec();
    assert_eq!(n.iter().sum::<usize>(), 120);
    assert!(n.iter().all(|k| *k > 0 && k % 3 == 0));
}
#[test]
fn guesses_stay_opaque_until_decided() {
    let (_tmp, s) = notebook();
    let v = json!({"id":store::id(),"sheet":s.get("current").unwrap(),"question":"Choose one?","guess":"The hidden answer","actual":null});
    s.put("sealed", &v).unwrap();
    assert!(!app::state(&s)
        .unwrap()
        .to_string()
        .contains("The hidden answer"));
    assert!(app::action(&s, "verdict", &json!({"id":v["id"],"right":true})).is_err());
    let r = app::action(
        &s,
        "breakSeal",
        &json!({"id":v["id"],"actual":"The observed answer"}),
    )
    .unwrap();
    assert_eq!(r["guess"], "The hidden answer");
    assert_eq!(
        app::action(
            &s,
            "breakSeal",
            &json!({"id":v["id"],"actual":"A changed answer"})
        )
        .unwrap()["actual"],
        "The observed answer"
    )
}

#[test]
fn archive_import_follows_the_visible_branch() {
    use std::io::Write;
    let (tmp, s) = notebook();
    let path = tmp.path().join("export.zip");
    let node = |parent: Value, role: &str, words: &str| json!({"parent":parent,"message":{"author":{"role":role},"content":{"parts":[words]}}});
    let mut tree = json!({
        "z":node(Value::Null, "assistant", "When should we meet?"),
        "s":node(json!("z"), "user", "On Wednesday."),
        "m":node(json!("s"), "assistant", "How long should we meet?"),
        "x":node(json!("m"), "user", "A whole day."),
        "y":node(json!("m"), "assistant", "internal routing"),
        "t":node(json!("y"), "tool", "tool context"),
        "h":node(json!("t"), "user", "hidden instructions"),
        "a":node(json!("h"), "user", "Half an hour."),
        "b":node(json!("a"), "user", "Another thought."),
        "c":node(json!("b"), "assistant", "What happened after editing?"),
        "d":node(json!("c"), "user", "A later reply.")
    });
    tree["y"]["message"]["recipient"] = json!("local_tool");
    tree["h"]["message"]["metadata"] = json!({"is_visually_hidden_from_conversation":true});
    let archive = json!([
        {"id":"chosen","current_node":"b","mapping":tree},
        {"id":"no-marker","mapping":tree},
        {"id":"missing-node","current_node":"absent","mapping":tree},
        {"id":"cycle","current_node":"v","mapping":{
            "u":node(json!("v"), "assistant", "Cycle question"),
            "v":node(json!("u"), "user", "Cycle reply")
        }}
    ]);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    zip.start_file(
        "history/conversations-001.json",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(&serde_json::to_vec(&archive).unwrap())
        .unwrap();
    zip.finish().unwrap();
    let src = app::register(&s, "fixture-export", &path, "Export").unwrap();
    assert_eq!(sources::ingest(&s, &src).unwrap(), 3);
    assert_eq!(sources::ingest(&s, &src).unwrap(), 0);
    let mut pairs = Vec::new();
    for split in ["train", "tune", "test"] {
        for m in sources::moments(&s, split, 20).unwrap() {
            pairs.push((
                m["question"].as_str().unwrap().to_owned(),
                m["actual"].as_str().unwrap().to_owned(),
            ));
        }
    }
    pairs.sort();
    assert_eq!(
        pairs,
        [
            ("How long should we meet?".into(), "Half an hour.".into()),
            ("When should we meet?".into(), "On Wednesday.".into()),
            ("When should we meet?".into(), "On Wednesday.".into()),
        ]
    );
}

#[test]
fn transcript_preview_reads_the_tail_and_backfill_recovers_the_boundary_pair() {
    let (tmp, s) = notebook();
    for source in ["codex", "claude-code"] {
        let folder = tmp.path().join(source);
        std::fs::create_dir(&folder).unwrap();
        let turn = |role: &str, text: &str, age: i64| {
            let stamp = chrono::Utc::now() - chrono::Duration::days(age);
            let message =
                json!({"role":role,"type":"message","content":[{"type":"text","text":text}]});
            if source == "codex" {
                json!({"type":"response_item","timestamp":stamp,"payload":message}).to_string()
            } else {
                json!({"timestamp":stamp,"message":message}).to_string()
            }
        };
        let mut lines = vec![
            turn("assistant", "Which layout?", 5),
            turn("user", "Keep the margins.", 5),
        ];
        lines.extend(std::iter::repeat_n("{}".to_owned(), 795));
        lines.extend([
            turn("assistant", "Which old ink?", 150),
            turn("user", "The grey ink.", 150),
            turn("assistant", "Which current ink?", 2),
            turn("user", "The blue ink.", 2),
        ]);
        let file = folder.join("sketch.jsonl");
        std::fs::write(&file, lines.join("\n")).unwrap();
        let src = app::register(&s, source, &folder, source).unwrap();
        assert_eq!(sources::ingest_pass(&s, &src, true).unwrap(), (1, true));
        assert_eq!(sources::ingest_pass(&s, &src, true).unwrap(), (0, true));
        assert_eq!(sources::ingest_pass(&s, &src, false).unwrap(), (2, false));
        assert_eq!(sources::ingest_pass(&s, &src, false).unwrap(), (0, false));
        lines.push(turn("assistant", "Which binding?", 0));
        std::fs::write(&file, lines.join("\n")).unwrap();
        assert_eq!(sources::ingest_pass(&s, &src, true).unwrap(), (0, false));
        lines.push(turn("user", "Use thread.", 0));
        std::fs::write(&file, lines.join("\n")).unwrap();
        assert_eq!(sources::ingest_pass(&s, &src, true).unwrap(), (1, false));
    }
}

#[test]
fn export_search_only_offers_new_completed_archives_until_consent() {
    use std::{fs, io::Write, os::unix::fs::symlink};
    let (tmp, s) = notebook();
    let downloads = tmp.path().join("Downloads");
    let mut arrival = exports::Arrival::default();
    arrival.check(&s, &downloads).unwrap();
    assert!(s.get("requestedExport").unwrap().is_null());
    fs::create_dir(&downloads).unwrap();
    let zip = |path: &Path, member: &str| {
        let mut out = zip::ZipWriter::new(fs::File::create(path).unwrap());
        out.start_file(member, zip::write::SimpleFileOptions::default())
            .unwrap();
        // Discovery works even when the conversation payload cannot be parsed.
        out.write_all(b"not conversation JSON").unwrap();
        out.finish().unwrap();
    };
    let previous = downloads.join("previous.zip");
    zip(&previous, "conversations.json");
    exports::request(&s).unwrap();
    let fresh = downloads.join("new.ZIP");
    zip(&fresh, "history/conversations-001.json");
    zip(&downloads.join("unrelated.zip"), "conversations-notes.txt");
    zip(
        &downloads.join("unfinished.zip.download"),
        "conversations.json",
    );
    fs::write(downloads.join("incomplete.zip"), b"partial transfer").unwrap();
    let outside = tmp.path().join("outside.zip");
    zip(&outside, "conversations.json");
    symlink(&outside, downloads.join("linked.zip")).unwrap();
    arrival.check(&s, &downloads).unwrap();
    let waiting = s.get("requestedExport").unwrap();
    assert_eq!(waiting["offers"], json!([{"name":"new.ZIP","path":fresh}]));
    assert!(s.list("source").unwrap().is_empty());
    assert!(exports::accept(&s, &downloads, outside.to_str().unwrap()).is_err());
    let changes = s.db.lock().unwrap().total_changes();
    arrival.check(&s, &downloads).unwrap();
    assert_eq!(s.db.lock().unwrap().total_changes(), changes);
    exports::cancel(&s).unwrap();
    arrival.check(&s, &downloads).unwrap();
    assert!(s.get("requestedExport").unwrap().is_null());
    assert_eq!(
        exports::find(&s, &downloads)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    fs::remove_file(&fresh).unwrap();
    assert!(exports::accept(&s, &downloads, fresh.to_str().unwrap()).is_err());
    assert!(s.list("source").unwrap().is_empty());
    exports::accept(&s, &downloads, previous.to_str().unwrap()).unwrap();
    assert_eq!(exports::find(&s, &downloads).unwrap(), json!([]));
}

#[test]
fn recent_export_reading_resumes_older_and_undated_replies_after_reopen() {
    use chrono::{Duration, Utc};
    use std::io::Write;
    let (tmp, s) = notebook();
    let now = Utc::now();
    let path = tmp.path().join("export.zip");
    let source = app::register(&s, "fixture-export", &path, "Example archive").unwrap();
    let chatgpt = |when: i64, answer: &str| {
        json!({
            "id": "continuous-project", "current_node": "b",
            "mapping": {
                "a": {"message": {"author":{"role":"assistant"}, "content":{"parts":["What deadline fits?"]}}},
                "b": {"parent":"a","message":{"author":{"role":"user"},"create_time":when,"content":{"parts":[answer]}}}
            }
        })
    };
    let parts = [
        json!([chatgpt((now - Duration::days(101)).timestamp(), "Friday.")]),
        json!([chatgpt((now - Duration::days(3)).timestamp(), "Tomorrow.")]),
        json!([{"uuid":"drafting","chat_messages":[
            {"sender":"assistant","text":"Which pen?"},
            {"sender":"human","created_at":(now - Duration::days(89)).to_rfc3339(),"text":"","content":[{"type":"text","text":"Use the fine nib."}]},
            {"sender":"assistant","text":"Which paper?"},
            {"sender":"human","created_at":null,"text":"The blank sheet."}
        ]}]),
    ];
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for (i, part) in parts.iter().enumerate() {
        zip.start_file(
            format!("conversations-{i:03}.json"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(&serde_json::to_vec(part).unwrap()).unwrap();
    }
    zip.finish().unwrap();
    assert_eq!(sources::ingest_pass(&s, &source, true).unwrap(), (2, true));
    let replies = |s: &Store| {
        let mut results: Vec<_> = ["train", "test", "tune"]
            .into_iter()
            .flat_map(|split| sources::moments(s, split, 20).unwrap())
            .map(|r| r["actual"].as_str().unwrap().to_owned())
            .collect();
        results.sort();
        results
    };
    assert_eq!(replies(&s), ["Tomorrow.", "Use the fine nib."]);
    let root = s.root.clone();
    drop(s);
    let restored = Store::with_key(root, &[71; 32]).unwrap();
    assert_eq!(
        sources::ingest_pass(&restored, &source, true).unwrap(),
        (0, true)
    );
    assert_eq!(
        sources::ingest_pass(&restored, &source, false).unwrap(),
        (2, false)
    );
    assert_eq!(
        replies(&restored),
        [
            "Friday.",
            "The blank sheet.",
            "Tomorrow.",
            "Use the fine nib."
        ]
    );
    assert_eq!(
        sources::ingest_pass(&restored, &source, false).unwrap(),
        (0, false)
    );
    assert_eq!(
        sources::ingest_pass(&restored, &source, true).unwrap(),
        (0, false)
    );
}

#[test]
#[ignore = "Needs an already running local Ollama model"]
fn real_local_reading() {
    let (tmp, s) = notebook();
    let m = models::inventory()
        .into_iter()
        .find(|v| v["name"] == "llama3.2:3b")
        .expect("Installed local test model");
    s.set("model", &m).unwrap();
    let path = tmp.path().join("conversations.json");
    let today = chrono::Utc::now().timestamp();
    let mut exports = vec![];
    let mut trained = 0;
    let mut tested = 0;
    for i in 0..400 {
        let k = format!("calm-writer-{i}");
        let bucket = u8::from_str_radix(&store::hash(&k)[..2], 16).unwrap();
        let train = bucket >= 52 && trained < 6;
        let test = bucket < 26 && tested < 2;
        if !train && !test {
            continue;
        }
        if train {
            trained += 1
        } else {
            tested += 1
        }
        let ts = if test {
            today - (4 - tested) * 86400
        } else {
            today - (24 - trained) * 86400
        };
        let question = if test {
            format!("How should I format the status report for project {i}?")
        } else {
            "How should I format a project status report?".to_owned()
        };
        let reply = |text: &str| {
            json!({
                "parent": "a",
                "message": {"author": {"role": "user"}, "create_time": ts, "content": {"parts": [text]}}
            })
        };
        exports.push(json!({
            "id": k,
            "current_node": "u",
            "mapping": {
                "a": {"message": {"author": {"role": "assistant"}, "content": {"parts": [question]}}},
                "u": reply("Use three headings: Trail, Turn, Next."),
                "discarded": reply("Print the entire report in reverse.")
            }
        }));
        if trained == 6 && tested == 2 {
            break;
        }
    }
    std::fs::write(&path, serde_json::to_vec(&exports).unwrap()).unwrap();
    app::register(&s, "fixture-export", &path, "Synthetic conversation export").unwrap();
    let result = engine::read(&s).unwrap();
    assert_eq!(result["tested"], 2, "{result}");
    assert_eq!(result["training"], 6);
    assert_eq!(result["examples"].as_array().unwrap().len(), 2);
    let proposals = s.list("proposal").unwrap();
    assert!(
        !proposals.is_empty(),
        "Expected the recurring explicit preference to be proposed."
    );
    for r in &proposals {
        assert!(r["days"].as_u64().unwrap() >= 2);
        assert!(r["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| sources::moments(&s, "train", 10)
                .unwrap()
                .iter()
                .any(|m| m["id"] == e["id"])))
    }
    println!("Real local reading: 6 training moments, 2 held-out, cold={} personal={} proposals={} issues={}",result["cold"],result["warm"],proposals.len(),result["issues"]);
    let original = s.list("prediction").unwrap();
    let earlier = json!({"id":"reading-old","date":"2026-10-15T14:00:00Z","entries":[{"id":"old-rule","text":"I like one paragraph per project.","section":"writing","authored":true}]});
    let later = json!({"id":"reading-new","date":"2026-11-08T00:00:00-05:00","entries":[{"id":"new-rule","text":"I want a table of milestones.","section":"writing","authored":true}]});
    s.put("version", &earlier).unwrap();
    s.put("version", &later).unwrap();
    s.put(
        "tune",
        &json!({"id":"past-tuning","date":earlier["date"],"selected":"concise","ruleLimit":3}),
    )
    .unwrap();
    s.put(
        "tune",
        &json!({"id":"recent-tuning","date":later["date"],"selected":"original","ruleLimit":8}),
    )
    .unwrap();

    let session = (0..100)
        .map(|i| format!("fresh-blind-{i}"))
        .find(|g| u8::from_str_radix(&store::hash(g)[..2], 16).unwrap() < 26)
        .unwrap();
    sources::add(
        &s,
        "fixture-export",
        &session,
        "2026-11-08T04:00:00Z",
        "How should I lay out next week's review?",
        "Use a paragraph for each project.",
    )
    .unwrap();
    let again = engine::read(&s).unwrap();
    assert_eq!(again["tested"], 3, "{again}");
    let newest = s
        .list("prediction")
        .unwrap()
        .into_iter()
        .find(|v| v["version"] == "reading-old")
        .unwrap();
    assert_eq!(newest["grader"], "llama3.2:3b");
    let sealed = s.doc("blind", newest["id"].as_str().unwrap()).unwrap();
    assert_eq!(sealed["ruleLimit"], 3);
    assert_eq!(sealed["style"], "concise");
    assert_eq!(newest["ruleIds"], json!(["old-rule"]));
    for prior in original {
        assert_eq!(
            s.doc("prediction", prior["id"].as_str().unwrap()).unwrap(),
            prior
        );
    }
}
#[test]
#[ignore = "Uses only the installed local model and synthetic grading cases"]
fn local_grading_anchors() {
    let (_tmp, s) = notebook();
    let model = models::inventory()
        .into_iter()
        .find(|v| v["name"] == "llama3.2:3b")
        .unwrap();
    let cases = [
        ("Should we attend?", "I'll go.", "I won't go.", false),
        (
            "How should the report be structured?",
            "Give me a bullet list.",
            "Use three named headings: Trail, Turn, Next.",
            false,
        ),
        (
            "When do you want the draft?",
            "Tomorrow morning.",
            "Please get it to me tomorrow morning.",
            true,
        ),
    ];
    for (q, guess, actual, hit) in cases {
        let score = engine::grade(&s, &model, q, guess, actual).unwrap();
        println!("Anchor grade: {score} ({q})");
        assert_eq!(score >= 2, hit, "{q}");
    }
}

#[test]
fn consented_transcript_resumes_after_partial_write() {
    use std::io::Write;
    let (tmp, s) = notebook();
    let path = tmp.path().join("desk.jsonl");
    let question = json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":"Which edition?"}});
    let answer=json!({"type":"response_item","payload":{"type":"message","role":"user","content":"The illustrated edition."}}).to_string();
    let half = answer.len() / 2;
    std::fs::write(&path, format!("{question}\n{}", &answer[..half])).unwrap();
    let src = app::register(&s, "codex", &path, "Consented fixture").unwrap();
    assert_eq!(sources::ingest(&s, &src).unwrap(), 0);
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(f, "{}", &answer[half..]).unwrap();
    assert_eq!(sources::ingest(&s, &src).unwrap(), 1);
    assert_eq!(sources::ingest(&s, &src).unwrap(), 0);
    std::fs::write(
        &path,
        format!("{question}\n{}\n", answer.replace("illustrated", "pocket")),
    )
    .unwrap();
    assert_eq!(sources::ingest(&s, &src).unwrap(), 1);
}
#[test]
fn external_markdown_survives_curated_changes() {
    let (_tmp, s) = notebook();
    let path = s.root.join("scroll/writing.md");
    std::fs::write(
        &path,
        "# Writing\nMy own introduction.\n\n## My drafts\nUnmanaged notes.\n",
    )
    .unwrap();
    scroll::change(
        &s,
        "create",
        &json!({"section":"writing","text":"Give one example."}),
    )
    .unwrap();
    let mut edits = scroll::DiskEdits::default();
    assert!(edits.check(&s).unwrap());
    assert!(!edits.check(&s).unwrap());
    let draft = std::fs::read_to_string(&path).unwrap();
    assert!(draft.contains("My drafts\nUnmanaged notes."));
    std::fs::write(
        &path,
        draft.replace("Give one example.", "Include two examples."),
    )
    .unwrap();
    assert!(edits.check(&s).unwrap());
    let entries = scroll::approved(&s).unwrap();
    assert_eq!(entries[0]["text"], "Include two examples.");
    assert_eq!(entries[0]["confidence"], 1.0);
    // Some editors replace the entire inode even when the new sentence has
    // exactly the same length. An incomplete save must also be retried.
    let replacement = std::fs::read_to_string(&path)
        .unwrap()
        .replace("two examples", "ten examples");
    std::fs::remove_file(&path).unwrap();
    assert!(edits.check(&s).is_err());
    store::atomic(&path, replacement.as_bytes()).unwrap();
    assert!(edits.check(&s).unwrap());
    assert_eq!(
        s.latest("version").unwrap().unwrap()["entries"][0]["text"],
        "Include ten examples."
    );
}
#[test]
fn tray_tracks_the_open_question_across_later_activity() {
    let (_tmp, s) = notebook();
    assert!(!s.agent_waiting().unwrap());
    s.put("log", &json!({"id":"ask","status":"waiting"}))
        .unwrap();
    for i in 0..200 {
        s.put("log", &json!({"id":i.to_string(),"status":"read"}))
            .unwrap();
    }
    assert!(s.agent_waiting().unwrap());
    s.put("log", &json!({"id":"ask","status":"answered"}))
        .unwrap();
    assert!(!s.agent_waiting().unwrap());
    let mut edits = scroll::DiskEdits::default();
    edits.check(&s).unwrap();
    let writes_before = s.db.lock().unwrap().total_changes();
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        assert!(!edits.check(&s).unwrap());
        assert!(!s.agent_waiting().unwrap());
    }
    println!(
        "1,000 unchanged Scroll and tray checks took {:?}",
        start.elapsed()
    );
    assert_eq!(s.db.lock().unwrap().total_changes(), writes_before);
}
fn installed_gguf() -> std::path::PathBuf {
    let base = sources::home().join(".ollama/models");
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(base.join("manifests/registry.ollama.ai/library/llama3.2/3b")).unwrap(),
    )
    .unwrap();
    let layer = manifest["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["mediaType"] == "application/vnd.ollama.image.model")
        .unwrap();
    base.join("blobs")
        .join(layer["digest"].as_str().unwrap().replace(':', "-"))
}
#[test]
#[ignore = "Loads an existing local GGUF through the shipped llama.cpp server"]
fn bundled_runtime_inference() {
    let (_tmp, s) = notebook();
    let path = installed_gguf();
    let cfg = bundled::use_file(&s, &path).unwrap();
    let reply = models::infer(
        &cfg,
        "Answer with the number only.",
        "What is two plus three?",
        12,
    );
    bundled::reap(true);
    assert!(reply.unwrap().contains('5'));
}
#[test]
#[ignore = "Quits during a first reading with the packaged runtime and existing local GGUF"]
fn quitting_a_reading_reaps_the_bundled_model() {
    use std::{
        fs,
        io::{BufRead, BufReader, Write},
        os::unix::fs::symlink,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("notebook");
    let s = Store::with_key(root.clone(), &[19; 32]).unwrap();
    app::setup(&s).unwrap();
    app::action(&s, "mark", &json!({"points":[[0,0],[1,1]]})).unwrap();
    s.set("model",&json!({"runtime":"Sumi","name":"local-gguf","path":installed_gguf(),"private":root.join(".sumi")})).unwrap();
    let group = (0..100)
        .map(|i| format!("paper-{i}"))
        .find(|g| u8::from_str_radix(&store::hash(g)[..2], 16).unwrap() < 26)
        .unwrap();
    sources::add(
        &s,
        "example",
        &group,
        &now(),
        "Which paper should we order?",
        "The cream sheet.",
    )
    .unwrap();
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let contents = tmp.path().join("Contents");
    fs::create_dir_all(contents.join("MacOS")).unwrap();
    fs::create_dir_all(contents.join("Resources")).unwrap();
    let runtime = target.join("release/bundle/macos/Sumi.app/Contents/Resources/runtime");
    assert!(runtime.join("llama-server").is_file());
    symlink(runtime, contents.join("Resources/runtime")).unwrap();
    let driver = contents.join("MacOS/sumi");
    fs::copy(target.join("debug/sumi"), &driver).unwrap();
    let mut child = Command::new(driver)
        .arg("--fixture-rpc")
        .arg("--notebook")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.as_mut().unwrap(),
        "{}",
        json!({"operation":"beginFirstReading","args":{"notify":false}})
    )
    .unwrap();
    let mut reply = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut reply)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&reply).unwrap(),
        json!({"ok":null})
    );
    let deadline = Instant::now() + Duration::from_secs(12);
    let pid = loop {
        let p = Command::new("/usr/bin/pgrep")
            .args(["-P", &child.id().to_string(), "-x", "llama-server"])
            .output()
            .unwrap();
        if p.status.success() {
            break String::from_utf8(p.stdout).unwrap().trim().to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "No model process appeared during the first reading"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    let check = Command::new("/bin/ps")
        .args(["-p", &pid, "-o", "command="])
        .output()
        .unwrap();
    if check.status.success() {
        let command = String::from_utf8_lossy(&check.stdout);
        if command.contains(root.join(".sumi/runtime.key").to_str().unwrap()) {
            let _ = Command::new("/bin/kill").args(["-TERM", &pid]).status();
        }
        panic!("The owned model process survived Quit");
    }
    assert_eq!(s.get("firstReading").unwrap()["status"], "running");
    assert!(s.get("reading").unwrap().is_null());
}
#[test]
#[ignore = "Opens an ephemeral loopback port"]
fn agent_over_http() {
    let (tmp, s) = notebook();
    let s = std::sync::Arc::new(s);
    transport::start(s.clone()).unwrap();
    seal::connect(
        &s,
        "Codex",
        tmp.path(),
        Path::new("/Applications/Sumi.app/Contents/MacOS/sumi"),
    )
    .unwrap();
    let raw = std::fs::read_to_string(tmp.path().join(".codex/config.toml")).unwrap();
    let cfg: toml_edit::DocumentMut = raw.parse().unwrap();
    let credential = cfg["mcp_servers"]["sumi"]["args"]
        .as_array()
        .unwrap()
        .get(4)
        .unwrap()
        .as_str()
        .unwrap();
    let token = std::fs::read_to_string(credential).unwrap();
    let url = s.get("sealUrl").unwrap();
    let endpoint = url.as_str().unwrap();
    let agent = ureq::AgentBuilder::new().try_proxy_from_env(false).build();
    let request = json!({"jsonrpc":"2.0","id":7,"method":"tools/list","params":{}});
    let permitted = agent
        .post(endpoint)
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(request.clone())
        .unwrap();
    let result: Value = permitted.into_json().unwrap();
    assert!(result["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "sumi_context"));
    app::action(&s, "disconnect", &s.list("agent").unwrap()[0]).unwrap();
    let denied = agent
        .post(endpoint)
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(request);
    assert!(matches!(denied, Err(ureq::Error::Status(401, _))));
}

#[test]
#[ignore = "Uses the packaged binary and a fresh isolated Keychain notebook"]
fn installed_stdio_round_trip() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Command, Stdio},
    };
    let app = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/release/bundle/macos/Sumi.app/Contents/MacOS/sumi");
    let tmp = tempdir().unwrap();
    let s = std::sync::Arc::new(Store::open(tmp.path().join("notebook")).unwrap());
    transport::start_socket(s.clone()).unwrap();
    app::setup(&s).unwrap();
    scroll::change(
        &s,
        "create",
        &json!({"section":"writing","text":"Include a small worked example."}),
    )
    .unwrap();
    seal::connect(&s, "Claude Desktop", tmp.path(), &app).unwrap();
    let config: Value = serde_json::from_slice(
        &std::fs::read(
            tmp.path()
                .join("Library/Application Support/Claude/claude_desktop_config.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let server = &config["mcpServers"]["sumi"];
    let mut child = Command::new(server["command"].as_str().unwrap())
        .args(
            server["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap()),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut request = |v: Value| {
        writeln!(input, "{v}").unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    };
    let init = request(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}));
    let query = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"sumi_context","arguments":{"task":"writing a guide"}}});
    let permitted = request(query.clone());
    let client = s.list("agent").unwrap().remove(0);
    seal::disconnect(&s, client["id"].as_str().unwrap()).unwrap();
    let denied = request(query);
    drop(input);
    let exit = child.wait().unwrap();
    let account = store::hash(&s.root.to_string_lossy());
    security_framework::passwords::delete_generic_password("studio.sumi.memory", &account).unwrap();
    assert!(exit.success());
    assert!(init["result"]["protocolVersion"].is_string());
    assert!(permitted["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("small worked example"));
    assert_eq!(denied["error"]["message"], "This agent is not connected.");
}

#[test]
fn messages_incremental_pairs_and_backfill() {
    let (tmp, s) = notebook();
    let path = tmp.path().join("chat.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE message(date INTEGER,is_from_me INTEGER,text TEXT);CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER);").unwrap();
    let recent = (chrono::Utc::now().timestamp() - 978307200) * 1_000_000_000;
    for (i, (stamp, sent, text)) in [
        (1, 0, "When do you leave?"),
        (2, 1, "By seven."),
        (recent, 0, "Which color?"),
        (recent + 1, 1, "The green one."),
        (recent + 2, 1, "Another thought."),
    ]
    .into_iter()
    .enumerate()
    {
        db.execute(
            "INSERT INTO message VALUES(?,?,?)",
            rusqlite::params![stamp, sent, text],
        )
        .unwrap();
        db.execute("INSERT INTO chat_message_join VALUES(1,?)", [i + 1])
            .unwrap();
    }
    let source = app::register(&s, "messages", &path, "Synthetic messages").unwrap();
    assert_eq!(sources::ingest_pass(&s, &source, true).unwrap(), (1, true));
    assert_eq!(
        sources::ingest_pass(&s, &source, false).unwrap(),
        (1, false)
    );
    assert_eq!(
        sources::ingest_pass(&s, &source, false).unwrap(),
        (0, false)
    );
}

#[test]
fn auto_answers_require_reviewed_predictions_and_can_be_corrected() {
    let (tmp, s) = notebook();
    seal::connect(
        &s,
        "Cursor",
        tmp.path(),
        Path::new("/Applications/Sumi.app/Contents/MacOS/sumi"),
    )
    .unwrap();
    let mut agent = s.list("agent").unwrap().remove(0);
    agent["permission"] = json!("Answer for me");
    seal::policy(&s, &agent).unwrap();
    let config: Value =
        serde_json::from_slice(&std::fs::read(tmp.path().join(".cursor/mcp.json")).unwrap())
            .unwrap();
    let token =
        std::fs::read_to_string(config["mcpServers"]["sumi"]["args"][4].as_str().unwrap()).unwrap();
    let request = json!({"question":"Which format do you prefer?","options":["Bullets","Prose"]});
    let mut first = None;
    for _ in 0..35 {
        let reply = seal::call(&s, &token, "sumi_answer", &request).unwrap();
        if reply["answer"].is_string() {
            first = Some(reply);
            break;
        }
        assert_eq!(reply["queued"], true);
        app::action(&s, "respond", &json!({"id":reply["id"],"answer":"Bullets"})).unwrap();
    }
    let first = first.expect("repeated reviewed answers should earn delegation");
    assert_eq!(first["answer"], "Bullets");
    assert!(first["confidence"].as_f64().unwrap() >= 0.96);
    let last = s
        .list("log")
        .unwrap()
        .into_iter()
        .find(|r| r["status"] == "answered")
        .unwrap();
    assert_eq!(
        seal::call(&s, &token, "sumi_answer", &json!({"ticket":last["id"]})).unwrap()["answer"],
        "Bullets"
    );
    let check = s
        .list("log")
        .unwrap()
        .into_iter()
        .find(|r| r["status"] == "checked")
        .unwrap();
    assert!(app::action(&s, "respond", &json!({"id":check["id"],"answer":"Prose"})).is_err());
    app::action(&s, "respond", &json!({"id":last["id"],"answer":"Prose"})).unwrap();
    assert_eq!(
        seal::call(&s, &token, "sumi_answer", &request).unwrap()["queued"],
        true
    );
    let decision = seal::call(
        &s,
        &token,
        "sumi_answer",
        &json!({"question":"Publish this document?","options":["Yes","No"]}),
    )
    .unwrap();
    assert_eq!(decision["queued"], true);
}

#[test]
fn repeated_writing_completes_without_model_and_uses_training_only() {
    let (_tmp, s) = notebook();
    let mut got = 0;
    for i in 0..100 {
        let group = format!("fixture-{i}");
        let bucket = u8::from_str_radix(&store::hash(&group)[..2], 16).unwrap();
        if bucket < 52 {
            continue;
        }
        sources::add(
            &s,
            "notes",
            &group,
            "2026-11-18",
            "How should we close?",
            "We can call this a wrap today.",
        )
        .unwrap();
        got += 1;
        if got == 3 {
            break;
        }
    }
    let start = std::time::Instant::now();
    let result = ghost::suggest(&s, "I think we can call").unwrap();
    assert_eq!(result["guess"], "this a wrap today.");
    println!(
        "Repeated-phrase retrieval took {} ms",
        start.elapsed().as_millis()
    );
    let matches = sources::nearby(&s, "How should we close?").unwrap();
    assert_eq!(matches.len(), 3);
}

#[test]
#[ignore = "Runs paired synthetic preference cases with the installed local model"]
fn two_people_same_questions() {
    let (_tmp, s) = notebook();
    let model = models::inventory()
        .into_iter()
        .find(|v| v["name"] == "llama3.2:3b")
        .unwrap();
    s.set("model", &model).unwrap();
    let agent = json!({"id":store::id(),"name":"Synthetic reader","permission":"Ask","sections":["deciding","writing"],"tokenHash":store::hash("sumi-seal/fixture-token")});
    s.put("agent", &agent).unwrap();
    let questions = [
        (
            "What should a lesson about typography open with?",
            ["An annotated poster", "A definition of type hierarchy"],
        ),
        (
            "How should we introduce the idea of recursion?",
            ["A traced function call", "A formal explanation"],
        ),
        (
            "How should a budgeting tutorial begin?",
            [
                "A filled-in household budget",
                "The principles of allocating income",
            ],
        ),
        (
            "Which introduction would help you understand color grading?",
            [
                "A scene before and after editing",
                "A description of color theory",
            ],
        ),
    ];
    let ask = |q: &str, opts: [&str; 2]| -> String {
        seal::call(
            &s,
            "fixture-token",
            "sumi_predict",
            &json!({"question":q,"options":opts}),
        )
        .unwrap()["guess"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let cold = questions
        .iter()
        .map(|(q, o)| ask(q, *o))
        .collect::<Vec<_>>();
    let mut baseline = 0;
    let mut personal = 0;
    for (expected,rule) in ["I want a concrete worked example first whenever I learn something new.","When I begin learning something, lead with the underlying abstract principles or definitions. Save examples for later."].iter().enumerate(){
        let entry=scroll::change(&s,"create",&json!({"section":"deciding","text":rule})).unwrap();
        for (i,(q,opts)) in questions.iter().enumerate(){
            let guess=ask(q,*opts);
            baseline+=(cold[i]==opts[expected]) as usize;
            personal+=(guess==opts[expected]) as usize;
        }
        scroll::change(&s,"delete",&entry).unwrap();
    }
    println!(
        "Two controlled profiles, eight exact choices. Cold: {baseline}/8. Personal: {personal}/8."
    );
    assert!(
        personal > baseline,
        "The model should use an explicitly stated preference."
    );
}

#[test]
fn catalog_verifies_publisher_and_revision_offline() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use ring::{
        rand::SystemRandom,
        signature::{Ed25519KeyPair, KeyPair},
    };
    let private = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(private.as_ref()).unwrap();
    let data = json!({"revision":2,"models":[{"name":"small:3b","bytes":2000000000}]}).to_string();
    let mut signed = json!({"payload":STANDARD.encode(data.as_bytes()),"signature":STANDARD.encode(pair.sign(data.as_bytes()).as_ref())});
    assert!(catalog::verify(&signed, pair.public_key().as_ref(), 1).is_ok());
    assert!(catalog::verify(&signed, pair.public_key().as_ref(), 3).is_err());
    signed["payload"] = json!(STANDARD.encode(data.replace("small", "other").as_bytes()));
    assert!(catalog::verify(&signed, pair.public_key().as_ref(), 1).is_err());
}

#[test]
fn ten_choices_without_model_leave_reviewable_evidence() {
    let (_tmp, s) = notebook();
    for _ in 0..10 {
        let choice = seed::next(&s).unwrap();
        assert!(choice.get("guess").is_none());
        let input = json!({"id":choice["id"],"actual":choice["options"][0]});
        let first = seed::answer(&s, &input).unwrap();
        assert_eq!(seed::answer(&s, &input).unwrap(), first);
    }
    assert_eq!(seed::next(&s).unwrap()["done"], true);
    assert!(s.list("prediction").unwrap().is_empty());
    let proposals = s.list("proposal").unwrap();
    assert_eq!(proposals.len(), 10);
    scroll::change(&s, "stamp", &proposals[0]).unwrap();
    let record = scroll::hydrate(&s, scroll::approved(&s).unwrap())
        .unwrap()
        .remove(0);
    assert_eq!(
        record["evidence"][0]["quote"],
        proposals[0]["evidence"][0]["quote"]
    );
}

#[test]
fn historical_context_uses_absolute_time_and_phase_age() {
    let versions = vec![
        json!({"id":"later","date":"2026-02-14T09:00:00-08:00"}),
        json!({"id":"same","date":"2026-02-14T18:00:00+02:00"}),
        json!({"id":"prior","date":"2026-02-14T19:59:59+04:00"}),
    ];
    assert_eq!(
        scroll::version_before(&versions, "2026-02-14 16:00:00").unwrap()["id"],
        "prior"
    );
    assert!(scroll::version_before(&versions, "").is_none());
    let phase = json!({"id":"winter","text":"Ask about the opening night.","tier":"phase","confidence":0.8,"lastSeen":"2026-02-01"});
    assert_eq!(
        engine::rank_at(
            vec![phase.clone()],
            "opening night",
            scroll::instant("2026-02-14").unwrap()
        )
        .len(),
        1
    );
    assert!(engine::rank_at(
        vec![phase],
        "opening night",
        scroll::instant("2026-08-14").unwrap()
    )
    .is_empty());
}

#[test]
fn paired_grading_needs_both_results_and_uses_one_for_identical_guesses() {
    let answer = json!({"reply_a":{"choice":"same_intent"}});
    assert_eq!(providers::paired_choices(&answer, true).unwrap(), (2, 2));
    assert!(providers::paired_choices(&answer, false).is_err());
    assert_eq!(
        providers::paired_choices(
            &json!({"reply_a":{"choice":"different"},"reply_b":{"choice":"same_words"}}),
            false
        )
        .unwrap(),
        (0, 3)
    );
}

#[test]
fn ghost_exposures_are_idempotent_and_follow_source_removal() {
    let (_tmp, s) = notebook();
    let sheet = s.get("current").unwrap();
    let row = |guess: &str| json!({"id":store::id(),"guess":guess,"context":"We can call","sources":["notes"],"sourceRevision":null});
    let candidate = row("it a day.");
    let shown =
        |c: &Value| app::action(&s, "ghostShown", &json!({"sheet":sheet,"candidate":c})).unwrap();
    shown(&candidate);
    shown(&candidate);
    let event = |status: &str, actual: &str| {
        app::action(
            &s,
            "ghostEvent",
            &json!({"id":candidate["id"],"status":status,"actual":actual}),
        )
        .unwrap()
    };
    event("edited", "token=supersecretvalue");
    assert_eq!(
        s.doc("prediction", candidate["id"].as_str().unwrap())
            .unwrap()["status"],
        "shown"
    );
    event("edited", "it a rehearsal.");
    event("accepted", "");
    shown(&candidate);
    let records = s.list("prediction").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["actual"], "it a rehearsal.");
    assert_eq!(records[0]["status"], "edited");
    let count: i64 =
        s.db.lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM moments WHERE source='desk' AND action='it a rehearsal.'",
                [],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(count, 1);
    let stale = row("it quits.");
    s.put("source", &json!({"id":"notes"})).unwrap();
    crate::forget::source(&s, "notes").unwrap();
    shown(&stale);
    assert!(s.list("prediction").unwrap().is_empty());
}

#[test]
fn ambiguous_and_partial_word_matches_leave_the_sheet_quiet() {
    let (_tmp, s) = notebook();
    for (i, word) in ["tea", "coffee", "tea", "coffee"].iter().enumerate() {
        s.db.lock()
            .unwrap()
            .execute(
                "INSERT INTO moments VALUES(?,?,?, ?,?,'train')",
                rusqlite::params![
                    i.to_string(),
                    "notes",
                    "2026-10-19",
                    "break",
                    format!("Snacks in the afternoon include {word}.")
                ],
            )
            .unwrap();
    }
    assert!(ghost::suggest(&s, "In the afternoon include")
        .unwrap()
        .is_null());
    assert!(ghost::suggest(&s, "Look acks in the").unwrap().is_null());
    assert!(
        ghost::suggest(&s, "api_key=longcredential in the afternoon")
            .unwrap()
            .is_null()
    );
    s.db.lock()
        .unwrap()
        .execute(
            "UPDATE moments SET action='İstanbul is where we stopped.'",
            [],
        )
        .unwrap();
    let passage = "old words ".repeat(500) + "İSTANBUL IS WHERE";
    assert_eq!(
        ghost::suggest(&s, &passage).unwrap()["guess"],
        "we stopped."
    );
}

#[test]
fn morning_visit_and_shortcut_preserve_drafts_and_do_not_duplicate_blank_sheets() {
    let (_tmp, s) = notebook();
    let first = s.get("current").unwrap();
    let enter = |date: &str, shortcut: bool| {
        app::action(&s, "enterDesk", &json!({"day":date,"shortcut":shortcut})).unwrap()
    };
    // Existing notebooks have no deskDay metadata. Keep their current sheet on migration.
    s.save_sheet(first.as_str().unwrap(), "A draft before upgrading.")
        .unwrap();
    assert_eq!(enter("2026-10-31", false)["newDay"], false);
    assert_eq!(s.get("current").unwrap(), first);
    // Writing through midnight must not displace the open draft on the next focus.
    app::action(
        &s,
        "save",
        &json!({"day":"2026-11-01","id":first,"text":"The line I finished after midnight."}),
    )
    .unwrap();
    assert_eq!(enter("2026-11-01", false)["newDay"], false);
    assert_eq!(s.get("current").unwrap(), first);
    std::thread::scope(|scope| {
        for shortcut in [true, false, false, true] {
            scope.spawn(move || enter("2026-11-02", shortcut));
        }
    });
    let blank = s.get("current").unwrap();
    assert_ne!(blank, first);
    assert!(s.sheet(blank.as_str().unwrap()).unwrap().is_empty());
    assert_eq!(s.list("sheet").unwrap().len(), 2);
    assert_eq!(
        s.sheet(first.as_str().unwrap()).unwrap(),
        "The line I finished after midnight."
    );
    app::action(&s, "openSheet", &json!({"id":first,"day":"2026-11-02"})).unwrap();
    assert_eq!(enter("2026-11-02", false)["newDay"], false);
    assert_eq!(s.get("current").unwrap(), first);
    assert_eq!(enter("2026-11-02", true)["newDay"], false);
    let sealed = s.get("current").unwrap();
    s.put(
        "sealed",
        &json!({"id":store::id(),"sheet":sealed,"guess":"A prior prediction."}),
    )
    .unwrap();
    enter("2026-11-03", false);
    assert_ne!(s.get("current").unwrap(), sealed);
}

#[test]
#[ignore = "Verifies the rule provenance of a sealed guess with the installed local model"]
fn sealed_decision_can_only_revise_a_rule_used_for_its_guess() {
    let (_tmp, s) = notebook();
    let model = models::inventory()
        .into_iter()
        .find(|m| m["name"] == "llama3.2:3b")
        .unwrap();
    s.set("model", &model).unwrap();
    s.set("ruleLimit", &json!(1)).unwrap();
    let add = |sentence: &str| {
        scroll::change(&s, "create", &json!({"section":"deciding","text":sentence})).unwrap()
    };
    let other = add("Choose blue ink for sketches.");
    let agenda = add("Review the agenda before adding meetings to the calendar.");
    let sheet = s.get("current").unwrap();
    s.save_sheet(
        sheet.as_str().unwrap(),
        "Should I check the agenda before adding meetings to my calendar?",
    )
    .unwrap();
    let sealed = app::action(&s, "sealGuess", &json!({"id":sheet})).unwrap();
    let id = sealed["id"].as_str().unwrap();
    assert_eq!(
        s.doc("sealed", id).unwrap()["ruleIds"],
        json!([agenda["id"]])
    );
    app::action(
        &s,
        "breakSeal",
        &json!({"id":id,"actual":"Accept the invitation this time."}),
    )
    .unwrap();
    app::action(&s, "verdict", &json!({"id":id,"right":false})).unwrap();
    let note = |rule: &Value| {
        app::action(
            &s,
            "surprise",
            &json!({"id":id,"ruleId":rule["id"],"text":"Make room for this workshop.","change":false}),
        )
    };
    assert!(note(&other).is_err());
    note(&agenda).unwrap();
    let rows = scroll::approved(&s).unwrap();
    assert_eq!(
        rows.iter().find(|r| r["id"] == other["id"]).unwrap()["text"],
        other["text"]
    );
    assert!(
        rows.iter().find(|r| r["id"] == agenda["id"]).unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("Make room for this workshop.")
    );
}

#[test]
fn shared_agent_configuration_survives_connect_repair_and_removal() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let (tmp, s) = notebook();
    let exe = Path::new("/Applications/Sumi.app/Contents/MacOS/sumi");
    let relocated = tmp.path().join("Moved/Sumi.app/Contents/MacOS/sumi");
    for name in seal::AGENTS {
        let path = seal::config_path(tmp.path(), name).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let target = tmp.path().join(format!("{}-dotfile", name));
        let initial = if name == "Codex" {
            "# The owner's comment\nmodel = 'local'\nmcp_servers = { sumi = { command = 'personal' } }\n"
        } else {
            r#"{"mcpServers":{"sumi":{"command":"personal"}},"keep":"original"}"#
        };
        fs::write(&target, initial).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        seal::connect(&s, name, tmp.path(), exe).unwrap();
        let a = s.list("agent").unwrap().remove(0);
        assert_eq!(a["link"]["slot"], "sumi-2");
        let key = a["id"].as_str().unwrap();
        let secret = Path::new(a["link"]["server"]["args"][4].as_str().unwrap());
        let token = fs::read_to_string(secret).unwrap();
        assert!(seal::authorized(&s, &token));
        let tools = |permission: &str| {
            let mut settings = a.clone();
            settings["permission"] = json!(permission);
            seal::policy(&s, &settings).unwrap();
            let rpc = seal::rpc(&s, &token, &json!({"id":1,"method":"tools/list"})).unwrap();
            rpc["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["name"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(tools("Read"), vec!["sumi_context", "sumi_observe"]);
        assert_eq!(
            tools("Ask"),
            vec!["sumi_context", "sumi_predict", "sumi_observe"]
        );
        assert_eq!(tools("Answer for me").len(), 4);
        let untouched = fs::read(&target).unwrap();
        seal::connect(&s, name, tmp.path(), exe).unwrap();
        assert_eq!(fs::read(&target).unwrap(), untouched);
        assert_eq!(s.doc("agent", key).unwrap()["permission"], "Answer for me");
        seal::connect(&s, name, tmp.path(), &relocated).unwrap();
        assert!(seal::authorized(&s, &token));
        assert!(fs::read_to_string(&target).unwrap().contains("Moved/Sumi"));
        assert!(fs::symlink_metadata(&path).unwrap().is_symlink());
        let pending = seal::call(
            &s,
            &token,
            "sumi_answer",
            &json!({"question":"Which sketch do you want?"}),
        )
        .unwrap();
        seal::disconnect(&s, key).unwrap();
        assert!(!secret.exists());
        assert!(!seal::authorized(&s, &token));
        assert_eq!(
            s.doc("log", pending["id"].as_str().unwrap()).unwrap()["status"],
            "closed"
        );
        let after = fs::read_to_string(&target).unwrap();
        assert!(!after.contains("sumi-2"));
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        if name == "Codex" {
            assert!(after.contains("# The owner's comment"));
            let doc: toml_edit::DocumentMut = after.parse().unwrap();
            assert_eq!(
                doc["mcp_servers"]["sumi"]["command"].as_str(),
                Some("personal")
            );
        } else {
            assert_eq!(
                serde_json::from_str::<Value>(&after).unwrap(),
                serde_json::from_str::<Value>(initial).unwrap()
            );
        }
        seal::disconnect(&s, key).unwrap();
    }
}

#[test]
fn unfinished_agent_removal_stays_revoked_and_is_retried_on_reopen() {
    use std::fs;
    let (tmp, s) = notebook();
    let path = tmp.path().join(".claude.json");
    let exe = Path::new("/Applications/Sumi.app/Contents/MacOS/sumi");
    seal::connect(&s, "Claude Code", tmp.path(), exe).unwrap();
    let a = s.list("agent").unwrap().remove(0);
    let key = a["id"].as_str().unwrap();
    let secret = Path::new(a["link"]["server"]["args"][4].as_str().unwrap());
    let token = fs::read_to_string(secret).unwrap();
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, "unfinished json").unwrap();
    assert!(seal::disconnect(&s, key).is_err());
    assert!(!seal::authorized(&s, &token));
    assert!(!secret.exists());
    assert_eq!(s.doc("agent", key).unwrap()["linkState"], "revoking");
    assert_eq!(fs::read_to_string(&path).unwrap(), "unfinished json");
    fs::write(&path, &original).unwrap();
    app::setup(&s).unwrap();
    assert!(s.list("agent").unwrap().is_empty());
    assert!(!fs::read_to_string(&path).unwrap().contains("--mcp"));
    // A fresh connection never makes the credential path from the old registration live again.
    seal::connect(&s, "Claude Code", tmp.path(), exe).unwrap();
    let b = s.list("agent").unwrap().remove(0);
    assert_ne!(
        a["link"]["server"]["args"][4],
        b["link"]["server"]["args"][4]
    );
    assert!(!secret.exists());
    assert_eq!(b["permission"], "Read");
    let mut edited: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    edited["mcpServers"]["sumi"]["env"] = json!({"theme":"blue"});
    fs::write(&path, edited.to_string()).unwrap();
    seal::disconnect(&s, key).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), edited.to_string());
}

#[test]
fn interrupted_install_and_legacy_agent_keep_their_permissions() {
    use std::fs;
    let (tmp, s) = notebook();
    let exe = Path::new("/Applications/Sumi.app/Contents/MacOS/sumi");
    let key = &store::hash("Codex")[..32];
    let path = tmp.path().join(".codex/config.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let secret = s.root.join(".sumi").join(format!("agent-{key}.token"));
    let token = store::id();
    fs::write(&secret, &token).unwrap();
    let mut legacy = json!({"id":key,"name":"Codex","permission":"Ask","sections":["words"],"threshold":0.98,"tokenHash":seal::token_hash(&token)});
    s.put("agent", &legacy).unwrap();
    let args = [
        "--mcp",
        "--notebook",
        s.root.to_str().unwrap(),
        "--credential",
        secret.to_str().unwrap(),
    ];
    fs::write(
        &path,
        format!(
            "[mcp_servers.sumi]\ncommand = '{}'\nargs = {}\n",
            exe.display(),
            json!(args)
        ),
    )
    .unwrap();
    seal::connect(&s, "Codex", tmp.path(), exe).unwrap();
    legacy = s.doc("agent", key).unwrap();
    assert_eq!(legacy["permission"], "Ask");
    assert_eq!(legacy["sections"], json!(["words"]));
    assert_eq!(legacy["link"]["slot"], "sumi");
    assert!(seal::authorized(&s, &token));
    legacy["linkState"] = json!("installing");
    s.put("agent", &legacy).unwrap();
    fs::write(
        &path,
        "# Edited while Sumi was closed\n[mcp_servers.calendar]\ncommand='my-calendar'\n",
    )
    .unwrap();
    assert!(!seal::authorized(&s, &token));
    app::setup(&s).unwrap();
    assert!(seal::authorized(&s, &token));
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("# Edited while Sumi was closed")
            && text.contains("my-calendar")
            && text.contains("mcp_servers.sumi")
    );
    let state = app::state(&s).unwrap();
    assert!(state["agents"][0]["link"].is_null() && state["agents"][0]["tokenHash"].is_null());
}

#[test]
fn two_notebooks_can_share_one_agent_configuration() {
    let (home, left) = notebook();
    let (_other, right) = notebook();
    let exe = Path::new("/Applications/Sumi.app/Contents/MacOS/sumi");
    let config_home = home.path();
    std::thread::scope(|scope| {
        for s in [&left, &right] {
            scope.spawn(move || seal::connect(s, "Cursor", config_home, exe).unwrap());
        }
    });
    let path = seal::config_path(config_home, "Cursor").unwrap();
    let read_config = || serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(read_config()["mcpServers"].as_object().unwrap().len(), 2);
    let left_agent = left.list("agent").unwrap().remove(0);
    let right_agent = right.list("agent").unwrap().remove(0);
    assert_ne!(left_agent["link"]["slot"], right_agent["link"]["slot"]);
    seal::disconnect(&left, left_agent["id"].as_str().unwrap()).unwrap();
    let config = read_config();
    let remaining = config["mcpServers"].as_object().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[right_agent["link"]["slot"].as_str().unwrap()]["args"][2],
        right.root.to_string_lossy().as_ref()
    );
}

#[test]
fn mirror_uses_distinct_recent_days_and_the_exact_belief_revision() {
    let (_tmp, s) = notebook();
    let rule = scroll::change(
        &s,
        "create",
        &json!({"section":"deciding","text":"I decline recurring invitations."}),
    )
    .unwrap();
    let id = rule["id"].as_str().unwrap();
    let at = chrono::Utc::now();
    let revisions = mirror::rule_revisions(&[rule.clone()]);
    let example = |i: usize, days: i64| json!({"id":i.to_string(),"observedAt":at-chrono::Duration::days(days),"predictedAt":at,"date":at,"scope":"incoming","category":"words","grade":0,"question":"Would you join?","guess":"I decline.","actual":"I will join.","ruleIds":[id],"ruleRevisions":revisions,"ruleCheck":{"id":id,"revision":revisions[id]}});
    let grouped = |scores: &[Value]| {
        mirror::reviews(scores, &[rule.clone()], at)
            .into_iter()
            .any(|c| c["ruleId"] == id)
    };
    assert!(!grouped(&[example(1, 15), example(2, 15), example(3, 1)]));
    assert!(!grouped(&[example(1, 12), example(2, 6), example(3, 1)]));
    assert!(!grouped(&[example(1, 40), example(2, 30), example(3, 20)]));
    let cases = [example(1, 21), example(2, 10), example(3, 1)];
    assert!(grouped(&cases));
    for (field, value) in [
        ("scope", json!("held-out")),
        ("reflection", json!("Already addressed.")),
        ("grade", json!(3)),
        ("ruleCheck", Value::Null),
        ("ruleRevisions", json!({id:"older"})),
    ] {
        let mut changed = cases.clone();
        changed[1][field] = value;
        assert!(!grouped(&changed), "{field}");
    }
    let mut held_out = example(9, 1);
    held_out["scope"] = json!("held-out");
    assert!(mirror::reviews(&[held_out], &[rule.clone()], at).is_empty());
    for v in &cases {
        s.put("prediction", v).unwrap();
    }
    let card = app::state(&s).unwrap()["surprises"][0].clone();
    let note = json!({"id":card["id"],"ruleId":id,"text":"I accept recurring meetings with the studio.","change":true});
    app::action(&s, "surprise", &note).unwrap();
    app::action(&s, "surprise", &note).unwrap();
    assert_eq!(s.list("version").unwrap().len(), 2);
    assert!(app::state(&s).unwrap()["surprises"]
        .as_array()
        .unwrap()
        .is_empty());
    for old in &cases {
        let reviewed = s.doc("prediction", old["id"].as_str().unwrap()).unwrap();
        assert_eq!(reviewed["grade"], old["grade"]);
        assert_eq!(reviewed["reflectionRule"], id);
        assert_eq!(reviewed["reflection"], note["text"]);
    }
    let mut stale = example(4, 1);
    stale["ruleCheck"] = Value::Null;
    s.put("prediction", &stale).unwrap();
    assert_eq!(
        app::state(&s).unwrap()["surprises"][0]["ruleIds"],
        json!([])
    );
    assert!(app::action(
        &s,
        "surprise",
        &json!({"id":"4","ruleId":id,"change":false,"text":"An exception."})
    )
    .is_err());
}
#[test]
fn interrupted_mirror_review_finishes_once_and_preserves_external_edits() {
    let (_tmp, s) = notebook();
    let rule = scroll::change(
        &s,
        "create",
        &json!({"section":"writing","text":"Use one illustration."}),
    )
    .unwrap();
    let id = &rule["id"];
    let prediction =
        json!({"id":"decision","guess":"One.","actual":"Two.","grade":0,"scope":"live"});
    s.put("prediction", &prediction).unwrap();
    let corrected = "Use two illustrations.";
    let plan = json!({"members":["decision"],"edit":{"id":id,"before":rule["text"],"after":corrected},"note":corrected,"selected":id,"key":"saved-review"});
    s.set("mirrorReviewPending", &plan).unwrap();
    // The interrupted process got the Markdown and snapshot to disk before
    // writing the reviewed prediction. Startup must avoid a duplicate version.
    scroll::change(&s, "edit", &json!({"id":id,"text":corrected})).unwrap();
    app::setup(&s).unwrap();
    assert_eq!(
        s.doc("prediction", "decision").unwrap()["reflection"],
        corrected
    );
    assert_eq!(s.list("version").unwrap().len(), 2);
    assert!(s.get("mirrorReviewPending").unwrap().is_null());
    s.put("prediction", &prediction).unwrap();
    s.set("mirrorReviewPending", &plan).unwrap();
    scroll::change(
        &s,
        "edit",
        &json!({"id":id,"text":"I choose illustrations case by case."}),
    )
    .unwrap();
    assert!(mirror::recover(&s).is_err());
    assert_eq!(
        scroll::approved(&s).unwrap()[0]["text"],
        "I choose illustrations case by case."
    );
    assert!(s.doc("prediction", "decision").unwrap()["reflection"].is_null());
}
#[test]
#[ignore = "Checks attribution of observed contradictions with the installed local model"]
fn local_model_distinguishes_a_contradiction_from_an_unrelated_miss() {
    let (_tmp, s) = notebook();
    let m = models::inventory()
        .into_iter()
        .find(|m| m["name"] == "llama3.2:3b")
        .unwrap();
    let r = scroll::change(
        &s,
        "create",
        &json!({"section":"deciding","text":"I always decline invitations to recurring meetings."}),
    )
    .unwrap();
    let id = r["id"].as_str().unwrap();
    let revisions = mirror::rule_revisions(std::slice::from_ref(&r));
    for (i, (question, guess, actual)) in [
        (
            "Will you attend the weekly planning meeting?",
            "I will decline the recurring invitation.",
            "I accept. I will attend every week.",
        ),
        (
            "How many copies should I print?",
            "Print six copies.",
            "Print ten copies.",
        ),
        (
            "When will you decline the recurring planning invitation?",
            "I will decline the recurring meeting on Monday.",
            "I will decline the recurring meeting on Friday.",
        ),
    ]
    .iter()
    .enumerate()
    {
        s.put("prediction",&json!({"id":i.to_string(),"scope":"live","category":"decisions","date":chrono::Utc::now(),"ruleIds":[id],"ruleRevisions":revisions,"question":question,"guess":guess,"actual":actual,"grade":0})).unwrap();
    }
    assert!(mirror::inspect_while(&s, &m, || false).unwrap());
    assert!(s.doc("prediction", "0").unwrap()["ruleCheck"].is_null());
    assert!(!mirror::inspect_while(&s, &m, || true).unwrap());
    assert_eq!(s.doc("prediction", "0").unwrap()["ruleCheck"]["id"], id);
    assert!(s.doc("prediction", "1").unwrap()["ruleCheck"]["id"].is_null());
    assert!(s.doc("prediction", "2").unwrap()["ruleCheck"]["id"].is_null());
    // Covered items should take no model calls when the scheduler returns.
    assert!(!mirror::inspect_while(&s, &Value::Null, || true).unwrap());
}
