//! Word-processor exports: Word (.docx), OpenDocument (.odt) and Rich Text
//! (.rtf). All three come from one layout of the book, so a chapter looks the
//! same in each: a title page, every chapter on a new page, the first
//! paragraph after a heading or break unindented, `**bold**`, `*italic*`,
//! centered and right-aligned lines, and `* * *` scene breaks.

use crate::export::{alignment, Book};
use std::fs;
use std::io::Write;
use std::path::Path;

/// A run of text with its emphasis.
struct Run {
    text: String,
    bold: bool,
    italic: bool,
}

enum Para {
    Title(String),
    Byline(String),
    /// A chapter heading; starts a new page.
    Chapter(String),
    /// A `##` heading inside a chapter.
    Sub(String),
    Break,
    Text { align: Option<&'static str>, lines: Vec<String>, first: bool },
}

/// Inline markdown -> runs: `**bold**`, `*italic*`, `[[target|alias]]` as its text.
fn runs(s: &str) -> Vec<Run> {
    let (mut out, mut cur) = (vec![], String::new());
    let (mut bold, mut italic) = (false, false);
    let mut rest = s;
    let flush = |out: &mut Vec<Run>, cur: &mut String, bold, italic| {
        if !cur.is_empty() {
            out.push(Run { text: std::mem::take(cur), bold, italic });
        }
    };
    while let Some(i) = rest.find(['*', '[']) {
        cur.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(r) = rest.strip_prefix("**") {
            flush(&mut out, &mut cur, bold, italic);
            bold = !bold;
            rest = r;
        } else if let Some(r) = rest.strip_prefix('*') {
            flush(&mut out, &mut cur, bold, italic);
            italic = !italic;
            rest = r;
        } else if let (Some(r), Some(end)) = (rest.strip_prefix("[["), rest.find("]]")) {
            let inner = &r[..end - 2];
            cur.push_str(inner.rsplit('|').next().unwrap_or(inner));
            rest = &rest[end + 2..];
        } else {
            cur.push('[');
            rest = &rest[1..];
        }
    }
    cur.push_str(rest);
    flush(&mut out, &mut cur, bold, italic);
    out
}

fn layout(book: &Book) -> Vec<Para> {
    let mut out = vec![];
    let starts_with_title = book.chapters.first().is_some_and(|c| c.title.is_none() && c.text.trim_start().starts_with("# "));
    if !starts_with_title {
        out.push(Para::Title(book.title.clone()));
        if !book.author.is_empty() {
            out.push(Para::Byline(book.author.clone()));
        }
    }
    for ch in &book.chapters {
        let mut first = true;
        if let Some(t) = &ch.title {
            out.push(Para::Chapter(t.clone()));
        }
        for para in ch.text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
            if matches!(para, "---" | "* * *" | "***" | "·") {
                out.push(Para::Break);
                first = true;
            } else if let Some(h) = para.strip_prefix("# ") {
                out.push(Para::Title(h.to_string()));
                first = true;
            } else if let Some(h) = para.strip_prefix("## ").or(para.strip_prefix("### ")) {
                out.push(Para::Sub(h.to_string()));
                first = true;
            } else {
                let align = alignment(para.lines().next().unwrap_or("")).0;
                let lines = para.lines().map(|l| if align.is_some() { alignment(l).1 } else { l }.to_string()).collect();
                out.push(Para::Text { align, lines, first });
                first = false;
            }
        }
    }
    out
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn zip_files(path: &Path, files: &[(&str, &str)], mimetype: Option<&str>) -> Result<(), String> {
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;
    let file = fs::File::create(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let err = |e: std::io::Error| e.to_string();
    if let Some(m) = mimetype {
        // ODF: the mimetype entry comes first, uncompressed.
        zip.start_file("mimetype", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).map_err(|e| e.to_string())?;
        zip.write_all(m.as_bytes()).map_err(err)?;
    }
    for (name, data) in files {
        zip.start_file(*name, SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).map_err(|e| e.to_string())?;
        zip.write_all(data.as_bytes()).map_err(err)?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------- .docx

fn docx_runs(lines: &[String]) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push_str("<w:r><w:br/></w:r>");
        }
        for r in runs(line) {
            let props = match (r.bold, r.italic) {
                (false, false) => "",
                (true, false) => "<w:rPr><w:b/></w:rPr>",
                (false, true) => "<w:rPr><w:i/></w:rPr>",
                (true, true) => "<w:rPr><w:b/><w:i/></w:rPr>",
            };
            out.push_str(&format!("<w:r>{props}<w:t xml:space=\"preserve\">{}</w:t></w:r>", xml(&r.text)));
        }
    }
    out
}

fn docx_para(style: &str, align: Option<&str>, lines: &[String]) -> String {
    let jc = align.map(|a| format!("<w:jc w:val=\"{a}\"/>")).unwrap_or_default();
    format!("<w:p><w:pPr><w:pStyle w:val=\"{style}\"/>{jc}</w:pPr>{}</w:p>\n", docx_runs(lines))
}


const DOCX_STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault><w:rPr><w:rFonts w:ascii="Georgia" w:hAnsi="Georgia" w:cs="Georgia" w:eastAsia="Georgia"/><w:sz w:val="24"/><w:szCs w:val="24"/><w:lang w:val="en-US"/></w:rPr></w:rPrDefault>
    <w:pPrDefault><w:pPr><w:spacing w:after="0" w:line="360" w:lineRule="auto"/></w:pPr></w:pPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="0" w:line="360" w:lineRule="auto"/><w:ind w:firstLine="432"/><w:jc w:val="both"/></w:pPr><w:rPr><w:rFonts w:ascii="Georgia" w:hAnsi="Georgia" w:cs="Georgia" w:eastAsia="Georgia"/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="FirstParagraph"><w:name w:val="First Paragraph"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:ind w:firstLine="0"/></w:pPr></w:style>
  <w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="FirstParagraph"/><w:qFormat/><w:pPr><w:spacing w:before="2880" w:after="240"/><w:ind w:firstLine="0"/><w:jc w:val="center"/></w:pPr><w:rPr><w:sz w:val="56"/><w:szCs w:val="56"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Byline"><w:name w:val="Byline"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:ind w:firstLine="0"/><w:jc w:val="center"/></w:pPr><w:rPr><w:i/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="FirstParagraph"/><w:qFormat/><w:pPr><w:keepNext/><w:pageBreakBefore/><w:spacing w:before="1440" w:after="480"/><w:ind w:firstLine="0"/><w:jc w:val="center"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:sz w:val="36"/><w:szCs w:val="36"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="FirstParagraph"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="360" w:after="120"/><w:ind w:firstLine="0"/><w:jc w:val="left"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="SceneBreak"><w:name w:val="Scene Break"/><w:basedOn w:val="Normal"/><w:next w:val="FirstParagraph"/><w:qFormat/><w:pPr><w:spacing w:before="240" w:after="240"/><w:ind w:firstLine="0"/><w:jc w:val="center"/></w:pPr></w:style>
</w:styles>
"#;

pub fn docx(book: &Book, path: &Path) -> Result<(), String> {
    let mut body = String::new();
    for p in layout(book) {
        body.push_str(&match p {
            Para::Title(t) => docx_para("Title", None, &[t]),
            Para::Byline(t) => docx_para("Byline", None, &[t]),
            Para::Chapter(t) => docx_para("Heading1", None, &[t]),
            Para::Sub(t) => docx_para("Heading2", None, &[t]),
            Para::Break => docx_para("SceneBreak", None, &["* * *".into()]),
            Para::Text { align, lines, first } => {
                let jc = align.map(|a| if a == "center" { "center" } else { "right" });
                docx_para(if first || align.is_some() { "FirstParagraph" } else { "Normal" }, jc, &lines)
            }
        });
    }
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
{body}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>
</w:body></w:document>
"#
    );
    let core = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <dc:title>{}</dc:title><dc:creator>{}</dc:creator>
</cp:coreProperties>
"#,
        xml(&book.title),
        xml(&book.author)
    );
    zip_files(
        path,
        &[
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
  <Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
</Types>
"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
</Relationships>
"#,
            ),
            (
                "word/_rels/document.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>
"#,
            ),
            ("word/document.xml", &document),
            ("word/styles.xml", DOCX_STYLES),
            ("docProps/core.xml", &core),
        ],
        None,
    )
}

