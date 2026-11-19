use crate::{engine, local, store::*};
use serde_json::{json, Value};
use std::{
    fs::{OpenOptions, TryLockError},
    sync::Arc,
};

const JOB: &str = "firstReading";

pub fn progress(s: &Store) -> Res<Value> {
    Ok(json!({"job":s.get(JOB)?, "progress":s.get("readingProgress")?}))
}

// Only an explicit request creates a job. On relaunch, resume the saved job
// with its original notification choice. The OS releases the lock on exit.
pub fn start(s: &Arc<Store>, notify: Option<bool>) -> Res<()> {
    if s.get("onboarded")? == true || (notify.is_none() && s.get(JOB)?["status"] != "running") {
        return Ok(());
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(s.root.join(".sumi/first-reading.lock"))
        .msg()?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(()),
        Err(TryLockError::Error(e)) => return Err(e.to_string()),
    }
    let saved = s.get(JOB)?;
    if !s.get("reading")?.is_null() {
        if !saved.is_null() {
            if saved["status"] == "running" && saved["notify"] == true {
                local::request_notifications();
                local::notify_ready();
            }
            s.set(JOB, &Value::Null)?;
        }
        return Ok(());
    }
    let resuming = saved["status"] == "running";
    if !resuming && notify.is_none() {
        return Ok(());
    }
    let job = if resuming {
        saved
    } else {
        json!({"status":"running","started":now(),"notify":notify.unwrap_or(false)})
    };
    s.set(JOB, &job)?;
    let notebook = s.clone();
    let launch = std::thread::Builder::new()
        .name("first-reading".into())
        .spawn(move || {
            let _owner = lock;
            let notifications = job["notify"] == true;
            if notifications {
                local::request_notifications();
            }
            match engine::read(&notebook) {
                Ok(_) => {
                    if notifications {
                        local::notify_ready();
                    }
                    let _ = notebook.set(JOB, &Value::Null);
                }
                Err(_) if crate::quitting() => {}
                Err(e) => {
                    let _ = notebook.set(
                        JOB,
                        &json!({"status":"failed","error":e,"notify":notifications}),
                    );
                }
            }
        });
    if let Err(e) = launch {
        let error = format!("Couldn’t start the first reading: {e}");
        s.set(JOB, &json!({"status":"failed","error":error}))?;
        return Err(error);
    }
    Ok(())
}
