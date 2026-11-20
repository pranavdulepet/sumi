use crate::store::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};

const URL: Option<&str> = option_env!("SUMI_CATALOG_URL");
const KEY: Option<&str> = option_env!("SUMI_CATALOG_PUBLIC_KEY");
pub fn configured() -> bool {
    URL.is_some() == KEY.is_some()
}
pub fn installed(s: &Store) -> Value {
    s.get("catalog")
        .ok()
        .filter(|v| v["models"].is_array())
        .unwrap_or_else(|| {
            serde_json::from_str(include_str!("../../catalog/payload.json"))
                .expect("invalid bundled model catalog")
        })
}
pub fn verify(raw: &Value, key: &[u8], current: u64) -> Res<Value> {
    let payload = STANDARD
        .decode(raw["payload"].as_str().ok_or("Missing catalog payload.")?)
        .msg()?;
    let signature = STANDARD
        .decode(
            raw["signature"]
                .as_str()
                .ok_or("Missing catalog signature.")?,
        )
        .msg()?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(&payload, &signature)
        .map_err(|_| "The model catalog signature did not verify.")?;
    let data: Value = serde_json::from_slice(&payload).msg()?;
    if data["revision"].as_u64().is_none_or(|n| n < current) {
        return Err("Outdated model catalog.".into());
    }
    let rows = data["models"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 40)
        .ok_or("Invalid catalog size.")?;
    for row in rows {
        let name = row["name"].as_str().unwrap_or("");
        if name.is_empty()
            || name.len() > 100
            || name.contains("cloud")
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
            || row["bytes"]
                .as_u64()
                .is_none_or(|n| n < 100_000_000 || n > 300_000_000_000)
        {
            return Err("Invalid local model in catalog.".into());
        }
    }
    Ok(data)
}
pub fn refresh(s: &Store) -> Res<()> {
    if !configured() {
        return Ok(());
    }
    let url = URL.unwrap_or(
        "https://raw.githubusercontent.com/pranavdulepet/sumi/model-catalog/catalog/models.json",
    );
    let key = KEY
        .unwrap_or(include_str!("../../catalog/publisher.pub"))
        .trim();
    if s.get("catalogOff")? == true {
        return Ok(());
    }
    let url = url::Url::parse(url).msg()?;
    if url.scheme() != "https"
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("The catalog requires a publisher HTTPS address.".into());
    }
    let mut bytes = vec![];
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(8))
        .build()
        .get(url.as_str())
        .call()
        .map_err(|_| "Model catalog unavailable. Keeping the installed list.")?
        .into_reader()
        .take(65537)
        .read_to_end(&mut bytes)
        .msg()?;
    if bytes.len() > 65536 {
        return Err("Model catalog too large.".into());
    }
    let before = installed(s);
    let after = verify(
        &serde_json::from_slice(&bytes).msg()?,
        &STANDARD.decode(key).msg()?,
        before["revision"].as_u64().unwrap_or(1),
    )?;
    if after["revision"] != before["revision"] {
        s.set("catalog", &after)?;
        s.set("catalogUpdated", &json!(now()))?;
    }
    Ok(())
}

#[cfg(test)]
mod publisher {
    use super::*;
    #[test]
    fn shipped_catalog_signature_matches_its_exact_payload() {
        let envelope: Value =
            serde_json::from_str(include_str!("../../catalog/models.json")).unwrap();
        let public = STANDARD
            .decode(include_str!("../../catalog/publisher.pub").trim())
            .unwrap();
        assert_eq!(
            verify(&envelope, &public, 1).unwrap(),
            serde_json::from_str::<Value>(include_str!("../../catalog/payload.json")).unwrap()
        );
    }
}
