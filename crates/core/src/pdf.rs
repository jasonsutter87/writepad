//! PDF import. A PDF only knows where glyphs sit on a page, so structure is
//! rebuilt from layout: larger type is a heading (and starts a block), a
//! taller gap or a first-line indent starts a paragraph, and short text that
//! repeats in the top or bottom margin (running heads, page numbers) is dropped.

use pdf_extract::{MediaBox, OutputDev, OutputError, Transform};
use std::collections::HashMap;

pub struct Section {
    /// None for text before the first heading.
    pub title: Option<String>,
    pub text: String,
}

#[derive(Clone, Debug)]
struct Line {
    page: u32,
    /// Distance from the top of the page.
    y: f64,
    x0: f64,
    x1: f64,
    size: f64,
    page_height: f64,
    text: String,
}

#[derive(Default)]
struct Collector {
    lines: Vec<Line>,
    page: u32,
    page_top: f64,
    page_height: f64,
    new_word: bool,
}

impl OutputDev for Collector {
    fn begin_page(&mut self, page_num: u32, media_box: &MediaBox, _: Option<(f64, f64, f64, f64)>) -> Result<(), OutputError> {
        self.page = page_num;
        self.page_top = media_box.ury;
        self.page_height = media_box.ury - media_box.lly;
        Ok(())
    }
    fn end_page(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
    fn output_character(&mut self, trm: &Transform, width: f64, _spacing: f64, font_size: f64, ch: &str) -> Result<(), OutputError> {
        // Same effective size pdf-extract's own text output uses.
        let (vx, vy) = (font_size * (trm.m11 + trm.m21), font_size * (trm.m12 + trm.m22));
        let size = (vx * vy).abs().sqrt();
        let (x, y) = (trm.m31, self.page_top - trm.m32);
        let end = x + width * size;
        let same_line = self.lines.last().is_some_and(|l| {
            l.page == self.page && (l.y - y).abs() < size.max(l.size) * 0.5 && x > l.x0 - size
        });
        if same_line {
            let l = self.lines.last_mut().unwrap();
            let gap = x - l.x1;
            if (self.new_word && gap > size * 0.1) || gap > size * 0.25 {
                if !l.text.ends_with(' ') {
                    l.text.push(' ');
                }
            }
            l.text.push_str(ch);
            l.x1 = l.x1.max(end);
            l.size = l.size.max(size);
        } else {
            self.lines.push(Line {
                page: self.page,
                y,
                x0: x,
                x1: end,
                size,
                page_height: self.page_height,
                text: ch.to_string(),
            });
        }
        self.new_word = false;
        Ok(())
    }
    fn begin_word(&mut self) -> Result<(), OutputError> {
        self.new_word = true;
        Ok(())
    }
    fn end_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
    fn end_line(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
}

fn round_half(v: f64) -> i64 {
    (v * 2.0).round() as i64
}

/// The most common value, weighting each line by its length.
fn mode_by<F: Fn(&Line) -> f64>(lines: &[&Line], f: F) -> f64 {
    let mut counts: HashMap<i64, usize> = HashMap::new();
    for l in lines {
        *counts.entry(round_half(f(l))).or_default() += l.text.chars().count();
    }
    counts.into_iter().max_by_key(|&(_, n)| n).map_or(0.0, |(k, _)| k as f64 / 2.0)
}

fn is_ornament(text: &str) -> bool {
    text.chars().all(|c| c.is_whitespace() || "·•∙*⁂~—–-=_".contains(c))
}

/// Margin text with the digits taken out, so "Page 12" and "Page 13" match.
fn furniture_key(text: &str) -> String {
    text.chars().filter(|c| !c.is_ascii_digit()).collect::<String>().trim().to_lowercase()
}

pub fn sections(path: &str) -> Result<Vec<Section>, String> {
    let mut doc = pdf_extract::Document::load(path).map_err(|e| format!("{path}: {e}"))?;
    if doc.is_encrypted() {
        doc.decrypt("").map_err(|_| "That PDF is password-protected.".to_string())?;
    }
    let mut c = Collector::default();
    pdf_extract::output_doc(&doc, &mut c).map_err(|e| format!("Couldn't read that PDF: {e}"))?;
    let lines: Vec<Line> = c
        .lines
        .into_iter()
        .map(|mut l| {
            l.text = l.text.split_whitespace().collect::<Vec<_>>().join(" ");
            l
        })
        .filter(|l| !l.text.is_empty())
        .collect();
    if lines.is_empty() {
        return Err("No text in that PDF. If it's a scan, it needs OCR first.".into());
    }
    Ok(structure(&lines))
}

fn structure(all: &[Line]) -> Vec<Section> {
    let pages = all.iter().map(|l| l.page).collect::<std::collections::HashSet<_>>().len().max(1);

    // Running heads and page numbers: short lines in the outer 8% of the page
    // that are bare numbers or repeat on many pages.
    let in_margin = |l: &Line| l.y < l.page_height * 0.08 || l.y > l.page_height * 0.92;
    let mut repeats: HashMap<String, usize> = HashMap::new();
    for l in all.iter().filter(|l| in_margin(l)) {
        *repeats.entry(furniture_key(&l.text)).or_default() += 1;
    }
    let furniture = |l: &Line| {
        in_margin(l)
            && l.text.chars().count() < 60
            && (furniture_key(&l.text).is_empty() || repeats[&furniture_key(&l.text)] * 4 >= pages && pages >= 4)
    };
    let lines: Vec<&Line> = all.iter().filter(|l| !furniture(l) && !is_ornament(&l.text)).collect();

    let body = mode_by(&lines, |l| l.size);
    let body_lines: Vec<&Line> = lines.iter().copied().filter(|l| round_half(l.size) == round_half(body)).collect();
    let left = mode_by(&body_lines, |l| l.x0);
    let mut rights: Vec<f64> = body_lines.iter().map(|l| l.x1).collect();
    rights.sort_by(f64::total_cmp);
    let right = rights.get(rights.len() * 9 / 10).copied().unwrap_or(left + 400.0);
    let mut gaps: Vec<f64> = body_lines
        .windows(2)
        .filter(|w| w[0].page == w[1].page && w[1].y > w[0].y)
        .map(|w| w[1].y - w[0].y)
        .collect();
    gaps.sort_by(f64::total_cmp);
    let leading = gaps.get(gaps.len() / 2).copied().unwrap_or(body * 1.4);

    let is_heading = |l: &Line| l.size >= body * 1.3 && l.text.chars().count() <= 80;

    let mut sections: Vec<Section> = vec![];
    let mut title: Option<String> = None;
    let mut paras: Vec<String> = vec![];
    let mut cur = String::new();
    let mut prev: Option<&Line> = None;

    let end_para = |cur: &mut String, paras: &mut Vec<String>| {
        let p = cur.trim().to_string();
        if !p.is_empty() {
            paras.push(p);
        }
        cur.clear();
    };

    for &l in &lines {
        if is_heading(l) {
            end_para(&mut cur, &mut paras);
            // A heading that wraps onto a second line continues the same title.
            let continues = prev.is_some_and(|p| is_heading(p) && p.page == l.page && l.y - p.y < l.size * 2.0);
            if continues && paras.is_empty() {
                if let Some(t) = title.as_mut() {
                    t.push(' ');
                    t.push_str(&l.text);
                }
            } else {
                if title.is_some() || !paras.is_empty() {
                    sections.push(Section { title: title.take(), text: paras.join("\n\n") });
                    paras.clear();
                }
                title = Some(l.text.clone());
            }
            prev = Some(l);
            continue;
        }

        let new_para = match prev {
            None => true,
            Some(p) if is_heading(p) => true,
            Some(p) => {
                let gap = if p.page == l.page { l.y - p.y } else { 0.0 };
                let indented = l.x0 > left + body * 0.8 && l.x0 < left + body * 4.0;
                let p_short = p.x1 < right - (right - left) * 0.12;
                let p_ended = p.text.ends_with(['.', '!', '?', '"', '”', '’', ':', ')']);
                let not_body = round_half(l.size) != round_half(p.size);
                gap > leading * 1.35 || gap < 0.0 || indented || (p_short && p_ended) || not_body
            }
        };
        if new_para {
            end_para(&mut cur, &mut paras);
            cur.push_str(&l.text);
        } else if cur.ends_with('-') && !cur.ends_with("--") && l.text.starts_with(|c: char| c.is_lowercase()) {
            // Typeset hyphenation: "pio-" + "neer".
            cur.pop();
            cur.push_str(&l.text);
        } else {
            cur.push(' ');
            cur.push_str(&l.text);
        }
        prev = Some(l);
    }
    end_para(&mut cur, &mut paras);
    if title.is_some() || !paras.is_empty() {
        sections.push(Section { title, text: paras.join("\n\n") });
    }

    // No headings at all: fall back to blocks of ~10 pages.
    if sections.len() == 1 && sections[0].title.is_none() {
        return by_pages(&lines, 10);
    }
    sections
}

fn by_pages(lines: &[&Line], per: u32) -> Vec<Section> {
    let mut groups: Vec<(u32, u32, Vec<&str>)> = vec![];
    for l in lines {
        let group = (l.page - 1) / per;
        match groups.last_mut() {
            Some((g, last, texts)) if *g == group => {
                *last = l.page;
                texts.push(&l.text);
            }
            _ => groups.push((group, l.page, vec![&l.text])),
        }
    }
    groups
        .into_iter()
        .map(|(g, last, texts)| Section { title: Some(format!("Pages {}–{}", g * per + 1, last)), text: texts.join(" ") })
        .collect()
}

/// `PDF=/path/book.pdf cargo test dump_pdf -- --ignored --nocapture`
#[cfg(test)]
#[test]
#[ignore]
fn dump_pdf() {
    let path = std::env::var("PDF").expect("set PDF");
    let s = sections(&path).unwrap();
    println!("{} sections", s.len());
    for sec in &s {
        let paras = sec.text.split("\n\n").count();
        println!("[{}] {} paras | {}", sec.title.as_deref().unwrap_or("-"), paras, sec.text.chars().take(90).collect::<String>().replace('\n', "⏎"));
    }
}

/// `PDF=... SECTION=1 cargo test print_pdf_section -- --ignored --nocapture`
#[cfg(test)]
#[test]
#[ignore]
fn print_pdf_section() {
    let (path, want) = (std::env::var("PDF").unwrap(), std::env::var("SECTION").unwrap());
    let s = sections(&path).unwrap();
    let sec = s.iter().find(|s| s.title.as_deref() == Some(want.as_str())).expect("no such section");
    println!("<<<{}>>>", sec.text);
}
