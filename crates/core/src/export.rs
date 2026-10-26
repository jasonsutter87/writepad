//! Export a book (a manuscript's live cut, or a single note) as Markdown,
//! plain text, a web page, EPUB, PDF, or MP3 audio; Word, OpenDocument and
//! RTF live in `office`.

use serde::Deserialize;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

#[derive(Deserialize)]
pub struct Chapter {
    /// None for front matter, which is exported as-is without a heading.
    pub title: Option<String>,
    pub text: String,
}

#[derive(Deserialize)]
pub struct Book {
    pub title: String,
    #[serde(default)]
    pub author: String,
    pub chapters: Vec<Chapter>,
}

pub fn markdown(book: &Book) -> String {
    let parts: Vec<String> = book
        .chapters
        .iter()
        .map(|c| match &c.title {
            Some(t) => format!("## {t}\n\n{}", c.text.trim()),
            None => c.text.trim().to_string(),
        })
        .collect();
    let starts_with_title = book.chapters.first().is_some_and(|c| c.title.is_none() && c.text.trim_start().starts_with("# "));
    let head = if starts_with_title { String::new() } else { format!("# {}\n\n", book.title) };
    head + &parts.join("\n\n") + "\n"
}

/// Markdown with the markup taken out: headings become plain lines,
/// emphasis markers and link syntax disappear.
/// Alignment markers, Fountain-style: `> centered <` and `>> right`.
/// Returns the alignment class (if any) and the line without its markers.
pub fn alignment(line: &str) -> (Option<&'static str>, &str) {
    let t = line.trim();
    if let Some(inner) = t.strip_prefix(">>") {
        return (Some("right"), inner.trim());
    }
    if let Some(inner) = t.strip_prefix('>').and_then(|x| x.strip_suffix('<')) {
        return (Some("center"), inner.trim());
    }
    (None, line)
}

pub fn plain_text(book: &Book) -> String {
    let md = markdown(book);
    let mut out = String::new();
    for line in md.lines() {
        let line = line.trim_start_matches('#').trim_start_matches(' ');
        let line = if line.trim() == "---" { "* * *" } else { alignment(line).1 };
        out.push_str(&strip_inline(line));
        out.push('\n');
    }
    out
}

