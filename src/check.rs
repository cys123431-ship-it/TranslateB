//! 번역 결과 자동 검사. 기계로 잡을 수 있는 누락·요약·코드 변경만 본다.

use regex::Regex;
use std::sync::OnceLock;

/// 원문 대비 번역 길이가 이보다 짧으면 요약으로 본다.
pub const MIN_RATIO: f64 = 0.3;
/// 원문 코드 줄이 이 비율 이상 그대로 남아 있어야 한다.
pub const MIN_CODE_KEPT: f64 = 0.8;

pub fn looks_like_code(line: &str) -> bool {
    let s = line.trim();
    s.starts_with("#include") || s.starts_with("#define") || (s.chars().count() > 3 && (s.ends_with(';') || s.ends_with('{')))
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// (원문 코드 줄 수, 그대로 남은 줄 수)
pub fn code_kept(src: &str, dst: &str) -> (usize, usize) {
    let flat = squash(dst);
    let code: Vec<String> = src.lines().filter(|l| looks_like_code(l)).map(squash).collect();
    let kept = code.iter().filter(|c| flat.contains(c.as_str())).count();
    (code.len(), kept)
}

pub fn ratio(src: &str, dst: &str) -> f64 {
    dst.chars().count() as f64 / src.chars().count().max(1) as f64
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
    fn finds_untranslated_and_terms() {
        let dst = "이것은 번역된 줄입니다.\nThis line was clearly left in English by the model, sadly.\n`pointer` 는 괜찮고 pointer 는 아니다.";
        assert_eq!(untranslated_lines(dst).len(), 1);
        let terms = glossary("pointer=포인터, array=배열");
        assert_eq!(glossary_misses(dst, &terms), vec![("pointer → 포인터".to_string(), 1)]);
    }
}
