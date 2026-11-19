use crate::{seal, store::*};
use serde_json::json;
use std::{io::Read, sync::Arc};
use tiny_http::{Header, Method, Response, Server};
pub fn start(s: Arc<Store>) -> Res<()> {
    let server = Server::http("127.0.0.1:0").map_err(|e| e.to_string())?;
    let addr = server
        .server_addr()
        .to_ip()
        .ok_or("Missing local address.")?;
    s.set("sealUrl", &json!(format!("http://{addr}/mcp")))?;
    std::thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let header = |key: &str| {
                req.headers()
                    .iter()
                    .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(key))
                    .map(|h| h.value.to_string())
            };
            let host = header("Host");
            let origin = header("Origin");
            let authorization = header("Authorization").unwrap_or_default();
            if host.as_deref() != Some(&addr.to_string())
                || origin.is_some()
                || req.url() != "/mcp"
                || req.method() != &Method::Post
            {
                let _ = req.respond(Response::empty(403));
                continue;
            }
            let Some(token) = authorization.strip_prefix("Bearer ") else {
                let _ = req.respond(Response::empty(401));
                continue;
            };
            if !seal::authorized(&s, token) {
                let _ = req.respond(Response::empty(401));
                continue;
            }
            let mut bytes = Vec::new();
            if req
                .as_reader()
                .take(64 * 1024 + 1)
                .read_to_end(&mut bytes)
                .is_err()
                || bytes.len() > 64 * 1024
            {
                let _ = req.respond(Response::empty(413));
                continue;
            }
            let Ok(v) = serde_json::from_slice(&bytes) else {
                let _ = req.respond(Response::empty(400));
                continue;
            };
            if let Some(out) = seal::rpc(&s, token, &v) {
                let _ =
                    req.respond(Response::from_string(out.to_string()).with_header(
                        Header::from_bytes("Content-Type", "application/json").unwrap(),
                    ));
            } else {
                let _ = req.respond(Response::empty(202));
            }
        }
    });
    Ok(())
}

/// Stdio clients relay to the running notebook so Keychain and all mutations
/// have one owner even when many coding agents run concurrently.
pub fn start_socket(s: Arc<Store>) -> Res<()> {
    use std::os::unix::{
        fs::FileTypeExt,
        net::{UnixListener, UnixStream},
    };
    let path = s.root.join(".sumi/agent.sock");
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if !meta.file_type().is_socket() {
            return Err("Unexpected file at the notebook socket path.".into());
        }
        if UnixStream::connect(&path).is_ok() {
            return Err("The notebook is already open.".into());
        }
        std::fs::remove_file(&path).msg()?;
    }
    let listener = UnixListener::bind(path).msg()?;
    std::thread::spawn(move || {
        for socket in listener.incoming().flatten() {
            let s = s.clone();
            std::thread::spawn(move || {
                let _ = handle_socket(&s, socket);
            });
        }
    });
    Ok(())
}
fn handle_socket(s: &Store, mut stream: std::os::unix::net::UnixStream) -> Res<()> {
    use std::io::{BufRead, BufReader, Write};
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(8)))
        .msg()?;
    let mut frame = Vec::new();
    BufReader::new(&stream)
        .take(65537)
        .read_until(b'\n', &mut frame)
        .msg()?;
    if frame.len() > 65536 {
        return Err("Agent request is too large.".into());
    }
    let frame: serde_json::Value = serde_json::from_slice(&frame).msg()?;
    let token = frame["credential"].as_str().ok_or("Missing credential.")?;
    let response = seal::rpc(s, token, &frame["request"]);
    writeln!(stream, "{}", json!(response)).msg()
}
pub fn relay(root: &std::path::Path, token: &str) -> Res<bool> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let path = root.join(".sumi/agent.sock");
    match UnixStream::connect(&path) {
        Ok(s) => drop(s),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(false)
        }
        Err(e) => return Err(e.to_string()),
    }
    for line in std::io::stdin().lock().lines() {
        let line = line.msg()?;
        let req: serde_json::Value = serde_json::from_str(&line).msg()?;
        let mut socket = UnixStream::connect(&path).msg()?;
        let frame = json!({"credential":token,"request":req}).to_string();
        if frame.len() > 65535 {
            return Err("Agent request is too large.".into());
        }
        writeln!(socket, "{frame}").msg()?;
        let mut out = String::new();
        BufReader::new(socket).read_line(&mut out).msg()?;
        if out.trim() != "null" && !out.trim().is_empty() {
            writeln!(std::io::stdout().lock(), "{}", out.trim()).msg()?;
        }
    }
    Ok(true)
}
