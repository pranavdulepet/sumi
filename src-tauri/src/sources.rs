use crate::store::*;
use regex::Regex;
use rusqlite::params;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
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
const FIRST_SESSION_LINES: usize = 800;
const FIRST_SESSION_BYTES: u64 = 4_000_000;

// Start at a complete line near the end of a long transcript. Fresh replies
// should be visible even when months of older turns precede them in one file.
fn recent_start(file: &mut fs::File, from: u64, end: u64) -> Res<u64> {
    let floor = from.max(end.saturating_sub(FIRST_SESSION_BYTES));
    let mut pos = end;
    let mut lines = 0;
    let mut earliest = end;
    let mut buf = [0u8; 8192];
    while pos > floor {
        let size = (pos - floor).min(buf.len() as u64) as usize;
        pos -= size as u64;
        file.seek(SeekFrom::Start(pos)).msg()?;
        file.read_exact(&mut buf[..size]).msg()?;
        for i in (0..size).rev() {
            let start = pos + i as u64 + 1;
            if buf[i] == b'\n' && start < end {
                earliest = start;
                lines += 1;
                if lines == FIRST_SESSION_LINES {
                    return Ok(start);
                }
            }
        }
    }
    Ok(if floor == from { from } else { earliest })
}

const RECENT_SECONDS: i64 = 90 * 86400;
fn recent_reply(stamp: &str, since: i64) -> bool {
    crate::scroll::instant(stamp)
        .is_some_and(|t| (since..=since + RECENT_SECONDS).contains(&t.timestamp()))
}

fn session(
    s: &Store,
    source: &str,
    p: &Path,
    cursor: &str,
    since: Option<i64>,
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
    let mut deferred = false;
    if since.is_some() {
        let start = recent_start(&mut file, offset, meta.len())?;
        if start > offset {
            previous.clear();
            offset = start;
            deferred = true;
        }
    }
    file.seek(SeekFrom::Start(offset)).msg()?;
    let mut reader = BufReader::new(file);
    let mut count = 0;
    let group = p.file_stem().unwrap_or_default().to_string_lossy();
    let mut bytes = Vec::new();
    let mut incomplete_tail = false;
    for _ in 0..if since.is_some() {
        FIRST_SESSION_LINES
    } else {
        10000
    } {
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
                let stamp = v["timestamp"].as_str().unwrap_or("");
                if since.is_some_and(|t| !recent_reply(stamp, t)) {
                    deferred = true;
                } else {
                    count += add(s, source, &group, stamp, &previous, &t)? as usize;
                }
                previous.clear()
            }
            _ => {}
        }
    }
    if deferred {
        // The idle reader resumes from the last contiguous cursor and picks
        // up any pair that straddled the beginning of this preview.
        return Ok((count, false));
    }
    let mut file = reader.into_inner();
    s.set(cursor, &json!({"offset":offset,"inode":meta.ino(),"boundary":boundary(&mut file,offset)?,"previous":previous}))?;
    Ok((count, incomplete_tail || offset >= meta.len()))
}
// A regenerated reply or an edited prompt creates sibling nodes. Without a
// selected node, only the prefix before the first fork is unambiguous.
fn chat_path<'a>(map: &'a serde_json::Map<String, Value>, current: &Value) -> Vec<&'a Value> {
    if !current.is_null() {
        let mut path = Vec::new();
        let mut seen = HashSet::new();
        let mut next = current;
        while !next.is_null() {
            let Some((id, node)) = next.as_str().and_then(|id| map.get_key_value(id)) else {
                return Vec::new();
            };
            if !seen.insert(id) {
                return Vec::new();
            }
            path.push(&node["message"]);
            next = &node["parent"];
        }
        path.reverse();
        return path;
    }

    let mut children: HashMap<Option<&str>, Vec<&str>> = HashMap::new();
    for (id, node) in map {
        let parent = &node["parent"];
        if parent.is_null() || parent.as_str().is_some_and(|p| map.contains_key(p)) {
            children.entry(parent.as_str()).or_default().push(id);
        } else {
            return Vec::new();
        }
    }
    let mut path = Vec::new();
    let mut parent = None;
    while let Some([child]) = children.get(&parent).map(Vec::as_slice) {
        path.push(&map[*child]["message"]);
        parent = Some(child);
    }
    path
}
#[derive(Clone, Copy)]
enum ExportWindow {
    All,
    Recent(i64),
    Backfill(i64),
}

