//! 원문 정리와 조각 나누기

use regex::Regex;
use std::collections::HashMap;

/// 조각 하나의 목표 글자 수. 무료 모델의 답변 한 번에 들어가는 크기.
pub const TARGET: usize = 5000;
/// 문장 끝을 못 찾을 때 강제로 자르는 크기
const HARD_MAX: usize = 8000;

/// 여러 쪽에 반복되는 머리말·꼬리말(쪽 번호, 워터마크, 장 제목)을 지운다.
/// 각 쪽의 처음 2줄·마지막 2줄 중 숫자만 다른 같은 줄이 30% 이상의 쪽에 나오면 반복 줄로 본다.
pub fn clean_pages(pages: &[String]) -> Vec<Vec<String>> {
    let digits = Regex::new(r"\d+").unwrap();
    let norm = |s: &str| digits.replace_all(s.trim(), "#").to_string();
    let split: Vec<Vec<String>> = pages
        .iter()
        .map(|p| {
            p.replace("\r\n", "\n")
                .replace('\r', "\n")
                .lines()
                .map(|l| l.trim_end().to_string())
                .filter(|l| !l.trim().is_empty())
                .collect()
        })
        .collect();

    let mut count: HashMap<String, usize> = HashMap::new();
    for lines in &split {
        let n = lines.len();
        let mut edge: Vec<String> = lines.iter().take(2).chain(lines.iter().skip(n.saturating_sub(2))).map(|l| norm(l)).collect();
        edge.sort();
        edge.dedup();
        for e in edge {
            *count.entry(e).or_default() += 1;
        }
    }
    let limit = (split.len() * 3 / 10).max(3);
    let only_num = Regex::new(r"^\s*\d+\s*$").unwrap();

    split
        .into_iter()
        .map(|lines| {
            let n = lines.len();
            lines
                .into_iter()
                .enumerate()
                .filter(|(i, l)| {
                    let edge = *i < 2 || *i + 2 >= n;
                    if edge && (only_num.is_match(l) || count.get(&norm(l)).copied().unwrap_or(0) >= limit) {
                        return false;
                    }
                    true
                })
                .map(|(_, l)| l)
                .collect()
        })
        .collect()
}

/// 문장이 끝나는 줄에서 약 TARGET 글자씩 자른다.
pub fn split(lines: &[String]) -> Vec<String> {
    let end = Regex::new(r#"[.:?!”"。)\]]\s*$"#).unwrap();
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut size = 0;
    for line in lines {
        cur.push(line);
        size += line.chars().count() + 1;
        if (size >= TARGET && end.is_match(line)) || size >= HARD_MAX {
            out.push(cur.join("\n"));
            cur.clear();
            size = 0;
        }
    }
    if !cur.is_empty() {
        let tail = cur.join("\n");
        // 아주 짧은 꼬리는 앞 조각에 붙인다
        match out.last_mut() {
            Some(last) if size < TARGET / 5 => {
                last.push('\n');
                last.push_str(&tail);
            }
            _ => out.push(tail),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_repeated_headers() {
        let words = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet"];
        let pages: Vec<String> = words
            .iter()
            .enumerate()
            .map(|(i, w)| format!("watermark123\n{} Chapter One\nOpening {w} line.\nClosing {w} line.\n{}", i + 1, i + 1))
            .collect();
        let cleaned = clean_pages(&pages);
        for (p, w) in cleaned.iter().zip(words) {
            assert_eq!(p, &vec![format!("Opening {w} line."), format!("Closing {w} line.")]);
        }
    }

    #[test]
    fn splits_at_sentence_end() {
        let lines: Vec<String> = (0..400).map(|i| format!("This is sentence number {i} of the sample.")).collect();
        let chunks = split(&lines);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.ends_with('.'));
            assert!(c.chars().count() <= HARD_MAX + 100);
        }
        assert_eq!(chunks.join("\n"), lines.join("\n"));
    }
}
