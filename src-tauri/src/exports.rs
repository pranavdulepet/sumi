use crate::{app, sources, store::*};
use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

const REQUEST: &str = "requestedExport";

pub fn request(s: &Store) -> Res<Value> {
    let _edit = s.edit.lock().unwrap();
    s.set(REQUEST, &json!({"since": now(), "offers": []}))?;
    Ok(Value::Null)
}

pub fn cancel(s: &Store) -> Res<Value> {
    let _edit = s.edit.lock().unwrap();
    s.set(REQUEST, &Value::Null)?;
    Ok(Value::Null)
}

fn candidates(s: &Store, dir: &Path, since: SystemTime) -> Res<Vec<Value>> {
    let entries = fs::read_dir(dir).map_err(|_| {
        "Sumi couldn’t check Downloads. You can use Import chat export to choose the file."
    })?;
    let known: HashSet<_> = s
        .list("source")?
        .into_iter()
        .filter_map(|r| r["path"].as_str().map(str::to_owned))
        .collect();
    let mut offers = Vec::new();
    for e in entries.filter_map(Result::ok) {
        let path = e.path();
        if !path
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("zip"))
            || known.contains(path.to_string_lossy().as_ref())
            || !e.file_type().is_ok_and(|t| t.is_file())
        {
            continue;
        }
        let Ok(meta) = e.metadata() else {
            continue;
        };
        if ![meta.modified(), meta.created()]
            .into_iter()
            .flatten()
            .any(|t| t >= since)
        {
            continue;
        }
        // Inspect only member names. Opening conversations requires separate consent.
        let Some(zip) = fs::File::open(&path)
            .ok()
            .and_then(|f| zip::ZipArchive::new(f).ok())
        else {
            continue;
        };
        if zip.file_names().any(sources::conversation_file) {
            offers.push(json!({"path":path, "name":e.file_name().to_string_lossy()}));
        }
    }
    offers.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(offers)
}

pub fn find(s: &Store, dir: &Path) -> Res<Value> {
    Ok(json!(candidates(
        s,
        dir,
        SystemTime::now() - Duration::from_secs(7 * 86400)
    )?))
}

pub fn accept(s: &Store, dir: &Path, selected: &str) -> Res<Value> {
    let _edit = s.edit.lock().unwrap();
    let request = s.get(REQUEST)?;
    let since = request["since"]
        .as_str()
        .and_then(crate::scroll::instant)
        .map(SystemTime::from)
        .unwrap_or(SystemTime::now());
    let since = since.min(SystemTime::now() - Duration::from_secs(7 * 86400));
    let offer = candidates(s, dir, since)?
        .into_iter()
        .find(|r| r["path"] == selected)
        .ok_or("This export is no longer available. Choose the file again.")?;
    let src = app::register(
        s,
        &hash(selected)[..32],
        Path::new(selected),
        offer["name"].as_str().unwrap_or("Chat export"),
    )?;
    if !request.is_null() {
        s.set(REQUEST, &Value::Null)?;
    }
    Ok(src)
}

#[derive(Default)]
pub struct Arrival {
    watcher: Option<notify::RecommendedWatcher>,
    dirty: Arc<AtomicBool>,
    since: String,
}

impl Arrival {
    pub fn check(&mut self, s: &Store, dir: &Path) -> Res<()> {
        let request = s.get(REQUEST)?;
        let Some(since) = request["since"].as_str() else {
            self.watcher = None;
            self.since.clear();
            return Ok(());
        };
        if self.watcher.is_none() {
            let signal = self.dirty.clone();
            if let Ok(mut watcher) =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if event.is_err() || event.is_ok_and(|e| !e.kind.is_access()) {
                        signal.store(true, Ordering::Release);
                    }
                })
            {
                if watcher.watch(dir, RecursiveMode::NonRecursive).is_ok() {
                    self.watcher = Some(watcher);
                }
            }
            self.dirty.store(true, Ordering::Release);
        }
        let dirty = self.dirty.swap(false, Ordering::AcqRel);
        if !dirty && self.since == since {
            return Ok(());
        }
        self.since = since.to_owned();
        let start = crate::scroll::instant(since).ok_or("Unknown export request date.")?;
        let mut update = request.clone();
        match candidates(s, dir, start.into()) {
            Ok(offers) => {
                update["offers"] = json!(offers);
                update.as_object_mut().unwrap().remove("issue");
            }
            Err(e) => {
                update["offers"] = json!([]);
                update["issue"] = json!(e);
            }
        }
        if update != request {
            let _edit = s.edit.lock().unwrap();
            if s.get(REQUEST)? == request {
                s.set(REQUEST, &update)?;
            }
        }
        Ok(())
    }
}
