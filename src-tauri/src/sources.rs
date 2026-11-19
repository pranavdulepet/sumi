use crate::store::*;
use regex::Regex;
use rusqlite::params;
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::LazyLock,
};
use walkdir::WalkDir;

static SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(-----BEGIN.{0,25}PRIVATE KEY|\b(?:sk-|ghp_|github_pat_|xox[bpa]-|AKIA)[A-Za-z0-9_-]{8,}|(?:password|passwd|api[_ -]?key|token|secret)\s*[:=]\s*\S+|bearer\s+[a-z0-9._-]{8,}|\b(?:\d[ -]?){13,19}\b)").unwrap()
});
static CONTACT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[\w.+-]+@[\w.-]+\.[a-z]{2,}\b|(?:/Users/|/home/)[^/\s]+|\b(?:\+?\d[ ().-]*){9,12}\b|@[\w.-]{2,}").unwrap()
});
static NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Z][a-z]{2,}(?: [A-Z][a-z]{2,})+\b").unwrap());
pub fn audit(v: &str) -> Option<String> {
    if SECRET.is_match(v) || v.trim().is_empty() {
        return None;
    }
    let redact = |c: &regex::Captures| format!("[person-{}]", &hash(&c[0])[..6]);
    let clean = CONTACT.replace_all(v, redact);
    let mut cleaned = NAME.replace_all(&clean, redact).into_owned();
    for name in crate::local::names(&cleaned) {
        if name.len() > 1 {
            cleaned = cleaned.replace(&name, &format!("[person-{}]", &hash(&name)[..6]))
        }
    }
    Some(
        cleaned
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(3500)
            .collect(),
    )
}
pub fn writing_context(text: &str) -> Option<String> {
    if SECRET.is_match(text) {
        return None;
    }
    let start = text
        .char_indices()
        .rev()
        .nth(3499)
        .map_or(0, |(idx, _)| idx);
    audit(&text[start..])
}
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("home directory")
}
pub fn locations() -> Vec<(String, PathBuf, String)> {
    vec![
        (
            "claude-code".into(),
            home().join(".claude/projects"),
            "Claude Code".into(),
        ),
        (
            "codex".into(),
            home().join(".codex/sessions"),
            "Codex".into(),
        ),
    ]
}
pub fn files(path: &Path, ext: &[&str]) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_owned()];
    }
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            !matches!(
                e.file_name().to_str(),
                Some(".git" | "node_modules" | ".sumi")
            )
        })
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_type().is_file()
                && ext.contains(&e.path().extension().and_then(|s| s.to_str()).unwrap_or(""))
        })
        .map(|e| e.into_path())
        .collect()
}
pub fn discover() -> Vec<Value> {
    locations()
        .into_iter()
        .filter(|(_, p, _)| p.is_dir())
        .map(|(id, p, label)| json!({"id":id,"label":label,"count":files(&p,&["jsonl"]).len()}))
        .collect()
}
pub fn add(
    s: &Store,
    source: &str,
    group: &str,
    stamp: &str,
    sit: &str,
    action: &str,
) -> Res<bool> {
    let (Some(q), Some(a)) = (audit(sit), audit(action)) else {
        return Ok(false);
    };
    if a.starts_with("<environment_context>")
        || a.starts_with("<codex_internal_context")
        || a.starts_with("# AGENTS.md")
        || a.starts_with("<INSTRUCTIONS>")
    {
        return Ok(false);
    }
    let key = hash(&format!("{source}\0{group}\0{q}\0{a}"));
    let bucket = u8::from_str_radix(&hash(group)[..2], 16).unwrap() as u32;
    let split = if bucket < 26 {
        "test"
    } else if bucket < 52 {
        "tune"
    } else {
        "train"
    };
    let mut c = s.db.lock().unwrap();
    let tx = c.transaction().msg()?;
    let n = tx
        .execute(
            "INSERT OR IGNORE INTO moments VALUES(?,?,?,?,?,?)",
            params![key, source, stamp, q, a, split],
        )
        .msg()?;
    if n > 0 {
        tx.execute(
            "INSERT INTO vectors(moment_id,feature) VALUES(?,?)",
            params![key, crate::local::embedding(&q)],
        )
        .msg()?;
    }
    tx.commit().msg()?;
    Ok(n > 0)
}
fn content(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_owned();
    }
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| {
            b.as_str().or_else(|| match b["type"].as_str() {
                Some("text" | "input_text" | "output_text") => b["text"].as_str(),
                _ => None,
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn boundary(file: &mut fs::File, offset: u64) -> Res<String> {
    use sha2::{Digest, Sha256};
    let width = offset.min(512);
    file.seek(SeekFrom::Start(offset - width)).msg()?;
    let mut bytes = vec![0; width as usize];
    file.read_exact(&mut bytes).msg()?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
fn session(
    s: &Store,
    source: &str,
    p: &Path,
    cursor: &str,
    max_lines: usize,
) -> Res<(usize, bool)> {
    use std::os::unix::fs::MetadataExt;
    let mut file = fs::File::open(p).msg()?;
    let meta = file.metadata().msg()?;
    let saved = s.get(cursor)?;
    let mut offset = saved["offset"].as_u64().unwrap_or(0);
    let mut previous = String::new();
    if meta.ino() == saved["inode"].as_u64().unwrap_or(0)
        && offset <= meta.len()
        && boundary(&mut file, offset)? == saved["boundary"]
    {
        previous = saved["previous"].as_str().unwrap_or("").to_owned();
    } else {
        offset = 0;
    }
    file.seek(SeekFrom::Start(offset)).msg()?;
    let mut reader = BufReader::new(file);
    let mut count = 0;
    let group = p.file_stem().unwrap_or_default().to_string_lossy();
    let mut bytes = Vec::new();
    let mut incomplete_tail = false;
    for _ in 0..max_lines {
        bytes.clear();
        let read = reader.read_until(b'\n', &mut bytes).msg()?;
        if read == 0 {
            break;
        }
        if bytes.len() > 4_000_000 {
            offset += read as u64;
            continue;
        }
        let value = serde_json::from_slice::<Value>(&bytes);
        if value.is_err() && bytes.last() != Some(&b'\n') {
            incomplete_tail = true;
            break;
        }
        offset += read as u64;
        let Ok(v) = value else {
            continue;
        };
        let m = if source == "codex" {
            if v["type"] != "response_item" || v["payload"]["type"] != "message" {
                continue;
            }
            &v["payload"]
        } else {
            &v["message"]
        };
        let t = content(&m["content"]);
        if t.is_empty() {
            continue;
        }
        match m["role"].as_str() {
            Some("assistant") => previous = audit(&t).unwrap_or_default(),
            Some("user") if !previous.is_empty() => {
                count += add(
                    s,
                    source,
                    &group,
                    v["timestamp"].as_str().unwrap_or(""),
                    &previous,
                    &t,
                )? as usize;
                previous.clear()
            }
            _ => {}
        }
    }
    let mut file = reader.into_inner();
    s.set(cursor, &json!({"offset":offset,"inode":meta.ino(),"boundary":boundary(&mut file,offset)?,"previous":previous}))?;
    Ok((count, incomplete_tail || offset >= meta.len()))
}
fn export(s: &Store, source: &str, v: &Value) -> Res<usize> {
    let mut n = 0;
    for c in v
        .as_array()
        .ok_or("Expected a conversation export array.")?
    {
        let group = c["id"].as_str().or(c["uuid"].as_str()).unwrap_or("export");
        if let Some(map) = c["mapping"].as_object() {
            for node in map.values() {
                let m = &node["message"];
                if m["author"]["role"] != "user" {
                    continue;
                }
                let parent = &map
                    .get(node["parent"].as_str().unwrap_or(""))
                    .unwrap_or(&Value::Null)["message"];
                if parent["author"]["role"] != "assistant" {
                    continue;
                }
                let ts = chrono::DateTime::from_timestamp(
                    m["create_time"].as_f64().unwrap_or(0.) as i64,
                    0,
                )
                .map(|x| x.to_rfc3339())
                .unwrap_or_default();
                n += add(
                    s,
                    source,
                    group,
                    &ts,
                    &content(&parent["content"]["parts"]),
                    &content(&m["content"]["parts"]),
                )? as usize
            }
        } else if let Some(ms) = c["chat_messages"].as_array() {
            let mut prev = String::new();
            for m in ms {
                let t = m["text"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| content(&m["content"]));
                if m["sender"] == "assistant" {
                    prev = t
                } else if m["sender"] == "human" && !prev.is_empty() {
                    n += add(
                        s,
                        source,
                        group,
                        m["created_at"].as_str().unwrap_or(""),
                        &prev,
                        &t,
                    )? as usize;
                    prev.clear()
                }
            }
        }
    }
    Ok(n)
}
#[cfg(test)]
pub fn ingest(s: &Store, source: &Value) -> Res<usize> {
    ingest_pass(s, source, false).map(|r| r.0)
}
pub fn ingest_pass(s: &Store, source: &Value, first: bool) -> Res<(usize, bool)> {
    let key = source["id"].as_str().ok_or("Missing source")?;
    let p = Path::new(source["path"].as_str().ok_or("Missing path")?);
    if !p.exists() {
        return Err("This source is no longer available.".into());
    }
    if key == "messages" {
        return messages(s, key, p, first);
    }
    let mut n = 0;
    let extensions: &[&str] = match key {
        "codex" | "claude-code" => &["jsonl"],
        _ if p.is_dir() => &["md", "txt"],
        _ => &["json", "zip"],
    };
    let mut paths = files(p, extensions);
    paths
        .sort_by_cached_key(|p| std::cmp::Reverse(fs::metadata(p).and_then(|m| m.modified()).ok()));
    let start = std::time::Instant::now();
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(90 * 86400);
    for path in paths {
        if start.elapsed().as_secs() >= if first { 12 } else { 40 } {
            return Ok((n, true));
        }
        let meta = fs::metadata(&path).msg()?;
        let fingerprint = format!(
            "{}:{}",
            meta.len(),
            meta.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let cursor = format!("cursor/{key}/{}", hash(&path.to_string_lossy()));
        if s.get(&cursor)? == fingerprint {
            continue;
        }
        if first && meta.modified().is_ok_and(|d| d < cutoff) {
            return Ok((n, true));
        }
        match path.extension().and_then(|x| x.to_str()) {
            Some("jsonl") => {
                let (added, done) = session(
                    s,
                    key,
                    &path,
                    &format!("{cursor}/position"),
                    if first { 800 } else { 10000 },
                )?;
                n += added;
                if !done {
                    return Ok((n, true));
                }
            }
            Some("md" | "txt") if meta.len() < 4_000_000 => {
                let text = fs::read_to_string(&path).msg()?;
                let stamp = meta
                    .modified()
                    .map(chrono::DateTime::<chrono::Utc>::from)
                    .map(|d| d.to_rfc3339())
                    .unwrap_or_else(|_| now());
                let mut prev = None;
                for para in text.split("\n\n").map(str::trim).filter(|x| !x.is_empty()) {
                    if let Some(q) = prev {
                        n += add(s, key, &path.to_string_lossy(), &stamp, q, para)? as usize
                    }
                    prev = Some(para)
                }
            }
            Some("json") if meta.len() < 256_000_000 => {
                n += export(
                    s,
                    key,
                    &serde_json::from_reader(fs::File::open(&path).msg()?).msg()?,
                )?
            }
            Some("zip") => {
                let mut z = zip::ZipArchive::new(fs::File::open(&path).msg()?).msg()?;
                let mut found = false;
                for i in 0..z.len() {
                    let f = z.by_index(i).msg()?;
                    let name = f.name().rsplit('/').next().unwrap_or("");
                    if (name == "conversations.json"
                        || name.starts_with("conversations-") && name.ends_with(".json"))
                        && f.size() < 256_000_000
                    {
                        let v = serde_json::from_reader(f.take(256_000_000)).msg()?;
                        n += export(s, key, &v)?;
                        found = true
                    }
                }
                if !found {
                    return Err("No supported conversations file was found in this archive.".into());
                }
            }
            _ => {}
        }
        s.set(&cursor, &json!(fingerprint))?;
    }
    Ok((n, false))
}
pub fn moments(s: &Store, split: &str, limit: i64) -> Res<Vec<Value>> {
    moments_before(s, split, limit, None)
}
pub fn moments_before(s: &Store, split: &str, limit: i64, cutoff: Option<&str>) -> Res<Vec<Value>> {
    let c = s.db.lock().unwrap();
    let mut st=c.prepare("SELECT id,source,stamp,situation,action FROM moments WHERE split=?1 AND (?2 IS NULL OR julianday(stamp)<julianday(?2)) ORDER BY julianday(stamp) DESC,id LIMIT ?3").msg()?;
    let rows=st.query_map(params![split,cutoff,limit],|r|Ok(json!({"id":r.get::<_,String>(0)?,"source":r.get::<_,String>(1)?,"date":r.get::<_,String>(2)?,"question":r.get::<_,String>(3)?,"actual":r.get::<_,String>(4)?}))).msg()?;
    rows.map(|r| r.msg()).collect()
}
pub fn nearby(s: &Store, question: &str) -> Res<Vec<Value>> {
    let feature = crate::local::embedding(question);
    let c = s.db.lock().unwrap();
    let mut st=c.prepare("SELECT m.action,m.situation,m.stamp,m.source FROM (SELECT moment_id,distance FROM vectors WHERE feature MATCH ? AND k=32 ORDER BY distance) AS hits JOIN moments m ON m.id=hits.moment_id WHERE m.split='train' ORDER BY hits.distance LIMIT 3").msg()?;
    let rows=st.query_map([feature],|r|Ok(json!({"reply":r.get::<_,String>(0)?,"situation":r.get::<_,String>(1)?,"date":r.get::<_,String>(2)?,"source":r.get::<_,String>(3)?}))).msg()?;
    rows.map(|r| r.msg()).collect()
}
fn messages(s: &Store, source: &str, path: &Path, first: bool) -> Res<(usize, bool)> {
    let db=rusqlite::Connection::open_with_flags(path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY|rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|_|"Messages is unavailable. You can grant Sumi Full Disk Access in macOS or leave this source off.")?;
    let cursor = format!("cursor/{source}/rows");
    let mut saved = s.get(&cursor)?;
    if saved.is_null() {
        let since = (chrono::Utc::now().timestamp() - 90 * 86400 - 978307200) * 1_000_000_000;
        let floor:i64=db.query_row("SELECT COALESCE(MIN(rowid),(SELECT COALESCE(MAX(rowid),0)+1 FROM message)) FROM message WHERE date>=?",[since],|r|r.get(0)).msg()?;
        saved = json!({"floor":floor,"live":floor.saturating_sub(1),"older":0});
    }
    let floor = saved["floor"].as_i64().unwrap_or(1);
    let live = saved["live"].as_i64().unwrap_or(0);
    let last: i64 = db
        .query_row("SELECT COALESCE(MAX(rowid),0) FROM message", [], |r| {
            r.get(0)
        })
        .msg()?;
    let older = saved["older"].as_i64().unwrap_or(0);
    let recent = live < last;
    let (lo, hi) = if recent {
        (live, last)
    } else if !first && older < floor - 1 {
        (older, floor - 1)
    } else {
        return Ok((0, older < floor - 1));
    };
    let hi = hi.min(lo + if first { 250 } else { 1000 });
    // For each sent text, look up the immediately preceding text in this chat.
    // This works across page boundaries and never persists an unmatched bystander message.
    let mut rows=db.prepare("SELECT j.chat_id,m.text,datetime(CASE WHEN m.date>1000000000000 THEN m.date/1000000000 ELSE m.date END+978307200,'unixepoch'),
    (SELECT CASE WHEN prior.is_from_me=0 THEN prior.text END FROM chat_message_join cj JOIN message prior ON prior.rowid=cj.message_id WHERE cj.chat_id=j.chat_id AND cj.message_id<m.rowid AND length(prior.text)>0 ORDER BY cj.message_id DESC LIMIT 1)
    FROM message m JOIN chat_message_join j ON j.message_id=m.rowid WHERE m.rowid>? AND m.rowid<=? AND m.is_from_me=1 AND length(m.text)>0 ORDER BY m.rowid").msg()?;
    let mut rows = rows.query(params![lo, hi]).msg()?;
    let mut n = 0;
    while let Some(row) = rows.next().msg()? {
        let prior: Option<String> = row.get(3).msg()?;
        if let Some(q) = prior {
            n += add(
                s,
                source,
                &format!("messages-{}", row.get::<_, i64>(0).msg()?),
                &row.get::<_, String>(2).msg()?,
                &q,
                &row.get::<_, String>(1).msg()?,
            )? as usize;
        }
    }
    saved[if recent { "live" } else { "older" }] = json!(hi);
    s.set(&cursor, &saved)?;
    Ok((
        n,
        saved["live"].as_i64().unwrap_or(0) < last
            || saved["older"].as_i64().unwrap_or(0) < floor - 1,
    ))
}
/// Retrieval for evaluation never returns the observed reply. Exact duplicates of
/// learning examples, including the same conversation in a second export, are excluded.
pub fn blind_cases(s: &Store, limit: i64) -> Res<Vec<Value>> {
    let c = s.db.lock().unwrap();
    let mut query=c.prepare("SELECT t.id,t.source,t.stamp,t.situation FROM moments t WHERE t.split='test' AND NOT EXISTS(SELECT 1 FROM moments m WHERE m.split!='test' AND m.situation=t.situation AND m.action=t.action) GROUP BY t.situation,t.action ORDER BY t.stamp DESC,t.id LIMIT ?").msg()?;
    let rows=query.query_map([limit],|r|Ok(json!({"id":r.get::<_,String>(0)?,"source":r.get::<_,String>(1)?,"date":r.get::<_,String>(2)?,"question":r.get::<_,String>(3)?}))).msg()?;
    rows.map(|v| v.msg()).collect()
}
pub fn unseal(s: &Store, key: &str) -> Res<String> {
    s.doc("blind", key)?;
    s.db.lock()
        .unwrap()
        .query_row(
            "SELECT action FROM moments WHERE id=? AND split='test'",
            [key],
            |r| r.get(0),
        )
        .msg()
}
