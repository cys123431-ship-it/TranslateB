//! 번역 결과 자동 검사. 기계로 잡을 수 있는 누락·요약·코드 변경만 본다.

use regex::Regex;
use std::sync::OnceLock;

/// 원문 대비 번역 길이가 이보다 짧으면 요약으로 본다.
pub const MIN_RATIO: f64 = 0.3;
/// 원문 코드 줄이 이 비율 이상 그대로 남아 있어야 한다.
pub const MIN_CODE_KEPT: f64 = 0.8;

/// 코드 기호 개수 (= ( ) { } [ ] * & < > % " ;)
fn code_symbols(s: &str) -> usize {
    s.chars().filter(|c| "=(){}[]*&<>%\";".contains(*c)).count()
}

/// 글자가 거의 없고 기호·숫자로 된 줄 (그림 데이터, 표 테두리 등). 그대로 남아야 한다.
pub fn looks_like_art(line: &str) -> bool {
    let s: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    let n = s.chars().count();
    n >= 8 && s.chars().filter(|c| c.is_alphabetic()).count() * 10 <= n
}

/// 원문에서 번역문에 그대로 남아야 하는 코드 줄인지.
/// 세미콜론으로 끝나는 영어 문장, `#include` 로 시작하는 제목·문장은 코드로 보지 않는다.
pub fn looks_like_code(line: &str) -> bool {
    static RE: OnceLock<(Regex, Regex, Regex)> = OnceLock::new();
    let (include, define, keyword) = RE.get_or_init(|| {
        (
            Regex::new(r#"^#\s*include\s*[<"]"#).unwrap(),
            Regex::new(r"^#\s*define\s+[A-Za-z_]").unwrap(),
            Regex::new(r"^(return|break|continue|goto|int|char|float|double|long|short|unsigned|signed|void|struct|union|enum|typedef|static|const|extern|register|volatile|auto|bool|size_t)\b").unwrap(),
        )
    });
    let s = code_part(line.trim());
    if s.starts_with('#') {
        return include.is_match(s) || define.is_match(s);
    }
    s.chars().count() > 3 && (s.ends_with(';') || s.ends_with('{')) && (code_symbols(s) >= 2 || keyword.is_match(s))
}

/// "a.    x = 1;" 의 번호, "설명:    int x;" 의 설명처럼 코드 앞에 붙은 부분을 떼어낸다.
/// (PDF 에서 칸 사이는 공백 여러 개로 나온다)
fn code_part(s: &str) -> &str {
    static LABEL: OnceLock<Regex> = OnceLock::new();
    let label = LABEL.get_or_init(|| Regex::new(r"^(?:[a-zA-Z]|\d{1,3})[.)]\s+").unwrap());
    let s = match label.find(s) {
        Some(m) => &s[m.end()..],
        None => s,
    };
    match s.rfind("  ") {
        Some(i) if s[..i].trim_end().ends_with(':') => s[i..].trim_start(),
        _ => s,
    }
}

/// 비교용: 공백·백틱을 지우고 둥근 따옴표를 곧은 따옴표로 바꾼다.
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '`')
        .map(|c| match c {
            '‘' | '’' => '\'',
            '“' | '”' => '"',
            _ => c,
        })
        .collect()
}

/// 원문에서 그대로 남아야 하는 줄 (코드 + 기호 그림)
fn must_keep(src: &str) -> Vec<String> {
    src.lines()
        .filter(|l| looks_like_code(l) || looks_like_art(l))
        .map(|l| squash(if looks_like_code(l) { code_part(l.trim()) } else { l }))
        .collect()
}

/// (원문 코드 줄 수, 그대로 남은 줄 수)
pub fn code_kept(src: &str, dst: &str) -> (usize, usize) {
    let flat = squash(dst);
    let code = must_keep(src);
    let kept = code.iter().filter(|c| flat.contains(c.as_str())).count();
    (code.len(), kept)
}

/// 번역 길이 / 원문 길이. 기호 그림 줄은 번역 대상이 아니라 원문 길이에서 뺀다.
pub fn ratio(src: &str, dst: &str) -> f64 {
    let art: usize = src.lines().filter(|l| looks_like_art(l)).map(|l| l.chars().count() + 1).sum();
    let src_len = src.chars().count().saturating_sub(art).max(1);
    dst.chars().count() as f64 / src_len as f64
}

/// 다시 시켜야 하면 이유를, 괜찮으면 None
pub fn verdict(src: &str, dst: &str, code_check: bool) -> Option<String> {
    let r = ratio(src, dst);
    if r < MIN_RATIO {
        return Some(format!("너무 짧음 (원문 {}자, 번역 {}자)", src.chars().count(), dst.chars().count()));
    }
    if code_check {
        let (n, kept) = code_kept(src, dst);
        if n > 0 && (kept as f64) < n as f64 * MIN_CODE_KEPT {
            return Some(format!("코드 줄 누락/변경 ({n}줄 중 {kept}줄만 그대로)"));
        }
    }
    None
}

/// 코드 블록·백틱 밖에서 영어가 대부분인 긴 줄 (번역이 안 된 것으로 의심)
pub fn untranslated_lines(dst: &str) -> Vec<String> {
    static TICK: OnceLock<Regex> = OnceLock::new();
    let tick = TICK.get_or_init(|| Regex::new(r"`[^`]*`|\([^)]*\)").unwrap());
    let mut in_code = false;
    let mut out = Vec::new();
    for line in dst.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        let plain = tick.replace_all(line, "");
        let letters = plain.chars().filter(|c| c.is_alphabetic()).count();
        let latin = plain.chars().filter(|c| c.is_ascii_alphabetic()).count();
        if letters >= 40 && latin as f64 > letters as f64 * 0.6 {
            out.push(line.chars().take(80).collect());
        }
    }
    out
}

