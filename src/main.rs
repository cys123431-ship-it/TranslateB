#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod check;
mod chunk;
mod engine;
mod job;
mod paths;
mod source;
mod update;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        // 원서 PDF 글자 뽑기 (프로그램이 스스로 따로 띄우는 작업)
        Some("--extract") if args.len() == 4 => {
            if let Err(e) = source::extract_cli(&args[2], &args[3]) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        // 배포 묶음을 만들 때 엔진을 미리 받아 두는 명령
        Some("--install-engines") => {
            let step = |name: &str, r: Result<String, String>| match r {
                Ok(v) => println!("{name} {v} 설치 완료"),
                Err(e) => {
                    eprintln!("{name} 설치 실패: {e}");
                    std::process::exit(1);
                }
            };
            step("OpenCode", update::latest_opencode().and_then(|l| update::install_opencode(&l, &|_| {})));
            step("PDFium", update::install_pdfium(&|_| {}));
        }
        // 기존 작업 폴더 가져오기 (화면의 [기존 작업 가져오기] 와 같음)
        Some("--import") if args.len() == 3 => match job::import(std::path::Path::new(&args[2]), "프로그래밍.md") {
            Ok((j, rules)) => {
                let (d, t) = j.state().map(|s| j.progress(&s)).unwrap_or((0, 0));
                println!("{} 가져옴: {d}/{t}조각 끝남, 규칙 {rules}", j.dir.display());
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        _ => app::run(),
    }
}
