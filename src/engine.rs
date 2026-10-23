//! 탑재한 OpenCode CLI 실행. 데이터 폴더를 프로그램 폴더 안으로 돌려서
//! 이 PC에 설치된 다른 OpenCode 와 섞이지 않고, 로그인 정보도 PC에 남지 않게 한다.

use crate::paths;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// OpenCode 는 데이터베이스 하나를 쓰므로 동시에 여러 개를 띄우면 "database is locked" 가 난다.
/// 이 프로그램 안에서는 한 번에 하나씩만 실행한다.
static ENGINE_LOCK: Mutex<()> = Mutex::new(());

fn engine_lock() -> std::sync::MutexGuard<'static, ()> {
    ENGINE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// 콘솔 창이 깜빡이지 않게 한다.
pub fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let _ = cmd;
}

fn base() -> Command {
    let home = paths::opencode_home();
    let mut cmd = Command::new(paths::opencode_exe());
    for (k, d) in [("XDG_DATA_HOME", "data"), ("XDG_CONFIG_HOME", "config"), ("XDG_CACHE_HOME", "cache"), ("XDG_STATE_HOME", "state")] {
        cmd.env(k, home.join(d));
    }
    cmd
}

pub fn installed() -> bool {
    paths::opencode_exe().exists()
}

fn mock() -> bool {
    std::env::var("TRANSLATEB_MOCK").is_ok()
}

fn output(args: &[&str]) -> Result<String, String> {
    let _guard = engine_lock();
    let mut last = String::new();
    // 다른 프로그램(OpenCode 앱 등)이 잠깐 데이터베이스를 잡고 있을 수 있어 몇 번 다시 해 본다
    for _ in 0..3 {
        let mut cmd = base();
        cmd.args(args).stdin(Stdio::null());
        hide_window(&mut cmd);
        let out = cmd.output().map_err(|e| format!("OpenCode 를 실행하지 못했습니다: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if out.status.success() {
            return Ok(text);
        }
        last = strip_ansi(&format!("{}{}", text, String::from_utf8_lossy(&out.stderr))).trim().to_string();
        if !last.contains("locked") {
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    Err(last)
}

fn strip_ansi(s: &str) -> String {
    regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap().replace_all(s, "").to_string()
}

/// "1.19.4" 같은 버전 문자열
pub fn version() -> Result<String, String> {
    if mock() {
        return Ok("mock".into());
    }
    Ok(output(&["--version"])?.trim().trim_start_matches('v').to_string())
}

/// 쓸 수 있는 모델 목록. 무료 모델이 앞에 온다.
pub fn models() -> Result<Vec<String>, String> {
    if mock() {
        return Ok(vec!["mock/echo-free".into()]);
    }
    let text = output(&["models"])?;
    let mut list: Vec<String> = text
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| l.contains('/') && !l.contains(' '))
        .collect();
    list.sort_by_key(|m| (!m.ends_with("-free"), m.clone()));
    Ok(list)
}

/// 로그인된 제공자 목록 (표시용)
pub fn auth_list() -> String {
    output(&["auth", "list"]).map(|s| strip_ansi(&s)).unwrap_or_else(|e| e)
}

/// OpenCode 가 쓰는 자격 증명 파일 (engine/home 안)
fn auth_file() -> std::path::PathBuf {
    paths::opencode_home().join("data").join("opencode").join("auth.json")
}

fn read_auth() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(auth_file())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// API 키를 OpenCode 와 같은 형식({"provider": {"type":"api","key":"..."}})으로 저장한다.
/// 키는 이 프로그램 폴더의 engine/home 안에만 남는다.
pub fn set_api_key(provider: &str, key: &str) -> Result<(), String> {
    let (provider, key) = (provider.trim().trim_end_matches('/'), key.trim());
    if provider.is_empty() || key.is_empty() {
        return Err("제공자 이름과 API 키를 모두 넣어 주세요.".into());
    }
    let mut auth = read_auth();
    auth.insert(provider.to_string(), serde_json::json!({ "type": "api", "key": key }));
    let path = auth_file();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::to_string_pretty(&auth).unwrap()).map_err(|e| format!("키를 저장하지 못했습니다: {e}"))
}

pub fn remove_api_key(provider: &str) -> Result<(), String> {
    let mut auth = read_auth();
    if auth.remove(provider).is_none() {
        return Ok(());
    }
    std::fs::write(auth_file(), serde_json::to_string_pretty(&auth).unwrap()).map_err(|e| e.to_string())
}

/// 키가 저장된 제공자 이름들 (키 값은 돌려주지 않는다)
pub fn saved_providers() -> Vec<String> {
    read_auth().keys().cloned().collect()
}

// ---------- 외부 제공자 (OpenAI 호환 API) ----------

/// OpenCode 설정 파일 (engine/home 안)
fn config_file() -> std::path::PathBuf {
    paths::opencode_home().join("config").join("opencode").join("opencode.json")
}

fn read_config() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(config_file())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn write_config(cfg: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    let path = config_file();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::to_string_pretty(cfg).unwrap()).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))
}

