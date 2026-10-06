//! Bounded, inert previews from an already authenticated regular-file descriptor.
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{ImageReader, Limits};
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
};
const INPUT: u64 = 16 * 1024 * 1024;
const TEXT: usize = 32 * 1024;

fn safe_text(bytes: &[u8]) -> String {
    let normalized = String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut result = String::new();
    let mut column = 0;
    for c in normalized.chars() {
        match c {
            '\n' => {
                result.push(c);
                column = 0;
            }
            '\t' => {
                let n = 4 - column % 4;
                result.extend(std::iter::repeat_n(' ', n));
                column += n;
            }
            c if c.is_control() => {
                let escaped = c.escape_default().to_string();
                column += escaped.len();
                result.push_str(&escaped);
            }
            c => {
                result.push(c);
                column += 1;
            }
        }
    }
    result
}
fn raster(bytes: &[u8]) -> Result<(Value, String)> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    // Read dimensions before full decompression; width/height alone do not bound RGBA allocation.
    let (w, h) = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .into_dimensions()?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) * 4 > 32 * 1024 * 1024 {
        bail!("image exceeds decoded preview limit");
    }
    let mut img = reader.decode()?.thumbnail(w.min(1280), h.min(960));
    // Small legacy payloads remain readable by older viewers. Larger previews use
    // compressed PNG, with a hard wire budget independent of image complexity.
    if img.width() <= 160 && img.height() <= 100 {
        let rgba = img.to_rgba8();
        return Ok((
            json!({"width":rgba.width(),"height":rgba.height(),"rgba":STANDARD.encode(rgba.as_raw())}),
            format!("{w} × {h}"),
        ));
    }
    loop {
        let mut png = Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png)?;
        if png.get_ref().len() <= 600_000 {
            return Ok((
                json!({"width":img.width(),"height":img.height(),"png":STANDARD.encode(png.into_inner())}),
                format!("{w} × {h}"),
            ));
        }
        let next_w = (img.width() * 3 / 4).max(1);
        let next_h = (img.height() * 3 / 4).max(1);
        img = img.thumbnail(next_w, next_h);
    }
}
fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}
pub(super) fn render(path: &Path, file: File) -> Result<Value> {
    render_inner(path, file, None)
}
pub(super) fn render_page(path: &Path, file: File, page: u32) -> Result<Value> {
    if !(1..=10000).contains(&page) {
        bail!("PDF page must be between 1 and 10000");
    }
    render_inner(path, file, Some(page))
}
fn render_inner(path: &Path, mut file: File, requested_page: Option<u32>) -> Result<Value> {
    let before = file.metadata()?;
    let size = before.len();
    let mut bytes = Vec::new();
    (&mut file)
        .take((TEXT + 1) as u64)
        .read_to_end(&mut bytes)?;
    let ext = extension(path);
    let pdf = bytes.starts_with(b"%PDF-");
    if requested_page.is_some() && !pdf {
        bail!("page preview requires a PDF file");
    }
    let page = requested_page.unwrap_or(1);
    let mut pages: Option<u32> = None;
    let image = image::guess_format(&bytes).is_ok();
    let tar = bytes.get(257..262) == Some(b"ustar");
    let gzip = bytes.starts_with(&[0x1f, 0x8b]) && matches!(ext.as_str(), "gz" | "tgz");
    let binary = bytes.contains(&0) || pdf || image || tar || gzip;
    let mut kind = if binary {
        "binary"
    } else {
        match ext.as_str() {
            "md" | "markdown" | "mdown" => "markdown",
            "rs" | "py" | "sh" | "fish" | "js" | "ts" | "tsx" | "jsx" | "c" | "h" | "cpp"
            | "go" | "java" | "json" | "toml" | "yaml" | "yml" | "css" | "html" | "rb" | "php"
            | "pl" | "lua" | "sql" | "xml" | "ini" | "cs" | "swift" | "r" | "R" | "make"
            | "cmake" | "tex" => "code",
            _ if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    matches!(
                        name,
                        "Makefile" | "Dockerfile" | "CMakeLists.txt" | ".bashrc" | ".zshrc"
                    )
                }) =>
            {
                "code"
            }
            _ => "text",
        }
    };
    let mut title = format!("{size} bytes");
    let mut img = Value::Null;
    let mut truncated = size > TEXT as u64;
    let mut text = if binary {
        format!("Binary file · {size} bytes")
    } else {
        safe_text(&bytes[..bytes.len().min(TEXT)])
    };
    if image || pdf || tar || gzip {
        kind = if image {
            "image"
        } else if pdf {
            "pdf"
        } else {
            "archive"
        };
        if size > INPUT {
            if requested_page.is_some() {
                bail!("PDF exceeds 16 MiB preview input limit");
            }
            text = format!("Preview unavailable · file exceeds 16 MiB input limit ({size} bytes)");
        } else {
            file.seek(SeekFrom::Start(0))?;
            bytes.clear();
            (&mut file).take(INPUT + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > INPUT {
                bail!("source grew beyond preview input limit");
            }
            if image {
                match raster(&bytes) {
                    Ok((value, dimensions)) => {
                        img = value;
                        title = dimensions;
                        text = String::new();
                        truncated = false;
                    }
                    Err(e) => {
                        text = format!(
                            "Image preview unavailable · {}",
                            super::display(&e.to_string())
                        )
                    }
                }
            } else if pdf {
                // Count and render only bounded bytes read from the authenticated descriptor.
                let result = (|| -> Result<Value> {
                    let count = pdf_page_count(&bytes)?;
                    pages = Some(count);
                    if page > count {
                        bail!("PDF page {page} is out of range (document has {count} pages)");
                    }
                    let png = pdf_page(&bytes, page)?;
                    Ok(raster(&png)?.0)
                })();
                match result {
                    Ok(value) => {
                        img = value;
                        title = format!("PDF · page {page}");
                        text = String::new();
                        truncated = pages.is_some_and(|count| count > 1);
                    }
                    Err(e) if requested_page.is_some() => return Err(e),
                    Err(e) => {
                        text = format!(
                            "PDF preview unavailable · {}",
                            super::display(&e.to_string())
                        )
                    }
                }
            } else {
                let reader: Box<dyn Read> = if gzip {
                    Box::new(flate2::read::GzDecoder::new(Cursor::new(bytes)))
                } else {
                    Box::new(Cursor::new(bytes))
                };
                // Bound decompression as well as input; never extract archive entries.
                let mut archive = tar::Archive::new(reader.take(4 * 1024 * 1024));
                text = String::new();
                truncated = false;
                match archive.entries() {
                    Ok(entries) => {
                        for (index, entry) in entries.enumerate() {
                            if index >= 100 || text.len() > TEXT {
                                truncated = true;
                                break;
                            }
                            match entry.and_then(|e| Ok((e.path()?.into_owned(), e.size()))) {
                                Ok((name, length)) => {
                                    // PAX names can be enormous even for an empty entry.
                                    // Bound before escaping/allocation and before appending.
                                    let name = name.to_string_lossy();
                                    let short: String = name.chars().take(512).collect();
                                    let clipped = name.chars().count() > 512;
                                    truncated |= clipped;
                                    let line = format!(
                                        "{}{} · {length} bytes\n",
                                        super::display(&short),
                                        if clipped { "…" } else { "" }
                                    );
                                    if text.len() + line.len() > TEXT {
                                        truncated = true;
                                        break;
                                    }
                                    text.push_str(&line);
                                }
                                Err(_) => {
                                    text.push_str("Archive listing incomplete or malformed\n");
                                    truncated = true;
                                    break;
                                }
                            }
                        }
                    }
                    Err(_) => text = "Archive listing unavailable or malformed".into(),
                }
                if text.is_empty() {
                    text = "Empty archive".into();
                }
                title = "Archive · entries (no extraction)".into();
            }
        }
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        kind = "media";
        title = "WAV audio".into();
        text = format!("WAV audio · {size} bytes\nPlayback is not started by preview.");
        if bytes.len() >= 36 && bytes.get(12..16) == Some(b"fmt ") {
            let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
            let rate = u32::from_le_bytes(bytes[24..28].try_into()?);
            let bits = u16::from_le_bytes([bytes[34], bytes[35]]);
            text.push_str(&format!("\n{channels} channels · {rate} Hz · {bits} bits"));
        }
    }
    let after = file.metadata()?;
    if after.len() != size || after.modified()? != before.modified()? {
        bail!("source changed during preview; retry");
    }
    let mut response = json!({"path":super::encode_path(path),"text":text,"truncated":truncated,"binary":binary,"kind":kind,"title":title,"image":img});
    if pdf {
        response["page"] = json!(page);
        response["pages"] = json!(pages);
    }
    Ok(response)
}

