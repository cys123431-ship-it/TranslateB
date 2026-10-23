//! 엔진(OpenCode CLI, PDFium) 내려받기와 업데이트. 공식 GitHub 릴리스만 쓰고,
//! 릴리스에 적힌 sha256 값과 맞는지 확인한 뒤에만 교체한다.

use crate::{engine, paths};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const OPENCODE_REPO: &str = "anomalyco/opencode";
const OPENCODE_ASSET: &str = "opencode-windows-x64.zip";
pub const PDFIUM_REPO: &str = "bblanchon/pdfium-binaries";
const PDFIUM_ASSET: &str = "pdfium-win-x64.tgz";

#[derive(Deserialize, Clone)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Clone, Debug)]
pub struct Latest {
    pub version: String,
    url: String,
    size: u64,
    sha256: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(600))).build().into()
}

fn latest(repo: &str, asset: &str) -> Result<Latest, String> {
    let rel: Release = agent()
        .get(&format!("https://api.github.com/repos/{repo}/releases/latest"))
        .header("User-Agent", "TranslateB")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("GitHub 에 접속하지 못했습니다: {e}"))?
        .body_mut()
        .read_json()
        .map_err(|e| e.to_string())?;
    let a = rel.assets.iter().find(|a| a.name == asset).ok_or(format!("{asset} 파일이 릴리스에 없습니다."))?;
    let sha256 = a
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .ok_or("릴리스에 확인값(sha256)이 없어 설치하지 않습니다.")?
        .to_string();
    Ok(Latest { version: rel.tag_name.trim_start_matches('v').to_string(), url: a.browser_download_url.clone(), size: a.size, sha256 })
}

pub fn latest_opencode() -> Result<Latest, String> {
    latest(OPENCODE_REPO, OPENCODE_ASSET)
}

/// 내려받으면서 sha256 을 계산한다. progress 는 0.0~1.0
fn download(l: &Latest, to: &Path, progress: &dyn Fn(f32)) -> Result<(), String> {
    let mut resp = agent()
        .get(&l.url)
        .header("User-Agent", "TranslateB")
        .call()
        .map_err(|e| format!("내려받기 실패: {e}"))?;
    let mut reader = resp.body_mut().with_config().limit(1 << 31).reader();
    let mut file = std::fs::File::create(to).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut got = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("내려받기 실패: {e}"))?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got += n as u64;
        progress(got as f32 / l.size.max(1) as f32);
    }
    let actual: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if actual != l.sha256 {
        let _ = std::fs::remove_file(to);
        return Err("받은 파일의 확인값이 공식 릴리스와 다릅니다. 설치하지 않았습니다.".into());
    }
    Ok(())
}

fn tmp_path(name: &str) -> PathBuf {
    paths::engine_dir().join(format!("{name}.download"))
}

/// OpenCode 설치/업데이트. 새 버전이 --version 에 답하지 않으면 이전 버전으로 되돌린다.
pub fn install_opencode(l: &Latest, progress: &dyn Fn(f32)) -> Result<String, String> {
    std::fs::create_dir_all(paths::engine_dir()).map_err(|e| e.to_string())?;
    let zip_path = tmp_path(OPENCODE_ASSET);
    download(l, &zip_path, progress)?;

    let exe = paths::opencode_exe();
    let new = exe.with_extension("new.exe");
    let old = exe.with_extension("old.exe");
    {
        let f = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
        let mut entry = zip.by_name("opencode.exe").map_err(|_| "압축 파일 안에 opencode.exe 가 없습니다.".to_string())?;
        let mut out = std::fs::File::create(&new).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_file(&zip_path);

    // MIT 라이선스 문구를 함께 둔다 (실패해도 설치는 계속)
    if let Ok(mut r) = agent()
        .get(&format!("https://raw.githubusercontent.com/{OPENCODE_REPO}/v{}/LICENSE", l.version))
        .header("User-Agent", "TranslateB")
        .call()
    {
        if let Ok(text) = r.body_mut().read_to_string() {
            let _ = std::fs::write(paths::engine_dir().join("LICENSE-opencode.txt"), text);
        }
    }

    let had_old = exe.exists();
    if had_old {
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old).map_err(|e| format!("기존 엔진을 바꾸지 못했습니다 (실행 중일 수 있음): {e}"))?;
    }
    std::fs::rename(&new, &exe).map_err(|e| e.to_string())?;
    match engine::version() {
        Ok(v) => Ok(v),
        Err(e) => {
            if had_old {
                let _ = std::fs::remove_file(&exe);
                let _ = std::fs::rename(&old, &exe);
            }
            Err(format!("새 버전이 실행되지 않아 이전 버전으로 되돌렸습니다: {e}"))
        }
    }
}

pub fn install_pdfium(progress: &dyn Fn(f32)) -> Result<String, String> {
    let l = latest(PDFIUM_REPO, PDFIUM_ASSET)?;
    std::fs::create_dir_all(paths::engine_dir()).map_err(|e| e.to_string())?;
    let tgz = tmp_path(PDFIUM_ASSET);
    download(&l, &tgz, progress)?;
    let f = std::fs::File::open(&tgz).map_err(|e| e.to_string())?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    let mut found = false;
    for entry in ar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path().map_err(|e| e.to_string())?.to_path_buf();
        match p.file_name().and_then(|n| n.to_str()) {
            Some("pdfium.dll") => {
                let mut out = std::fs::File::create(paths::pdfium_dll()).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
                found = true;
            }
            Some("LICENSE") => {
                let mut out = std::fs::File::create(paths::engine_dir().join("LICENSE-pdfium.txt")).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            }
            _ => {}
        }
    }
    let _ = std::fs::remove_file(&tgz);
    if !found {
        return Err("압축 파일 안에 pdfium.dll 이 없습니다.".into());
    }
    Ok(l.version)
}

/// "1.19.4" 와 "1.20.0" 비교: a 가 b 보다 새로우면 true
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |s: &str| s.split('.').map(|x| x.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parse(a) > parse(b)
}

#[cfg(test)]
mod tests {
    #[test]
    fn compares_versions() {
        assert!(super::newer("1.20.0", "1.19.4"));
        assert!(super::newer("1.19.10", "1.19.4"));
        assert!(!super::newer("1.19.4", "1.19.4"));
    }
}
