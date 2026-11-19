use crate::{seal, sources, store::*};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize, PartialEq)]
struct Server {
    command: String,
    args: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Binding {
    config: PathBuf,
    slot: String,
    server: Server,
    // During an app move or credential repair, the old stanza still belongs to this notebook.
    previous: Option<Server>,
}

pub fn notebook_lock(s: &Store) -> Res<fs::File> {
    let lock = fs::File::open(s.root.join(".sumi")).msg()?;
    lock.lock().msg()?;
    Ok(lock)
}

fn server(s: &Store, exe: &Path, secret: &Path) -> Server {
    Server {
        command: exe.to_string_lossy().into_owned(),
        args: vec![
            "--mcp".into(),
            "--notebook".into(),
            s.root.to_string_lossy().into_owned(),
            "--credential".into(),
            secret.to_string_lossy().into_owned(),
        ],
    }
}

fn credential(s: &Store, spec: &Server) -> Res<PathBuf> {
    let path = PathBuf::from(spec.args.last().ok_or("Missing agent credential path.")?);
    if path.parent() != Some(s.root.join(".sumi").as_path())
        || !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("agent-") && n.ends_with(".token"))
    {
        return Err("Invalid notebook credential path.".into());
    }
    Ok(path)
}

enum Config {
    Json(Value),
    Toml(toml_edit::DocumentMut),
}

impl Config {
    fn parse(name: &str, bytes: Option<&str>) -> Res<Self> {
        let invalid =
            format!("The {name} configuration needs repair. Sumi kept the file as it was.");
        if name == "Codex" {
            let doc: toml_edit::DocumentMut =
                bytes.unwrap_or("").parse().map_err(|_| invalid.clone())?;
            if doc.get("mcp_servers").is_some_and(|v| !v.is_table_like()) {
                return Err(invalid);
            }
            Ok(Self::Toml(doc))
        } else {
            let doc: Value =
                serde_json::from_str(bytes.unwrap_or("{}")).map_err(|_| invalid.clone())?;
            if !doc.is_object() || doc.get("mcpServers").is_some_and(|v| !v.is_object()) {
                return Err(invalid);
            }
            Ok(Self::Json(doc))
        }
    }
    // The outer Option marks an occupied name, even for a different type of MCP server.
    fn entry(&self, slot: &str) -> Option<Option<Server>> {
        match self {
            Self::Json(doc) => doc["mcpServers"].get(slot).map(|v| {
                if v.as_object()?.len() != 2 {
                    return None;
                }
                serde_json::from_value(v.clone()).ok()
            }),
            Self::Toml(doc) => doc.get("mcp_servers")?.get(slot).map(|v| {
                let t = v.as_table_like()?;
                if t.len() != 2 {
                    return None;
                }
                Some(Server {
                    command: t.get("command")?.as_str()?.to_owned(),
                    args: t
                        .get("args")?
                        .as_array()?
                        .iter()
                        .map(|v| v.as_str().map(str::to_owned))
                        .collect::<Option<_>>()?,
                })
            }),
        }
    }
    fn install(&mut self, slot: &str, spec: &Server) {
        match self {
            Self::Json(doc) => doc["mcpServers"][slot] = json!(spec),
            Self::Toml(doc) => {
                if !doc.contains_key("mcp_servers") {
                    doc["mcp_servers"] = toml_edit::table();
                }
                let mut entry = toml_edit::Table::new();
                entry["command"] = toml_edit::value(spec.command.as_str());
                let args: toml_edit::Array = spec.args.iter().map(String::as_str).collect();
                entry["args"] = toml_edit::value(args);
                doc["mcp_servers"]
                    .as_table_like_mut()
                    .unwrap()
                    .insert(slot, toml_edit::Item::Table(entry));
            }
        }
    }
    fn remove(&mut self, slot: &str) {
        match self {
            Self::Json(doc) => {
                doc["mcpServers"].as_object_mut().unwrap().remove(slot);
            }
            Self::Toml(doc) => {
                doc["mcp_servers"].as_table_like_mut().unwrap().remove(slot);
            }
        }
    }
    fn render(&self) -> Res<String> {
        match self {
            Self::Json(doc) => Ok(serde_json::to_string_pretty(doc).msg()? + "\n"),
            Self::Toml(doc) => Ok(doc.to_string()),
        }
    }
}

