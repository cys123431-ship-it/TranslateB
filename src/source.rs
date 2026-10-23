//! 원서(PDF, DOCX)를 읽어 단원 목록으로 만든다.

use crate::{chunk, paths};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;
use std::process::Command;

/// 원서의 단원 하나
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    /// 단원 안의 절 제목 (목차.md 와 검토용)
    pub sections: Vec<String>,
    /// 정리된 본문 줄
    pub lines: Vec<String>,
    /// PDF 쪽 범위 (1부터, 표시용)
    pub pages: Option<(usize, usize)>,
}

impl Chapter {
    pub fn chars(&self) -> usize {
        self.lines.iter().map(|l| l.chars().count() + 1).sum()
    }
}

pub fn load(path: &Path) -> Result<Vec<Chapter>, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let chapters = match ext.as_str() {
        "pdf" => load_pdf(path)?,
        "docx" => load_docx(path)?,
        _ => return Err("PDF 또는 DOCX 파일만 넣을 수 있습니다.".into()),
    };
    if chapters.iter().all(|c| c.lines.is_empty()) {
        return Err("글자를 하나도 읽지 못했습니다.".into());
    }
    Ok(chapters)
}

// ---------- PDF ----------

/// PDFium 이 뽑은 원본 자료. 별도 프로세스(--extract)에서 만들어 JSON 으로 넘긴다.
#[derive(Serialize, Deserialize)]
pub struct PdfDump {
    pub pages: Vec<String>,
    /// (단계, 제목, 0부터 시작하는 쪽 번호)
    pub outline: Vec<(usize, String, usize)>,
}

/// `translateb --extract <pdf> <out.json>` 의 본체. 깨진 PDF 때문에 프로그램 전체가 죽지 않도록 따로 돈다.
pub fn extract_cli(pdf: &str, out: &str) -> Result<(), String> {
    use pdfium_render::prelude::*;
    let bindings = Pdfium::bind_to_library(paths::pdfium_dll()).map_err(|e| format!("pdfium.dll 을 불러오지 못했습니다: {e}"))?;
    let pdfium = Pdfium::new(bindings);
    let doc = pdfium.load_pdf_from_file(pdf, None).map_err(|e| format!("PDF 를 열지 못했습니다: {e}"))?;

    let mut pages = Vec::new();
    for page in doc.pages().iter() {
        pages.push(page.text().map(|t| t.all()).unwrap_or_default());
    }

    let mut outline = Vec::new();
    fn walk(b: Option<PdfBookmark<'_>>, level: usize, out: &mut Vec<(usize, String, usize)>) {
        let mut cur = b;
        while let Some(bm) = cur {
            if level <= 2 {
                let page = bm.destination().and_then(|d| d.page_index().ok());
                if let (Some(t), Some(p)) = (bm.title(), page) {
                    out.push((level, t.trim().to_string(), p as usize));
                }
                walk(bm.first_child(), level + 1, out);
            }
            cur = bm.next_sibling();
        }
    }
    walk(doc.bookmarks().root(), 1, &mut outline);

    let json = serde_json::to_string(&PdfDump { pages, outline }).map_err(|e| e.to_string())?;
    std::fs::write(out, json).map_err(|e| e.to_string())
}

fn load_pdf(path: &Path) -> Result<Vec<Chapter>, String> {
    if !paths::pdfium_dll().exists() {
        return Err("PDF 엔진(pdfium.dll)이 없습니다. [점검] 화면에서 설치해 주세요.".into());
    }
    let tmp = std::env::temp_dir().join(format!("translateb_{}.json", std::process::id()));
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg("--extract").arg(path).arg(&tmp);
    crate::engine::hide_window(&mut cmd);
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let dump: PdfDump = serde_json::from_slice(&std::fs::read(&tmp).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp);

    let total: usize = dump.pages.iter().map(|p| p.trim().chars().count()).sum();
    if dump.pages.is_empty() || total < dump.pages.len() * 50 {
        return Err("글자가 거의 없는 PDF 입니다. 스캔(사진)으로 된 PDF 는 지원하지 않습니다.".into());
    }
    Ok(pdf_chapters(&dump))
}

/// 1단계 책갈피로 단원을 나눈다. 책갈피가 없으면 20쪽씩 나눈다.
pub fn pdf_chapters(dump: &PdfDump) -> Vec<Chapter> {
    let cleaned = chunk::clean_pages(&dump.pages);
    let n = cleaned.len();
    let mut tops: Vec<(String, usize)> = dump.outline.iter().filter(|o| o.0 == 1).map(|o| (o.1.clone(), o.2)).collect();
    tops.sort_by_key(|t| t.1);
    tops.dedup_by_key(|t| t.1);

    let mut ranges: Vec<(String, usize, usize)> = Vec::new();
    if tops.len() >= 2 {
        if tops[0].1 > 0 {
            ranges.push(("앞부분".into(), 0, tops[0].1));
        }
        for (i, (t, s)) in tops.iter().enumerate() {
            let e = tops.get(i + 1).map(|x| x.1).unwrap_or(n);
            if e > *s {
                ranges.push((t.clone(), *s, e));
            }
        }
    } else {
        let mut s = 0;
        while s < n {
            let e = (s + 20).min(n);
            ranges.push((format!("{}~{}쪽", s + 1, e), s, e));
            s = e;
        }
    }

    ranges
        .into_iter()
        .map(|(title, s, e)| Chapter {
            sections: dump.outline.iter().filter(|o| o.0 == 2 && o.2 >= s && o.2 < e).map(|o| o.1.clone()).collect(),
            lines: cleaned[s..e].concat(),
            pages: Some((s + 1, e)),
            title,
        })
        .collect()
}