/// 규칙 파일의 "english=한국어" 용어 쌍
pub fn glossary(rules: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in rules.lines().filter(|l| l.contains('=') && l.contains(',')) {
        for pair in line.split(',') {
            if let Some((en, ko)) = pair.split_once('=') {
                let en = en.trim();
                if en.is_ascii() && !en.is_empty() && en.len() < 40 {
                    out.push((en.to_string(), ko.trim().to_string()));
                }
            }
        }
    }
    out
}

/// 번역문 본문(코드·백틱·괄호 밖)에 영어 용어가 그대로 남은 횟수
pub fn glossary_misses(dst: &str, terms: &[(String, String)]) -> Vec<(String, usize)> {
    let strip = Regex::new(r"(?s)```.*?```|`[^`]*`|\([^)]*\)").unwrap();
    let plain = strip.replace_all(dst, "").to_lowercase();
    let mut out = Vec::new();
    for (en, ko) in terms {
        for word in en.split('/') {
            let w = word.trim().to_lowercase();
            if w.len() < 4 {
                continue;
            }
            let re = Regex::new(&format!(r"\b{}\b", regex::escape(&w))).unwrap();
            let n = re.find_iter(&plain).count();
            if n > 0 {
                out.push((format!("{} → {}", word.trim(), ko), n));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_answer_is_rejected() {
        let src = "a".repeat(1000);
        assert!(verdict(&src, "요약", false).is_some());
        assert!(verdict(&src, &"가".repeat(500), false).is_none());
    }

    #[test]
    fn changed_code_is_rejected() {
        let src = "Text.\nint x = 1;\nprintf(\"%d\", x);\nreturn 0;\n";
        let good = "본문.\n```c\nint x = 1;\nprintf(\"%d\", x);\nreturn 0;\n```";
        let bad = "본문.\n```c\nint y = 1;\n```";
        assert!(verdict(src, good, true).is_none());
        assert!(verdict(src, &format!("{bad}{}", "가".repeat(30)), true).is_some());
    }

    #[test]
    fn prose_and_headings_are_not_code() {
        // 세미콜론으로 끝나는 문장, #include 로 시작하는 제목·문장
        assert!(!looks_like_code("the compiler reads the whole file before it starts;"));
        assert!(!looks_like_code("#include  Directives and Other Things"));
        assert!(!looks_like_code("#include  line shows up here. In effect, it is a copy step."));
        assert!(looks_like_code("#include <stdio.h>"));
        assert!(looks_like_code("#define LIMIT 60"));
        assert!(looks_like_code("return 0;"));
        assert!(looks_like_code("while (n < 10) {"));
    }

    #[test]
    fn labeled_code_lines_compare_only_the_code() {
        let src = "a.    total = (2 + 3) * 6;\nDeclaration example:    int count;\nb.    show('Q');";
        let dst = "a. `total = (2 + 3) * 6;`\n선언 예: `int count;`\nb. show(‘Q’);";
        assert_eq!(code_kept(src, dst), (3, 3));
    }

    #[test]
    fn symbol_art_must_stay_but_does_not_count_as_text() {
        let art = "#####****%%%%####\n**%%##..##%%**";
        let src = format!("{art}\n{}", "Some exercise text here. ".repeat(20));
        let translated = "연습 문제 문장입니다. ".repeat(15);
        // 그림 줄을 빼먹으면 코드 검사에 걸린다
        assert!(verdict(&src, &translated, true).is_some());
        // 그림을 그대로 두면 통과하고, 그림 길이는 원문 길이에 넣지 않는다
        let with_art = format!("```\n{art}\n```\n{translated}");
        assert!(verdict(&src, &with_art, true).is_none());
        assert!(ratio(&src, &translated) > 0.3);
    }

    #[test]
    fn finds_untranslated_and_terms() {
        let dst = "이것은 번역된 줄입니다.\nThis line was clearly left in English by the model, sadly.\n`pointer` 는 괜찮고 pointer 는 아니다.";
        assert_eq!(untranslated_lines(dst).len(), 1);
        let terms = glossary("pointer=포인터, array=배열");
        assert_eq!(glossary_misses(dst, &terms), vec![("pointer → 포인터".to_string(), 1)]);
    }
}

/// 실제 작업 폴더의 원문 조각을 새 규칙으로 다시 분류한다 (숫자만 출력).
/// TB_JOB=<작업 폴더> TB_IDS=02/002,05/012 cargo test reclassify -- --ignored --nocapture
#[cfg(test)]
mod reclassify {
    #[test]
    #[ignore]
    fn reclassify() {
        let job = std::env::var("TB_JOB").unwrap();
        for id in std::env::var("TB_IDS").unwrap().split(',') {
            let src = std::fs::read_to_string(format!("{job}/원문_조각/{id}.txt")).unwrap();
            let code = src.lines().filter(|l| super::looks_like_code(l)).count();
            let art = src.lines().filter(|l| super::looks_like_art(l)).count();
            let art_len: usize = src.lines().filter(|l| super::looks_like_art(l)).map(|l| l.chars().count() + 1).sum();
            println!("{id}: 코드 {code}줄, 기호그림 {art}줄, 번역 대상 길이 {}자", src.chars().count() - art_len);
        }
    }
}