/// OpenAI 호환 API 를 OpenCode 제공자로 등록한다. 키는 같은 이름으로 [API 키] 에 따로 저장한다.
/// 모델은 "제공자/모델" 이름으로 모델 목록에 나타난다.
pub fn set_provider(id: &str, base_url: &str, models: &[String]) -> Result<(), String> {
    let (id, base_url) = (id.trim().trim_end_matches('/'), base_url.trim().trim_end_matches('/'));
    let models: Vec<&str> = models.iter().map(|m| m.trim()).filter(|m| !m.is_empty()).collect();
    if id.is_empty() || id.contains('/') || id.contains(' ') {
        return Err("제공자 이름은 띄어쓰기와 / 없이 넣어 주세요. (예: river)".into());
    }
    if !base_url.starts_with("https://") && !base_url.starts_with("http://") {
        return Err("주소는 https:// 로 시작해야 합니다. (예: https://api.river.ai/v1)".into());
    }
    if models.is_empty() {
        return Err("모델 이름을 하나 이상 넣어 주세요.".into());
    }
    let mut model_map = serde_json::Map::new();
    for m in &models {
        model_map.insert(m.to_string(), serde_json::json!({ "name": m }));
    }
    let mut cfg = read_config();
    cfg.entry("$schema").or_insert_with(|| serde_json::json!("https://opencode.ai/config.json"));
    let providers = cfg.entry("provider").or_insert_with(|| serde_json::json!({}));
    if !providers.is_object() {
        *providers = serde_json::json!({});
    }
    providers.as_object_mut().unwrap().insert(
        id.to_string(),
        serde_json::json!({
            "npm": "@ai-sdk/openai-compatible",
            "name": id,
            "options": { "baseURL": base_url },
            "models": model_map,
        }),
    );
    write_config(&cfg)
}

pub fn remove_provider(id: &str) -> Result<(), String> {
    let mut cfg = read_config();
    let removed = cfg.get_mut("provider").and_then(|p| p.as_object_mut()).and_then(|p| p.remove(id)).is_some();
    if removed { write_config(&cfg) } else { Ok(()) }
}

