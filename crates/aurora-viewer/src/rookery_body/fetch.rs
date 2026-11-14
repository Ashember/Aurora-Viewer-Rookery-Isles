//! The body pack comes from the grid (GET /rookery/body/<file>) and is kept
//! in the viewer's cache (`cache/rookery-body/`). Every file is checked
//! against the grid's manifest (size and SHA-256): a file is fetched only
//! when it is missing or changed there, and one that does not match is never
//! used. The body itself (body.json, body.bin) is fetched up front; a skin,
//! hair or garment file only the first time it is worn.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Where the grid serves the pack, after its service address.
pub const PATH: &str = "/rookery/body/";
/// The files a body needs before anything else.
pub const CORE: [&str; 2] = ["body.json", "body.bin"];
/// The largest file a pack may hold.
const MAX_FILE: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub files: Vec<Entry>,
}

impl Manifest {
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.files.iter().find(|f| f.name == name)
    }
}

/// The cache folder of the pack, or a pack folder of this machine given by
/// AURORA_ROOKERY_PACK_DIR (development and the offline demo: the pack is
/// then read where it is, nothing downloaded).
pub fn folder() -> PathBuf {
    local_pack().unwrap_or_else(|| crate::settings::cache_dir().join("rookery-body"))
}

/// AURORA_ROOKERY_PACK_DIR, when it names a pack folder.
pub fn local_pack() -> Option<PathBuf> {
    let folder = PathBuf::from(std::env::var_os("AURORA_ROOKERY_PACK_DIR")?);
    folder.join("manifest.json").exists().then_some(folder)
}

/// The manifest of a pack folder of this machine.
pub fn local_manifest(folder: &Path) -> Result<Manifest, String> {
    let bytes = std::fs::read(folder.join("manifest.json")).map_err(|e| format!("body pack manifest: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("body pack manifest: {e}"))
}

/// The pack's address on a grid whose service is `service` (no trailing slash).
pub fn base_url(service: &str) -> String {
    format!("{}{PATH}", service.trim_end_matches('/'))
}

/// A relative pack path the cache may hold: no parent steps, no roots.
fn safe(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.contains(':')
        && name.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

fn digest(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    let mut hash = Sha256::new();
    hash.update(&data);
    Some(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether the cached copy of `entry` is the one the manifest names.
pub fn cached(folder: &Path, entry: &Entry) -> bool {
    let path = folder.join(&entry.name);
    std::fs::metadata(&path).is_ok_and(|m| m.len() == entry.bytes) && digest(&path).is_some_and(|d| d == entry.sha256)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .connect_timeout(std::time::Duration::from_secs(15))
        .user_agent(aurora_version::user_agent())
        .build()
        .map_err(|e| e.to_string())
}

/// The grid's manifest, saved beside the cached files.
pub async fn manifest(base: &str, folder: &Path) -> Result<Manifest, String> {
    let response = client()?
        .get(format!("{base}manifest.json"))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("body pack manifest: {e}"))?;
    let bytes = response.bytes().await.map_err(|e| format!("body pack manifest: {e}"))?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| format!("body pack manifest: {e}"))?;
    if manifest.version != super::pack::VERSION {
        return Err(format!(
            "the grid's body pack is version {} and this viewer reads {}: update the viewer",
            manifest.version,
            super::pack::VERSION
        ));
    }
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    std::fs::write(folder.join("manifest.json"), &bytes).map_err(|e| e.to_string())?;
    Ok(manifest)
}

/// Fetches `names` that the cache does not hold as the manifest has them.
/// Each file is written beside its final name and renamed once whole and
/// checked, so a broken download never stands in the cache.
pub async fn ensure(base: &str, folder: &Path, manifest: &Manifest, names: &[String]) -> Result<(), String> {
    let client = client()?;
    for name in names {
        let entry = manifest.entry(name).ok_or_else(|| format!("{name} is not in the grid's body pack"))?;
        if !safe(&entry.name) || entry.bytes > MAX_FILE {
            return Err(format!("{name}: refused"));
        }
        if cached(folder, entry) {
            continue;
        }
        let response = client
            .get(format!("{base}{}", entry.name))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| format!("{name}: {e}"))?;
        let bytes = response.bytes().await.map_err(|e| format!("{name}: {e}"))?;
        if bytes.len() as u64 != entry.bytes {
            return Err(format!("{name}: {} bytes, the manifest says {}", bytes.len(), entry.bytes));
        }
        let mut hash = Sha256::new();
        hash.update(&bytes);
        let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != entry.sha256 {
            return Err(format!("{name}: its contents do not match the manifest"));
        }
        let path = folder.join(&entry.name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let partial = path.with_extension("part");
        std::fs::write(&partial, &bytes).map_err(|e| format!("{name}: {e}"))?;
        std::fs::rename(&partial, &path).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(())
}

/// The names the cache still needs among `names` (for progress).
#[cfg_attr(not(test), allow(dead_code))]
pub fn missing(folder: &Path, manifest: &Manifest, names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|n| manifest.entry(n).is_none_or(|e| !cached(folder, e)))
        .cloned()
        .collect()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_relative_names_reach_the_cache() {
        for good in ["body.json", "textures/skin-a.jpg", "content/hair-b.bin"] {
            assert!(safe(good), "{good}");
        }
        for bad in ["", "/etc/passwd", "../x", "a/../../b", "C:/x", "a\\b", "a//b", "./a"] {
            assert!(!safe(bad), "{bad}");
        }
        assert_eq!(base_url("http://grid.example:42000/"), "http://grid.example:42000/rookery/body/");
    }

    #[test]
    fn a_cached_file_counts_only_when_size_and_hash_match() {
        let dir = std::env::temp_dir().join(format!("rookery-body-fetch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("a.bin"), b"hello").expect("write");
        let right = Entry {
            name: "a.bin".into(),
            bytes: 5,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into(),
        };
        assert!(cached(&dir, &right));
        let wrong = Entry {
            sha256: "00".repeat(32),
            ..right.clone()
        };
        assert!(!cached(&dir, &wrong));
        let manifest = Manifest {
            version: 1,
            files: vec![right, Entry { name: "b.bin".into(), ..wrong }],
        };
        assert_eq!(missing(&dir, &manifest, &["a.bin".into(), "b.bin".into()]), ["b.bin"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
