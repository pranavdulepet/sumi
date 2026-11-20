use crate::store::*;
use std::{
    ffi::{c_char, CStr, CString},
    sync::Once,
};
extern "C" {
    fn sumi_semantic(p: *const c_char, out: *mut f32, n: i32) -> i32;
    fn sumi_request_notifications();
    fn sumi_notify_when_ready();
    fn sumi_entities(p: *const c_char) -> *mut c_char;
    fn sumi_free(p: *mut c_char);
    fn sumi_heavy_allowed() -> i32;
    fn sumi_background_thread();
}
pub fn names(s: &str) -> Vec<String> {
    let Ok(c) = CString::new(s) else {
        return vec![];
    };
    unsafe {
        let p = sumi_entities(c.as_ptr());
        if p.is_null() {
            return vec![];
        }
        let names = serde_json::from_slice(CStr::from_ptr(p).to_bytes()).unwrap_or_default();
        sumi_free(p);
        names
    }
}
pub fn idle() -> bool {
    unsafe { sumi_heavy_allowed() != 0 }
}
pub fn background_thread() {
    unsafe { sumi_background_thread() }
}
pub fn vectors() {
    static LOAD: Once = Once::new();
    LOAD.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(_, _, _) -> _,
        >(
            sqlite_vec::sqlite3_vec_init as *const ()
        )));
    })
}
pub(crate) fn free_bytes(path: &std::path::Path) -> u64 {
    std::process::Command::new("/bin/df")
        .args(["-Pk"])
        .arg(path)
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8(o.stdout)
                .ok()?
                .lines()
                .last()?
                .split_whitespace()
                .nth(3)?
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0)
        * 1024
}
pub fn fit(s: &Store) -> serde_json::Value {
    let bytes = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .and_then(|r| String::from_utf8(r.stdout).ok()?.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let disk = free_bytes(&s.root);
    let model_dir = std::env::var_os("OLLAMA_MODELS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(crate::sources::home);
    let ollama_disk = model_dir
        .ancestors()
        .find(|p| p.is_dir())
        .map(free_bytes)
        .unwrap_or(0);
    let catalog = crate::catalog::installed(s);
    let best = catalog["models"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| {
            let size = m["bytes"].as_u64().unwrap_or(0);
            bytes
                >= m["ram"]
                    .as_u64()
                    .map(|g| g * 1_073_741_824)
                    .unwrap_or(size * 2 + 4_000_000_000)
                && ollama_disk > size + 4_000_000_000
        });
    serde_json::json!({"memory":bytes,"freeDisk":disk,"recommendation":best})
}
pub fn sensitive(s: &str) -> bool {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)\b(cancer|diagnos\w*|medication|therap\w*|bipolar|diabet\w*|depress\w*|suicid\w*|sexual\w*|gay|lesbian|queer|religio\w*|christian|muslim|jewish|politic\w*|democrat\w*|republican\w*|vot(?:e|ed|ing)|financ\w*|salary|bank\w*|debt|income|invest\w*|mortgage)\b").unwrap()
    });
    RE.is_match(s)
}
// The encrypted vector table stores normalized local lexical features even when a neural model is offline.
pub fn embedding(t: &str) -> Vec<u8> {
    let mut v = [0f32; 192];
    let semantic = CString::new(t)
        .ok()
        .is_some_and(|c| unsafe { sumi_semantic(c.as_ptr(), v.as_mut_ptr(), 192) != 0 });
    if !semantic {
        for w in t
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            let digest = hash(&w.to_lowercase());
            let n = u32::from_str_radix(&digest[..8], 16).unwrap();
            v[n as usize % 192] += if n & 0x80000000 == 0 { 1. } else { -1. }
        }
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1.);
    v.iter().flat_map(|x| (x / norm).to_le_bytes()).collect()
}

pub fn notify_ready() {
    unsafe { sumi_notify_when_ready() }
}

pub fn request_notifications() {
    unsafe { sumi_request_notifications() }
}