// ---------------------------------------------------------------- .odt

fn odt_runs(lines: &[String]) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push_str("<text:line-break/>");
        }
        for r in runs(line) {
            // Runs of spaces would collapse; ODF spells them out.
            let t = xml(&r.text).replace("  ", " <text:s/>");
            out.push_str(&match (r.bold, r.italic) {
                (false, false) => t,
                (true, false) => format!("<text:span text:style-name=\"B\">{t}</text:span>"),
                (false, true) => format!("<text:span text:style-name=\"I\">{t}</text:span>"),
                (true, true) => format!("<text:span text:style-name=\"BI\">{t}</text:span>"),
            });
        }
    }
    out
}

const ODF_NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" office:version="1.2""#;

pub fn odt(book: &Book, path: &Path) -> Result<(), String> {
    let mut body = String::new();
    for p in layout(book) {
        body.push_str(&match p {
            Para::Title(t) => format!("<text:p text:style-name=\"Title\">{}</text:p>\n", odt_runs(&[t])),
            Para::Byline(t) => format!("<text:p text:style-name=\"Byline\">{}</text:p>\n", odt_runs(&[t])),
            Para::Chapter(t) => format!("<text:h text:style-name=\"Heading_20_1\" text:outline-level=\"1\">{}</text:h>\n", odt_runs(&[t])),
            Para::Sub(t) => format!("<text:h text:style-name=\"Heading_20_2\" text:outline-level=\"2\">{}</text:h>\n", odt_runs(&[t])),
            Para::Break => "<text:p text:style-name=\"Scene_20_Break\">* * *</text:p>\n".into(),
            Para::Text { align, lines, first } => {
                let style = match align {
                    Some("center") => "Center",
                    Some(_) => "Right",
                    None if first => "First_20_Paragraph",
                    None => "Text_20_body",
                };
                format!("<text:p text:style-name=\"{style}\">{}</text:p>\n", odt_runs(&lines))
            }
        });
    }
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content {ODF_NS}>
  <office:automatic-styles>
    <style:style style:name="B" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>
    <style:style style:name="I" style:family="text"><style:text-properties fo:font-style="italic"/></style:style>
    <style:style style:name="BI" style:family="text"><style:text-properties fo:font-weight="bold" fo:font-style="italic"/></style:style>
    <style:style style:name="Center" style:family="paragraph" style:parent-style-name="First_20_Paragraph"><style:paragraph-properties fo:text-align="center"/></style:style>
    <style:style style:name="Right" style:family="paragraph" style:parent-style-name="First_20_Paragraph"><style:paragraph-properties fo:text-align="end"/></style:style>
  </office:automatic-styles>
  <office:body><office:text>
{body}  </office:text></office:body>
</office:document-content>
"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-styles {ODF_NS}>
  <office:styles>
    <style:default-style style:family="paragraph"><style:text-properties style:font-name="Georgia" fo:font-family="Georgia" fo:font-size="12pt" fo:language="en" fo:country="US"/></style:default-style>
    <style:style style:name="Standard" style:family="paragraph" style:class="text"/>
    <style:style style:name="Text_20_body" style:display-name="Text body" style:family="paragraph" style:parent-style-name="Standard" style:class="text"><style:paragraph-properties fo:text-indent="0.3in" fo:line-height="150%" fo:text-align="justify" fo:margin-top="0in" fo:margin-bottom="0in"/></style:style>
    <style:style style:name="First_20_Paragraph" style:display-name="First Paragraph" style:family="paragraph" style:parent-style-name="Text_20_body" style:class="text"><style:paragraph-properties fo:text-indent="0in"/></style:style>
    <style:style style:name="Title" style:family="paragraph" style:parent-style-name="Standard" style:next-style-name="First_20_Paragraph" style:class="chapter"><style:paragraph-properties fo:text-align="center" fo:margin-top="2in" fo:margin-bottom="0.17in"/><style:text-properties fo:font-size="28pt"/></style:style>
    <style:style style:name="Byline" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:text-align="center"/><style:text-properties fo:font-style="italic"/></style:style>
    <style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:parent-style-name="Standard" style:next-style-name="First_20_Paragraph" style:default-outline-level="1" style:class="text"><style:paragraph-properties fo:break-before="page" fo:text-align="center" fo:margin-top="1in" fo:margin-bottom="0.33in" fo:keep-with-next="always"/><style:text-properties fo:font-size="18pt"/></style:style>
    <style:style style:name="Heading_20_2" style:display-name="Heading 2" style:family="paragraph" style:parent-style-name="Standard" style:next-style-name="First_20_Paragraph" style:default-outline-level="2" style:class="text"><style:paragraph-properties fo:margin-top="0.25in" fo:margin-bottom="0.08in" fo:keep-with-next="always"/><style:text-properties fo:font-weight="bold"/></style:style>
    <style:style style:name="Scene_20_Break" style:display-name="Scene Break" style:family="paragraph" style:parent-style-name="Standard" style:next-style-name="First_20_Paragraph"><style:paragraph-properties fo:text-align="center" fo:margin-top="0.17in" fo:margin-bottom="0.17in"/></style:style>
  </office:styles>
  <office:automatic-styles>
    <style:page-layout style:name="Page"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" fo:margin-top="1in" fo:margin-bottom="1in" fo:margin-left="1in" fo:margin-right="1in"/></style:page-layout>
  </office:automatic-styles>
  <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="Page"/></office:master-styles>
</office:document-styles>
"#
    );
    let meta = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-meta {ODF_NS}><office:meta><dc:title>{}</dc:title><dc:creator>{}</dc:creator></office:meta></office:document-meta>
