//! EPUB import: each chapter in the spine becomes one block of markdown.
//!
//! An EPUB is a zip: `META-INF/container.xml` points at the OPF package, whose
//! spine lists the chapter files in reading order. Chapter titles come from the
//! table of contents (EPUB 2 NCX or EPUB 3 nav), since many books set chapter
//! headings as images.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;

type Zip = zip::ZipArchive<File>;

pub struct Chapter {
    pub title: String,
    pub text: String,
}

/// Parse XML, allowing the `<!DOCTYPE>` lines most EPUB 2 files carry.
fn xml(src: &str) -> Result<roxmltree::Document<'_>, String> {
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    roxmltree::Document::parse_with_options(src, opts).map_err(|e| e.to_string())
}

fn read(zip: &mut Zip, path: &str) -> Result<String, String> {
    let mut f = zip.by_name(path).map_err(|e| format!("{path}: {e}"))?;
    let mut s = String::new();
    f.read_to_string(&mut s).map_err(|e| format!("{path}: {e}"))?;
    Ok(s)
}

/// Resolve `href` (relative, maybe %-encoded, maybe with #fragment) against `base_dir`.
fn join(base_dir: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let href = percent_decode(href);
    let mut parts: Vec<&str> = base_dir.split('/').filter(|p| !p.is_empty()).collect();
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

/// The book's title and its chapters in reading order.
pub fn chapters(path: &str) -> Result<(String, Vec<Chapter>), String> {
    let file = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("Not an EPUB (zip): {e}"))?;

    let container = read(&mut zip, "META-INF/container.xml")?;
    let doc = xml(&container)?;
    let opf_path = doc
        .descendants()
        .find(|n| n.has_tag_name("rootfile"))
        .and_then(|n| n.attribute("full-path"))
        .ok_or("EPUB has no package file")?
        .to_string();
    let opf_dir = dir_of(&opf_path).to_string();

    let opf = read(&mut zip, &opf_path)?;
    let doc = xml(&opf).map_err(|e| format!("Package file: {e}"))?;
    let book_title = doc
        .descendants()
        .find(|n| n.tag_name().name() == "title")
        .and_then(|n| n.text())
        .unwrap_or("Untitled")
        .trim()
        .to_string();

    // manifest id -> (path in zip, media type, properties)
    let mut manifest: HashMap<&str, (String, &str, &str)> = HashMap::new();
    for item in doc.descendants().filter(|n| n.has_tag_name("item")) {
        if let (Some(id), Some(href)) = (item.attribute("id"), item.attribute("href")) {
            manifest.insert(
                id,
                (
                    join(&opf_dir, href),
                    item.attribute("media-type").unwrap_or(""),
                    item.attribute("properties").unwrap_or(""),
                ),
            );
        }
    }

    // Table of contents: chapter file -> title.
    let mut toc: HashMap<String, String> = HashMap::new();
    let ncx_id = doc.descendants().find(|n| n.has_tag_name("spine")).and_then(|n| n.attribute("toc"));
    let ncx = ncx_id
        .and_then(|id| manifest.get(id))
        .or_else(|| manifest.values().find(|(_, mt, _)| *mt == "application/x-dtbncx+xml"))
        .map(|(p, _, _)| p.clone());
    let nav = manifest.values().find(|(_, _, props)| props.split_whitespace().any(|p| p == "nav")).map(|(p, _, _)| p.clone());
    if let Some(ncx) = ncx {
        if let Ok(src) = read(&mut zip, &ncx) {
            if let Ok(d) = xml(&src) {
                for np in d.descendants().filter(|n| n.has_tag_name("navPoint")) {
                    let label = np.descendants().find(|n| n.has_tag_name("text")).and_then(|n| n.text());
                    let src = np.children().find(|n| n.has_tag_name("content")).and_then(|n| n.attribute("src"));
                    if let (Some(label), Some(src)) = (label, src) {
                        toc.entry(join(dir_of(&ncx), src)).or_insert_with(|| label.trim().to_string());
                    }
                }
            }
        }
    } else if let Some(nav) = nav {
        if let Ok(src) = read(&mut zip, &nav) {
            if let Ok(d) = xml(&src) {
                for a in d.descendants().filter(|n| n.has_tag_name("a")) {
                    if let Some(href) = a.attribute("href") {
                        let label: String = a.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).collect();
                        toc.entry(join(dir_of(&nav), href)).or_insert_with(|| label.trim().to_string());
                    }
                }
            }
        }
    }

    let spine: Vec<&str> = doc
        .descendants()
        .filter(|n| n.has_tag_name("itemref") && n.attribute("linear") != Some("no"))
        .filter_map(|n| n.attribute("idref"))
        .collect();

    let mut out = vec![];
    for (i, idref) in spine.iter().enumerate() {
        let Some((path, _, _)) = manifest.get(idref) else { continue };
        let Ok(html) = read(&mut zip, path) else { continue };
        let (text, heading) = html_to_markdown(&html);
        if text.trim().is_empty() {
            continue; // cover art, image-only title pages
        }
        let title = toc
            .get(path)
            .filter(|t| !t.is_empty())
            .cloned()
            .or(heading)
            .unwrap_or_else(|| format!("Section {}", i + 1));
        out.push(Chapter { title, text });
    }
    if out.is_empty() {
        return Err("No readable chapters in that EPUB.".into());
    }
    Ok((book_title, out))
}