fn strip_inline(s: &str) -> String {
    if s.trim() == "* * *" {
        return s.to_string();
    }
    // [[target|alias]] -> alias, [[target]] -> target
    let mut text = String::with_capacity(s.len());
    let mut rest = s;
    while let (Some(a), Some(b)) = (rest.find("[["), rest.find("]]")) {
        if b < a {
            break;
        }
        text.push_str(&rest[..a]);
        let inner = &rest[a + 2..b];
        text.push_str(inner.rsplit('|').next().unwrap_or(inner));
        rest = &rest[b + 2..];
    }
    text.push_str(rest);
    // Emphasis markers go; a leading "* " bullet stays.
    let (bullet, body) = match text.strip_prefix("* ") {
        Some(b) => ("* ", b),
        None => ("", text.as_str()),
    };
    format!("{bullet}{}", body.replace("**", "").replace("__", "").replace('*', ""))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Inline markdown -> XHTML: **strong**, *em*, [[links]] as text.
fn inline_html(s: &str) -> String {
    let s = escape(s);
    let mut out = String::with_capacity(s.len() + 16);
    let (mut strong, mut em) = (false, false);
    let mut rest = s.as_str();
    while let Some(i) = rest.find(['*', '[']) {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(r) = rest.strip_prefix("**") {
            out.push_str(if strong { "</strong>" } else { "<strong>" });
            strong = !strong;
            rest = r;
        } else if let Some(r) = rest.strip_prefix('*') {
            out.push_str(if em { "</em>" } else { "<em>" });
            em = !em;
            rest = r;
        } else if let (Some(r), Some(end)) = (rest.strip_prefix("[["), rest.find("]]")) {
            let inner = &r[..end - 2];
            out.push_str(inner.rsplit('|').next().unwrap_or(inner));
            rest = &rest[end + 2..];
        } else {
            out.push('[');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    if em {
        out.push_str("</em>");
    }
    if strong {
        out.push_str("</strong>");
    }
    out
}

/// Block markdown -> XHTML body: paragraphs, headings, scene breaks.
fn body_html(md: &str) -> String {
    let mut out = String::new();
    for para in md.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        if matches!(para, "---" | "* * *" | "***" | "·") {
            out.push_str("<hr/>\n");
        } else if let Some(h) = para.strip_prefix("# ") {
            out.push_str(&format!("<h1>{}</h1>\n", inline_html(h)));
        } else if let Some(h) = para.strip_prefix("## ").or(para.strip_prefix("### ")) {
            out.push_str(&format!("<h3>{}</h3>\n", inline_html(h)));
        } else if let (Some(class), _) = alignment(para.lines().next().unwrap_or("")) {
            let lines: Vec<String> = para.lines().map(|l| inline_html(alignment(l).1)).collect();
            out.push_str(&format!("<p class=\"{class}\">{}</p>\n", lines.join("<br/>")));
        } else {
            let lines: Vec<String> = para.lines().map(inline_html).collect();
            out.push_str(&format!("<p>{}</p>\n", lines.join("<br/>")));
        }
    }
    out
}

const BOOK_CSS: &str = r#"
body { font-family: "Iowan Old Style", Charter, Palatino, Georgia, serif; line-height: 1.5; margin: 0 5%; }
h1 { text-align: center; font-weight: 600; margin: 3em 0 0.4em; }
h2 { text-align: center; font-weight: 600; margin: 2.5em 0 1.5em; }
p { margin: 0; text-indent: 1.4em; text-align: justify; hyphens: auto; }
h1 + p, h2 + p, h3 + p, hr + p, p:first-child { text-indent: 0; }
hr { border: none; text-align: center; margin: 1.4em 0; }
hr::after { content: "* * *"; letter-spacing: 0.4em; }
p.center { text-align: center; text-indent: 0; }
p.right { text-align: right; text-indent: 0; }
.byline { text-align: center; font-style: italic; text-indent: 0; margin-bottom: 3em; }
"#;

fn xhtml_page(title: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"en\">\n<head><meta charset=\"UTF-8\"/><title>{}</title><link rel=\"stylesheet\" href=\"style.css\"/></head>\n<body>\n{body}</body>\n</html>\n",
        escape(title)
    )
}

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's algorithm.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn iso_now() -> String {
    let secs = crate::manuscript::now() as i64;
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", t / 3600, t / 60 % 60, t % 60)
}