// ---------- DOCX ----------

fn load_docx(path: &Path) -> Result<Vec<Chapter>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "DOCX 파일을 열지 못했습니다.".to_string())?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|_| "DOCX 안에 본문(word/document.xml)이 없습니다.".to_string())?
        .read_to_string(&mut xml)
        .map_err(|e| e.to_string())?;
    let paras = docx_paragraphs(&xml)?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("문서").to_string();
    Ok(docx_chapters(&paras, &stem))
}

/// (제목 단계 0=본문, 글자) 목록
pub fn docx_paragraphs(xml: &str) -> Result<Vec<(usize, String)>, String> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let heading = regex::Regex::new(r"(?i)^(heading|제목)\s*([1-9])$|^([1-9])$").unwrap();
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let (mut text, mut level, mut in_t) = (String::new(), 0usize, false);
    loop {
        match reader.read_event().map_err(|e| format!("DOCX 해석 실패: {e}"))? {
            Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                "t" => in_t = true,
                "tab" => text.push('\t'),
                "br" | "cr" => text.push('\n'),
                "pStyle" => {
                    for a in e.attributes().flatten() {
                        if a.key.local_name().as_ref() == "val" {
                            let v = a.value.to_string();
                            if let Some(c) = heading.captures(&v) {
                                level = c.get(2).or(c.get(3)).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
                            }
                        }
                    }
                }
                _ => {}
            },
            Event::Text(t) if in_t => text.push_str(&t.xml10_content()),
            Event::GeneralRef(r) if in_t => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    text.push(c);
                } else {
                    text.push_str(match r.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                "t" => in_t = false,
                "p" => {
                    out.push((level, std::mem::take(&mut text)));
                    level = 0;
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// 가장 높은 제목 단계로 단원을, 그다음 단계로 절을 나눈다.
pub fn docx_chapters(paras: &[(usize, String)], stem: &str) -> Vec<Chapter> {
    let top = paras.iter().filter(|p| p.0 > 0).map(|p| p.0).min();
    let Some(top) = top else {
        let lines = paras.iter().map(|p| p.1.clone()).filter(|l| !l.trim().is_empty()).collect();
        return vec![Chapter { title: stem.into(), sections: vec![], lines, pages: None }];
    };
    let mut chapters: Vec<Chapter> = Vec::new();
    for (lv, t) in paras {
        if t.trim().is_empty() {
            continue;
        }
        if *lv == top || chapters.is_empty() {
            let title = if *lv == top { t.trim().to_string() } else { "앞부분".into() };
            chapters.push(Chapter { title, sections: vec![], lines: vec![], pages: None });
        }
        let ch = chapters.last_mut().unwrap();
        if *lv == top + 1 {
            ch.sections.push(t.trim().to_string());
        }
        ch.lines.push(t.clone());
    }
    chapters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docx_headings_split_chapters() {
        let xml = r#"<w:document xmlns:w="x"><w:body>
            <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Part A</w:t></w:r></w:p>
            <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Intro</w:t></w:r></w:p>
            <w:p><w:r><w:t>Tom &amp; Jerry</w:t><w:tab/><w:t>x</w:t></w:r></w:p>
            <w:p><w:pPr><w:pStyle w:val="1"/></w:pPr><w:r><w:t>Part B</w:t></w:r></w:p>
            <w:p><w:r><w:t>Body</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let paras = docx_paragraphs(xml).unwrap();
        let ch = docx_chapters(&paras, "doc");
        assert_eq!(ch.len(), 2);
        assert_eq!(ch[0].title, "Part A");
        assert_eq!(ch[0].sections, vec!["Intro"]);
        assert!(ch[0].lines.contains(&"Tom & Jerry\tx".to_string()));
        assert_eq!(ch[1].lines, vec!["Part B", "Body"]);
    }

    #[test]
    fn pdf_without_outline_uses_page_groups() {
        let dump = PdfDump { pages: (0..45).map(|i| format!("Text of page {i} goes here.")).collect(), outline: vec![] };
        let ch = pdf_chapters(&dump);
        assert_eq!(ch.len(), 3);
        assert_eq!(ch[2].pages, Some((41, 45)));
    }
}
