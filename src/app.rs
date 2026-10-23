//! 화면: [점검] [새 번역] [작업 목록]

use crate::job::{self, Job, RunOptions};
use crate::source::{self, Chapter};
use crate::{engine, paths, update};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_RULES: [(&str, &str); 2] = [
    ("프로그래밍.md", include_str!("../rules/프로그래밍.md")),
    ("일반.md", include_str!("../rules/일반.md")),
];
/// 조각 하나에 걸리는 대략적인 시간 (예상 시간 표시용)
const SECS_PER_CHUNK: u64 = 60;

pub fn run() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TranslateB — 원서 1차 번역 도우미")
            .with_inner_size([880.0, 680.0])
            .with_min_inner_size([640.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    let _ = eframe::run_native("TranslateB", options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))));
}

#[derive(Serialize, Deserialize, Default)]
struct Settings {
    model: String,
    rules: String,
    delay_secs: u64,
}

impl Settings {
    fn load() -> Self {
        std::fs::read_to_string(paths::settings_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    fn save(&self) {
        let _ = std::fs::write(paths::settings_file(), serde_json::to_string_pretty(self).unwrap());
    }
}

enum Msg {
    Version(Result<String, String>),
    Latest(Result<update::Latest, String>),
    Models(Result<Vec<String>, String>),
    Test(Result<String, String>),
    Installed(&'static str, Result<String, String>),
    Download(f32),
    Loaded(PathBuf, Result<Vec<Chapter>, String>),
    Auth(String),
    JobDone(Result<(), String>),
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Check,
    New,
    Jobs,
}

struct Running {
    name: String,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    events: Receiver<job::Event>,
    done: usize,
    total: usize,
    status: String,
}

struct App {
    tab: Tab,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    settings: Settings,
    rules_list: Vec<String>,

    // 점검
    oc_version: Option<Result<String, String>>,
    oc_latest: Option<Result<update::Latest, String>>,
    models: Vec<String>,
    models_err: Option<String>,
    test: Option<Result<String, String>>,
    auth: String,
    key_provider: String,
    key_input: String,
    prov_id: String,
    prov_url: String,
    prov_models: String,
    busy: Option<String>,
    download: f32,
    notice: Option<String>,

    // 새 번역
    file: Option<PathBuf>,
    loading: bool,
    chapters: Vec<Chapter>,
    selected: Vec<bool>,
    book_name: String,
    load_err: Option<String>,

    // 작업
    running: Option<Running>,
    logs: Vec<String>,
    finished: Option<String>,
    confirm_force: Option<PathBuf>,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        load_korean_font(ctx);
        let _ = std::fs::create_dir_all(paths::rules_dir());
        let _ = std::fs::create_dir_all(paths::jobs_dir());
        for (name, body) in DEFAULT_RULES {
            let p = paths::rules_dir().join(name);
            if !p.exists() {
                let _ = std::fs::write(p, body);
            }
        }
        let (tx, rx) = channel();
        let mut settings = Settings::load();
        let rules_list = list_rules();
        if !rules_list.contains(&settings.rules) {
            settings.rules = rules_list.first().cloned().unwrap_or_default();
        }
        let mut app = App {
            tab: Tab::Check,
            tx,
            rx,
            ctx: ctx.clone(),
            settings,
            rules_list,
            oc_version: None,
            oc_latest: None,
            models: vec![],
            models_err: None,
            test: None,
            auth: String::new(),
            key_provider: "opencode".into(),
            key_input: String::new(),
            prov_id: String::new(),
            prov_url: String::new(),
            prov_models: String::new(),
            busy: None,
            download: 0.0,
            notice: None,
            file: None,
            loading: false,
            chapters: vec![],
            selected: vec![],
            book_name: String::new(),
            load_err: None,
            running: None,
            logs: vec![],
            finished: None,
            confirm_force: None,
        };
        app.refresh_engine();
        app.bg(|| Msg::Latest(update::latest_opencode()));
        app
    }

    fn bg(&self, f: impl FnOnce() -> Msg + Send + 'static) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
    }

    fn refresh_engine(&mut self) {
        if !engine::installed() {
            self.oc_version = Some(Err("없음".into()));
            return;
        }
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Version(engine::version()));
            let _ = tx.send(Msg::Models(engine::models()));
            let _ = tx.send(Msg::Auth(engine::auth_list()));
            ctx.request_repaint();
        });
    }

    fn install_opencode(&mut self, l: update::Latest) {
        if self.running.is_some() {
            self.notice = Some("번역이 돌아가는 중에는 엔진을 바꿀 수 없습니다.".into());
            return;
        }
        self.busy = Some(format!("OpenCode {} 내려받는 중", l.version));
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.bg(move || {
            let p = move |f: f32| {
                let _ = tx.send(Msg::Download(f));
                ctx.request_repaint();
            };
            Msg::Installed("OpenCode", update::install_opencode(&l, &p))
        });
    }

    fn install_pdfium(&mut self) {
        self.busy = Some("PDF 엔진 내려받는 중".into());
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.bg(move || {
            let p = move |f: f32| {
                let _ = tx.send(Msg::Download(f));
                ctx.request_repaint();
            };
            Msg::Installed("PDF 엔진", update::install_pdfium(&p))
        });
    }

    fn open_file(&mut self, path: PathBuf) {
        self.loading = true;
        self.load_err = None;
        self.chapters.clear();
        self.book_name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        self.file = Some(path.clone());
        self.bg(move || {
            let r = source::load(&path);
            Msg::Loaded(path, r)
        });
    }

    fn start(&mut self, job: Job, force: bool) {
        let st = match job.state() {
            Ok(s) => s,
            Err(e) => {
                self.notice = Some(e);
                return;
            }
        };
        if let (Some(host), false) = (job.busy_elsewhere(&st), force) {
            self.notice = Some(format!("다른 PC({host})에서 이 작업을 돌리는 중으로 보입니다. 같은 조각을 겹쳐 번역할 수 있습니다."));
            self.confirm_force = Some(job.dir.clone());
            return;
        }
        if self.settings.model.is_empty() {
            self.notice = Some("[점검] 화면에서 모델을 먼저 불러와 주세요.".into());
            return;
        }
        let (done, total) = job.progress(&st);
        let (etx, erx) = channel();
        let opt = RunOptions {
            model: self.settings.model.clone(),
            delay_secs: self.settings.delay_secs,
            stop: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
        };
        self.running = Some(Running {
            name: job.name(),
            stop: opt.stop.clone(),
            pause: opt.pause.clone(),
            events: erx,
            done,
            total,
            status: "시작하는 중".into(),
        });
        self.finished = None;
        self.logs.clear();
        self.confirm_force = None;
        self.tab = Tab::Jobs;
        self.bg(move || Msg::JobDone(job::run(&job, &opt, &etx)));
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Version(v) => self.oc_version = Some(v),
            Msg::Latest(l) => self.oc_latest = Some(l),
            Msg::Models(Ok(list)) => {
                if !list.contains(&self.settings.model) {
                    self.settings.model = list.first().cloned().unwrap_or_default();
                }
                self.models = list;
                self.models_err = None;
            }
            Msg::Models(Err(e)) => self.models_err = Some(e),
            Msg::Test(r) => {
                self.busy = None;
                self.test = Some(r.map_err(|e| {
                    if e.starts_with(engine::FREE_LIMIT) {
                        format!("{} 무료 사용 한도에 걸렸습니다. 무료 모델은 한도를 함께 쓰므로 나중에 다시 하거나 외부 제공자 모델을 쓰세요.", self.settings.model)
                    } else if let Some(why) = e.strip_prefix(engine::AUTH_ERROR) {
                        format!("{why}. 제공자 이름과 키를 확인해 주세요.")
                    } else {
                        e
                    }
                }));
            }
            Msg::Installed(what, r) => {
                self.busy = None;
                self.download = 0.0;
                self.notice = Some(match r {
                    Ok(v) => format!("{what} {v} 설치 완료"),
                    Err(e) => format!("{what} 설치 실패: {e}"),
                });
                self.refresh_engine();
            }
            Msg::Download(f) => self.download = f,
            Msg::Loaded(p, r) => {
                if self.file.as_ref() != Some(&p) {
                    return;
                }
                self.loading = false;
                match r {
                    Ok(ch) => {
                        // 목차·색인처럼 보이는 단원은 기본으로 빼 둔다
                        let skip = regex::Regex::new(r"(?i)contents|index|copyright|목차|차례|찾아보기|색인").unwrap();
                        self.selected = ch.iter().map(|c| !skip.is_match(&c.title) && !c.lines.is_empty()).collect();
                        self.chapters = ch;
                    }
                    Err(e) => self.load_err = Some(e),
                }
            }
            Msg::Auth(a) => self.auth = a,
            Msg::JobDone(r) => {
                if let Err(e) = r {
                    self.finished = Some(format!("멈춤: {e}"));
                }
                if let Some(run) = self.running.take() {
                    while let Ok(ev) = run.events.try_recv() {
                        if let job::Event::Finished(m) = ev {
                            self.finished = Some(m);
                        }
                    }
                }
            }
        }
    }

    fn drain_job_events(&mut self) {
        let Some(run) = self.running.as_mut() else { return };
        while let Ok(ev) = run.events.try_recv() {
            match ev {
                job::Event::Status(s) => run.status = s,
                job::Event::Progress { done, total, .. } => {
                    run.done = done;
                    run.total = total;
                }
                job::Event::Log(l) => {
                    self.logs.push(l);
                    if self.logs.len() > 300 {
                        self.logs.remove(0);
                    }
                }
                job::Event::Finished(m) => self.finished = Some(m),
            }
        }
    }

    // ---------- 화면 ----------

    fn ui_check(&mut self, ui: &mut egui::Ui) {
        ui.heading("점검");
        ui.label("처음 한 번은 엔진 두 개를 설치하고, 시험 번역이 초록불인지 확인하세요.");
        ui.add_space(8.0);
        egui::Grid::new("check").num_columns(3).spacing([16.0, 10.0]).show(ui, |ui| {
            // OpenCode
            ui.label("OpenCode 엔진");
            match &self.oc_version {
                Some(Ok(v)) => ok(ui, &format!("v{v}")),
                Some(Err(_)) | None => bad(ui, "없음"),
            }
            ui.horizontal(|ui| {
                let latest = self.oc_latest.clone();
                match latest {
                    Some(Ok(l)) => {
                        let cur = self.oc_version.clone().and_then(|v| v.ok());
                        let label = match &cur {
                            None => Some(format!("설치 (v{})", l.version)),
                            Some(c) if update::newer(&l.version, c) => Some(format!("업데이트 (v{})", l.version)),
                            _ => None,
                        };
                        match label {
                            Some(t) => {
                                if ui.add_enabled(self.busy.is_none(), egui::Button::new(t)).clicked() {
                                    self.install_opencode(l);
                                }
                            }
                            None => {
                                ui.label("최신 버전");
                            }
                        }
                    }
                    Some(Err(e)) => {
                        ui.label(format!("업데이트 확인 실패: {e}")).on_hover_text("인터넷이 막혀 있으면 지금 버전 그대로 쓰면 됩니다.");
                        if ui.button("다시 확인").clicked() {
                            self.oc_latest = None;
                            self.bg(|| Msg::Latest(update::latest_opencode()));
                        }
                    }
                    None => {
                        ui.spinner();
                        ui.label("GitHub 에서 확인 중");
                    }
                }
            });
            ui.end_row();

            // PDFium
            ui.label("PDF 엔진");
            if paths::pdfium_dll().exists() { ok(ui, "있음") } else { bad(ui, "없음 (PDF 를 읽으려면 필요)") }
            if ui.add_enabled(self.busy.is_none(), egui::Button::new(if paths::pdfium_dll().exists() { "다시 설치" } else { "설치" })).clicked() {
                self.install_pdfium();
            }
            ui.end_row();

            // 모델
            ui.label("모델");
            if let Some(e) = &self.models_err {
                bad(ui, &format!("불러오기 실패: {}", short(e)));
            } else if self.models.is_empty() {
                bad(ui, "없음");
            } else {
                let free = self.models.iter().filter(|m| m.ends_with("-free")).count();
                ok(ui, &format!("{}개 (무료 {free}개)", self.models.len()));
            }
            if ui.add_enabled(engine::installed(), egui::Button::new("다시 불러오기")).clicked() {
                self.bg(|| Msg::Models(engine::models()));
            }
            ui.end_row();

            // 사용할 모델 (시험·새 번역·이어하기에 모두 쓰임)
            ui.label("사용할 모델");
            model_combo(ui, "model_check", &self.models, &mut self.settings.model);
            ui.label(if self.settings.model.ends_with("-free") { "무료 모델" } else { "유료일 수 있음" });
            ui.end_row();

            // 시험 번역
            ui.label("시험 번역");
            match &self.test {
                Some(Ok(t)) => ok(ui, &format!("성공: {}", short(t))),
                Some(Err(e)) => bad(ui, &format!("실패: {}", short(e))),
                None => {
                    ui.label("아직 안 함");
                }
            }
            let can = engine::installed() && !self.settings.model.is_empty() && self.busy.is_none();
            if ui.add_enabled(can, egui::Button::new("시험하기")).clicked() {
                let model = self.settings.model.clone();
                self.busy = Some("시험 번역 중".into());
                self.bg(move || {
                    let stop = Arc::new(AtomicBool::new(false));
                    Msg::Test(engine::ask(&model, "Translate into Korean. Output only the translation: The quick brown fox jumps over the lazy dog.", "TranslateB 시험", &stop))
                });
            }
            ui.end_row();

            // 로그인
            ui.label("OpenCode 로그인");
            ui.label(auth_summary(&self.auth))
                .on_hover_text(format!("{}

무료 모델은 보통 로그인 없이 됩니다. 시험 번역이 인증 오류로 실패할 때만 로그인하세요.", self.auth.trim()));
            ui.horizontal(|ui| {
                if ui.add_enabled(engine::installed(), egui::Button::new("로그인")).clicked() {
                    self.notice = engine::open_console(&["auth", "login"]).err();
                }
                if ui.add_enabled(engine::installed(), egui::Button::new("로그아웃 (작업 끝내기)")).clicked() {
                    self.notice = engine::open_console(&["auth", "logout"]).err();
                }
                if ui.small_button("새로고침").clicked() {
                    self.bg(|| Msg::Auth(engine::auth_list()));
                }
            });
            ui.end_row();

            // 외부 제공자 (OpenAI 호환 API) 등록
            ui.label("외부 제공자").on_hover_text("OpenAI 와 같은 형식의 API 를 쓰는 곳(River 등)을 등록합니다. 키는 아래 [API 키] 에 같은 이름으로 저장하세요.");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.prov_id).hint_text("이름 (예: river)").desired_width(80.0));
                    ui.add(egui::TextEdit::singleline(&mut self.prov_url).hint_text("주소 (https://…/v1)").desired_width(200.0));
                });
                ui.add(egui::TextEdit::singleline(&mut self.prov_models).hint_text("모델 이름 (여러 개면 쉼표로)").desired_width(288.0));
            });
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    if ui.button("River 값 채우기").clicked() {
                        self.prov_id = "river".into();
                        self.prov_url = "https://api.river.ai/v1".into();
                        self.prov_models = "deepseek-ai/DeepSeek-V4.1-Flash, deepseek-ai/DeepSeek-V4.1-Flash-1017".into();
                        self.key_provider = "river".into();
                    }
                    if ui.button("등록").clicked() {
                        let models: Vec<String> = self.prov_models.split(',').map(String::from).collect();
                        self.notice = Some(match engine::set_provider(&self.prov_id, &self.prov_url, &models) {
                            Ok(()) => {
                                self.key_provider = self.prov_id.trim().to_string();
                                format!("{} 을(를) 등록했습니다. 아래 [API 키] 에 {} 키를 저장한 뒤 [사용할 모델] 에서 {}/… 모델을 고르세요.", self.prov_id.trim(), self.prov_id.trim(), self.prov_id.trim())
                            }
                            Err(e) => e,
                        });
                        self.bg(|| Msg::Models(engine::models()));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for (id, url, models) in engine::custom_providers() {
                        if ui
                            .small_button(format!("{id} 삭제"))
                            .on_hover_text(format!("{url}\n{}", models.join("\n")))
                            .clicked()
                        {
                            self.notice = Some(match engine::remove_provider(&id) {
                                Ok(()) => format!("{id} 등록을 지웠습니다. 키는 [API 키] 에서 따로 지우세요."),
                                Err(e) => e,
                            });
                            self.bg(|| Msg::Models(engine::models()));
                        }
                    }
                });
            });
            ui.end_row();

            // API 키 직접 넣기
            ui.label("API 키");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.key_provider).desired_width(80.0))
                    .on_hover_text("제공자 이름. OpenCode Zen 키는 opencode 그대로 두세요.");
                ui.add(egui::TextEdit::singleline(&mut self.key_input).password(true).hint_text("API 키 붙여넣기").desired_width(200.0));
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.key_input.trim().is_empty(), egui::Button::new("저장")).clicked() {
                    self.notice = Some(match engine::set_api_key(&self.key_provider, &self.key_input) {
                        Ok(()) => format!("{} 키를 저장했습니다. [시험하기] 로 확인해 보세요.", self.key_provider.trim()),
                        Err(e) => e,
                    });
                    // 저장한 뒤에는 화면에서 키를 지운다
                    self.key_input.clear();
                    self.bg(|| Msg::Auth(engine::auth_list()));
                    self.bg(|| Msg::Models(engine::models()));
                }
                for p in engine::saved_providers() {
                    if ui.small_button(format!("{p} 키 삭제")).on_hover_text("공용 PC에서는 다 쓴 뒤 지우세요.").clicked() {
                        self.notice = Some(match engine::remove_api_key(&p) {
                            Ok(()) => format!("{p} 키를 지웠습니다."),
                            Err(e) => e,
                        });
                        self.bg(|| Msg::Auth(engine::auth_list()));
                        self.bg(|| Msg::Models(engine::models()));
                    }
                }
            });
            ui.end_row();
        });

        if let Some(b) = &self.busy {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(b);
            });
            if self.download > 0.0 {
                ui.add(egui::ProgressBar::new(self.download).show_percentage());
            }
        }
        ui.add_space(14.0);
        ui.separator();
        ui.label("엔진과 로그인 정보는 모두 이 프로그램 폴더의 engine 안에만 저장됩니다. USB 나 클라우드 폴더째 들고 다녀도 됩니다.");
    }

    fn ui_new(&mut self, ui: &mut egui::Ui) {
        ui.heading("새 번역");
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(70.0);
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                ui.label("여기에 PDF 또는 DOCX 파일을 끌어다 놓으세요");
                if ui.button("파일 선택…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("원서", &["pdf", "docx"]).pick_file() {
                        self.open_file(p);
                    }
                }
            });
        });
        ui.add_space(6.0);
        if self.loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("원서를 읽는 중…");
            });
            return;
        }
        if let Some(e) = &self.load_err {
            ui.colored_label(egui::Color32::from_rgb(200, 60, 60), e);
            return;
        }
        if self.chapters.is_empty() {
            return;
        }

        ui.horizontal(|ui| {
            ui.label("작업 이름");
            ui.text_edit_singleline(&mut self.book_name);
        });
        ui.horizontal(|ui| {
            ui.label("규칙");
            egui::ComboBox::from_id_salt("rules").selected_text(self.settings.rules.clone()).show_ui(ui, |ui| {
                for r in self.rules_list.clone() {
                    ui.selectable_value(&mut self.settings.rules, r.clone(), r);
                }
            });
            ui.label("모델");
            model_combo(ui, "model_new", &self.models, &mut self.settings.model);
        });
        ui.horizontal(|ui| {
            ui.label("조각 사이 쉬는 시간");
            ui.add(egui::Slider::new(&mut self.settings.delay_secs, 0..=120).suffix("초"));
        })
        .response
        .on_hover_text("무료 모델 사용 한도에 자주 걸리면 늘리세요.");

        ui.horizontal(|ui| {
            ui.label(format!("단원 {}개", self.chapters.len()));
            if ui.small_button("전체 선택").clicked() {
                self.selected.iter_mut().for_each(|s| *s = true);
            }
            if ui.small_button("전체 해제").clicked() {
                self.selected.iter_mut().for_each(|s| *s = false);
            }
        });
        let est_chunks: usize = self
            .chapters
            .iter()
            .zip(&self.selected)
            .filter(|(_, s)| **s)
            .map(|(c, _)| c.chars().div_ceil(crate::chunk::TARGET).max(1))
            .sum();
        egui::ScrollArea::vertical().max_height(ui.available_height() - 60.0).show(ui, |ui| {
            for (i, c) in self.chapters.iter().enumerate() {
                let pages = c.pages.map(|(a, b)| format!(" · {a}~{b}쪽")).unwrap_or_default();
                ui.checkbox(&mut self.selected[i], format!("{:02}  {}{pages} · {}자", i + 1, c.title, c.chars()));
            }
        });
        ui.separator();
        let mins = est_chunks as u64 * (SECS_PER_CHUNK + self.settings.delay_secs) / 60;
        ui.label(format!("약 {est_chunks}조각, 예상 {}시간 {}분 (무료 모델 상태에 따라 달라짐)", mins / 60, mins % 60));
        let can = est_chunks > 0 && self.running.is_none() && engine::installed() && !self.settings.model.is_empty();
        if ui.add_enabled(can, egui::Button::new("번역 시작")).clicked() {
            let picked: Vec<Chapter> = self.chapters.iter().zip(&self.selected).filter(|(_, s)| **s).map(|(c, _)| c.clone()).collect();
            let src = self.file.clone().unwrap_or_default();
            match job::prepare(&self.book_name, &src, &picked, &self.settings.rules) {
                Ok(j) => {
                    self.settings.save();
                    self.chapters.clear();
                    self.file = None;
                    self.start(j, false);
                }
                Err(e) => self.notice = Some(format!("작업 폴더를 만들지 못했습니다: {e}")),
            }
        }
        if self.running.is_some() {
            ui.label("다른 번역이 돌아가는 중입니다. 끝난 뒤에 시작할 수 있습니다.");
        }
    }

    fn ui_jobs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("작업 목록");
            if ui
                .button("기존 작업 가져오기…")
                .on_hover_text("다른 도구로 하던 작업 폴더(원문_조각 폴더가 있는 곳)를 가져와서 이어서 합니다. 원래 폴더는 그대로 둡니다.")
                .clicked()
            {
                if let Some(dir) = rfd::FileDialog::new().set_title("가져올 작업 폴더 선택").pick_folder() {
                    self.notice = Some(match job::import(&dir, &self.settings.rules) {
                        Ok((j, rules)) => {
                            self.rules_list = list_rules();
                            let (d, t) = j.state().map(|s| j.progress(&s)).unwrap_or((0, 0));
                            format!("{} 을(를) 가져왔습니다: {d}/{t}조각 끝남, 규칙 {rules}. [이어하기] 로 남은 조각을 번역합니다.", j.name())
                        }
                        Err(e) => format!("가져오지 못했습니다: {e}"),
                    });
                }
            }
        });
        if let Some(run) = &self.running {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.strong(format!("진행 중: {}", run.name));
                let f = run.done as f32 / run.total.max(1) as f32;
                ui.add(egui::ProgressBar::new(f).text(format!("{}/{}조각", run.done, run.total)));
                let left = (run.total - run.done.min(run.total)) as u64 * (SECS_PER_CHUNK + self.settings.delay_secs) / 60;
                ui.label(format!("{} · 남은 시간 약 {}시간 {}분", run.status, left / 60, left % 60));
                ui.horizontal(|ui| {
                    let paused = run.pause.load(Ordering::Relaxed);
                    if ui.button(if paused { "다시 시작" } else { "일시정지" }).clicked() {
                        run.pause.store(!paused, Ordering::Relaxed);
                    }
                    if ui.button("중지").clicked() {
                        run.stop.store(true, Ordering::Relaxed);
                    }
                });
                egui::ScrollArea::vertical().max_height(140.0).stick_to_bottom(true).show(ui, |ui| {
                    for l in &self.logs {
                        ui.small(l);
                    }
                });
            });
        }
        if let Some(f) = &self.finished {
            ui.colored_label(egui::Color32::from_rgb(40, 150, 70), f);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("이어서 할 때 쓸 모델");
            model_combo(ui, "model_jobs", &self.models, &mut self.settings.model);
        });
        ui.separator();

        let jobs = job::list_jobs();
        if jobs.is_empty() {
            ui.label("아직 작업이 없습니다. [새 번역] 에서 원서를 넣어 주세요.");
        }
        let mut start: Option<(Job, bool)> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for j in jobs {
                let Ok(st) = j.state() else { continue };
                let (d, t) = j.progress(&st);
                let running_this = self.running.as_ref().is_some_and(|r| r.name == j.name());
                ui.horizontal(|ui| {
                    ui.strong(j.name());
                    ui.label(format!("{d}/{t}조각"));
                    if d == t && t > 0 {
                        ok(ui, "검토 준비 완료");
                    }
                    if let Some(h) = j.busy_elsewhere(&st) {
                        bad(ui, &format!("{h} 에서 작업 중"));
                    }
                });
                ui.horizontal(|ui| {
                    if ui.add_enabled(self.running.is_none() && d < t, egui::Button::new("이어하기")).clicked() {
                        start = Some((Job::open(j.dir.clone()), false));
                    }
                    if self.confirm_force.as_ref() == Some(&j.dir) && ui.button("그래도 시작").clicked() {
                        start = Some((Job::open(j.dir.clone()), true));
                    }
                    if ui.button("폴더 열기").clicked() {
                        let _ = std::process::Command::new("explorer").arg(&j.dir).spawn();
                    }
                    if ui.add_enabled(!running_this, egui::Button::new("보고서 다시 만들기")).clicked() {
                        self.notice = Some(match job::report(&j) {
                            Ok(()) => "자동검사_보고서.md 를 다시 만들었습니다.".into(),
                            Err(e) => e,
                        });
                    }
                });
                ui.separator();
            }
        });
        if let Some((j, force)) = start {
            self.start(j, force);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        while let Ok(m) = self.rx.try_recv() {
            self.handle(m);
        }
        self.drain_job_events();
        if self.running.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
        let dropped: Vec<PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(p) = dropped.into_iter().next() {
            self.tab = Tab::New;
            self.open_file(p);
        }

        egui::Panel::top("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Check, "점검");
                ui.selectable_value(&mut self.tab, Tab::New, "새 번역");
                ui.selectable_value(&mut self.tab, Tab::Jobs, "작업 목록");
                if let Some(r) = &self.running {
                    ui.separator();
                    ui.spinner();
                    ui.label(format!("{} {}/{}", r.name, r.done, r.total));
                }
            });
        });
        if let Some(n) = self.notice.clone() {
            egui::Panel::bottom("notice").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(n);
                    if ui.small_button("닫기").clicked() {
                        self.notice = None;
                    }
                });
            });
        }
        egui::CentralPanel::default_margins().show(ui, |ui| match self.tab {
            Tab::Check => self.ui_check(ui),
            Tab::New => self.ui_new(ui),
            Tab::Jobs => self.ui_jobs(ui),
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.settings.save();
        if let Some(r) = &self.running {
            // 돌고 있는 OpenCode 를 끊고 상태를 저장할 시간을 준다
            r.stop.store(true, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(800));
        }
    }
}

