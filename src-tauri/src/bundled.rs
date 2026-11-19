use crate::store::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
const FILE: &str = "qwen2.5-1.5b-instruct-q4_k_m.gguf";
const SIZE: u64 = 1117320736;
const SHA: &str = "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e";
struct Running {
    child: Child,
    path: PathBuf,
    port: u16,
    token: String,
    last: Instant,
}
static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
fn stop(r: &mut Option<Running>) {
    if let Some(mut r) = r.take() {
        let _ = r.child.kill();
        let _ = r.child.wait();
    }
}
pub fn reap(force: bool) {
    if let Ok(mut r) = RUNNING.try_lock() {
        if force
            || r.as_ref()
                .is_some_and(|r| r.last.elapsed() > Duration::from_secs(240))
        {
            stop(&mut r)
        }
    }
}
fn binary() -> Res<PathBuf> {
    let exe = std::env::current_exe().msg()?;
    let packaged = exe
        .parent()
        .unwrap()
        .join("../Resources/runtime/llama-server");
    if packaged.exists() {
        return Ok(packaged);
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime/llama-server");
    if cfg!(debug_assertions) && dev.exists() {
        Ok(dev)
    } else {
        Err("The bundled runtime is missing. Reinstall Sumi.".into())
    }
}
pub fn use_file(s: &Store, path: &Path) -> Res<Value> {
    let mut magic = [0; 4];
    std::fs::File::open(path)
        .msg()?
        .read_exact(&mut magic)
        .msg()?;
    if &magic != b"GGUF" {
        return Err("Choose a GGUF model file.".into());
    }
    let config = json!({"runtime":"Sumi","name":path.file_stem().and_then(|n|n.to_str()).unwrap_or("Local model"),"path":path,"private":s.root.join(".sumi")});
    crate::models::infer(&config, "Reply with one word.", "Ready?", 6)?;
    s.set("model", &config)?;
    Ok(config)
}
pub fn download(s: &Store) -> Res<Value> {
    let folder = s.root.join(".sumi/models");
    std::fs::create_dir_all(&folder).msg()?;
    let dest = folder.join(FILE);
    if dest.exists() {
        return use_file(s, &dest);
    }
    if crate::local::fit(s)["freeDisk"].as_u64().unwrap_or(0) < SIZE + 3_000_000_000 {
        return Err("Free another 1.2 GB before downloading this model.".into());
    }
    let tmp = folder.join(format!("{}.partial", id()));
    let result = (|| -> Res<()> {
        let url =
            format!("https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/{FILE}");
        let mut r = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(90))
            .build()
            .get(&url)
            .call()
            .map_err(|_| "Model download failed. Check the connection and try again.")?
            .into_reader();
        let mut f = std::fs::File::create(&tmp).msg()?;
        let mut h = Sha256::new();
        let mut buf = [0u8; 65536];
        let mut bytes = 0;
        loop {
            let n = r.read(&mut buf).msg()?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            if bytes > SIZE {
                return Err("The model exceeds its published size.".into());
            }
            h.update(&buf[..n]);
            f.write_all(&buf[..n]).msg()?;
        }
        if bytes != SIZE || hex::encode(h.finalize()) != SHA {
            return Err("Model checksum mismatch. The download was discarded.".into());
        }
        f.sync_all().msg()?;
        std::fs::rename(&tmp, &dest).msg()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    use_file(s, &dest)
}
pub fn infer(cfg: &Value, messages: Value, limit: u32, schema: Option<&Value>) -> Res<String> {
    let path = PathBuf::from(cfg["path"].as_str().ok_or("Missing model file.")?);
    let base = Path::new(
        cfg["private"]
            .as_str()
            .ok_or("Missing notebook directory.")?,
    );
    let mut state = RUNNING.lock().unwrap();
    let live = state
        .as_mut()
        .is_some_and(|r| r.path == path && r.child.try_wait().ok() == Some(None));
    let h = ureq::AgentBuilder::new()
        .redirects(0)
        .try_proxy_from_env(false)
        .timeout_read(Duration::from_secs(120))
        .build();
    if !live {
        stop(&mut state);
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .msg()?
            .local_addr()
            .msg()?
            .port();
        let token = id() + &id();
        let keyfile = base.join("runtime.key");
        atomic(&keyfile, token.as_bytes())?;
        let child = Command::new(binary()?)
            .args([
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "-c",
                "4096",
                "--parallel",
                "1",
                "--no-webui",
                "--offline",
                "--api-key-file",
            ])
            .arg(&keyfile)
            .arg("-m")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .msg()?;
        *state = Some(Running {
            child,
            path,
            port,
            token,
            last: Instant::now(),
        });
        let r = state.as_mut().unwrap();
        let start = Instant::now();
        loop {
            if r.child.try_wait().msg()?.is_some() {
                return Err("The local runtime could not load this model.".into());
            }
            if h.get(&format!("http://127.0.0.1:{port}/health"))
                .call()
                .is_ok()
            {
                break;
            }
            if start.elapsed() > Duration::from_secs(80) {
                stop(&mut state);
                return Err("Model loading timed out.".into());
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }
    let r = state.as_mut().unwrap();
    r.last = Instant::now();
    let mut body =
        json!({"model":cfg["name"],"messages":messages,"temperature":0,"max_tokens":limit});
    if let Some(schema) = schema {
        body["response_format"] = json!({"type":"json_schema","json_schema":{"name":"result","strict":true,"schema":schema}});
    }
    let v: Value = h
        .post(&format!("http://127.0.0.1:{}/v1/chat/completions", r.port))
        .set("Authorization", &format!("Bearer {}", r.token))
        .send_json(body)
        .msg()?
        .into_json()
        .msg()?;
    v.pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or("No completion from the bundled runtime.".into())
}
