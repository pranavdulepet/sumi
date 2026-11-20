use crate::{scroll, store::*};
use rusqlite::params;
use serde_json::{json, Value};
use std::{collections::HashSet, fs};

fn depends(entry: &Value, source: &str) -> bool {
    entry["source"] == source
        || entry["sources"]
            .as_array()
            .is_some_and(|v| v.iter().any(|x| x == source))
}
fn keep(entries: &[Value], source: &str) -> Vec<Value> {
    entries
        .iter()
        .filter_map(|e| {
            if !depends(e, source) {
                return Some(e.clone());
            }
            // An explicit correction belongs to its author independently of the deleted inference.
            if e["authored"] == true {
                let mut owned = e.clone();
                owned["evidence"] = json!([]);
                owned["sources"] = json!([]);
                owned["days"] = json!(0);
                return Some(owned);
            }
            None
        })
        .collect()
}
fn contains_id(v: &Value, ids: &HashSet<String>) -> bool {
    match v {
        Value::String(s) => ids.contains(s),
        Value::Array(a) => a.iter().any(|x| contains_id(x, ids)),
        Value::Object(m) => m.values().any(|x| contains_id(x, ids)),
        _ => false,
    }
}
pub fn recover(s: &Store) -> Res<()> {
    if let Some(src) = s.get("forgetPending")?.as_str() {
        source(s, src)?
    }
    Ok(())
}
pub fn source(s: &Store, source: &str) -> Res<()> {
    let _guard = s.lifecycle.write().unwrap();
    let _edit = s.edit.lock().unwrap();
    let state = s.root.join(".sumi");
    let backup = state.join("forget-old-git");
    let staging = state.join("forget-replacement");
    if !state.join("owned-history").is_file() {
        return Err("This notebook uses an external Git repository. Remove its archived evidence separately.".into());
    }
    let pending = s.get("forgetPending")?;
    if pending.is_null() {
        s.doc("source", source)?;
        let refs = scroll::git(&s.root, &["for-each-ref", "--format=%(refname)"])?;
        let tracked = scroll::git(&s.root, &["ls-files"])?;
        if refs.lines().count() > 1
            || tracked
                .lines()
                .any(|p| p != ".gitignore" && !p.starts_with("scroll/"))
            || !scroll::git(&s.root, &["remote"])?.trim().is_empty()
        {
            return Err("This notebook has additional Git history. Move your additional branches or remotes before purging the source.".into());
        }
        s.set("forgetPending", &json!(source))?;
    } else if pending != source {
        return Err("Another source removal is in progress.".into());
    }
    if backup.exists() && !s.root.join(".git").exists() {
        fs::rename(&backup, s.root.join(".git")).msg()?;
    }
    s.set("sourceRevision", &json!(id()))?;
    let all = scroll::approved(s)?;
    let mut versions = s.list("version")?;
    versions.reverse();
    let mut affected: HashSet<String> = HashSet::new();
    for e in all
        .iter()
        .chain(
            versions
                .iter()
                .flat_map(|v| v["entries"].as_array().into_iter().flatten()),
        )
        .chain(s.list("proposal")?.iter())
    {
        if depends(e, source) {
            if let Some(id) = e["id"].as_str() {
                affected.insert(id.to_owned());
            }
        }
    }
    let current = keep(&all, source);
    if staging.exists() {
        fs::remove_dir_all(&staging).msg()?
    }
    fs::create_dir_all(staging.join("scroll/people")).msg()?;
    scroll::git(&staging, &["init", "-q"])?;
    // Rebuild each historical tree from its actual Markdown. A user may have
    // inserted their own headings, comments or paragraphs between Sumi entries.
    let known =
        scroll::SECTIONS.map(|section| scroll::section_path(std::path::Path::new(""), section));
    let commits = if scroll::git(&s.root, &["rev-parse", "--verify", "HEAD"]).is_ok() {
        scroll::git(&s.root, &["rev-list", "--reverse", "HEAD"])?
    } else {
        String::new()
    };
    for sha in commits.lines() {
        let tracked = scroll::git(&s.root, &["ls-tree", "-r", "--name-only", sha])?;
        if tracked.lines().any(|name| {
            name != ".gitignore" && !known.iter().any(|p| p == std::path::Path::new(name))
        }) || scroll::git(&s.root, &["show", "-s", "--format=%ae", sha])?.trim()
            != "local@sumi.invalid"
        {
            s.set("forgetPending", &Value::Null)?;
            return Err("This notebook includes separately authored Git history. Its owner must review that history before it is rewritten.".into());
        }
        for path in &known {
            let name = path.to_str().ok_or("Unknown Scroll path.")?;
            let text = if tracked.lines().any(|p| p == name) {
                scroll::git(&s.root, &["show", &format!("{sha}:{name}")])?
            } else {
                String::new()
            };
            atomic(&staging.join(path), text.as_bytes())?;
        }
        let rows = keep(&scroll::approved_at(&staging)?, source);
        scroll::write_files(&staging, &rows)?;
        let message = scroll::git(&s.root, &["show", "-s", "--format=%s", sha])?;
        archive(&staging, message.trim())?;
    }
    for v in &mut versions {
        v["entries"] = json!(keep(
            v["entries"].as_array().map(Vec::as_slice).unwrap_or(&[]),
            source
        ));
    }
    for path in &known {
        atomic(&staging.join(path), &fs::read(s.root.join(path)).msg()?)?;
    }
    scroll::write_files(&staging, &current)?;
    archive(&staging, "Retain the remaining Scroll")?;
    // Persistent journal makes filesystem replacement and the SQL transaction restartable.
    scroll::write_files(&s.root, &current)?;
    if backup.exists() {
        fs::remove_dir_all(&backup).msg()?
    }
    fs::rename(s.root.join(".git"), &backup).msg()?;
    fs::rename(staging.join(".git"), s.root.join(".git")).msg()?;
    let mut remove = vec![];
    for kind in [
        "proposal",
        "blind",
        "prediction",
        "sealed",
        "log",
        "surprise",
    ] {
        for v in s.list(kind)? {
            if depends(&v, source) || contains_id(&v, &affected) {
                remove.push((kind, v["id"].as_str().unwrap_or("").to_string()));
            }
        }
    }
    {
        let mut c = s.db.lock().unwrap();
        let tx = c.transaction().msg()?;
        tx.execute(
            "DELETE FROM vectors WHERE moment_id IN (SELECT id FROM moments WHERE source=?)",
            [source],
        )
        .msg()?;
        tx.execute("DELETE FROM imported_messages WHERE source=?", [source])
            .msg()?;
        tx.execute("DELETE FROM moments WHERE source=?", [source])
            .msg()?;
        for (k, id) in remove {
            tx.execute("DELETE FROM docs WHERE kind=? AND id=?", params![k, id])
                .msg()?;
        }
        tx.execute("DELETE FROM kv WHERE k='mirrorReviewPending' AND EXISTS (SELECT 1 FROM json_each(kv.v,'$.members') AS m WHERE NOT EXISTS (SELECT 1 FROM docs WHERE kind='prediction' AND id=m.value))", []).msg()?;
        for v in versions {
            tx.execute(
                "UPDATE docs SET v=? WHERE kind='version' AND id=?",
                params![v.to_string(), v["id"].as_str()],
            )
            .msg()?;
        }
        tx.execute("DELETE FROM docs WHERE kind='source' AND id=?", [source])
            .msg()?;
        tx.execute("DELETE FROM docs WHERE kind='tune'", []).msg()?;
        tx.execute("DELETE FROM kv WHERE k LIKE ? OR k LIKE 'reflected/%' OR k IN ('scrollDigest','readingProgress','reading','tuning','lastTune','prompt','ruleLimit','ghostRepeat')",[format!("cursor/{source}/%")]).msg()?;
        tx.commit().msg()?;
    }
    fs::remove_dir_all(&backup).msg()?;
    fs::remove_dir_all(&staging).msg()?;
    scroll::snapshot(s, "Forget a source")?;
    s.db.lock()
        .unwrap()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
        .msg()?;
    s.set("forgetPending", &Value::Null)
}
fn archive(root: &std::path::Path, message: &str) -> Res<()> {
    scroll::git(root, &["add", "--", "scroll"])?;
    if !scroll::git(root, &["diff", "--cached", "--name-only"])?
        .trim()
        .is_empty()
    {
        scroll::git(root, &["commit", "-qm", message, "--", "scroll"])?;
    }
    Ok(())
}
