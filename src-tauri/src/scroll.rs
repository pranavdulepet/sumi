use crate::store::*;
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};
pub const SECTIONS: [&str; 6] = ["deciding", "writing", "people", "words", "goals", "facts"];
pub(crate) fn git(root: &Path, args: &[&str]) -> Res<String> {
    let out = Command::new("/usr/bin/git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Sumi",
            "-c",
            "user.email=local@sumi.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .msg()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into())
    }
}
pub fn init(s: &Store) -> Res<()> {
    if !s.root.join(".git").exists() {
        git(&s.root, &["init", "-q"])?;
        atomic(
            &s.root.join(".gitignore"),
            b"*\n!scroll/\n!scroll/**\n!.gitignore\n",
        )?
    }
    for sec in SECTIONS {
        let p = section_path(&s.root, sec);
        if !p.exists() {
            atomic(&p, format!("# {}\n", label(sec)).as_bytes())?
        }
    }
    Ok(())
}
pub fn label(sec: &str) -> &str {
    match sec {
        "writing" => "How you write",
        "deciding" => "How you decide",
        "words" => "Your words",
        "people" => "People",
        "goals" => "What you are working toward",
        _ => "Facts",
    }
}
pub(crate) fn section_path(root: &Path, sec: &str) -> std::path::PathBuf {
    if sec == "people" {
        root.join("scroll/people/relationships.md")
    } else {
        root.join("scroll").join(format!("{sec}.md"))
    }
}
/// Source adapters emit both RFC 3339 and SQLite's UTC timestamps.
pub(crate) fn instant(stamp: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(stamp)
        .ok()
        .map(|t| t.to_utc())
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S%.f")
                .ok()
                .map(|t| t.and_utc())
        })
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(stamp, "%Y-%m-%d")
                .ok()?
                .and_hms_opt(0, 0, 0)
                .map(|t| t.and_utc())
        })
}
pub(crate) fn version_before<'a>(versions: &'a [Value], stamp: &str) -> Option<&'a Value> {
    let observed = instant(stamp)?;
    versions
        .iter()
        .filter_map(|v| Some((instant(v["date"].as_str()?)?, v)))
        .filter(|(date, _)| *date < observed)
        .max_by_key(|(date, _)| *date)
        .map(|(_, v)| v)
}
pub fn approved(s: &Store) -> Res<Vec<Value>> {
    approved_at(&s.root)
}
pub(crate) fn approved_at(root: &Path) -> Res<Vec<Value>> {
    let mut all = vec![];
    for sec in SECTIONS {
        let text = fs::read_to_string(section_path(root, sec)).msg()?;
        for block in text.split("\n## ").skip(1) {
            let Some((title, body)) = block.split_once('\n') else {
                continue;
            };
            let Some((sentence, meta)) = body.rsplit_once("<!-- sumi ") else {
                continue;
            };
            let Some(meta) = meta.trim().strip_suffix("-->") else {
                continue;
            };
            if let Ok(mut record) = serde_json::from_str::<Value>(meta.trim()) {
                if record["id"].as_str().is_none() {
                    continue;
                }
                record["text"] = json!(sentence.trim());
                record["title"] = json!(title);
                record["section"] = json!(sec);
                all.push(record)
            }
        }
    }
    Ok(all)
}
fn snippet(v: &Value) -> String {
    let mut metadata = v.clone();
    if let Some(items) = metadata["evidence"].as_array() {
        metadata["evidence"] = json!(items
            .iter()
            .map(|e| if e.is_object() {
                e["id"].clone()
            } else {
                e.clone()
            })
            .collect::<Vec<_>>());
    }
    let sentence = metadata
        .as_object_mut()
        .and_then(|r| r.remove("text"))
        .unwrap_or(Value::Null);
    let text = sentence.as_str().unwrap_or("");
    let title = v["title"].as_str().map(str::to_owned).unwrap_or_else(|| {
        text.split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join(" ")
    });
    format!("\n## {title}\n{text}\n<!-- sumi {metadata} -->\n")
}
fn rewrite(s: &Store, rows: &[Value]) -> Res<()> {
    write_files(&s.root, rows)
}
pub(crate) fn write_files(root: &Path, rows: &[Value]) -> Res<()> {
    use std::collections::{HashMap, HashSet};
    for sec in SECTIONS {
        let path = if sec == "people" {
            root.join("scroll/people/relationships.md")
        } else {
            root.join("scroll").join(format!("{sec}.md"))
        };
        let old = fs::read_to_string(&path).unwrap_or_else(|_| format!("# {}\n", label(sec)));
        let wanted: HashMap<_, _> = rows
            .iter()
            .filter(|v| v["section"] == sec)
            .filter_map(|v| Some((v["id"].as_str()?, v)))
            .collect();
        let mut written = HashSet::new();
        let mut blocks = old.split("\n## ");
        let mut out = blocks.next().unwrap_or("").to_owned();
        for block in blocks {
            let meta = block
                .rsplit_once("<!-- sumi ")
                .and_then(|(_, tail)| tail.trim().strip_suffix("-->"))
                .and_then(|raw| serde_json::from_str::<Value>(raw.trim()).ok());
            if let Some(id) = meta.as_ref().and_then(|m| m["id"].as_str()) {
                if let Some(v) = wanted.get(id) {
                    out.push_str(&snippet(v));
                    written.insert(id.to_owned());
                }
            } else {
                out.push_str("\n## ");
                out.push_str(block);
            }
        }
        for v in rows.iter().filter(|v| v["section"] == sec) {
            if let Some(id) = v["id"].as_str() {
                if !written.contains(id) {
                    out.push_str(&snippet(v));
                }
            }
        }
        if old != out || !path.exists() {
            atomic(&path, out.as_bytes())?;
        }
    }
    Ok(())
}
#[derive(Default)]
pub struct DiskEdits {
    files: Option<Vec<(u64, i64, i64)>>,
}
impl DiskEdits {
    pub fn check(&mut self, s: &Store) -> Res<bool> {
        use std::os::unix::fs::MetadataExt;
        let mut files = Vec::with_capacity(SECTIONS.len());
        for section in SECTIONS {
            let meta = fs::metadata(section_path(&s.root, section)).msg()?;
            files.push((meta.ino(), meta.ctime(), meta.ctime_nsec()));
        }
        if self.files.as_ref() == Some(&files) {
            return Ok(false);
        }
        snapshot(s, "Read edits from disk")?;
        // Cache the state from before the read. An editor can save again during it.
        self.files = Some(files);
        Ok(true)
    }
}
pub fn snapshot(s: &Store, reason: &str) -> Res<()> {
    let mut rows = approved(s)?;
    let digest_before = hash(&serde_json::to_string(&rows).msg()?);
    if s.get("scrollDigest")? == digest_before {
        return Ok(());
    }
    let mut digest = digest_before;
    if reason == "Read edits from disk" {
        if let Some(last) = s.latest("version")? {
            let mut changed = false;
            for row in &mut rows {
                if last["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|old| old["id"] == row["id"] && old["text"] != row["text"])
                {
                    row["authored"] = json!(true);
                    row["confidence"] = json!(1.0);
                    row["importance"] = json!(1.0);
                    row["updated"] = json!(now());
                    changed = true;
                }
            }
            if changed {
                rewrite(s, &rows)?;
                digest = hash(&serde_json::to_string(&rows).msg()?);
            }
        }
    }
    git(&s.root, &["add", "--", "scroll"])?;
    if !git(&s.root, &["diff", "--cached", "--name-only"])?
        .trim()
        .is_empty()
    {
        git(&s.root, &["commit", "-qm", reason, "--", "scroll"])?;
    }
    s.put(
        "version",
        &json!({"id":id(),"date":now(),"reason":reason,"entries":rows}),
    )?;
    s.set("scrollDigest", &json!(digest))
}
pub fn hydrate(s: &Store, mut rows: Vec<Value>) -> Res<Vec<Value>> {
    let c = s.db.lock().unwrap();
    for row in &mut rows {
        row["currentConfidence"] = json!(strength(row));
        let mut evidence = vec![];
        for id in row["evidence"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            use rusqlite::OptionalExtension;
            let found=c.query_row("SELECT stamp,action,source FROM moments WHERE id=?", [id],|r|Ok(json!({"id":id,"date":r.get::<_,String>(0)?,"quote":r.get::<_,String>(1)?,"source":r.get::<_,String>(2)?}))).optional().msg()?;
            if let Some(v) = found {
                evidence.push(v)
            }
        }
        row["evidence"] = json!(evidence)
    }
    Ok(rows)
}
pub fn change(s: &Store, action: &str, a: &Value) -> Res<Value> {
    let _guard = s.edit.lock().unwrap();
    change_locked(s, action, a)
}
pub(crate) fn change_locked(s: &Store, action: &str, a: &Value) -> Res<Value> {
    let mut rows = approved(s)?;
    let k = a["id"].as_str().unwrap_or("");
    let mut record = match action {
        "stamp" => s.doc("proposal", k)?,
        "edit" | "delete" => rows
            .iter()
            .find(|v| v["id"] == k)
            .cloned()
            .ok_or("Entry no longer exists.")?,
        _ => {
            json!({"id":if action == "createFromReview" { k.to_owned() } else { id() },"section":a["section"],"confidence":1.0,"evidence":[],"sources":[],"days":0})
        }
    };
    if action != "delete" {
        if action != "stamp" {
            let t = a["text"].as_str().unwrap_or("").trim();
            if t.is_empty() || t.len() > 1600 || t.contains("<!--") || t.contains("\n#") {
                return Err("Use one or two plain sentences.".into());
            }
            record["text"] = json!(t);
            record["confidence"] = json!(1.);
            record["authored"] = json!(true);
            record["importance"] = json!(1.);
            record["tier"] = json!("trait");
            record["lastSeen"] = json!(now())
        }
        if !SECTIONS.contains(&record["section"].as_str().unwrap_or("")) {
            return Err("Choose a Scroll section.".into());
        }
        record["updated"] = json!(now());
        record["status"] = json!("stamped");
    }
    rows.retain(|v| v["id"] != record["id"]);
    if action != "delete" {
        rows.push(record.clone())
    }
    rewrite(s, &rows)?;
    snapshot(
        s,
        match action {
            "stamp" => "Stamp a proposed entry",
            "edit" => "Correct a Scroll entry",
            "delete" => "Remove a Scroll entry",
            _ => "Write a Scroll entry",
        },
    )?;
    if action == "stamp" {
        s.del("proposal", k)?
    }
    Ok(record)
}
pub fn strength(row: &Value) -> f64 {
    strength_at(row, chrono::Utc::now())
}
pub(crate) fn strength_at(row: &Value, at: chrono::DateTime<chrono::Utc>) -> f64 {
    let confidence = row["confidence"].as_f64().unwrap_or(0.5);
    if row["authored"] == true {
        return confidence;
    }
    let age = row["lastSeen"]
        .as_str()
        .or(row["updated"].as_str())
        .and_then(instant)
        .map(|t| (at - t).num_days().max(0) as f64)
        .unwrap_or(0.);
    let days = if row["tier"] == "phase" { 28. } else { 365. };
    confidence * 0.5f64.powf(age / days)
}
