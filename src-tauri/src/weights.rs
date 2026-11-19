use crate::store::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

const FILE: &str = "qwen2.5-1.5b-instruct-q4_k_m.gguf";
const REVISION: &str = "91cad51170dc346986eccefdc2dd33a9da36ead9";
const SIZE: u64 = 1_117_320_736;
const SHA: &str = "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e";
const RESERVE: u64 = 3_000_000_000;

struct Download<'a> {
    dest: PathBuf,
    url: &'a str,
    size: u64,
    sha: &'a str,
}
impl Download<'_> {
    fn partial(&self) -> PathBuf {
        self.dest.with_extension("gguf.partial")
    }
    fn saved(&self) -> u64 {
        fs::metadata(self.partial())
            .ok()
            .map(|m| m.len())
            .filter(|n| *n <= self.size)
            .unwrap_or(0)
    }
    fn valid(&self, path: &Path) -> Res<bool> {
        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.to_string()),
        };
        if file.metadata().msg()?.len() != self.size {
            return Ok(false);
        }
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash).msg()?;
        Ok(hex::encode(hash.finalize()) == self.sha)
    }
    fn acquire(&self, free: u64) -> Res<()> {
        let dir = self.dest.parent().ok_or("Missing model directory.")?;
        fs::create_dir_all(dir).msg()?;
        let lock = File::open(dir).msg()?;
        lock.try_lock()
            .map_err(|_| "A model download is already in progress in this notebook.")?;
        if self.valid(&self.dest)? {
            return Ok(());
        }
        let partial = self.partial();
        if self.valid(&partial)? {
            return fs::rename(partial, &self.dest).msg();
        }
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(&partial)
            .msg()?;
        let mut offset = file.metadata().msg()?.len();
        if offset >= self.size {
            file.set_len(0).msg()?;
            offset = 0;
        }
        let required = self.size - offset + RESERVE;
        if free < required {
            return Err(format!(
                "Free {:.2} GB more on the notebook’s volume to finish the download.",
                (required - free) as f64 / 1e9
            ));
        }
        let client = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(90))
            .build();
        let mut request = client.get(self.url).set("Accept-Encoding", "identity");
        if offset > 0 {
            request = request.set("Range", &format!("bytes={offset}-"));
        }
        let response = request
            .call()
            .map_err(|_| "The download was interrupted. Retry to continue from the saved bytes.")?;
        match response.status() {
            206 => {
                let expected = format!("bytes {offset}-{}/{}", self.size - 1, self.size);
                if response.header("Content-Range") != Some(expected.as_str()) {
                    return Err("The server returned a different byte range. Retry to continue the download.".into());
                }
                file.seek(SeekFrom::Start(offset)).msg()?;
            }
            200 => {
                // Some mirrors ignore Range. Start from their complete response.
                file.set_len(0).msg()?;
                offset = 0;
            }
            _ => return Err("The server did not return model weights.".into()),
        }
        let mut input = response.into_reader();
        let mut buf = [0; 65536];
        loop {
            let n = input.read(&mut buf).map_err(|_| {
                "The download was interrupted. Retry to continue from the saved bytes."
            })?;
            if n == 0 {
                break;
            }
            if offset + n as u64 > self.size {
                file.set_len(0).msg()?;
                return Err("The download exceeds the model’s published size.".into());
            }
            file.write_all(&buf[..n]).msg()?;
            offset += n as u64;
        }
        if offset < self.size {
            return Err(
                "The download was interrupted. Retry to continue from the saved bytes.".into(),
            );
        }
        file.sync_all().msg()?;
        if !self.valid(&partial)? {
            fs::remove_file(&partial).msg()?;
            return Err("The model checksum did not match. Retry to download a fresh copy.".into());
        }
        fs::rename(&partial, &self.dest).msg()
    }
}
fn spec<'a>(s: &Store, url: &'a str) -> Download<'a> {
    Download {
        dest: s.root.join(".sumi/models").join(FILE),
        url,
        size: SIZE,
        sha: SHA,
    }
}
pub fn status(s: &Store) -> Value {
    let model = spec(s, "");
    let saved = model.saved();
    json!({"bytes":saved,"total":SIZE,"required":SIZE-saved+RESERVE,"cached":model.dest.is_file()})
}
pub fn download(s: &Store) -> Res<Value> {
    let url =
        format!("https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/{REVISION}/{FILE}");
    let model = spec(s, &url);
    model.acquire(crate::local::free_bytes(&s.root))?;
    crate::bundled::use_file(s, &model.dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_partial_is_verified_before_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let digest = hash("GGUFexample");
        let model = Download {
            dest: dir.path().join("test.gguf"),
            url: "unused",
            size: 11,
            sha: &digest,
        };
        fs::write(&model.dest, b"GGUFcorrupt").unwrap();
        fs::write(model.partial(), b"GGUFexample").unwrap();
        model.acquire(0).unwrap();
        assert_eq!(fs::read(&model.dest).unwrap(), b"GGUFexample");
        assert!(!model.partial().exists());
        model.acquire(0).unwrap();
        fs::write(&model.dest, b"GGUFcorrupt").unwrap();
        assert!(model.acquire(0).unwrap_err().contains("Free "));
    }

    #[test]
    #[ignore = "Reads model ranges from a temporary loopback HTTP server"]
    fn resumable_model_download_handles_short_invalid_and_ignored_ranges() {
        use tiny_http::{Header, Response, Server};
        let dir = tempfile::tempdir().unwrap();
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}/weights", server.server_addr());
        let digest = hash("GGUFexample");
        let model = Download {
            dest: dir.path().join("test.gguf"),
            url: &url,
            size: 11,
            sha: &digest,
        };
        let file = model.partial();
        let download = |expected: Option<&str>, status: u16, range: Option<&str>, body: &str| {
            std::thread::scope(|scope| {
                let handler = scope.spawn(|| {
                    let req = server
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap()
                        .expect("Expected a download");
                    assert_eq!(
                        req.headers()
                            .iter()
                            .find(|h| h.field.equiv("Range"))
                            .map(|h| h.value.as_str()),
                        expected
                    );
                    let mut r = Response::from_string(body).with_status_code(status);
                    if let Some(range) = range {
                        r.add_header(Header::from_bytes("Content-Range", range).unwrap());
                    }
                    req.respond(r).unwrap();
                });
                let result = model.acquire(RESERVE + 11);
                handler.join().unwrap();
                result
            })
        };
        assert!(download(None, 200, None, "GGUF").is_err());
        assert_eq!(fs::read(&file).unwrap(), b"GGUF");
        assert!(download(Some("bytes=4-"), 206, Some("bytes 2-10/11"), "corrupted").is_err());
        assert_eq!(fs::read(&file).unwrap(), b"GGUF");
        download(Some("bytes=4-"), 206, Some("bytes 4-10/11"), "example").unwrap();
        assert_eq!(fs::read(&model.dest).unwrap(), b"GGUFexample");
        fs::write(&model.dest, b"GGUFcorrupt").unwrap();
        fs::write(&file, b"GG").unwrap();
        assert!(download(Some("bytes=2-"), 200, None, "GGUFinvalid").is_err());
        assert!(!file.exists());
        fs::write(&file, b"GG").unwrap();
        download(Some("bytes=2-"), 200, None, "GGUFexample").unwrap();
        assert_eq!(fs::read(&model.dest).unwrap(), b"GGUFexample");
    }
}