fn entity(name: &str) -> Option<String> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code).map(String::from);
    }
    Some(
        match name {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" => "'",
            "nbsp" => " ",
            "mdash" => "—",
            "ndash" => "–",
            "hellip" => "…",
            "lsquo" => "‘",
            "rsquo" => "’",
            "ldquo" => "“",
            "rdquo" => "”",
            "eacute" => "é",
            "copy" => "©",
            _ => return None,
        }
        .to_string(),
    )
}

/// Tolerant XHTML -> markdown: paragraphs, *italics*, **bold**, scene breaks.
/// Returns the text and the first heading's text, if any.
pub fn html_to_markdown(html: &str) -> (String, Option<String>) {
    let mut paras: Vec<String> = vec![];
    let mut cur = String::new();
    // Open emphasis markers: (marker, byte position in `cur`, saw text since).
    let mut emph: Vec<(&'static str, usize, bool)> = vec![];
    let mut skip_depth = 0usize; // inside <head>, <script>, <style>
    let mut in_heading = false;
    let mut heading: Option<String> = None;
    let mut heading_buf = String::new();

    let end_para = |cur: &mut String, paras: &mut Vec<String>| {
        let p = cur.split_whitespace().collect::<Vec<_>>().join(" ");
        if !p.is_empty() {
            paras.push(p);
        }
        cur.clear();
    };

    let mut rest = html;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if skip_depth == 0 {
                push_text(&mut cur, &mut emph, rest);
            }
            break;
        };
        if lt > 0 && skip_depth == 0 {
            let text = &rest[..lt];
            if in_heading {
                heading_buf.push_str(text);
            }
            push_text(&mut cur, &mut emph, text);
        }
        rest = &rest[lt..];
        let close = if rest.starts_with("<!--") { rest.find("-->").map(|i| i + 3) } else { rest.find('>').map(|i| i + 1) };
        let Some(gt) = close else { break };
        let tag = &rest[1..gt - 1];
        rest = &rest[gt..];
        if tag.starts_with('!') || tag.starts_with('?') {
            continue;
        }
        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();

        match name.as_str() {
            "head" | "script" | "style" | "title" => {
                if closing {
                    skip_depth = skip_depth.saturating_sub(1);
                } else if !self_closing {
                    skip_depth += 1;
                }
            }
            _ if skip_depth > 0 => {}
            "p" | "div" | "li" | "blockquote" | "tr" | "section" | "article" => end_para(&mut cur, &mut paras),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                end_para(&mut cur, &mut paras);
                if closing {
                    if heading.is_none() {
                        let h = heading_buf.split_whitespace().collect::<Vec<_>>().join(" ");
                        if !h.is_empty() {
                            heading = Some(decode(&h));
                        }
                    }
                    in_heading = false;
                } else {
                    heading_buf.clear();
                    in_heading = true;
                }
            }
            "br" => cur.push('\n'),
            "hr" => {
                end_para(&mut cur, &mut paras);
                paras.push("* * *".into());
            }
            "i" | "em" | "b" | "strong" | "cite" => {
                let marker = if matches!(name.as_str(), "b" | "strong") { "**" } else { "*" };
                if self_closing {
                } else if !closing {
                    emph.push((marker, cur.len(), false));
                    cur.push_str(marker);
                } else if let Some(pos) = emph.iter().rposition(|(m, _, _)| *m == marker) {
                    let (m, at, saw) = emph.remove(pos);
                    if !saw {
                        cur.replace_range(at..at + m.len(), "");
                    } else {
                        // Keep trailing space outside the marker: "*word* " not "*word *".
                        let trimmed = cur.trim_end().len();
                        let tail = cur[trimmed..].to_string();
                        cur.truncate(trimmed);
                        cur.push_str(m);
                        cur.push_str(&tail);
                    }
                }
            }
            _ => {}
        }
    }
    end_para(&mut cur, &mut paras);

    // A chapter whose only paragraph is its own heading has no prose.
    if paras.len() == 1 && heading.as_deref() == Some(paras[0].as_str()) {
        paras.clear();
    }
    (paras.iter().map(|p| decode(p)).collect::<Vec<_>>().join("\n\n"), heading)
}

