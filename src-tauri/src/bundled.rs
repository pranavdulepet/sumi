use crate::store::*;
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
struct Running {
    child: Child,
    path: PathBuf,
    port: u16,
    token: String,
    last: Instant,
}
// Serialize model switches and requests. Keep ownership of the child available
// to the quit handler while the model loads or an HTTP response is pending.
static REQUEST: Mutex<()> = Mutex::new(());
static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
fn stop(r: &mut Option<Running>) {
    if let Some(mut r) = r.take() {
        let _ = r.child.kill();
        let _ = r.child.wait();
    }
}
pub fn reap(force: bool) {
    if force {
        stop(&mut RUNNING.lock().unwrap());
        return;
    }
    if let Ok(_request) = REQUEST.try_lock() {
        let mut r = RUNNING.lock().unwrap();
        if r.as_ref()
            .is_some_and(|r| r.last.elapsed() > Duration::from_secs(240))
        {
            stop(&mut r);
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
    crate::models::infer(&config, "Reply with one word.", "Ready?", 32)?;
    s.set("model", &config)?;
    Ok(config)
}
pub fn infer(
    cfg: &Value,
    messages: Value,
    limit: u32,
    schema: Option<&Value>,
    sink: Option<&mut dyn FnMut(&str)>,
) -> Res<String> {
    let path = PathBuf::from(cfg["path"].as_str().ok_or("Missing model file.")?);
    let base = Path::new(
        cfg["private"]
            .as_str()
            .ok_or("Missing notebook directory.")?,
    );
    let _request = REQUEST.lock().unwrap();
    let mut state = RUNNING.lock().unwrap();
    if crate::quitting() {
        return Err("Sumi is quitting.".into());
    }
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
                // Short classification and readiness requests need their budget for visible output.
                "--reasoning",
                "off",
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
    }
    let r = state.as_mut().unwrap();
    r.last = Instant::now();
    let port = r.port;
    let token = r.token.clone();
    drop(state);
    if !live {
        let start = Instant::now();
        loop {
            {
                let mut state = RUNNING.lock().unwrap();
                let r = state.as_mut().ok_or("The local runtime stopped.")?;
                if r.child.try_wait().msg()?.is_some() {
                    return Err("The local runtime could not load this model.".into());
                }
                if start.elapsed() > Duration::from_secs(80) {
                    stop(&mut state);
                    return Err("Model loading timed out.".into());
                }
            }
            if h.get(&format!("http://127.0.0.1:{port}/health"))
                .call()
                .is_ok()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }
    let mut body =
        json!({"model":cfg["name"],"messages":messages,"temperature":0,"max_tokens":limit});
    if let Some(schema) = schema {
        body["response_format"] = json!({"type":"json_schema","json_schema":{"name":"result","strict":true,"schema":schema}});
    }
    body["stream"] = json!(sink.is_some());
    let response = h
        .post(&format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(body)
        .msg()?;
    if let Some(sink) = sink {
        return crate::models::consume_chat(response.into_reader(), false, limit, sink);
    }
    let v: Value = response.into_json().msg()?;
    v.pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or("No completion from the bundled runtime.".into())
}
