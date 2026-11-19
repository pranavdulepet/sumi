use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

pub type Res<T> = Result<T, String>;
pub trait Msg<T> {
    fn msg(self) -> Res<T>;
}
impl<T, E: std::fmt::Display> Msg<T> for Result<T, E> {
    fn msg(self) -> Res<T> {
        self.map_err(|e| e.to_string())
    }
}
pub fn id() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}
pub fn hash(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}
pub fn now() -> String {
    Utc::now().to_rfc3339()
}
pub fn atomic(path: &Path, data: &[u8]) -> Res<()> {
    atomic_with_permissions(path, data, None)
}
pub fn atomic_with_permissions(
    path: &Path,
    data: &[u8],
    permissions: Option<fs::Permissions>,
) -> Res<()> {
    let tmp = path.with_extension(format!("{}.tmp", id()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .msg()?;
    if let Err(e) = f
        .write_all(data)
        .and_then(|_| match permissions {
            Some(p) => f.set_permissions(p),
            None => Ok(()),
        })
        .and_then(|_| f.sync_all())
        .and_then(|_| fs::rename(&tmp, path))
    {
        let _ = fs::remove_file(tmp);
        return Err(e.to_string());
    }
    Ok(())
}
pub fn ident(s: &str) -> Res<&str> {
    if s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit()) {
        Ok(s)
    } else {
        Err("Invalid record identifier.".into())
    }
}
pub fn keychain(service: &str, account: &str) -> Res<Vec<u8>> {
    security_framework::passwords::get_generic_password(service, account).msg()
}
pub struct Store {
    pub root: PathBuf,
    pub db: Mutex<Connection>,
    pub edit: Mutex<()>,
    pub lifecycle: std::sync::RwLock<()>,
    pub reading: std::sync::atomic::AtomicBool,
}
impl Store {
    pub fn open(root: PathBuf) -> Res<Self> {
        let account = hash(&root.to_string_lossy());
        let key = match security_framework::passwords::get_generic_password(
            "studio.sumi.memory",
            &account,
        ) {
            Ok(k) => k,
            Err(e) if e.code() == -25300 => {
                if root.join(".sumi/moments.db").exists() {
                    return Err("The notebook key is missing from Keychain. Restore the key before opening this notebook.".into());
                }
                let k = rand::random::<[u8; 32]>();
                security_framework::passwords::set_generic_password(
                    "studio.sumi.memory",
                    &account,
                    &k,
                )
                .msg()?;
                k.to_vec()
            }
            Err(e) => return Err(format!("Unlock your login Keychain to open Sumi: {e}")),
        };
        Self::with_key(root, &key)
    }
    pub fn with_key(root: PathBuf, key: &[u8]) -> Res<Self> {
        for dir in ["", ".sumi", "scroll", "scroll/people", "desk"] {
            let p = root.join(dir);
            fs::create_dir_all(&p).msg()?;
            fs::set_permissions(p, fs::Permissions::from_mode(0o700)).msg()?
        }
        crate::local::vectors();
        let conn = Connection::open(root.join(".sumi/moments.db")).msg()?;
        conn.execute_batch(&format!("PRAGMA key=\"x'{}'\";", hex::encode(key)))
            .msg()?;
        let _: String = conn
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .msg()?;
        conn.busy_timeout(Duration::from_secs(12)).msg()?;
        conn.execute_batch("PRAGMA journal_mode=WAL;PRAGMA foreign_keys=ON;PRAGMA secure_delete=ON;
    CREATE VIRTUAL TABLE IF NOT EXISTS vectors USING vec0(moment_id text primary key, feature float[192]);
    CREATE TABLE IF NOT EXISTS kv(k TEXT PRIMARY KEY,v TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS moments(id TEXT PRIMARY KEY,source TEXT NOT NULL,stamp TEXT NOT NULL,situation TEXT NOT NULL,action TEXT NOT NULL,split TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS moments_pair ON moments(situation,action,split);
    CREATE INDEX IF NOT EXISTS moments_split ON moments(split,stamp);
    CREATE TABLE IF NOT EXISTS docs(kind TEXT NOT NULL,id TEXT NOT NULL,v TEXT NOT NULL,PRIMARY KEY(kind,id));
    CREATE INDEX IF NOT EXISTS docs_kind ON docs(kind);
    CREATE INDEX IF NOT EXISTS waiting_agent_log ON docs(kind) WHERE kind='log' AND json_extract(v,'$.status')='waiting';").msg()?;
        Ok(Self {
            root,
            db: Mutex::new(conn),
            edit: Mutex::new(()),
            lifecycle: std::sync::RwLock::new(()),
            reading: false.into(),
        })
    }
    pub fn get(&self, k: &str) -> Res<Value> {
        let s: Option<String> = self
            .db
            .lock()
            .unwrap()
            .query_row("SELECT v FROM kv WHERE k=?", [k], |r| r.get(0))
            .optional()
            .msg()?;
        s.map(|v| serde_json::from_str(&v).msg())
            .unwrap_or(Ok(Value::Null))
    }
    pub fn set(&self, k: &str, v: &Value) -> Res<()> {
        self.db
            .lock()
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO kv VALUES (?,?)",
                params![k, v.to_string()],
            )
            .msg()?;
        Ok(())
    }
    pub fn dismiss_margin(&self, key: &str) -> Res<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .msg()?;
        let stored: Option<String> = tx
            .query_row("SELECT v FROM kv WHERE k='marginMuted'", [], |r| r.get(0))
            .optional()
            .msg()?;
        let mut muted: Value = serde_json::from_str(stored.as_deref().unwrap_or("{}")).msg()?;
        if !muted.is_object() {
            muted = json!({});
        }
        let now = Utc::now();
        if muted[key]["until"]
            .as_str()
            .and_then(crate::scroll::instant)
            .is_some_and(|until| until > now)
        {
            return Ok(());
        }
        let count = muted[key]["count"].as_u64().unwrap_or(0).saturating_add(1);
        let days = if count == 1 { 7 } else { 30 };
        muted[key] =
            json!({"count": count, "until": (now + chrono::Duration::days(days)).to_rfc3339()});
        muted["quietUntil"] = json!((now + chrono::Duration::days(1)).to_rfc3339());
        tx.execute(
            "INSERT OR REPLACE INTO kv VALUES ('marginMuted', ?)",
            [muted.to_string()],
        )
        .msg()?;
        tx.commit().msg()
    }
    pub fn put(&self, kind: &str, v: &Value) -> Res<()> {
        let k = v["id"].as_str().ok_or("Missing record ID")?;
        self.db
            .lock()
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO docs VALUES (?,?,?)",
                params![kind, k, v.to_string()],
            )
            .msg()?;
        Ok(())
    }
    pub fn doc(&self, kind: &str, k: &str) -> Res<Value> {
        let s: String = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT v FROM docs WHERE kind=? AND id=?",
                params![kind, k],
                |r| r.get(0),
            )
            .msg()?;
        serde_json::from_str(&s).msg()
    }
    pub fn list(&self, kind: &str) -> Res<Vec<Value>> {
        let conn = self.db.lock().unwrap();
        let mut q = conn
            .prepare("SELECT v FROM docs WHERE kind=? ORDER BY rowid DESC")
            .msg()?;
        let rows = q.query_map([kind], |r| r.get::<_, String>(0)).msg()?;
        rows.map(|v| serde_json::from_str(&v.msg()?).msg())
            .collect()
    }
    pub fn latest(&self, kind: &str) -> Res<Option<Value>> {
        let last: Option<String> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT v FROM docs WHERE kind=? ORDER BY rowid DESC LIMIT 1",
                [kind],
                |r| r.get(0),
            )
            .optional()
            .msg()?;
        last.map(|raw| serde_json::from_str(&raw).msg()).transpose()
    }
    pub fn agent_waiting(&self) -> Res<bool> {
        self.db.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM docs WHERE kind='log' AND json_extract(v,'$.status')='waiting')",
            [], |r| r.get(0)
        ).msg()
    }
    pub fn del(&self, kind: &str, k: &str) -> Res<()> {
        self.db
            .lock()
            .unwrap()
            .execute("DELETE FROM docs WHERE kind=? AND id=?", params![kind, k])
            .msg()?;
        Ok(())
    }
    pub fn sheet(&self, k: &str) -> Res<String> {
        fs::read_to_string(self.root.join("desk").join(format!("{}.md", ident(k)?))).msg()
    }
    pub fn save_sheet(&self, k: &str, s: &str) -> Res<()> {
        atomic(
            &self.root.join("desk").join(format!("{}.md", ident(k)?)),
            s.as_bytes(),
        )
    }
    fn create_sheet(&self, tx: &Transaction<'_>) -> Res<String> {
        let k = id();
        self.save_sheet(&k, "")?;
        tx.execute(
            "INSERT INTO docs VALUES('sheet',?,?)",
            params![k, json!({"id":k,"date":now()}).to_string()],
        )
        .msg()?;
        tx.execute(
            "INSERT OR REPLACE INTO kv VALUES('current',?)",
            [json!(k).to_string()],
        )
        .msg()?;
        Ok(k)
    }
    pub fn new_sheet(&self) -> Res<String> {
        let mut conn = self.db.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .msg()?;
        let key = self.create_sheet(&tx)?;
        tx.commit().msg()?;
        Ok(key)
    }
    // Called on foreground entry only. Background agents and notebook refreshes must
    // never change the sheet beneath someone who is still writing after midnight.
    pub fn enter_desk(&self, day: chrono::NaiveDate, shortcut: bool) -> Res<bool> {
        let mut conn = self.db.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .msg()?;
        let prior: Option<String> = tx
            .query_row(
                "SELECT json_extract(v,'$') FROM kv WHERE k='deskDay'",
                [],
                |r| r.get(0),
            )
            .optional()
            .msg()?;
        let day = day.to_string();
        let new_day = prior.as_ref().is_some_and(|last| *last != day);
        if new_day || shortcut {
            let current: String = tx
                .query_row(
                    "SELECT json_extract(v,'$') FROM kv WHERE k='current'",
                    [],
                    |r| r.get(0),
                )
                .msg()?;
            let sealed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM docs WHERE kind='sealed' AND json_extract(v,'$.sheet')=?)",[&current],|r|r.get(0)).msg()?;
            if sealed || !self.sheet(&current)?.is_empty() {
                self.create_sheet(&tx)?;
            }
        }
        if prior.as_ref() != Some(&day) {
            tx.execute(
                "INSERT OR REPLACE INTO kv VALUES('deskDay',?)",
                [json!(day).to_string()],
            )
            .msg()?;
        }
        tx.commit().msg()?;
        Ok(new_day)
    }
    pub fn touch_desk(&self, sheet: &str, day: chrono::NaiveDate) -> Res<()> {
        self.db.lock().unwrap().execute("UPDATE kv SET v=?1 WHERE k='deskDay' AND v<>?1 AND EXISTS(SELECT 1 FROM kv WHERE k='current' AND json_extract(v,'$')=?2)",params![json!(day.to_string()).to_string(),sheet]).msg()?;
        Ok(())
    }
}
