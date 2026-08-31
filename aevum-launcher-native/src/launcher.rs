//! Real Minecraft launcher engine: Mojang metadata, downloads, and Java process launch.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::Value;

pub const MANIFEST_URL: &str = "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json";
const RESOURCES_URL: &str = "https://resources.download.minecraft.net/";
const UA: &str = "AevumLauncher/2.1.0";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Phase {
    #[default]
    Idle,
    Fetching,
    Downloading,
    Extracting,
    Launching,
    Running,
    Exited,
    Error,
}

#[derive(Clone, Default)]
pub struct LaunchReport {
    pub phase: Phase,
    pub message: String,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    /// Tail of the game log + crash-report summary when the game exits non-zero.
    pub crash_tail: Option<String>,
}

impl LaunchReport {
    pub fn progress_pct(&self) -> f32 {
        if self.bytes_total == 0 {
            return 0.0;
        }
        (self.bytes_done as f32 / self.bytes_total as f32).min(1.0)
    }
}

#[derive(Clone)]
pub struct Profile {
    pub version_id: String,
    pub username: String,
    pub ram_mb: u32,
}

#[derive(Clone)]
pub struct VersionEntry {
    pub id: String,
    pub kind: String,
    pub url: String,
}

// ---- paths -----------------------------------------------------------

pub fn root_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    PathBuf::from(home).join(".aevum-launcher")
}

pub fn game_dir() -> PathBuf {
    root_dir().join("game")
}

pub fn libs_dir() -> PathBuf {
    root_dir().join("libraries")
}

pub fn assets_dir() -> PathBuf {
    root_dir().join("assets")
}

pub fn natives_dir() -> PathBuf {
    root_dir().join("natives")
}

pub fn versions_dir() -> PathBuf {
    root_dir().join("versions")
}

pub fn manifest_path() -> PathBuf {
    root_dir().join("version_manifest.json")
}

// ---- manifest ---------------------------------------------------------

pub fn fetch_manifest() -> Result<Vec<VersionEntry>, String> {
    let data = http_get_bytes(MANIFEST_URL)?;
    std::fs::create_dir_all(root_dir()).map_err(|e| e.to_string())?;
    let _ = std::fs::write(manifest_path(), &data);
    parse_manifest(&data)
}

pub fn load_cached_manifest() -> Result<Vec<VersionEntry>, String> {
    let data = std::fs::read(manifest_path()).map_err(|e| e.to_string())?;
    parse_manifest(&data)
}

pub fn parse_manifest(data: &[u8]) -> Result<Vec<VersionEntry>, String> {
    let v: Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    let arr = v
        .get("versions")
        .and_then(|x| x.as_array())
        .ok_or("manifest has no versions")?;
    let mut out = Vec::with_capacity(arr.len());
    for e in arr {
        out.push(VersionEntry {
            id: e.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            kind: e.get("type").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            url: e.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        });
    }
    Ok(out)
}

// ---- http ------------------------------------------------------------

fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = ureq::get(url)
        .set("User-Agent", UA)
        .call()
        .map_err(|e| format!("request failed for {}: {}", url, e))?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(512 * 1024 * 1024)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

fn set(report: &Arc<Mutex<LaunchReport>>, phase: Phase, message: String) {
    if let Ok(mut r) = report.lock() {
        r.phase = phase;
        r.message = message;
    }
}

fn set_totals(report: &Arc<Mutex<LaunchReport>>, bytes_total: u64) {
    if let Ok(mut r) = report.lock() {
        r.bytes_total = bytes_total;
    }
}

fn set_error(report: &Arc<Mutex<LaunchReport>>, err: String) {
    if let Ok(mut r) = report.lock() {
        r.phase = Phase::Error;
        r.error = Some(err);
    }
}

fn set_running(report: &Arc<Mutex<LaunchReport>>, pid: u32) {
    if let Ok(mut r) = report.lock() {
        r.phase = Phase::Running;
        r.pid = Some(pid);
    }
}