/// 등록된 외부 제공자: (이름, 주소, 모델들)
pub fn custom_providers() -> Vec<(String, String, Vec<String>)> {
    let cfg = read_config();
    let Some(p) = cfg.get("provider").and_then(|p| p.as_object()) else { return vec![] };
    p.iter()
        .map(|(id, v)| {
            let url = v["options"]["baseURL"].as_str().unwrap_or("").to_string();
            let models = v["models"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
            (id.clone(), url, models)
        })
        .collect()
}

/// OpenCode 자체 로그인/로그아웃 화면을 새 콘솔 창으로 연다. 비밀번호는 이 프로그램을 거치지 않는다.
pub fn open_console(args: &[&str]) -> Result<(), String> {
    // 경로에 공백이 있어도 되도록 engine 폴더에서 상대 경로로 실행한다.
    // NoDefaultCurrentDirectoryInExePath 가 켜진 PC에서도 찾도록 .\ 를 붙인다.
    let mut cmd = Command::new("cmd");
    cmd.args(["/c", "start", "OpenCode", "cmd", "/k", r".\opencode.exe"]).args(args).current_dir(paths::engine_dir());
    for (k, v) in base().get_envs() {
        if let Some(v) = v {
            cmd.env(k, v);
        }
    }
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// 메시지 하나를 새 세션으로 보내고 답변 글자만 돌려준다. stop 이 켜지면 바로 끊는다.
pub fn ask(model: &str, message: &str, title: &str, stop: &Arc<AtomicBool>) -> Result<String, String> {
    if mock() {
        return Ok(mock_answer(message));
    }
    let work = paths::opencode_home().join("work");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let _guard = engine_lock();
    let mut cmd = base();
    // --print-logs: 무료 한도에 걸리면 OpenCode 는 끝나지 않고 몇 시간 뒤 재시도를 기다린다.
    // 그때 stdout 에는 아무것도 없고 로그(stderr)에만 오류가 찍히므로 로그를 지켜본다.
    // --pure 는 쓰지 않는다: 외부 제공자(River 등)를 불러오지 못하고 멈춘다.
    cmd.args(["run", "--print-logs", "--model", model, "--format", "json", "--title", title, message])
        .current_dir(&work)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("OpenCode 를 실행하지 못했습니다: {e}"))?;

    let mut so = child.stdout.take().unwrap();
    let se = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = so.read_to_string(&mut s);
        s
    });
    let limited = Arc::new(AtomicBool::new(false));
    let auth_err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let err_t = {
        let limited = limited.clone();
        let auth_err = auth_err.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            // 로그가 길 수 있어 오류 줄만 모은다
            let mut errors = String::new();
            for line in std::io::BufReader::new(se).lines().map_while(Result::ok) {
                if is_limit_line(&line) {
                    limited.store(true, Ordering::Relaxed);
                }
                if let Some(kind) = auth_problem(&line) {
                    *auth_err.lock().unwrap_or_else(|e| e.into_inner()) = Some(kind.to_string());
                }
                if line.contains("ERROR") && errors.len() < 8000 {
                    errors.push_str(&line);
                    errors.push('\n');
                }
            }
            errors
        })
    };

    let start = Instant::now();
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        if limited.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{FREE_LIMIT}0"));
        }
        if let Some(kind) = auth_err.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{AUTH_ERROR}{kind}"));
        }
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("중지됨".into());
        }
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("응답 시간 초과".into());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();
    if let Some(ms) = free_limit(&stdout) {
        return Err(format!("{FREE_LIMIT}{ms}"));
    }
    if limited.load(Ordering::Relaxed) {
        return Err(format!("{FREE_LIMIT}0"));
    }
    if let Some(kind) = auth_err.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return Err(format!("{AUTH_ERROR}{kind}"));
    }
    let text = parse_events(&stdout);
    if text.trim().is_empty() {
        let msg = strip_ansi(&format!("{stderr}\n{stdout}"));
        let msg = msg.trim();
        let tail: String = msg.chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(if tail.is_empty() { "빈 응답".into() } else { tail });
    }
    Ok(text.trim().to_string())
}

/// 무료 사용 한도 초과 오류의 머리말. 뒤에 한도가 풀리기까지 남은 밀리초가 붙는다.
pub const FREE_LIMIT: &str = "FREE_LIMIT:";

/// 키·잔액 문제 오류의 머리말. 뒤에 사람이 읽을 이유가 붙는다. 다시 시도해도 소용없다.
pub const AUTH_ERROR: &str = "AUTH_ERROR:";

/// OpenCode 오류 로그 줄이 키가 틀렸거나 잔액이 없는 경우인지
pub fn auth_problem(line: &str) -> Option<&'static str> {
    if !line.contains("ERROR") {
        return None;
    }
    let l = line.to_lowercase();
    if ["invalid api key", "incorrect api key", "invalid_api_key", "unauthorized", "authentication failed", "api key not found"].iter().any(|p| l.contains(p)) {
        Some("API 키가 잘못됐습니다")
    } else if ["payment required", "insufficient balance", "insufficient_quota", "insufficient credit", "insufficient funds"].iter().any(|p| l.contains(p)) {
        Some("잔액(크레딧)이 부족합니다")
    } else {
        None
    }
}

/// OpenCode 로그 줄이 무료 한도 초과 오류인지
pub fn is_limit_line(line: &str) -> bool {
    line.contains("Free usage limit") || line.contains("FreeUsageLimit") || line.contains("free-limit")
}

/// 무료 사용 한도에 걸렸으면 풀리기까지 남은 밀리초 (모르면 0)
pub fn free_limit(stdout: &str) -> Option<u64> {
    stdout.lines().find_map(|line| {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        let kind = v["error"]["type"].as_str()?;
        (v["type"] == "error" && kind.contains("free-limit")).then(|| v["error"]["retryAfterMs"].as_u64().unwrap_or(0))
    })
}

/// `--format json` 출력에서 text 조각만 이어 붙인다.
pub fn parse_events(stdout: &str) -> String {
    let mut out = String::new();
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v["type"] == "text" {
            if let Some(t) = v["part"]["text"].as_str() {
                out.push_str(t);
            }
        }
    }
    out
}

