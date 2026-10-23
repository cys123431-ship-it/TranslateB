//! 프로그램 폴더 기준 경로. exe 옆에 모든 것을 두는 휴대용 구조다.

use std::path::PathBuf;

/// 프로그램 폴더. 개발 중에는 TRANSLATEB_HOME 으로 바꿀 수 있다.
pub fn root() -> PathBuf {
    if let Ok(p) = std::env::var("TRANSLATEB_HOME") {
        return PathBuf::from(p);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn engine_dir() -> PathBuf {
    root().join("engine")
}

/// 탑재한 OpenCode CLI
pub fn opencode_exe() -> PathBuf {
    engine_dir().join("opencode.exe")
}

/// OpenCode 전용 데이터 폴더 (로그인 정보·세션 기록이 여기에만 남는다)
pub fn opencode_home() -> PathBuf {
    engine_dir().join("home")
}

pub fn pdfium_dll() -> PathBuf {
    engine_dir().join("pdfium.dll")
}

pub fn rules_dir() -> PathBuf {
    root().join("rules")
}

pub fn jobs_dir() -> PathBuf {
    root().join("작업")
}

pub fn settings_file() -> PathBuf {
    root().join("settings.json")
}

/// 지금 PC 이름 (여러 PC 동시 실행 감지용)
pub fn host_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}