/// id 는 화면마다 고정값이어야 한다 (바뀌면 목록을 펼치자마자 닫힌다)
fn model_combo(ui: &mut egui::Ui, id: &str, models: &[String], model: &mut String) {
    egui::ComboBox::from_id_salt(id).selected_text(model.clone()).width(280.0).show_ui(ui, |ui| {
        for m in models {
            ui.selectable_value(model, m.clone(), m);
        }
    });
}

fn list_rules() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(paths::rules_dir())
        .map(|r| r.flatten().filter_map(|e| e.file_name().to_str().map(String::from)).filter(|n| n.ends_with(".md")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn ok(ui: &mut egui::Ui, t: &str) {
    ui.colored_label(egui::Color32::from_rgb(40, 150, 70), format!("● {t}"));
}

fn bad(ui: &mut egui::Ui, t: &str) {
    ui.colored_label(egui::Color32::from_rgb(200, 60, 60), format!("● {t}"));
}

/// `auth list` 출력에서 "N credentials" 만 뽑아 짧게 보여준다.
fn auth_summary(s: &str) -> String {
    let re = regex::Regex::new(r"(\d+)\s+credentials?").unwrap();
    match re.captures(s).and_then(|c| c[1].parse::<usize>().ok()) {
        Some(0) => "로그인 안 함 (무료 모델은 보통 필요 없음)".into(),
        Some(n) => format!("로그인 {n}개"),
        None if s.trim().is_empty() => "-".into(),
        None => short(s).chars().take(40).collect(),
    }
}

fn short(s: &str) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 90 { one.chars().take(90).collect::<String>() + "…" } else { one }
}

/// 윈도우에 기본으로 있는 한글 글꼴을 불러온다 (글꼴 파일을 프로그램에 넣지 않는다).
fn load_korean_font(ctx: &egui::Context) {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    for name in ["malgun.ttf", "gulim.ttc"] {
        if let Ok(bytes) = std::fs::read(PathBuf::from(&dir).join("Fonts").join(name)) {
            use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
            ctx.add_font(FontInsert::new(
                "korean",
                egui::FontData::from_owned(bytes),
                vec![
                    InsertFontFamily { family: egui::FontFamily::Proportional, priority: FontPriority::Highest },
                    InsertFontFamily { family: egui::FontFamily::Monospace, priority: FontPriority::Lowest },
                ],
            ));
            return;
        }
    }
}