pub fn epub(book: &Book, path: &Path) -> Result<(), String> {
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;
    let file = fs::File::create(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let put = |zip: &mut zip::ZipWriter<fs::File>, name: &str, data: &str, opts| -> Result<(), String> {
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        zip.write_all(data.as_bytes()).map_err(|e| e.to_string())
    };

    // The mimetype entry must come first and be uncompressed.
    put(&mut zip, "mimetype", "application/epub+zip", stored)?;
    put(
        &mut zip,
        "META-INF/container.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>
"#,
        deflated,
    )?;
    put(&mut zip, "OEBPS/style.css", BOOK_CSS, deflated)?;

    // Title page, then one file per chapter.
    let mut title_body = format!("<h1>{}</h1>\n", escape(&book.title));
    if !book.author.is_empty() {
        title_body.push_str(&format!("<p class=\"byline\">{}</p>\n", escape(&book.author)));
    }
    put(&mut zip, "OEBPS/title.xhtml", &xhtml_page(&book.title, &title_body), deflated)?;

    let mut items = vec![];
    for (i, ch) in book.chapters.iter().enumerate() {
        let file = format!("ch{:03}.xhtml", i + 1);
        let title = ch.title.clone().unwrap_or_else(|| book.title.clone());
        let body = match &ch.title {
            Some(t) => format!("<h2>{}</h2>\n{}", inline_html(t), body_html(&ch.text)),
            None => body_html(&ch.text),
        };
        put(&mut zip, &format!("OEBPS/{file}"), &xhtml_page(&title, &body), deflated)?;
        items.push((file, ch.title.clone()));
    }

    let nav_items: String = items
        .iter()
        .filter_map(|(f, t)| t.as_ref().map(|t| format!("      <li><a href=\"{f}\">{}</a></li>\n", escape(t))))
        .collect();
    let nav = xhtml_page(
        "Contents",
        &format!("<nav epub:type=\"toc\" id=\"toc\"><h2>Contents</h2>\n    <ol>\n      <li><a href=\"title.xhtml\">{}</a></li>\n{nav_items}    </ol>\n</nav>\n", escape(&book.title)),
    );
    put(&mut zip, "OEBPS/nav.xhtml", &nav, deflated)?;

    let manifest: String = items
        .iter()
        .map(|(f, _)| format!("    <item id=\"{}\" href=\"{f}\" media-type=\"application/xhtml+xml\"/>\n", f.trim_end_matches(".xhtml")))
        .collect();
    let spine: String = items.iter().map(|(f, _)| format!("    <itemref idref=\"{}\"/>\n", f.trim_end_matches(".xhtml"))).collect();
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="bookid">urn:uuid:{uuid}</dc:identifier>
    <dc:title>{title}</dc:title>
    <dc:creator>{author}</dc:creator>
    <dc:language>en</dc:language>
    <meta property="dcterms:modified">{modified}</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="css" href="style.css" media-type="text/css"/>
    <item id="title" href="title.xhtml" media-type="application/xhtml+xml"/>
{manifest}  </manifest>
  <spine>
    <itemref idref="title"/>
{spine}  </spine>
</package>
"#,
        uuid = uuid::Uuid::new_v4(),
        title = escape(&book.title),
        author = escape(&book.author),
        modified = iso_now(),
    );
    put(&mut zip, "OEBPS/content.opf", &opf, deflated)?;
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// The book as one web page, for reading in a browser or pasting elsewhere.
fn web_html(book: &Book) -> String {
    print_html(book).replacen(
        "<style>",
        "<style>\n@media screen { body { max-width: 36em; margin: 3em auto !important; padding: 0 1.2em; font-size: 19px; } .titlepage { height: auto !important; margin: 4em 0 2em; } section.chapter { margin-top: 4em; } }\n",
        1,
    )
}

/// A print-ready HTML book: title page, then each chapter on a new page.
fn print_html(book: &Book) -> String {
    let mut body = format!("<section class=\"titlepage\"><h1>{}</h1>", escape(&book.title));
    if !book.author.is_empty() {
        body.push_str(&format!("<p class=\"byline\">{}</p>", escape(&book.author)));
    }
    body.push_str("</section>\n");
    for ch in &book.chapters {
        body.push_str("<section class=\"chapter\">\n");
        if let Some(t) = &ch.title {
            body.push_str(&format!("<h2>{}</h2>\n", inline_html(t)));
        }
        body.push_str(&body_html(&ch.text));
        body.push_str("</section>\n");
    }
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title><style>{BOOK_CSS}
@page {{ size: 5.5in 8.5in; margin: 0.75in 0.7in 0.85in; }}
body {{ margin: 0; font-size: 11pt; }}
section.chapter {{ break-before: page; }}
.titlepage {{ height: 6.5in; display: flex; flex-direction: column; justify-content: center; }}
.titlepage h1 {{ font-size: 28pt; margin: 0 0 0.3em; }}
</style></head><body>{body}</body></html>",
        escape(&book.title)
    )
}

/// Chrome, Edge (always there on Windows 10/11) and Brave under each Program Files root.
fn windows_browsers() -> Vec<String> {
    let roots = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"].iter().filter_map(|v| std::env::var(v).ok());
    let apps = ["Google\\Chrome\\Application\\chrome.exe", "Microsoft\\Edge\\Application\\msedge.exe", "BraveSoftware\\Brave-Browser\\Application\\brave.exe"];
    roots.flat_map(|r| apps.iter().map(move |a| format!("{r}\\{a}"))).collect()
}

fn find_chrome() -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_string(),
        format!("{home}/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        "/Applications/Chromium.app/Contents/MacOS/Chromium".to_string(),
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".to_string(),
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser".to_string(),
        "/usr/bin/google-chrome".to_string(),
        "/usr/bin/chromium".to_string(),
    ]
    .into_iter()
    .chain(windows_browsers())
    .find(|p| Path::new(p).exists())
}