fn export(s: &Store, source: &str, v: &Value, window: ExportWindow) -> Res<(usize, bool)> {
    let mut n = 0;
    let mut deferred = false;
    let mut record = |group: &str, stamp: &str, question: &str, reply: &str| -> Res<()> {
        match window {
            ExportWindow::Recent(since) if !recent_reply(stamp, since) => {
                deferred = true;
                return Ok(());
            }
            ExportWindow::Backfill(since) if recent_reply(stamp, since) => return Ok(()),
            _ => {}
        }
        n += add(s, source, group, stamp, question, reply)? as usize;
        Ok(())
    };
    for c in v
        .as_array()
        .ok_or("Expected a conversation export array.")?
    {
        let group = c["id"].as_str().or(c["uuid"].as_str()).unwrap_or("export");
        if let Some(map) = c["mapping"].as_object() {
            let mut previous = String::new();
            for m in chat_path(map, &c["current_node"]) {
                if m["metadata"]["is_visually_hidden_from_conversation"] == true
                    || m["metadata"]["is_user_system_message"] == true
                    || m["recipient"].as_str().is_some_and(|r| r != "all")
                {
                    continue;
                }
                match m["author"]["role"].as_str() {
                    Some("assistant") => previous = content(&m["content"]["parts"]),
                    Some("user") if !previous.is_empty() => {
                        let stamp = m["create_time"]
                            .as_f64()
                            .and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0))
                            .map(|t| t.to_rfc3339())
                            .unwrap_or_default();
                        record(group, &stamp, &previous, &content(&m["content"]["parts"]))?;
                        previous.clear();
                    }
                    _ => {}
                }
            }
        } else if let Some(ms) = c["chat_messages"].as_array() {
            let mut prev = String::new();
            for m in ms {
                let t = m["text"]
                    .as_str()
                    .filter(|text| !text.trim().is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| content(&m["content"]));
                if m["sender"] == "assistant" {
                    prev = t
                } else if m["sender"] == "human" && !prev.is_empty() {
                    record(group, m["created_at"].as_str().unwrap_or(""), &prev, &t)?;
                    prev.clear()
                }
            }
        }
    }
    Ok((n, deferred))
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
    let cutoff =
        std::time::SystemTime::now() - std::time::Duration::from_secs(RECENT_SECONDS as u64);
    let since = chrono::DateTime::<chrono::Utc>::from(cutoff).timestamp();
    let mut pending = false;
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
        let saved = s.get(&cursor)?;
        if saved == fingerprint {
            continue;
        }
        let ext = path.extension().and_then(|x| x.to_str());
        let is_export = matches!(ext, Some("json" | "zip"));
        if first && !is_export && meta.modified().is_ok_and(|d| d < cutoff) {
            return Ok((n, true));
        }
        // Freeze the boundary between passes. Reopening the notebook tomorrow
        // must not drop replies that crossed the 90-day boundary overnight.
        let window = match saved["since"]
            .as_i64()
            .filter(|_| saved["file"] == fingerprint)
        {
            Some(_) if first => {
                pending = true;
                continue;
            }
            Some(t) => ExportWindow::Backfill(t),
            None if first => ExportWindow::Recent(since),
            None => ExportWindow::All,
        };
        let mut deferred = false;
        match ext {
            Some("jsonl") => {
                let (added, done) = session(
                    s,
                    key,
                    &path,
                    &format!("{cursor}/position"),
                    first.then_some(since),
                )?;
                n += added;
                if !done {
                    if first {
                        deferred = true;
                    } else {
                        return Ok((n, true));
                    }
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
                let (added, older) = export(
                    s,
                    key,
                    &serde_json::from_reader(fs::File::open(&path).msg()?).msg()?,
                    window,
                )?;
                n += added;
                deferred = older;
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
                        let (added, older) = export(s, key, &v, window)?;
                        n += added;
                        deferred |= older;
                        found = true
                    }
                }
                if !found {
                    return Err("No supported conversations file was found in this archive.".into());
                }
            }
            _ => {}
        }
        if deferred {
            s.set(&cursor, &json!({"file": fingerprint, "since": since}))?;
            pending = true;
        } else {
            s.set(&cursor, &json!(fingerprint))?;
        }
    }
    Ok((n, pending))
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