fn set_exited(report: &Arc<Mutex<LaunchReport>>, code: Option<i32>) {
    if let Ok(mut r) = report.lock() {
        r.phase = Phase::Exited;
        r.exit_code = code;
        r.pid = None;
        if code != Some(0) {
            r.crash_tail = game_exit_note();
        }
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_esc = false;
    for ch in s.chars() {
        if in_esc {
            if ch == 'm' {
                in_esc = false;
            }
            continue;
        }
        if ch == '\x1b' {
            in_esc = true;
            continue;
        }
        out.push(ch);
    }
    out
}

/// Priority of a log line as crash evidence. Lower = more important.
fn line_priority(l: &str) -> Option<u8> {
    let up = l.to_uppercase();
    if up.contains("EXCEPTION IN THREAD")
        || up.contains("FATAL ERROR")
        || up.contains("ERROR IN THREAD")
    {
        Some(0)
    } else if l.starts_with("java.")
        || up.starts_with("CAUSED BY")
        || up.contains("UNSATISFIEDLINK")
        || up.contains("NOCLASSDEF")
    {
        Some(1)
    } else if up.contains("GLFW") {
        Some(2)
    } else if up.contains("PROGRAM WILL EXIT") || up.contains("A FATAL EXCEPTION") {
        Some(8)
    } else if up.contains("UNRECOGNIZED OPTION")
        || up.contains("COULD NOT CREATE")
        || up.contains("INITIALIZATION OF VM")
        || up.contains("RESERVE ENOUGH SPACE")
        || up.contains("INSIDE JVM")
        || up.contains("UNSUPPORTED")
        || up.contains("OUTOFMEMORY")
        || up.contains("ACCESSDENIED")
    {
        Some(1)
    } else if up.contains("EXCEPTION") || up.contains("FATAL") {
        Some(3)
    } else if up.contains("ERROR") {
        Some(4)
    } else if up.contains("FAILED") {
        Some(5)
    } else if up.trim_start().starts_with("at ") || up.trim_start().starts_with("...") {
        Some(6)
    } else {
        None
    }
}

/// Extract a readable crash summary from the game's log and crash-reports.
fn game_exit_note() -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    // Tail of latest.log: pick the highest-priority evidence lines in order.
    let log_path = game_dir().join("logs").join("latest.log");
    if let Ok(f) = std::fs::File::open(&log_path) {
        use std::io::{BufRead, BufReader, Seek, SeekFrom};
        let len = f.metadata().ok().map(|m| m.len()).unwrap_or(0);
        let start = len.saturating_sub(128 * 1024);
        let mut br = BufReader::new(f);
        let _ = br.seek(SeekFrom::Start(start));
        let lines: Vec<String> = br
            .lines()
            .flatten()
            .map(|l| strip_ansi(&l))
            .filter(|l| !l.trim().is_empty())
            .collect();
        let lines = if lines.len() > 500 {
            lines[lines.len() - 500..].to_vec()
        } else {
            lines
        };

        let mut scored: Vec<(u8, String)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for l in lines.iter() {
            if let Some(p) = line_priority(l) {
                let key = l.trim().to_string();
                if seen.insert(key) {
                    scored.push((p, l.clone()));
                }
            }
        }
        scored.sort_by_key(|(p, _)| *p);
        let mut joined: String = String::new();
        for (_, l) in scored.iter().take(8) {
            let mut line = l.trim().to_string();
            if line.len() > 220 {
                line.truncate(220);
            }
            if !joined.is_empty() {
                joined.push('\n');
            }
            joined.push_str(&line);
        }
        if joined.is_empty() {
            joined = lines.iter().rev().take(6).cloned().collect::<Vec<_>>().join("\n");
        }
        if joined.len() > 1600 {
            joined.truncate(1600);
        }
        if !joined.trim().is_empty() {
            parts.push(joined);
        }
    }

    // Newest crash-report, if any: grab the description and first exception.
    if let Ok(rd) = std::fs::read_dir(game_dir().join("crash-reports")) {
        let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("txt") {
                continue;
            }
            if let Ok(meta) = e.metadata() {
                if let Ok(t) = meta.modified() {
                    if newest.as_ref().map(|(nt, _)| t > *nt).unwrap_or(true) {
                        newest = Some((t, p));
                    }
                }
            }
        }
        if let Some((_, p)) = newest {
            if let Ok(content) = std::fs::read_to_string(p) {
                let mut summary = String::new();
                let mut desc: Option<String> = None;
                let mut exc: Option<String> = None;
                for line in content.lines().take(24) {
                    let t = line.trim();
                    if desc.is_none() && t.starts_with("Description:") {
                        desc = Some(t.trim_start_matches("Description:").trim().to_string());
                    }
                    if exc.is_none() && t.starts_with("java.") && t.contains("Exception") {
                        exc = Some(t.to_string());
                    }
                }
                if let Some(d) = desc {
                    summary.push_str(&format!("crash-reports: {}", d));
                    if let Some(e) = &exc {
                        summary.push_str(&format!(" — {}", e));
                    }
                }
                if !summary.is_empty() {
                    parts.push(summary);
                }
            }
        }
    }

    let summary = if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    };

    // Persist a copy the user can open/copy: game/logs/crash-latest.txt
    if let Some(s) = &summary {
        use std::io::Write;
        let crash_file = game_dir().join("logs").join("crash-latest.txt");
        if let Some(parent) = crash_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::File::create(crash_file) {
            let _ = f.write_all(s.as_bytes());
            let _ = f.write_all(b"\n");
        }
    }

    summary
}