#[cfg(unix)]
fn pdf_page_count(bytes: &[u8]) -> Result<u32> {
    let output = run_converter(bytes, std::ffi::OsStr::new("pdfinfo"), &["-"], 64 * 1024)?;
    parse_page_count(&output)
}
fn parse_page_count(output: &[u8]) -> Result<u32> {
    let text = std::str::from_utf8(output).context("invalid PDF page metadata")?;
    let mut counts = text.lines().filter_map(|line| line.strip_prefix("Pages:"));
    let count: u32 = counts
        .next()
        .context("PDF page count unavailable")?
        .trim()
        .parse()
        .context("invalid PDF page count")?;
    if count == 0 || counts.next().is_some() {
        bail!("invalid PDF page count");
    }
    Ok(count)
}
#[cfg(unix)]
fn pdf_page(bytes: &[u8], page: u32) -> Result<Vec<u8>> {
    let page = page.to_string();
    run_converter(
        bytes,
        std::ffi::OsStr::new("pdftoppm"),
        &[
            "-f",
            &page,
            "-l",
            &page,
            "-singlefile",
            "-scale-to",
            "1280",
            "-png",
            "-",
        ],
        2 * 1024 * 1024,
    )
}
#[cfg(all(unix, test))]
fn pdf_converter(bytes: &[u8], program: &std::ffi::OsStr) -> Result<Vec<u8>> {
    run_converter(
        bytes,
        program,
        &[
            "-f",
            "1",
            "-l",
            "1",
            "-singlefile",
            "-scale-to",
            "1280",
            "-png",
            "-",
        ],
        2 * 1024 * 1024,
    )
}
#[cfg(unix)]
fn run_converter(
    bytes: &[u8],
    program: &std::ffi::OsStr,
    args: &[&str],
    output_limit: usize,
) -> Result<Vec<u8>> {
    use std::{
        os::unix::{io::AsRawFd, process::CommandExt},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            for (resource, limit) in [
                (libc::RLIMIT_AS, 256 * 1024 * 1024),
                (libc::RLIMIT_CPU, 2),
                (libc::RLIMIT_FSIZE, 2 * 1024 * 1024),
                (libc::RLIMIT_NOFILE, 64),
            ] {
                let bound = libc::rlimit {
                    rlim_cur: limit,
                    rlim_max: limit,
                };
                if libc::setrlimit(resource, &bound) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command.spawn().with_context(|| {
        format!(
            "optional {} converter not available",
            program.to_string_lossy()
        )
    })?;
    // Feed only the bounded snapshot read from the validated descriptor. A
    // growing file cannot extend the converter's input or reopen a swapped name.
    let mut stdin = child.stdin.take().context("converter input unavailable")?;
    let input = bytes.to_vec();
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        let _ = stdin.write_all(&input);
    });
    let mut stdout = child
        .stdout
        .take()
        .context("converter output unavailable")?;
    let fd = stdout.as_raw_fd();
    if unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.wait();
        let _ = writer.join();
        bail!("could not bound converter output");
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut output = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut reaped = false;
    let result = (|| -> Result<()> {
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => {
                    if let Some(status) = child.try_wait()? {
                        reaped = true;
                        if !status.success() {
                            bail!("converter rejected PDF");
                        }
                        break;
                    }
                }
                Ok(n) => {
                    if output.len() + n > output_limit {
                        bail!("converter output exceeds limit");
                    }
                    output.extend_from_slice(&chunk[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
            if Instant::now() >= deadline {
                bail!("converter timed out");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    })();
    // A private process group is owned by this invocation only.
    if !reaped {
        // Until wait/reap, this PID cannot be reused by another user process.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    let _ = child.wait();
    let _ = writer.join();
    result?;
    Ok(output)
}
#[cfg(not(unix))]
fn pdf_page(_: &[u8], _: u32) -> Result<Vec<u8>> {
    bail!("PDF converter unavailable on this platform")
}

#[cfg(not(unix))]
fn pdf_page_count(_: &[u8]) -> Result<u32> {
    bail!("PDF converter unavailable on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str, bytes: &[u8]) -> Value {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        super::render(&path, super::super::open_read(&path).unwrap()).unwrap()
    }
    #[test]
    fn crlf_cr_tabs_and_hostile_controls() {
        let v = fixture("notes.md", b"one\r\ntwo\rthree\tend\x1b[31m\x07");
        assert_eq!(v["kind"], "markdown");
        assert_eq!(v["text"], "one\ntwo\nthree   end\\u{1b}[31m\\u{7}");
        assert!(!v["text"].as_str().unwrap().contains('\r'));
    }
    #[test]
    fn real_png_thumbnail_and_magic_detection() {
        let img = image::RgbaImage::from_pixel(320, 200, image::Rgba([255, 0, 0, 128]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        let v = fixture("misleading.txt", out.get_ref());
        assert_eq!(v["kind"], "image");
        assert_eq!(v["image"]["width"], 320);
        assert_eq!(v["image"]["height"], 200);
        let png = STANDARD
            .decode(v["image"]["png"].as_str().unwrap())
            .unwrap();
        let rgba = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(rgba.dimensions(), (320, 200));
        assert_eq!(&rgba.as_raw()[..4], &[255, 0, 0, 128]);
    }
    #[test]
    fn complex_large_image_stays_inside_compressed_wire_budget() {
        let mut random = 42u32;
        let image = image::RgbaImage::from_fn(640, 480, |_, _| {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            image::Rgba(random.to_le_bytes())
        });
        let mut source = Cursor::new(Vec::new());
        image
            .write_to(&mut source, image::ImageFormat::Png)
            .unwrap();
        let result = fixture("noise.png", source.get_ref());
        let encoded = result["image"]["png"].as_str().unwrap();
        assert!(encoded.len() <= 800_000);
        let png = STANDARD.decode(encoded).unwrap();
        let raster = image::load_from_memory(&png).unwrap();
        assert_eq!(
            raster.width() as u64,
            result["image"]["width"].as_u64().unwrap()
        );
        assert!(serde_json::to_vec(&result).unwrap().len() < 1024 * 1024);
    }
    #[test]
    fn supported_image_formats_have_real_thumbnails() {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            12,
            12,
            image::Rgb([12, 34, 56]),
        ));
        for format in [
            image::ImageFormat::Jpeg,
            image::ImageFormat::Gif,
            image::ImageFormat::WebP,
            image::ImageFormat::Bmp,
            image::ImageFormat::Ico,
        ] {
            let mut out = Cursor::new(Vec::new());
            if format == image::ImageFormat::Ico {
                image.to_rgba8().write_to(&mut out, format).unwrap();
            } else {
                image.write_to(&mut out, format).unwrap();
            }
            let v = fixture("image.bin", out.get_ref());
            assert_eq!(v["kind"], "image", "{format:?}");
            assert_eq!(v["image"]["width"], 12, "{format:?} {}", v["text"]);
            assert_eq!(v["image"]["height"], 12, "{format:?} {}", v["text"]);
        }
    }
    #[test]
    fn malformed_images_and_oversized_input() {
        let v = fixture("bad.png", b"\x89PNG\r\n\x1a\nno image");
        assert_eq!(v["kind"], "image");
        assert!(v["image"].is_null());
        assert!(v["text"].as_str().unwrap().contains("unavailable"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.png");
        let mut file = File::create(&path).unwrap();
        use std::io::Write;
        file.write_all(b"\x89PNG\r\n\x1a\n").unwrap();
        file.set_len(INPUT + 1).unwrap();
        drop(file);
        let v = render(&path, File::open(&path).unwrap()).unwrap();
        assert!(v["text"].as_str().unwrap().contains("16 MiB"));
    }
    #[test]
    fn archives_are_listed_without_extraction() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "folder/example.txt", Cursor::new(b"abc"))
            .unwrap();
        builder.finish().unwrap();
        let bytes = builder.into_inner().unwrap();
        let v = fixture("archive.tar", &bytes);
        assert_eq!(v["kind"], "archive");
        assert!(v["text"].as_str().unwrap().contains("folder/example.txt"));
    }
    #[test]
    fn enormous_pax_name_stays_inside_response_budget() {
        let mut builder = tar::Builder::new(Vec::new());
        let name = "a".repeat(1_100_000);
        builder
            .append_pax_extensions([("path", name.as_bytes())])
            .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "short", Cursor::new(b""))
            .unwrap();
        builder.finish().unwrap();
        let v = fixture("long-name.tar", &builder.into_inner().unwrap());
        assert_eq!(v["truncated"], true);
        assert!(v["text"].as_str().unwrap().len() < TEXT);
        assert!(serde_json::to_vec(&v).unwrap().len() < TEXT + 1024);
    }

    #[test]
    fn wav_metadata_is_inert() {
        let mut wav = vec![0; 44];
        wav[..4].copy_from_slice(b"RIFF");
        wav[8..12].copy_from_slice(b"WAVE");
        wav[12..16].copy_from_slice(b"fmt ");
        wav[22..24].copy_from_slice(&2u16.to_le_bytes());
        wav[24..28].copy_from_slice(&48000u32.to_le_bytes());
        wav[34..36].copy_from_slice(&16u16.to_le_bytes());
        let v = fixture("a.wav", &wav);
        assert_eq!(v["kind"], "media");
        assert!(v["text"].as_str().unwrap().contains("48000 Hz"));
    }
    #[cfg(unix)]
    #[test]
    fn missing_converter_is_explicit_and_timeout_is_bounded() {
        let missing = pdf_converter(
            b"%PDF-",
            std::ffi::OsStr::new("/cx-definitely-missing-pdf-converter"),
        )
        .unwrap_err();
        assert!(missing.to_string().contains("not available"));
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("slow-converter");
        std::fs::write(&script, b"#!/bin/sh\nsleep 30\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let start = std::time::Instant::now();
        let error = pdf_converter(b"%PDF-", script.as_os_str()).unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }
    #[cfg(unix)]
    #[test]
    fn converter_output_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("noisy-converter");
        std::fs::write(&script, b"#!/bin/sh\nhead -c 3000000 /dev/zero\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let error = pdf_converter(b"%PDF-", script.as_os_str()).unwrap_err();
        assert!(
            error.to_string().contains("output exceeds limit"),
            "{error}"
        );
    }
    #[test]
    fn excessive_declared_dimensions_rejected_before_decode() {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Bmp).unwrap();
        let mut bytes = out.into_inner();
        // BMP width/height fields: preserve a valid header but declare 100k².
        bytes[18..22].copy_from_slice(&100_000i32.to_le_bytes());
        bytes[22..26].copy_from_slice(&100_000i32.to_le_bytes());
        let v = fixture("bomb.bmp", &bytes);
        assert!(v["image"].is_null());
        assert!(v["text"].as_str().unwrap().contains("limit"));
    }
    fn three_page_pdf() -> Vec<u8> {
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0];
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R 5 0 R 7 0 R] /Count 3 >>".to_owned(),
        ];
        for (index, color) in ["1 0 0", "0 1 0", "0 0 1"].iter().enumerate() {
            objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << >> /Contents {} 0 R >>",4+index*2));
            let content = format!("{color} rg 0 0 200 100 re f\n");
            objects.push(format!(
                "<< /Length {} >>\nstream\n{content}endstream",
                content.len()
            ));
        }
        for (index, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets.iter().skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }
    #[test]
    fn actual_three_page_pdf_exact_pages_and_bounds() {
        if std::process::Command::new("pdfinfo")
            .arg("-v")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("- PDF\u{1b}[31m.pdf");
        std::fs::write(&path, three_page_pdf()).unwrap();
        for page in 1..=3 {
            let response = render_page(&path, File::open(&path).unwrap(), page).unwrap();
            assert_eq!(response["pages"], 3);
            assert_eq!(response["page"], page);
            assert_eq!(response["title"], format!("PDF · page {page}"));
            let png = STANDARD
                .decode(response["image"]["png"].as_str().unwrap())
                .unwrap();
            let image = image::load_from_memory(&png).unwrap().to_rgb8();
            let pixel = image.get_pixel(image.width() / 2, image.height() / 2);
            for channel in 0..3 {
                assert_eq!(
                    pixel[channel],
                    if channel == page as usize - 1 { 255 } else { 0 }
                );
            }
            assert!(serde_json::to_vec(&response).unwrap().len() < 1024 * 1024);
        }
        let original = std::fs::read(&path).unwrap();
        for page in [0, 4, 10001] {
            assert!(render_page(&path, File::open(&path).unwrap(), page).is_err());
        }
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let default = render(&path, File::open(&path).unwrap()).unwrap();
        assert_eq!(default["page"], 1);
        assert_eq!(default["pages"], 3);
        std::fs::write(&path, b"%PDF-1.4 broken").unwrap();
        assert!(render_page(&path, File::open(&path).unwrap(), 1).is_err());
        assert!(render(&path, File::open(&path).unwrap()).unwrap()["image"].is_null());
        std::fs::write(&path, b"text file").unwrap();
        assert!(render_page(&path, File::open(&path).unwrap(), 1).is_err());
    }
    #[test]
    fn page_metadata_is_allowlisted_and_precise() {
        assert_eq!(
            parse_page_count(b"Title: malicious\x1b[31m\nPages: 3\n").unwrap(),
            3
        );
        for data in [
            b"Pages: 0\n".as_slice(),
            b"Pages: 3\nPages: 4\n",
            b"Pages: junk\n",
            b"Pages: 4294967296\n",
            b"Pages: 3\x1b[31m\n",
            b"No pages\n",
        ] {
            assert!(parse_page_count(data).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn metadata_converter_output_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("noisy-metadata");
        std::fs::write(&script, b"#!/bin/sh\nhead -c 70000 /dev/zero\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let error = run_converter(b"%PDF-", script.as_os_str(), &["-"], 64 * 1024).unwrap_err();
        assert!(error.to_string().contains("output exceeds limit"));
    }
    #[test]
    fn real_pdf_first_page_when_poppler_available() {
        if std::process::Command::new("pdftoppm")
            .arg("-v")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_err()
        {
            return;
        }
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0];
        for (i,body) in ["<< /Type /Catalog /Pages 2 0 R >>","<< /Type /Pages /Kids [3 0 R] /Count 1 >>","<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << >> /Contents 4 0 R >>","<< /Length 0 >>\nstream\n\nendstream"].iter().enumerate() {
            offsets.push(pdf.len());pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n",i+1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        let v = fixture("a.pdf", &pdf);
        assert_eq!(v["kind"], "pdf");
        assert!(!v["image"].is_null(), "{v}");
        assert_eq!(v["title"], "PDF · page 1");
    }
}