struct ConfigFile {
    path: PathBuf,
    before: Option<String>,
    data: Config,
    _directory_lock: fs::File,
}
fn contents(path: &Path) -> Res<Option<String>> {
    match fs::read_to_string(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
impl ConfigFile {
    fn load(path: &Path, name: &str, create: bool) -> Res<Option<Self>> {
        let parent = path
            .parent()
            .ok_or("Missing agent configuration directory.")?;
        if create {
            fs::create_dir_all(parent).msg()?;
        } else if !parent.try_exists().msg()? {
            return Ok(None);
        }
        let target = match fs::symlink_metadata(path) {
            Ok(m) if m.is_symlink() => path
                .canonicalize()
                .map_err(|_| "The agent configuration points to an unavailable file.")?,
            Ok(_) => path.to_owned(),
            Err(e) if e.kind() == ErrorKind::NotFound => path.to_owned(),
            Err(e) => return Err(e.to_string()),
        };
        // Coordinate Sumi notebooks which share this agent configuration, including dotfile symlinks.
        let directory =
            fs::File::open(target.parent().ok_or("Missing configuration directory.")?).msg()?;
        directory.lock().msg()?;
        let before = contents(&target)?;
        let data = Config::parse(name, before.as_deref())?;
        Ok(Some(Self {
            path: target,
            before,
            data,
            _directory_lock: directory,
        }))
    }
    fn save(&self) -> Res<()> {
        if contents(&self.path)? != self.before {
            return Err("The agent configuration changed during this edit. Try again to use its latest contents.".into());
        }
        let permissions = if self.before.is_some() {
            Some(fs::metadata(&self.path).msg()?.permissions())
        } else {
            None
        };
        atomic_with_permissions(&self.path, self.data.render()?.as_bytes(), permissions)
    }
}

fn get_binding(s: &Store, agent: &Value, userhome: &Path, exe: &Path) -> Res<Binding> {
    if !agent["link"].is_null() {
        return serde_json::from_value(agent["link"].clone()).msg();
    }
    let name = agent["name"].as_str().ok_or("Missing agent name.")?;
    let id = ident(agent["id"].as_str().unwrap_or(""))?;
    Ok(Binding {
        config: seal::config_path(userhome, name)?,
        slot: "sumi".into(),
        server: server(
            s,
            exe,
            &s.root.join(".sumi").join(format!("agent-{id}.token")),
        ),
        previous: None,
    })
}

fn unused_slot(config: &Config, preferred: &str, binding: &Binding) -> String {
    let available = |slot: &str| match config.entry(slot) {
        None => true,
        Some(Some(v)) => v == binding.server || binding.previous.as_ref() == Some(&v),
        Some(None) => false,
    };
    if available(preferred) {
        return preferred.into();
    }
    (2..)
        .map(|i| format!("sumi-{i}"))
        .find(|slot| available(slot))
        .unwrap()
}

fn install(s: &Store, agent: &mut Value, mut link: Binding, mut file: ConfigFile) -> Res<()> {
    link.slot = unused_slot(&file.data, &link.slot, &link);
    agent["link"] = json!(link);
    agent["linkState"] = json!("installing");
    s.put("agent", agent)?;
    let secret = credential(s, &link.server)?;
    let token = match contents(&secret)? {
        Some(t) if agent["tokenHash"] == seal::token_hash(&t) => t,
        _ => {
            let t = id() + &id();
            atomic(&secret, t.as_bytes())?;
            t
        }
    };
    if file.data.entry(&link.slot) != Some(Some(link.server.clone())) {
        file.data.install(&link.slot, &link.server);
        file.save()?;
    }
    if let Some(old) = link.previous.take() {
        let prior = credential(s, &old)?;
        if prior != secret {
            remove_secret(&prior)?;
        }
    }
    agent["link"] = json!(link);
    agent["tokenHash"] = json!(seal::token_hash(&token));
    agent.as_object_mut().unwrap().remove("linkState");
    s.set("sealExportPending", &json!(true))?;
    s.put("agent", agent)?;
    seal::export(s)
}

pub fn connect(s: &Store, name: &str, userhome: &Path, exe: &Path) -> Res<Value> {
    let config = seal::config_path(userhome, name)?;
    let _lock = notebook_lock(s)?;
    let key = hash(name)[..32].to_string();
    let active = s.list("agent")?.into_iter().find(|v| v["id"] == key);
    let fresh = active.is_none();
    let mut a=active.unwrap_or_else(||json!({"id":key,"name":name,"permission":"Read","threshold":0.96,"sections":["deciding","writing","words","goals"]}));
    if a["linkState"] == "revoking" {
        return Err("Finish removing this connection before connecting again.".into());
    }
    let mut link = get_binding(s, &a, userhome, exe)?;
    let file = ConfigFile::load(&config, name, true)?.ok_or("Missing configuration.")?;
    if a["linkState"] != "installing" {
        let previous = link.server.clone();
        let old_path = credential(s, &previous)?;
        let unchanged =
            link.config == config && file.data.entry(&link.slot) == Some(Some(previous.clone()));
        let good_token =
            !fresh && contents(&old_path)?.is_some_and(|t| a["tokenHash"] == seal::token_hash(&t));
        if unchanged
            && good_token
            && previous.command == exe.to_string_lossy()
            && !a["link"].is_null()
        {
            seal::export(s)?;
            return Ok(json!(true));
        }
        let path = if unchanged && good_token {
            old_path
        } else {
            s.root
                .join(".sumi")
                .join(format!("agent-{key}-{}.token", id()))
        };
        link.previous = (!fresh).then_some(previous);
        link.server = server(s, exe, &path);
        link.config = config;
    }
    install(s, &mut a, link, file)?;
    Ok(json!(true))
}

fn remove_secret(path: &Path) -> Res<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
fn remove(s: &Store, a: &mut Value, link: Binding) -> Res<()> {
    a["linkState"] = json!("revoking");
    a["link"] = json!(link);
    a.as_object_mut().unwrap().remove("tokenHash");
    s.put("agent", a)?;
    s.db.lock().unwrap().execute(
        "UPDATE docs SET v=json_set(v,'$.status','closed') WHERE kind='log' AND json_extract(v,'$.agent')=? AND json_extract(v,'$.status')='waiting'",
        [a["id"].as_str().unwrap()]
    ).msg()?;
    // Invalidate access before touching a potentially offline or manually edited config.
    remove_secret(&credential(s, &link.server)?)?;
    if let Some(prev) = &link.previous {
        remove_secret(&credential(s, prev)?)?;
    }
    seal::export(s)?;
    if let Some(mut file) = ConfigFile::load(&link.config, a["name"].as_str().unwrap(), false)? {
        if let Some(Some(existing)) = file.data.entry(&link.slot) {
            if existing == link.server || link.previous.as_ref() == Some(&existing) {
                file.data.remove(&link.slot);
                file.save()?;
            }
        }
    }
    s.del("agent", a["id"].as_str().unwrap())
}

pub fn disconnect(s: &Store, key: &str) -> Res<()> {
    let _lock = notebook_lock(s)?;
    let Some(mut a) = s.list("agent")?.into_iter().find(|r| r["id"] == key) else {
        return Ok(());
    };
    let link = get_binding(s, &a, &sources::home(), &std::env::current_exe().msg()?)?;
    remove(s, &mut a, link)
}

pub fn resume(s: &Store) -> Res<()> {
    let _lock = notebook_lock(s)?;
    for mut a in s.list("agent")? {
        let Some(phase) = a["linkState"].as_str() else {
            continue;
        };
        let link: Binding = serde_json::from_value(a["link"].clone()).msg()?;
        let result = match phase {
            "installing" => ConfigFile::load(&link.config, a["name"].as_str().unwrap(), true)
                .and_then(|file| install(s, &mut a, link, file.ok_or("Missing agent config.")?)),
            "revoking" => remove(s, &mut a, link),
            _ => continue,
        };
        if let Err(e) = result {
            s.set("backgroundIssue", &json!(e))?;
        }
    }
    if s.get("sealExportPending")? == true {
        if let Err(e) = seal::export(s) {
            s.set("backgroundIssue", &json!(e))?;
        }
    }
    Ok(())
}