pub fn pdf(book: &Book, path: &Path, scratch: &Path) -> Result<(), String> {
    let chrome = find_chrome().ok_or("PDF export uses Google Chrome (or Chromium, Edge, Brave) to typeset, and none was found.")?;
    fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    let html = scratch.join("print.html");
    fs::write(&html, print_html(book)).map_err(|e| e.to_string())?;
    let status = Command::new(chrome)
        .args(["--headless", "--disable-gpu", "--no-pdf-header-footer", "--virtual-time-budget=10000"])
        .arg(format!("--print-to-pdf={}", path.display()))
        .arg(format!("file://{}", html.display()))
        .output()
        .map_err(|e| e.to_string())?;
    if !path.exists() {
        return Err(format!("Chrome didn't produce a PDF: {}", String::from_utf8_lossy(&status.stderr)));
    }
    Ok(())
}

pub fn write_book(book: &Book, format: &str, path: &Path, scratch: &Path) -> Result<(), String> {
    match format {
        "md" => fs::write(path, markdown(book)).map_err(|e| e.to_string()),
        "txt" => fs::write(path, plain_text(book)).map_err(|e| e.to_string()),
        "html" => fs::write(path, web_html(book)).map_err(|e| e.to_string()),
        "docx" => crate::office::docx(book, path),
        "odt" => crate::office::odt(book, path),
        "rtf" => fs::write(path, crate::office::rtf(book)).map_err(|e| e.to_string()),
        "epub" => epub(book, path),
        "pdf" => pdf(book, path, scratch),
        other => Err(format!("Unknown export format: {other}")),
    }
}

/// One spoken segment of an audio chapter.
#[derive(Deserialize)]
pub struct Segment {
    pub text: String,
    pub voice: String,
    /// Seconds of silence after it.
    #[serde(default)]
    pub pause: f32,
}

/// Concatenate rendered WAVs (24 kHz mono, 16-bit) plus pauses and encode to MP3.
pub fn mp3(wavs: Vec<(Vec<u8>, f32)>, path: &Path, title: &str, album: &str, artist: &str) -> Result<(), String> {
    use mp3lame_encoder::{Bitrate, Builder, FlushNoGap, Id3Tag, MonoPcm, Quality};
    let mut pcm: Vec<i16> = vec![];
    let mut rate = 24_000;
    for (wav, pause) in wavs {
        let mut reader = hound::WavReader::new(std::io::Cursor::new(wav)).map_err(|e| e.to_string())?;
        rate = reader.spec().sample_rate;
        pcm.extend(reader.samples::<i16>().filter_map(Result::ok));
        pcm.extend(std::iter::repeat(0).take((pause.max(0.0) * rate as f32) as usize));
    }
    let mut b = Builder::new().ok_or("Couldn't start the MP3 encoder")?;
    b.set_num_channels(1).map_err(|e| e.to_string())?;
    b.set_sample_rate(rate).map_err(|e| e.to_string())?;
    b.set_brate(Bitrate::Kbps96).map_err(|e| e.to_string())?;
    b.set_quality(Quality::Good).map_err(|e| e.to_string())?;
    b.set_id3_tag(Id3Tag {
        title: title.as_bytes(),
        artist: artist.as_bytes(),
        album: album.as_bytes(),
        album_art: &[],
        year: &[],
        comment: b"Narrated with WRITEpad",
    })
    .map_err(|e| format!("{e:?}"))?;
    let mut enc = b.build().map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(mp3lame_encoder::max_required_buffer_size(pcm.len()));
    enc.encode_to_vec(MonoPcm(&pcm), &mut out).map_err(|e| e.to_string())?;
    enc.flush_to_vec::<FlushNoGap>(&mut out).map_err(|e| e.to_string())?;
    fs::write(path, out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Book {
        Book {
            title: "Test & Book".into(),
            author: "Me".into(),
            chapters: vec![
                Chapter { title: None, text: "Front *matter*.".into() },
                Chapter { title: Some("1".into()), text: "*Clear. Proceed.*\n\nThe van **isn't** braking.\n\n---\n\nSee [[Unit 12|the ward]].".into() },
            ],
        }
    }

    #[test]
    fn alignment_markers() {
        assert_eq!(body_html("> THE END <\n\n>> Signed, J.\n\nPlain."), "<p class=\"center\">THE END</p>\n<p class=\"right\">Signed, J.</p>\n<p>Plain.</p>\n");
        assert_eq!(alignment("> a quote"), (None, "> a quote"));
    }

    #[test]
    fn inline_markup() {
        assert_eq!(inline_html("a *b* **c** <d> [[x|y]]"), "a <em>b</em> <strong>c</strong> &lt;d&gt; y");
        assert_eq!(strip_inline("*Clear.* **Bold** [[Unit 12|ward]]"), "Clear. Bold ward");
        assert_eq!(strip_inline("* * *"), "* * *");
    }

    #[test]
    fn writes_valid_epub_zip() {
        let path = std::env::temp_dir().join(format!("wp-test-{}.epub", std::process::id()));
        epub(&book(), &path).unwrap();
        let mut z = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(z.by_index(0).unwrap().name(), "mimetype");
        let mut ch = String::new();
        std::io::Read::read_to_string(&mut z.by_name("OEBPS/ch002.xhtml").unwrap(), &mut ch).unwrap();
        assert!(ch.contains("<p><em>Clear. Proceed.</em></p>"), "{ch}");
        assert!(ch.contains("<hr/>"));
        // Round-trips through our own importer.
        let (title, chapters) = crate::epub::chapters(path.to_str().unwrap()).unwrap();
        assert_eq!(title, "Test & Book");
        let titles: Vec<&str> = chapters.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles.last(), Some(&"1"), "{titles:?}");
        fs::remove_file(path).unwrap();
    }
}