"#,
        xml(&book.title),
        xml(&book.author)
    );
    zip_files(
        path,
        &[
            (
                "META-INF/manifest.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
  <manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.text"/>
  <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
  <manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/>
  <manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/>
</manifest:manifest>
"#,
            ),
            ("content.xml", &content),
            ("styles.xml", &styles),
            ("meta.xml", &meta),
        ],
        Some("application/vnd.oasis.opendocument.text"),
    )
}

// ---------------------------------------------------------------- .rtf

/// RTF is ASCII: escape its control characters, and write everything else as
/// `\uN?` (UTF-16, signed), which every reader since Word 97 understands.
fn rtf_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            '\t' => out.push_str("\\tab "),
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
    out
}

fn rtf_runs(lines: &[String]) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push_str("\\line ");
        }
        for r in runs(line) {
            let t = rtf_text(&r.text);
            out.push_str(&match (r.bold, r.italic) {
                (false, false) => t,
                (true, false) => format!("{{\\b {t}}}"),
                (false, true) => format!("{{\\i {t}}}"),
                (true, true) => format!("{{\\b\\i {t}}}"),
            });
        }
    }
    out
}

pub fn rtf(book: &Book) -> String {
    // Twips: 1440 to the inch. Body: 12pt, 1.5 lines, first-line indent.
    let mut out = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1{\\fonttbl{\\f0\\froman Georgia;}}\n");
    out.push_str(&format!("{{\\info{{\\title {}}}{{\\author {}}}}}\n", rtf_text(&book.title), rtf_text(&book.author)));
    out.push_str("\\paperw12240\\paperh15840\\margl1440\\margr1440\\margt1440\\margb1440\\f0\\fs24\n");
    let mut started = false;
    for p in layout(book) {
        let para = match p {
            Para::Title(t) => format!("\\pard\\qc\\sb2880\\sa240\\fs56 {}\\fs24\\par\n", rtf_runs(&[t])),
            Para::Byline(t) => format!("\\pard\\qc\\i {}\\i0\\par\n", rtf_runs(&[t])),
            Para::Chapter(t) => format!("{}\\pard\\qc\\sb1440\\sa480\\keepn\\fs36 {}\\fs24\\par\n", if started { "\\page\n" } else { "" }, rtf_runs(&[t])),
            Para::Sub(t) => format!("\\pard\\sb360\\sa120\\keepn\\b {}\\b0\\par\n", rtf_runs(&[t])),
            Para::Break => "\\pard\\qc\\sb240\\sa240 * * *\\par\n".into(),
            Para::Text { align, lines, first } => {
                let (q, indent) = match align {
                    Some("center") => ("\\qc", 0),
                    Some(_) => ("\\qr", 0),
                    None => ("\\qj", if first { 0 } else { 432 }),
                };
                format!("\\pard{q}\\fi{indent}\\sl360\\slmult1 {}\\par\n", rtf_runs(&lines))
            }
        };
        started = true;
        out.push_str(&para);
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::Chapter;

    fn book() -> Book {
        Book {
            title: "Night & Day".into(),
            author: "Ana Ruiz".into(),
            chapters: vec![
                Chapter { title: Some("One".into()), text: "It was **very** *dark*. Café.\n\nShe waited.\n\n---\n\n> THE END <".into() },
                Chapter { title: Some("Two".into()), text: "A [[Note|link]] here.".into() },
            ],
        }
    }

    #[test]
    fn runs_keep_emphasis_and_drop_link_syntax() {
        let r = runs("a **b *c*** [[x|y]]");
        let got: Vec<(&str, bool, bool)> = r.iter().map(|r| (r.text.as_str(), r.bold, r.italic)).collect();
        assert_eq!(got, vec![("a ", false, false), ("b ", true, false), ("c", true, true), (" y", false, false)]);
    }

    #[test]
    fn rtf_escapes_and_lays_out() {
        let r = rtf(&book());
        assert!(r.starts_with("{\\rtf1"));
        assert!(r.contains("Caf\\u233?."), "accents as \\u");
        assert!(r.contains("{\\b very}") && r.contains("{\\i dark}"));
        assert_eq!(r.matches("\\page").count(), 2, "each chapter starts a page, after the title page");
        assert!(r.contains("\\qc") && r.contains("THE END"));
        assert!(r.contains("Night & Day"));
    }

    #[test]
    fn docx_and_odt_are_zips_with_the_text() {
        let dir = std::env::temp_dir().join(format!("wp-office-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for (ext, part) in [("docx", "word/document.xml"), ("odt", "content.xml")] {
            let path = dir.join(format!("b.{ext}"));
            if ext == "docx" { docx(&book(), &path).unwrap() } else { odt(&book(), &path).unwrap() }
            let mut z = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
            if ext == "odt" {
                assert_eq!(z.by_index(0).unwrap().name(), "mimetype");
            }
            let mut text = String::new();
            std::io::Read::read_to_string(&mut z.by_name(part).unwrap(), &mut text).unwrap();
            assert!(text.contains("Night &amp; Day") && text.contains("Café") && text.contains("link"), "{ext}");
            assert!(!text.contains("[["), "{ext}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