/// 시험용 가짜 번역: 원문 부분을 줄마다 "[번역]" 을 붙여 돌려준다. 코드처럼 보이는 줄은 그대로 둔다.
fn mock_answer(message: &str) -> String {
    std::thread::sleep(Duration::from_millis(50));
    let body = message.split("[원문]").nth(1).unwrap_or(message);
    body.lines()
        .map(|l| if crate::check::looks_like_code(l) { l.to_string() } else { format!("[번역] {l}") })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_text_events() {
        let s = r#"{"type":"step_start","part":{}}
{"type":"text","part":{"text":"안녕"}}
not json
{"type":"text","part":{"text":" OK"}}"#;
        assert_eq!(super::parse_events(s), "안녕 OK");
    }

    #[test]
    fn saves_and_removes_api_key() {
        let _g = crate::paths::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("translateb_auth_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        unsafe { std::env::set_var("TRANSLATEB_HOME", &home) };
        super::set_api_key("other", "k1").unwrap();
        super::set_api_key("opencode/", " test-key ").unwrap();
        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(super::auth_file()).unwrap()).unwrap();
        assert_eq!(saved["opencode"], serde_json::json!({ "type": "api", "key": "test-key" }));
        assert_eq!(saved["other"]["key"], "k1");
        super::remove_api_key("opencode").unwrap();
        assert_eq!(super::saved_providers(), vec!["other".to_string()]);
        assert!(super::set_api_key("opencode", "  ").is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn registers_and_removes_provider() {
        let _g = crate::paths::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("translateb_prov_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        unsafe { std::env::set_var("TRANSLATEB_HOME", &home) };
        super::set_provider("river", "https://api.river.ai/v1/", &["deepseek-ai/DeepSeek-V4.1-Flash".into(), " ".into()]).unwrap();
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(super::config_file()).unwrap()).unwrap();
        let p = &cfg["provider"]["river"];
        assert_eq!(p["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(p["options"]["baseURL"], "https://api.river.ai/v1");
        assert!(p["models"]["deepseek-ai/DeepSeek-V4.1-Flash"].is_object());
        assert_eq!(super::custom_providers().len(), 1);
        assert!(super::set_provider("ri ver", "https://x", &["m".into()]).is_err());
        assert!(super::set_provider("river", "api.river.ai", &["m".into()]).is_err());
        assert!(super::set_provider("river", "https://x", &[]).is_err());
        super::remove_provider("river").unwrap();
        assert!(super::custom_providers().is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn detects_auth_problems() {
        let bad_key = r#"timestamp=x level=ERROR message=process error="Invalid API key" stack="AI_APICallError: Invalid API key"#;
        assert_eq!(super::auth_problem(bad_key), Some("API 키가 잘못됐습니다"));
        assert_eq!(super::auth_problem("level=ERROR message=x error=\"Payment Required\""), Some("잔액(크레딧)이 부족합니다"));
        // 오류 줄이 아니면 무시, 메시지 id 에 401 같은 숫자가 있어도 무시
        assert_eq!(super::auth_problem("level=INFO message=\"Invalid API key\""), None);
        assert_eq!(super::auth_problem("level=ERROR messageID=msg_1401abc stream error"), None);
    }

    #[test]
    fn detects_free_limit() {
        let s = r#"{"type":"error","error":{"type":"provider.free-limit","retryAfterMs":54534000}}"#;
        assert_eq!(super::free_limit(s), Some(54534000));
        assert_eq!(super::free_limit(r#"{"type":"text","part":{"text":"x"}}"#), None);
        assert!(super::is_limit_line(r#"level=ERROR message="stream error" error.error="AI_APICallError: Free usage limit reached. Subscribe""#));
        assert!(!super::is_limit_line("level=INFO message=stream"));
    }
}

/// 실제 엔진으로 한도 감지를 확인한다 (무료 한도에 걸린 상태에서만 의미 있음).
/// TRANSLATEB_HOME=<엔진이 있는 폴더> cargo test real_limit -- --ignored
#[cfg(test)]
mod live_tests {
    #[test]
    #[ignore]
    fn real_bad_key_is_detected_quickly() {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let start = std::time::Instant::now();
        let r = super::ask("river/deepseek-ai/DeepSeek-V4.1-Flash", "Reply with exactly: OK", "TranslateB 키 확인", &stop);
        println!("{:?} after {:?}", r, start.elapsed());
        assert!(r.unwrap_err().starts_with(super::AUTH_ERROR));
    }

    #[test]
    #[ignore]
    fn real_limit_is_detected_quickly() {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let start = std::time::Instant::now();
        let r = super::ask("opencode/deepseek-v4.1-flash-free", "Reply with exactly: OK", "TranslateB 한도 확인", &stop);
        println!("{:?} after {:?}", r, start.elapsed());
        assert!(start.elapsed() < std::time::Duration::from_secs(120));
    }
}