fn file_sha1(path: &Path) -> Result<String, String> {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    let d = h.finalize();
    Ok(d.iter().map(|b| format!("{:02x}", b)).collect())
}

/// Download `url` to `dest`. Skips when a valid cached copy exists. Reports bytes to `report`.
fn download_to(
    report: &Arc<Mutex<LaunchReport>>,
    url: &str,
    dest: &Path,
    sha1: Option<&str>,
    size: Option<u64>,
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if dest.exists() {
        if let Some(h) = sha1 {
            if let Ok(digest) = file_sha1(dest) {
                if digest == h {
                    if let Ok(mut r) = report.lock() {
                        r.bytes_done += size.unwrap_or(0);
                    }
                    return Ok(());
                }
            }
        } else {
            if let Ok(mut r) = report.lock() {
                r.bytes_done += size.unwrap_or(0);
            }
            return Ok(());
        }
    }
    let resp = ureq::get(url)
        .set("User-Agent", UA)
        .call()
        .map_err(|e| format!("request failed for {}: {}", url, e))?;
    let mut reader = resp.into_reader();
    let tmp = dest.with_extension("part");
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 64 * 1024];
    let mut acc: u64 = 0;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        use std::io::Write;
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        acc += n as u64;
        if let Ok(mut r) = report.lock() {
            r.bytes_done += n as u64;
        }
    }
    drop(f);
    if let Some(h) = sha1 {
        let digest = file_sha1(&tmp)?;
        if digest != h {
            return Err(format!(
                "checksum mismatch for {} (expected {}, got {})",
                dest.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default(),
                h,
                digest
            ));
        }
    }
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    let _ = acc;
    Ok(())
}

// ---- platform helpers -------------------------------------------------

fn os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

fn os_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    }
}

fn java_exe() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

