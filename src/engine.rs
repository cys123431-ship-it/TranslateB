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

/// OpenCode 자체 로그인/로그아웃 화면을 새 콘솔 창으로 연다. 비밀번호는 이 프로그램을 거치지 않는다.
pub fn open_console(args: &[&str]) -> Result<(), String> {
    // 경로에 공백이 있어도 되도록 engine 폴더에서 상대 이름으로 실행한다
    let mut cmd = Command::new("cmd");
    cmd.args(["/c", "start", "OpenCode", "cmd", "/k", "opencode.exe"]).args(args).current_dir(paths::engine_dir());
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
    cmd.args(["run", "--pure", "--model", model, "--format", "json", "--title", title, message])
        .current_dir(&work)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("OpenCode 를 실행하지 못했습니다: {e}"))?;

    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = so.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = se.read_to_string(&mut s);
        s
    });

    let start = Instant::now();
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
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
    let text = parse_events(&stdout);
    if text.trim().is_empty() {
        let msg = strip_ansi(&format!("{stderr}\n{stdout}"));
        let msg = msg.trim();
        let tail: String = msg.chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(if tail.is_empty() { "빈 응답".into() } else { tail });
    }
    Ok(text.trim().to_string())
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
}