/// Append text inside a paragraph, moving leading whitespace in front of a
/// just-opened emphasis marker: "<i> word</i>" -> " *word*".
fn push_text(cur: &mut String, emph: &mut [(&'static str, usize, bool)], text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = emph.last_mut() {
        if !last.2 && last.1 + last.0.len() == cur.len() {
            let body = text.trim_start();
            if body.is_empty() {
                cur.push_str(text);
                return;
            }
            let lead = &text[..text.len() - body.len()];
            cur.insert_str(last.1, lead);
            last.1 += lead.len();
            cur.push_str(body);
            for e in emph.iter_mut() {
                e.2 = true;
            }
            return;
        }
    }
    if !text.trim().is_empty() {
        for e in emph.iter_mut() {
            e.2 = true;
        }
    }
    cur.push_str(text);
}

fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        match rest[1..].find(';').filter(|&i| i <= 10) {
            Some(semi) => match entity(&rest[1..semi + 1]) {
                Some(rep) => {
                    out.push_str(&rep);
                    rest = &rest[semi + 2..];
                }
                None => {
                    out.push('&');
                    rest = &rest[1..];
                }
            },
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_emphasis_and_entities() {
        let html = r#"<html><head><title>X</title></head><body>
            <h1 class="chapter"><img src="a.jpg"/></h1>
            <p>She said<i> hello</i> &amp; left&#8230;</p>
            <p><strong>Bold</strong> start.<b></b></p><hr/><p>After.</p></body></html>"#;
        let (md, heading) = html_to_markdown(html);
        assert_eq!(heading, None);
        assert_eq!(md, "She said *hello* & left…\n\n**Bold** start.\n\n* * *\n\nAfter.");
    }

    #[test]
    fn joins_relative_paths() {
        assert_eq!(join("OEBPS", "text/c01.htm#p1"), "OEBPS/text/c01.htm");
        assert_eq!(join("OEBPS/toc", "../c%2001.htm"), "OEBPS/c 01.htm");
    }
}

/// `EPUB=/path/book.epub cargo test dump_epub -- --ignored --nocapture`
#[cfg(test)]
#[test]
#[ignore]
fn dump_epub() {
    let path = std::env::var("EPUB").expect("set EPUB");
    let (title, chapters) = chapters(&path).unwrap();
    println!("TITLE {title} · {} chapters", chapters.len());
    for c in &chapters {
        let words = c.text.split_whitespace().count();
        println!("{:<28} {:>6} words | {}", c.title, words, c.text.chars().take(70).collect::<String>().replace('\n', " "));
    }
}

/// The book's author (`dc:creator`), if the package names one.
pub fn author(path: &str) -> Option<String> {
    let mut zip = zip::ZipArchive::new(File::open(path).ok()?).ok()?;
    let container = read(&mut zip, "META-INF/container.xml").ok()?;
    let opf_path = xml(&container).ok()?.descendants().find(|n| n.has_tag_name("rootfile"))?.attribute("full-path")?.to_string();
    let opf = read(&mut zip, &opf_path).ok()?;
    let doc = xml(&opf).ok()?;
    let name = doc.descendants().find(|n| n.tag_name().name() == "creator")?.text()?.trim().to_string();
    (!name.is_empty()).then_some(name)
}