fn find_java() -> Option<PathBuf> {
    if let Ok(jh) = std::env::var("JAVA_HOME") {
        let p = PathBuf::from(jh).join("bin").join(java_exe());
        if p.exists() {
            return Some(p);
        }
    }
    let path = std::env::var("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ';' } else { ':' };
    for dir in path.split(sep) {
        let p = PathBuf::from(dir).join(java_exe());
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Collapse multi-line process output into a single line for UI messages.
fn one_line(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        return "unknown error".to_string();
    }
    t.lines().rev().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()).unwrap_or_default()
}

/// Check the JVM can actually create a VM with the given max heap.
/// Runs `java -Xmx<heap>M -version`; the heap is reserved up front, so this
/// reproduces "Could not create the Java Virtual Machine" failures exactly.
fn validate_jvm(java: &Path, heap_mb: u32) -> Result<(), String> {
    let out = Command::new(java)
        .arg(format!("-Xmx{}M", heap_mb.max(256)))
        .arg("-version")
        .output()
        .map_err(|e| format!("failed to run java ({})", e))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if stderr.is_empty() {
        Err(format!("JVM rejected a {} MB heap", heap_mb))
    } else {
        Err(stderr)
    }
}

/// Parse the installed Java's major version from `java -version`.
/// Handles both `openjdk version "17.0.5"` and legacy `java version "1.8.0_352"`.
fn java_major_version(java: &Path) -> Result<u16, String> {
    let out = Command::new(java)
        .arg("-version")
        .output()
        .map_err(|e| format!("failed to run java ({})", e))?;
    let raw = if out.stderr.is_empty() {
        out.stdout
    } else {
        out.stderr
    };
    let s = String::from_utf8_lossy(&raw);
    for line in s.lines() {
        let line = line.trim();
        let start = match line.find("version \"") {
            Some(i) => i + "version \"".len(),
            None => continue,
        };
        let rest = match line[start..].find('"') {
            Some(end) => &line[start..start + end],
            None => &line[start..],
        };
        let mut it = rest.split('.');
        let first = it.next().unwrap_or("").parse::<u16>().unwrap_or(0);
        if first == 1 {
            // Legacy naming: "1.8.0_352" -> Java 8.
            if let Some(second) = it.next() {
                if let Ok(v) = second.parse::<u16>() {
                    return Ok(v);
                }
            }
            return Ok(first);
        }
        return Ok(first);
    }
    Err("could not parse `java -version` output".into())
}

/// Drop any JVM arg the installed JVM rejects. Newer Minecraft versions ship
/// experimental flags (e.g. `--sun-misc-unsafe-memory-access=allow`,
/// `--enable-native-access=ALL-UNNAMED`) that older JVMs refuse with
/// "Unrecognized option"; without this the whole launch dies on the first
/// unsupported flag. Probes `java <args> -version` and removes offenders.
fn strip_unsupported_jvm_args(java: &Path, jvm: &mut Vec<String>) {
    loop {
        let out = match Command::new(java).args(jvm.iter()).arg("-version").output() {
            Ok(o) => o,
            Err(_) => return,
        };
        if out.status.success() {
            return;
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let offending = stderr
            .lines()
            .find_map(|l| l.trim().strip_prefix("Unrecognized option: ").map(|s| s.trim().to_string()));
        let Some(offending) = offending else { return };
        if let Some(pos) = jvm.iter().position(|a| *a == offending) {
            jvm.remove(pos);
        } else {
            return;
        }
    }
}

fn rules_allow(rules: &[Value]) -> bool {
    if rules.is_empty() {
        return true;
    }
    let os = os_name();
    let arch = os_arch();
    let mut allow = false;
    for r in rules {
        let action = r.get("action").and_then(|a| a.as_str()).unwrap_or("allow");
        let matches = match r.get("os") {
            Some(o) => {
                let name_ok = match o.get("name").and_then(|n| n.as_str()) {
                    Some(n) => n == os,
                    None => true,
                };
                let arch_ok = match o.get("arch").and_then(|a| a.as_str()) {
                    Some(a) => a == arch || (a == "x86" && arch == "x86_64"),
                    None => true,
                };
                name_ok && arch_ok
            }
            None => true,
        };
        // This launcher never enables launcher features (no demo mode, no
        // custom resolution, no quick play), so any rule that requires a
        // feature to be true (e.g. 26.2's quick-play args) must not match.
        // Otherwise all four --quickPlay* options are passed at once and the
        // game aborts with "Only one quick play option can be specified".
        let features_ok = match r.get("features") {
            Some(f) => match f.as_object() {
                Some(obj) => obj.values().all(|v| v.as_bool() == Some(false)),
                None => true,
            },
            None => true,
        };
        if matches && features_ok {
            allow = action == "allow";
        }
    }
    allow
}

fn offline_uuid(name: &str) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(format!("OfflinePlayer:{}", name));
    let hex: String = h.finalize().iter().map(|b| format!("{:02x}", b)).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

// ---- asset / natives --------------------------------------------------

fn download_asset_index(
    report: &Arc<Mutex<LaunchReport>>,
    vj: &Value,
) -> Result<(String, Vec<(String, String, u64)>), String> {
    let idx = vj.get("assetIndex").ok_or("version has no asset index")?;
    let idx_id = idx.get("id").and_then(|x| x.as_str()).unwrap_or("legacy").to_string();
    let idx_url = idx.get("url").and_then(|x| x.as_str()).ok_or("no asset index url")?.to_string();
    let idx_sha = idx.get("sha1").and_then(|x| x.as_str());
    let idx_size = idx.get("size").and_then(|x| x.as_u64());
    let idx_path = assets_dir().join("indexes").join(format!("{}.json", idx_id));
    if !idx_path.exists() {
        download_to(report, &idx_url, &idx_path, idx_sha, idx_size)?;
    }
    let data = std::fs::read(&idx_path).map_err(|e| e.to_string())?;
    let parsed: Value = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
    let mut objects = Vec::new();
    if let Some(map) = parsed.get("objects").and_then(|o| o.as_object()) {
        for (path, meta) in map {
            let hash = meta.get("hash").and_then(|h| h.as_str()).unwrap_or("").to_string();
            let size = meta.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
            if !hash.is_empty() {
                objects.push((path.clone(), hash, size));
            }
        }
    }
    Ok((idx_id, objects))
}

fn extract_natives_from_bytes(bytes: Vec<u8>, out_dir: &Path) -> Result<(), String> {
    use std::io::{Cursor, Write};
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = f.name().to_string();
        if name.ends_with('/') || name.contains("META-INF") {
            continue;
        }
        let out_path = out_dir.join(&name);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
        std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
        let _ = Write::flush(&mut out);
    }
    Ok(())
}

// ---- command building -------------------------------------------------

fn replace_tokens(s: &str, prof: &Profile, uuid: &str, version: &str, vtype: &str, toks: &[(String, String)]) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < s.len() {
        let c = bytes[i] as char;
        if c == '$' && i + 1 < s.len() && s.as_bytes()[i + 1] == b'{' {
            if let Some(end) = s[i + 2..].find('}') {
                let key = &s[i + 2..i + 2 + end];
                let val = match key {
                    "auth_player_name" => prof.username.clone(),
                    "profile_name" => prof.username.clone(),
                    "auth_uuid" => uuid.to_string(),
                    "auth_access_token" => "0".to_string(),
                    "auth_session" => "0".to_string(),
                    "auth_xuid" => "0".to_string(),
                    "clientid" => "0".to_string(),
                    "user_type" => "legacy".to_string(),
                    "user_properties" => "{}".to_string(),
                    "version_name" => version.to_string(),
                    "version_type" => vtype.to_string(),
                    "game_directory" => game_dir().to_string_lossy().to_string(),
                    "assets_root" => assets_dir().to_string_lossy().to_string(),
                    "assets_index_name" => toks.iter().find(|(k, _)| k == "assets_index").map(|(_, v)| v.clone()).unwrap_or_default(),
                    "natives_directory" => toks.iter().find(|(k, _)| k == "natives").map(|(_, v)| v.clone()).unwrap_or_default(),
                    "classpath" => toks.iter().find(|(k, _)| k == "classpath").map(|(_, v)| v.clone()).unwrap_or_default(),
                    "library_directory" => libs_dir().to_string_lossy().to_string(),
                    "launcher_name" => "AevumLauncher".to_string(),
                    "launcher_version" => "2.1.0".to_string(),
                    "resolution_width" => "854".to_string(),
                    "resolution_height" => "480".to_string(),
                    _ => {
                        if key.starts_with("feature.") {
                            "false".to_string()
                        } else {
                            format!("${{{}}}", key)
                        }
                    }
                };
                out.push_str(&val);
                i += 2 + end + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    let _ = (uuid, vtype);
    out
}

fn build_command(
    prof: &Profile,
    vj: &Value,
    classpath: &str,
    natives_path: &Path,
    asset_index_id: &str,
    version: &str,
    vtype: &str,
    java: Option<&Path>,
) -> Result<Vec<String>, String> {
    let uuid = offline_uuid(&prof.username);

    let mut jvm: Vec<String> = Vec::new();
    jvm.push(format!("-Xmx{}M", prof.ram_mb.max(256)));
    jvm.push(format!("-Djava.library.path={}", natives_path.display()));
    jvm.push("-cp".to_string());
    jvm.push(classpath.to_string());

    if let Some(log) = vj.get("logging").and_then(|l| l.get("client")).and_then(|c| c.get("file")) {
        if let Some(url) = log.get("url").and_then(|u| u.as_str()) {
            let dest = game_dir().join("log4j2.xml");
            let _ = download_logging(url, &dest);
            jvm.push(format!("-Dlog4j.configurationFile={}", dest.display()));
        }
    }

    let toks: Vec<(String, String)> = vec![
        ("assets_index".to_string(), asset_index_id.to_string()),
        ("natives".to_string(), natives_path.to_string_lossy().to_string()),
        ("classpath".to_string(), classpath.to_string()),
    ];

    let mut game: Vec<String> = Vec::new();

    if let Some(args) = vj.get("arguments").and_then(|a| a.as_object()) {
        if let Some(list) = args.get("jvm").and_then(|j| j.as_array()) {
            for a in list {
                if let Some(s) = a.as_str() {
                    jvm.push(replace_tokens(s, prof, &uuid, version, vtype, &toks));
                } else {
                    let rules = a.get("rules").and_then(|r| r.as_array()).map(|r| r.as_slice()).unwrap_or(&[]);
                    if rules_allow(rules) {
                        if let Some(vals) = a.get("value").and_then(|v| v.as_array()) {
                            for v in vals {
                                if let Some(s) = v.as_str() {
                                    jvm.push(replace_tokens(s, prof, &uuid, version, vtype, &toks));
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(list) = args.get("game").and_then(|g| g.as_array()) {
            for a in list {
                if let Some(s) = a.as_str() {
                    game.push(replace_tokens(s, prof, &uuid, version, vtype, &toks));
                } else {
                    let rules = a.get("rules").and_then(|r| r.as_array()).map(|r| r.as_slice()).unwrap_or(&[]);
                    if rules_allow(rules) {
                        if let Some(vals) = a.get("value").and_then(|v| v.as_array()) {
                            for v in vals {
                                if let Some(s) = v.as_str() {
                                    game.push(replace_tokens(s, prof, &uuid, version, vtype, &toks));
                                }
                            }
                        }
                    }
                }
            }
        }
    } else if let Some(mc) = vj.get("minecraftArguments").and_then(|m| m.as_str()) {
        for t in mc.split_whitespace() {
            game.push(replace_tokens(t, prof, &uuid, version, vtype, &toks));
        }
    }

    let main_class = vj
        .get("mainClass")
        .and_then(|m| m.as_str())
        .unwrap_or("net.minecraft.client.main.Main")
        .to_string();

    if let Some(j) = java {
        strip_unsupported_jvm_args(j, &mut jvm);
    }

    let mut full = Vec::new();
    full.extend(jvm);
    full.push(main_class);
    full.extend(game);
    Ok(full)
}

fn download_logging(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if dest.exists() {
        return Ok(());
    }
    let resp = ureq::get(url).set("User-Agent", UA).call().map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut out = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    std::io::copy(&mut reader, &mut out).map_err(|e| e.to_string())?;
    Ok(())
}

// ---- main pipeline -----------------------------------------------------

pub fn run_launch(profile: Profile, report: Arc<Mutex<LaunchReport>>) {
    if let Ok(mut r) = report.lock() {
        *r = LaunchReport::default();
    }
    if let Err(e) = do_launch(&profile, &report) {
        set_error(&report, e);
    }
}

fn do_launch(profile: &Profile, report: &Arc<Mutex<LaunchReport>>) -> Result<(), String> {
    set(report, Phase::Fetching, "Resolving version metadata".into());

    // Version JSON (cached).
    let vj_path = versions_dir()
        .join(&profile.version_id)
        .join(format!("{}.json", profile.version_id));
    let manifest = load_cached_manifest()
        .or_else(|_| fetch_manifest())
        .map_err(|e| format!("could not load version manifest: {}", e))?;
    let entry = manifest
        .iter()
        .find(|v| v.id == profile.version_id)
        .ok_or_else(|| format!("version not found in manifest: {}", profile.version_id))?;
    if !vj_path.exists() {
        set(report, Phase::Fetching, format!("Fetching {} metadata", profile.version_id));
        download_to(report, &entry.url, &vj_path, None, None)?;
    }
    let data = std::fs::read(&vj_path).map_err(|e| e.to_string())?;
    let vj: Value = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
    let version = vj.get("id").and_then(|i| i.as_str()).unwrap_or(&profile.version_id).to_string();
    let vtype = vj.get("type").and_then(|t| t.as_str()).unwrap_or("release").to_string();

    // Client jar.
    let client = vj.get("downloads").and_then(|d| d.get("client")).ok_or("no client download entry")?;
    let client_url = client.get("url").and_then(|u| u.as_str()).ok_or("no client jar url")?;
    let client_sha = client.get("sha1").and_then(|s| s.as_str());
    let client_size = client.get("size").and_then(|s| s.as_u64());
    let client_path = versions_dir().join(&version).join(format!("{}.jar", version));

    // Collect libraries + natives (honoring rules).
    let mut bytes_total = client_size.unwrap_or(0);
    let mut libs: Vec<(String, String, Option<String>, Option<u64>)> = Vec::new();
    let mut native_jars: Vec<(String, String)> = Vec::new();
    let libs_arr = vj.get("libraries").and_then(|l| l.as_array()).cloned().unwrap_or_default();
    for lib in &libs_arr {
        let rules = lib.get("rules").and_then(|r| r.as_array()).map(|r| r.as_slice()).unwrap_or(&[]);
        if !rules_allow(rules) {
            continue;
        }
        if let Some(dl) = lib.get("downloads").and_then(|d| d.as_object()) {
            if let Some(art) = dl.get("artifact") {
                if let Some(url) = art.get("url").and_then(|u| u.as_str()) {
                    let path = art.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
                    let sha = art.get("sha1").and_then(|s| s.as_str()).map(String::from);
                    let size = art.get("size").and_then(|s| s.as_u64());
                    bytes_total += size.unwrap_or(0);
                    libs.push((path, url.to_string(), sha, size));
                }
            }
            if let Some(cls) = dl.get("classifiers") {
                let key = format!("natives-{}", os_name());
                if let Some(obj) = cls.get(&key) {
                    if let Some(url) = obj.get("url").and_then(|u| u.as_str()) {
                        let sha = obj.get("sha1").and_then(|s| s.as_str()).unwrap_or("").to_string();
                        bytes_total += obj.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                        native_jars.push((url.to_string(), sha));
                    }
                }
            }
        }
    }

    // Asset index + objects.
    let (asset_index_id, objects) = download_asset_index(report, &vj)?;
    let mut assets_total: u64 = 0;
    for (_, _, size) in &objects {
        assets_total += size;
    }
    bytes_total += assets_total;

    set_totals(report, bytes_total);

    // Libraries.
    set(report, Phase::Downloading, format!("Downloading {} libraries", libs.len()));
    for (i, (path, url, sha, size)) in libs.iter().enumerate() {
        set(
            report,
            Phase::Downloading,
            format!("Library {}/{} — {}", i + 1, libs.len(), Path::new(path).file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default()),
        );
        download_to(report, url, &libs_dir().join(path), sha.as_deref(), *size)?;
        if let Ok(mut r) = report.lock() {
            r.files_done = (i + 1) as u64;
        }
    }

    // Client jar.
    download_to(report, client_url, &client_path, client_sha, client_size)?;

    // Assets.
    set(
        report,
        Phase::Downloading,
        format!("Downloading assets — {} objects", objects.len()),
    );
    for (i, (_, hash, size)) in objects.iter().enumerate() {
        let dest = assets_dir().join("objects").join(&hash[..2]).join(hash);
        let url = format!("{}{}/{}", RESOURCES_URL, &hash[..2], hash);
        download_to(report, &url, &dest, Some(hash), Some(*size))?;
        if let Ok(mut r) = report.lock() {
            r.files_done = (i + 1) as u64;
        }
    }

    // Natives.
    set(report, Phase::Extracting, "Preparing native libraries".into());
    let nat_dir = natives_dir().join(&version);
    std::fs::create_dir_all(&nat_dir).map_err(|e| e.to_string())?;
    for (url, _sha) in &native_jars {
        let bytes = http_get_bytes(url)?;
        extract_natives_from_bytes(bytes, &nat_dir)?;
    }

    // Launch.
    set(report, Phase::Launching, "Building launch command".into());
    let java = find_java().ok_or_else(|| {
        "No Java runtime found. Install Java 17 or newer (Java 21 recommended) and ensure it is on PATH or set JAVA_HOME.".to_string()
    })?;

    // Newer versions ship JVM flags older runtimes reject (e.g. 26.2's
    // `--sun-misc-unsafe-memory-access=allow` needs Java 22+). Fail fast with a
    // clear message instead of the JVM's own "Unrecognized option" crash.
    if let Some(req) = vj
        .get("javaVersion")
        .and_then(|j| j.get("majorVersion"))
        .and_then(|m| m.as_u64())
    {
        if let Ok(found) = java_major_version(&java) {
            if (found as u64) < req {
                return Err(format!(
                    "{} requires Java {} but the installed Java is version {}. \
                     Install a matching 64-bit Java runtime (Java {} or newer), \
                     restart the launcher, and try again.",
                    version, req, found, req
                ));
            }
        }
    }

    if cfg!(target_os = "linux") {
        let has_display = std::env::var("DISPLAY")
            .map(|d| !d.trim().is_empty())
            .unwrap_or(false);
        let has_wayland = std::env::var("WAYLAND_DISPLAY")
            .map(|d| !d.trim().is_empty())
            .unwrap_or(false);
        if !has_display && !has_wayland {
            return Err(
                "No display server available (DISPLAY/WAYLAND_DISPLAY is not set). \
                 Minecraft opens its own window, so it needs a graphical desktop session. \
                 Launch from a desktop environment, or export DISPLAY (e.g. ':0')."
                    .to_string(),
            );
        }
    }

    // The JVM reserves the requested max heap up front. On machines with little RAM
    // (or a 32-bit JVM) a large -Xmx makes java exit with "Could not create the Java
    // Virtual Machine". Probe the JVM with the requested heap and fall back to smaller
    // values so the game can still start.
    let requested_heap = profile.ram_mb.max(256);
    let mut heap_mb = requested_heap;
    match validate_jvm(&java, heap_mb) {
        Ok(()) => {}
        Err(raw) => {
            let mut recovered: Option<u32> = None;
            while heap_mb > 512 {
                heap_mb /= 2;
                if validate_jvm(&java, heap_mb).is_ok() {
                    recovered = Some(heap_mb);
                    break;
                }
            }
            if let Some(m) = recovered {
                set(
                    report,
                    Phase::Launching,
                    format!(
                        "{} MB heap unavailable ({}) — using {} MB",
                        requested_heap,
                        one_line(&raw),
                        m
                    ),
                );
            } else {
                return Err(format!(
                    "Java could not create a virtual machine with any heap size. \
                     Last error: {}. Install a 64-bit Java runtime (Java 17 or newer) \
                     with enough RAM for Minecraft.",
                    one_line(&raw)
                ));
            }
        }
    }

    let mut cp: Vec<PathBuf> = libs
        .iter()
        .map(|(path, _, _, _)| libs_dir().join(path))
        .collect();
    cp.push(client_path.clone());
    let sep = if cfg!(windows) { ";" } else { ":" };
    let classpath = cp
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(sep);

    let mut eff_profile = profile.clone();
    eff_profile.ram_mb = heap_mb;
    let cmd = build_command(&eff_profile, &vj, &classpath, &nat_dir, &asset_index_id, &version, &vtype, Some(&java))?;

    set(
        report,
        Phase::Launching,
        format!("Launching {} on {}", version, java.display()),
    );
    std::fs::create_dir_all(game_dir()).map_err(|e| e.to_string())?;
    let log_file = game_dir().join("logs").join("latest.log");
    if let Some(parent) = log_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let out = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_file)
        .map_err(|e| format!("failed to open game log {}: {}", log_file.display(), e))?;
    let mut child: Child = Command::new(&java)
        .args(&cmd)
        .current_dir(game_dir())
        .stdout(Stdio::from(out.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(out))
        .spawn()
        .map_err(|e| format!("failed to start java: {}", e))?;

    let pid = child.id();
    set_running(report, pid);
    let status = child.wait().map_err(|e| e.to_string())?;
    set_exited(report, status.code());
    Ok(())
}

/// Terminate a running game process.
pub fn kill_pid(pid: u32) {
    if cfg!(windows) {
        let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).status();
    } else {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn crash_note_prioritizes_exception_over_trailer() {
        let log = game_dir().join("logs").join("latest.log");
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        let saved = std::fs::read_to_string(&log).unwrap_or_default();
        {
            let mut f = std::fs::File::create(&log).unwrap();
            let _ = f.write_all(b"[12:00:00] [Render thread/INFO]: Setting user: NovaPilot\n");
            let _ = f.write_all(b"Exception in thread \"main\" java.lang.UnsatisfiedLinkError: Could not load library\n");
            let _ = f.write_all(b"\tat com.mojang.foo.bar(SourceFile:1)\n");
            let _ = f.write_all(b"Caused by: java.lang.ExceptionInInitializerError\n");
            let _ = f.write_all(b"A fatal exception has occurred. Program will exit.\n");
        }
        let note = game_exit_note().expect("note should be produced");
        assert!(note.contains("UnsatisfiedLinkError"), "missing cause: {note}");
        assert!(note.contains("A fatal exception has occurred"));
        let cause = note.find("UnsatisfiedLinkError").unwrap();
        let trailer = note.find("A fatal exception").unwrap();
        assert!(cause < trailer, "exception line must precede trailer:\n{note}");
        std::fs::write(&log, saved).ok();
    }

    #[test]
    fn java_major_version_parses() {
        let Some(java) = find_java() else {
            return; // no java installed in this environment; nothing to assert
        };
        let v = java_major_version(&java).unwrap();
        assert!(v >= 8, "unexpected java major {v}");
    }

    #[test]
    fn feature_gated_rules_never_match() {
        let quick_play: Vec<Value> = serde_json::from_str(r#"[{"action":"allow","features":{"is_quick_play_singleplayer":true},"value":["--quickPlaySingleplayer","x"]}]"#).unwrap();
        assert!(!rules_allow(&quick_play), "feature=required true must not match");

        let demo: Vec<Value> = serde_json::from_str(r#"[{"action":"allow","features":{"is_demo_user":true}}]"#).unwrap();
        assert!(!rules_allow(&demo), "demo rule must not match");

        let plain_os: Vec<Value> = serde_json::from_str(r#"[{"action":"allow","os":{"name":"linux"}}]"#).unwrap();
        assert!(rules_allow(&plain_os), "plain os rule still matches");
    }

    #[test]
    fn strips_unsupported_jvm_args() {
        let Some(java) = find_java() else {
            return; // no java installed in this environment; nothing to assert
        };
        let mut args = vec![
            "-Xmx1G".to_string(),
            "--definitely-not-a-real-jvm-flag=1".to_string(),
            "-Dfoo=bar".to_string(),
        ];
        strip_unsupported_jvm_args(&java, &mut args);
        assert_eq!(args, vec!["-Xmx1G".to_string(), "-Dfoo=bar".to_string()]);
    }
}