/// End-to-end: `BOOK=/path/book.md OUT=/dir cargo test export_real_book -- --ignored --nocapture`
#[cfg(test)]
#[test]
#[ignore]
fn export_real_book() {
    let (src, out) = (std::env::var("BOOK").unwrap(), std::path::PathBuf::from(std::env::var("OUT").unwrap()));
    let text = fs::read_to_string(&src).unwrap();
    let mut chapters = vec![];
    for (i, part) in text.split("\n## ").enumerate() {
        let part = part.trim().trim_end_matches("---").trim();
        if i == 0 {
            chapters.push(Chapter { title: None, text: part.to_string() });
        } else {
            let (t, body) = part.split_once('\n').unwrap_or((part, ""));
            chapters.push(Chapter { title: Some(t.trim().into()), text: body.trim().trim_end_matches("---").trim().into() });
        }
    }
    let book = Book { title: "The Last Thinker".into(), author: "Test Author".into(), chapters };
    for fmt in ["md", "txt", "epub", "pdf"] {
        let p = out.join(format!("book.{fmt}"));
        write_book(&book, fmt, &p, &out.join("scratch")).unwrap();
        println!("{fmt}: {} bytes", fs::metadata(&p).unwrap().len());
    }
    let (_, back) = crate::epub::chapters(out.join("book.epub").to_str().unwrap()).unwrap();
    println!("epub reimports as {} chapters, first: {}", back.len(), back[1].title);

    // Two sentences through Kokoro, then LAME.
    let home = std::env::var("HOME").unwrap();
    let cya = format!("{home}/Documents/ChooseYourAdventure");
    let mut w = crate::narrate::Worker::start(&format!("{cya}/.venv-tts/bin/python"), &format!("{cya}/tts_models"), &out.join("cache")).unwrap();
    let lex = Default::default();
    let a = crate::narrate::synth(&mut w, &out.join("cache"), "Section one.", "af_heart", 1.0, &lex).unwrap();
    let b = crate::narrate::synth(&mut w, &out.join("cache"), "Clear. Proceed.", "af_nicole", 1.0, &lex).unwrap();
    mp3(vec![(a, 1.0), (b, 0.0)], &out.join("ch.mp3"), "Section one", "The Last Thinker", "Test Author").unwrap();
    println!("mp3: {} bytes", fs::metadata(out.join("ch.mp3")).unwrap().len());
}
