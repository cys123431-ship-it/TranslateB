//! 책 한 권의 작업 폴더: 준비(조각 만들기), 번역 진행, 합치기, 검토용 보고서.
//!
//! 작업/<책이름>/
//!   원문_조각/NN/001.txt   번역_조각/NN/001.md   번역/NN.md
//!   목차.md  자동검사_보고서.md  실패목록.txt  상태.json

use crate::source::Chapter;
use crate::{check, chunk, engine, paths};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TRIES: usize = 3;
/// 다른 PC의 실행 표시가 이보다 오래되면 끝난 것으로 본다.
const LOCK_STALE_SECS: u64 = 10 * 60;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ChapterInfo {
    pub num: String,
    pub title: String,
    pub chunks: usize,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Lock {
    pub host: String,
    pub heartbeat: u64,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct State {
    pub source: String,
    pub rules: String,
    pub code_check: bool,
    pub chapters: Vec<ChapterInfo>,
    /// "01/003" -> 번역한 모델
    pub chunk_models: BTreeMap<String, String>,
    pub lock: Option<Lock>,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub struct Job {
    pub dir: PathBuf,
}

impl Job {
    pub fn open(dir: PathBuf) -> Self {
        Job { dir }
    }

    pub fn name(&self) -> String {
        self.dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("상태.json")
    }

    pub fn state(&self) -> Result<State, String> {
        let s = std::fs::read_to_string(self.state_path()).map_err(|e| format!("상태.json 을 읽지 못했습니다: {e}"))?;
        serde_json::from_str(&s).map_err(|e| e.to_string())
    }

    pub fn save(&self, st: &State) -> Result<(), String> {
        let tmp = self.dir.join("상태.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(st).unwrap()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, self.state_path()).map_err(|e| e.to_string())
    }

    fn src(&self, num: &str) -> PathBuf {
        self.dir.join("원문_조각").join(num)
    }
    fn parts(&self, num: &str) -> PathBuf {
        self.dir.join("번역_조각").join(num)
    }

    /// (끝난 조각, 전체 조각)
    pub fn progress(&self, st: &State) -> (usize, usize) {
        let total = st.chapters.iter().map(|c| c.chunks).sum();
        let done = st.chapters.iter().map(|c| self.done_in(&c.num)).sum();
        (done, total)
    }

    pub fn done_in(&self, num: &str) -> usize {
        std::fs::read_dir(self.parts(num))
            .map(|r| r.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "md")).count())
            .unwrap_or(0)
    }

    /// 다른 PC에서 실행 중이면 그 PC 이름
    pub fn busy_elsewhere(&self, st: &State) -> Option<String> {
        let l = st.lock.as_ref()?;
        (l.host != paths::host_name() && now().saturating_sub(l.heartbeat) < LOCK_STALE_SECS).then(|| l.host.clone())
    }
}

/// 같은 이름이 있으면 뒤에 번호를 붙인다.
fn unique_dir(name: &str) -> PathBuf {
    let clean: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
    let clean = clean.trim().trim_end_matches('.').to_string();
    let base = paths::jobs_dir().join(if clean.is_empty() { "책".into() } else { clean });
    if !base.exists() {
        return base;
    }
    (2..).map(|i| PathBuf::from(format!("{}_{i}", base.display()))).find(|p| !p.exists()).unwrap()
}

/// 고른 단원으로 작업 폴더를 만들고 원문 조각을 쓴다.
pub fn prepare(name: &str, source: &Path, chapters: &[Chapter], rules: &str) -> Result<Job, String> {
    let dir = unique_dir(name);
    let job = Job::open(dir.clone());
    let mut st = State {
        source: source.display().to_string(),
        rules: rules.to_string(),
        code_check: rules_code_check(rules),
        ..Default::default()
    };
    let mut toc = String::from("# 원문 목차\n\n번역본의 절 제목이 이 목록과 맞는지 검토할 때 씁니다.\n\n");
    for (i, ch) in chapters.iter().enumerate() {
        let num = format!("{:02}", i + 1);
        let pieces = chunk::split(&ch.lines);
        let d = job.src(&num);
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        for (k, p) in pieces.iter().enumerate() {
            std::fs::write(d.join(format!("{:03}.txt", k + 1)), p).map_err(|e| e.to_string())?;
        }
        let pages = ch.pages.map(|(a, b)| format!(" (원본 {a}~{b}쪽)")).unwrap_or_default();
        toc.push_str(&format!("## {num}. {}{pages} — {}조각\n", ch.title, pieces.len()));
        for s in &ch.sections {
            toc.push_str(&format!("- {s}\n"));
        }
        toc.push('\n');
        st.chapters.push(ChapterInfo { num, title: ch.title.clone(), chunks: pieces.len() });
    }
    std::fs::create_dir_all(dir.join("번역")).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("목차.md"), toc).map_err(|e| e.to_string())?;
    job.save(&st)?;
    Ok(job)
}

pub fn rules_code_check(rules: &str) -> bool {
    std::fs::read_to_string(paths::rules_dir().join(rules)).map(|s| s.contains("<!-- code-check -->")).unwrap_or(false)
}

pub enum Event {
    Status(String),
    Progress { done: usize, total: usize },
    Log(String),
    Finished(String),
}

pub struct RunOptions {
    pub model: String,
    pub delay_secs: u64,
    pub stop: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
}

fn wait(secs: u64, opt: &RunOptions) -> bool {
    for _ in 0..secs * 10 {
        if opt.stop.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

/// 남은 조각을 모두 번역한다. 끝난 조각은 건너뛰므로 언제 멈춰도 이어서 할 수 있다.
pub fn run(job: &Job, opt: &RunOptions, tx: &Sender<Event>) -> Result<(), String> {
    let mut st = job.state()?;
    let rules = std::fs::read_to_string(paths::rules_dir().join(&st.rules)).map_err(|_| format!("규칙 파일 {} 이 없습니다.", st.rules))?;
    let host = paths::host_name();
    let mut fail_log = String::new();

    let chapters = st.chapters.clone();
    let (mut done, total) = job.progress(&st);
    // 무료 사용 한도에 걸리면 다시 시도해도 소용없으므로 전체를 멈춘다
    let mut limit: Option<u64> = None;
    'outer: for ch in &chapters {
        let part_dir = job.parts(&ch.num);
        std::fs::create_dir_all(&part_dir).map_err(|e| e.to_string())?;
        for k in 1..=ch.chunks {
            let id = format!("{:03}", k);
            let dst = part_dir.join(format!("{id}.md"));
            if dst.exists() {
                continue;
            }
            while opt.pause.load(Ordering::Relaxed) && !opt.stop.load(Ordering::Relaxed) {
                let _ = tx.send(Event::Status("일시정지됨".into()));
                std::thread::sleep(Duration::from_millis(300));
            }
            if opt.stop.load(Ordering::Relaxed) {
                break 'outer;
            }
            st.lock = Some(Lock { host: host.clone(), heartbeat: now() });
            job.save(&st)?;

            let src = std::fs::read_to_string(job.src(&ch.num).join(format!("{id}.txt"))).map_err(|e| e.to_string())?;
            let message = format!("{rules}\n\n[원문]\n{src}");
            let mut reason = String::new();
            for attempt in 1..=TRIES {
                let _ = tx.send(Event::Status(format!("{} {id}/{:03} 번역 중 (시도 {attempt})", ch.title, ch.chunks)));
                match engine::ask(&opt.model, &message, &format!("{} {}-{id}", job.name(), ch.num), &opt.stop) {
                    Ok(text) => match check::verdict(&src, &text, st.code_check) {
                        None => {
                            std::fs::write(&dst, format!("{text}\n")).map_err(|e| e.to_string())?;
                            st.chunk_models.insert(format!("{}/{id}", ch.num), opt.model.clone());
                            reason.clear();
                            break;
                        }
                        Some(r) => reason = r,
                    },
                    Err(e) if e == "중지됨" => break 'outer,
                    Err(e) if e.starts_with(engine::FREE_LIMIT) => {
                        limit = Some(e[engine::FREE_LIMIT.len()..].parse().unwrap_or(0));
                        break 'outer;
                    }
                    Err(e) => reason = format!("오류: {e}"),
                }
                if attempt == TRIES {
                    break;
                }
                let _ = tx.send(Event::Log(format!("{}-{id} 다시 시도: {reason}", ch.num)));
                if !wait(30 * attempt as u64, opt) {
                    break 'outer;
                }
            }
            if reason.is_empty() {
                done += 1;
            } else {
                fail_log.push_str(&format!("{}/{id}: {reason}\n", ch.num));
                let _ = tx.send(Event::Log(format!("{}-{id} 실패, 건너뜀", ch.num)));
            }
            let _ = tx.send(Event::Progress { done, total });
            if opt.delay_secs > 0 && !wait(opt.delay_secs, opt) {
                break 'outer;
            }
        }
        if merge(job, ch)? {
            let _ = tx.send(Event::Log(format!("{} 완료 → 번역/{}.md", ch.title, ch.num)));
        }
    }

    st.lock = None;
    job.save(&st)?;
    if !fail_log.is_empty() {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(job.dir.join("실패목록.txt")).map_err(|e| e.to_string())?;
        let _ = f.write_all(fail_log.as_bytes());
    }
    report(job)?;
    let (d, t) = job.progress(&st);
    let msg = if let Some(ms) = limit {
        let when = if ms > 0 {
            let m = ms / 60_000;
            format!("약 {}시간 {}분 뒤에 풀립니다", m / 60, m % 60)
        } else {
            "풀리는 시각은 알 수 없습니다".into()
        };
        format!("{} 무료 사용 한도에 걸려 멈췄습니다 ({d}/{t}조각). {when}. 그때 [이어하기] 를 누르거나, 다른 무료 모델을 골라 바로 이어서 할 수 있습니다.", opt.model)
    } else if opt.stop.load(Ordering::Relaxed) {
        format!("중지됨 ({d}/{t}조각). 다시 시작하면 이어서 합니다.")
    } else if d == t {
        "검토 준비 완료".to_string()
    } else {
        format!("{}조각 실패. 다시 시작하면 실패한 조각만 재시도합니다.", t - d)
    };
    let _ = tx.send(Event::Finished(msg));
    Ok(())
}

/// 단원의 조각이 다 모였으면 합쳐서 번역/NN.md 를 만든다.
fn merge(job: &Job, ch: &ChapterInfo) -> Result<bool, String> {
    if job.done_in(&ch.num) < ch.chunks {
        return Ok(false);
    }
    let mut parts = Vec::new();
    for k in 1..=ch.chunks {
        parts.push(std::fs::read_to_string(job.parts(&ch.num).join(format!("{k:03}.md"))).map_err(|e| e.to_string())?.trim().to_string());
    }
    std::fs::create_dir_all(job.dir.join("번역")).map_err(|e| e.to_string())?;
    std::fs::write(job.dir.join("번역").join(format!("{}.md", ch.num)), parts.join("\n\n") + "\n").map_err(|e| e.to_string())?;
    Ok(true)
}

/// 검토 직전 상태 보고서: 조각별 길이 비율·코드 보존·미번역 의심·용어 불일치
pub fn report(job: &Job) -> Result<(), String> {
    let st = job.state()?;
    let rules = std::fs::read_to_string(paths::rules_dir().join(&st.rules)).unwrap_or_default();
    let terms = check::glossary(&rules);
    let (done, total) = job.progress(&st);
    let mut out = format!(
        "# 자동검사 보고서\n\n- 원서: {}\n- 규칙: {}\n- 진행: {done}/{total}조각\n- 기준: 길이 비율 {:.0}% 미만은 요약 의심, 코드 줄 보존 {:.0}% 미만은 코드 변경 의심\n\n",
        st.source,
        st.rules,
        check::MIN_RATIO * 100.0,
        check::MIN_CODE_KEPT * 100.0
    );
    let mut models: BTreeMap<&str, usize> = BTreeMap::new();
    for m in st.chunk_models.values() {
        *models.entry(m).or_default() += 1;
    }
    out.push_str("## 사용 모델\n");
    for (m, n) in &models {
        out.push_str(&format!("- {m}: {n}조각\n"));
    }
    out.push_str("\n## 단원별 요약\n| 단원 | 조각 | 완료 | 평균 길이 비율 | 주의 조각 |\n|---|---|---|---|---|\n");
    let mut detail = String::from("\n## 주의할 조각\n");
    let mut term_total: BTreeMap<String, usize> = BTreeMap::new();
    for ch in &st.chapters {
        let mut ratios = Vec::new();
        let mut flagged = 0;
        for k in 1..=ch.chunks {
            let id = format!("{k:03}");
            let Ok(dst) = std::fs::read_to_string(job.parts(&ch.num).join(format!("{id}.md"))) else { continue };
            let src = std::fs::read_to_string(job.src(&ch.num).join(format!("{id}.txt"))).unwrap_or_default();
            let r = check::ratio(&src, &dst);
            ratios.push(r);
            let mut notes = Vec::new();
            if r < 0.45 {
                notes.push(format!("길이 비율 {:.0}% (짧은 편)", r * 100.0));
            }
            if st.code_check {
                let (n, kept) = check::code_kept(&src, &dst);
                if n > 0 && kept < n {
                    notes.push(format!("코드 {n}줄 중 {kept}줄 그대로"));
                }
            }
            let un = check::untranslated_lines(&dst);
            if !un.is_empty() {
                notes.push(format!("미번역 의심 {}줄 (예: \"{}\")", un.len(), un[0]));
            }
            for (t, n) in check::glossary_misses(&dst, &terms) {
                *term_total.entry(t).or_default() += n;
            }
            if !notes.is_empty() {
                flagged += 1;
                detail.push_str(&format!("- {}/{id}: {}\n", ch.num, notes.join(", ")));
            }
        }
        let avg = if ratios.is_empty() { "-".into() } else { format!("{:.0}%", ratios.iter().sum::<f64>() / ratios.len() as f64 * 100.0) };
        out.push_str(&format!("| {} {} | {} | {} | {avg} | {flagged} |\n", ch.num, ch.title, ch.chunks, job.done_in(&ch.num)));
    }
    out.push_str(&detail);
    out.push_str("\n## 용어집과 다르게 남은 영어 용어 (코드·백틱·괄호 밖)\n");
    if term_total.is_empty() {
        out.push_str("- 없음\n");
    }
    for (t, n) in &term_total {
        out.push_str(&format!("- {t}: {n}번\n"));
    }
    if let Ok(f) = std::fs::read_to_string(job.dir.join("실패목록.txt")) {
        out.push_str("\n## 실패 기록\n");
        out.push_str(&f);
    }
    std::fs::write(job.dir.join("자동검사_보고서.md"), out).map_err(|e| e.to_string())
}

pub fn list_jobs() -> Vec<Job> {
    let mut v: Vec<Job> = std::fs::read_dir(paths::jobs_dir())
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.join("상태.json").exists()).map(Job::open).collect())
        .unwrap_or_default();
    v.sort_by_key(|j| j.name());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 가짜 엔진으로 준비 → 번역 → 합치기 → 보고서 → 이어하기까지 한 번에 확인한다.
    #[test]
    fn mock_pipeline_end_to_end() {
        let home = std::env::temp_dir().join(format!("translateb_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("rules")).unwrap();
        std::fs::write(home.join("rules/test.md"), "<!-- code-check -->\n규칙\npointer=포인터, array=배열\n").unwrap();
        unsafe {
            std::env::set_var("TRANSLATEB_HOME", &home);
            std::env::set_var("TRANSLATEB_MOCK", "1");
        }

        let mut lines = Vec::new();
        for i in 0..300 {
            lines.push(format!("Sentence {i} explains how a pointer refers to memory."));
            if i % 50 == 0 {
                lines.push("int value = 42;".into());
            }
        }
        let chapters = vec![
            Chapter { title: "One".into(), sections: vec!["Intro".into()], lines: lines.clone(), pages: Some((1, 10)) },
            Chapter { title: "Two".into(), sections: vec![], lines: lines[..20].to_vec(), pages: None },
        ];
        let job = prepare("시험 책", Path::new("sample.pdf"), &chapters, "test.md").unwrap();
        let st = job.state().unwrap();
        assert!(st.code_check);
        assert!(st.chapters[0].chunks > 1);

        let (tx, rx) = std::sync::mpsc::channel();
        let opt = RunOptions { model: "mock/echo-free".into(), delay_secs: 0, stop: Arc::new(AtomicBool::new(false)), pause: Arc::new(AtomicBool::new(false)) };
        run(&job, &opt, &tx).unwrap();
        let finished: Vec<String> = rx.try_iter().filter_map(|e| if let Event::Finished(m) = e { Some(m) } else { None }).collect();
        assert_eq!(finished, vec!["검토 준비 완료".to_string()]);

        let merged = std::fs::read_to_string(job.dir.join("번역/01.md")).unwrap();
        assert!(merged.contains("[번역] Sentence 299"));
        assert!(merged.contains("int value = 42;"));
        assert!(job.dir.join("번역/02.md").exists());
        let report = std::fs::read_to_string(job.dir.join("자동검사_보고서.md")).unwrap();
        assert!(report.contains("mock/echo-free"));
        assert!(job.state().unwrap().lock.is_none());

        // 조각 하나를 지우고 다시 돌리면 그 조각만 번역한다
        std::fs::remove_file(job.dir.join("번역_조각/01/002.md")).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        run(&job, &opt, &tx).unwrap();
        let translated = rx.try_iter().filter(|e| matches!(e, Event::Progress { .. })).count();
        assert_eq!(translated, 1);

        let _ = std::fs::remove_dir_all(&home);
    }
}
