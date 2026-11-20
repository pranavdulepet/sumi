use crate::store::*;
use ring::digest::{Context, SHA256};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

struct Tier {
    ram: u64,
    label: &'static str,
    repo: &'static str,
    revision: &'static str,
    file: &'static str,
    size: u64,
    sha: &'static str,
}
const TIERS: [Tier; 3] = [
    Tier {
        ram: 32,
        label: "Qwen3.6 35B A3B",
        repo: "unsloth/Qwen3.6-35B-A3B-GGUF",
        revision: "a483e9e6cbd595906af30beda3187c2663a1118c",
        file: "Qwen3.6-35B-A3B-UD-Q4_K_M.gguf",
        size: 22134528992,
        sha: "ac0e2c1189e055faa36eff361580e79c5bd6f8e76bffb4ce547f167d53e31a61",
    },
    Tier {
        ram: 16,
        label: "Gemma 4 12B",
        repo: "google/gemma-4-12B-it-qat-q4_0-gguf",
        revision: "29d097773436b69ff9feafd636ab4cf873786537",
        file: "gemma-4-12b-it-qat-q4_0.gguf",
        size: 6975879296,
        sha: "93567e57a8fe10b23569b9d9ec38cd005deedf71e29477c421a4b83f418a538b",
    },
    Tier {
        ram: 0,
        label: "Qwen3 4B",
        repo: "unsloth/Qwen3-4B-Instruct-2507-GGUF",
        revision: "a06e946bb6b655725eafa393f4a9745d460374c9",
        file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        size: 2497281120,
        sha: "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
    },
];
fn tier(s: &Store) -> &'static Tier {
    let fit = crate::local::fit(s);
    let ram = fit["memory"].as_u64().unwrap_or(0) / 1_073_741_824;
    let free = fit["freeDisk"].as_u64().unwrap_or(0);
    TIERS
        .iter()
        .find(|t| {
            let file = spec(s, t, "");
            t.ram <= ram && (file.dest.is_file() || free >= t.size - file.saved() + RESERVE)
        })
        .unwrap_or(&TIERS[2])
}
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
        let mut digest = Context::new(&SHA256);
        let mut buffer = vec![0; 1024 * 1024];
        loop {
            let bytes = file.read(&mut buffer).msg()?;
            if bytes == 0 {
                break;
            }
            digest.update(&buffer[..bytes]);
        }
        Ok(hex::encode(digest.finish().as_ref()) == self.sha)
    }
    fn acquire(&self, free: u64) -> Res<()> {
        let dir = self.dest.parent().ok_or("Missing model directory.")?;
        fs::create_dir_all(dir).msg()?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(dir.join(".download.lock"))
            .msg()?;
        lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => {
                "A model download is already in progress in this notebook.".to_string()
            }
            std::fs::TryLockError::Error(error) => error.to_string(),
        })?;
        let result = self.acquire_locked(free);
        // Children spawned during a download may briefly inherit the open descriptor.
        lock.unlock().msg()?;
        result
    }
    fn acquire_locked(&self, free: u64) -> Res<()> {
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
fn spec<'a>(s: &Store, t: &Tier, url: &'a str) -> Download<'a> {
    Download {
        dest: s.root.join(".sumi/models").join(t.file),
        url,
        size: t.size,
        sha: t.sha,
    }
}
pub fn status(s: &Store) -> Value {
    let t = tier(s);
    let model = spec(s, t, "");
    let saved = model.saved();
    json!({"label":t.label,"file":t.file,"bytes":saved,"total":t.size,"required":t.size-saved+RESERVE,"cached":model.dest.is_file()})
}
pub fn download(s: &Store) -> Res<Value> {
    let t = tier(s);
    let url = format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        t.repo, t.revision, t.file
    );
    let model = spec(s, t, &url);
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
        let other = File::open(dir.path().join(".download.lock")).unwrap();
        other.lock().unwrap();
        assert!(model
            .acquire(0)
            .unwrap_err()
            .contains("already in progress"));
        other.unlock().unwrap();
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
