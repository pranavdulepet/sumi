use crate::store::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use quick_xml::{events::Event, Reader};
use serde_json::{json, Value};
use std::{
    ffi::{CStr, CString},
    io::{Cursor, Read},
};

extern "C" {
    fn sumi_read_document(
        bytes: *const u8,
        length: usize,
        kind: *const std::ffi::c_char,
    ) -> *mut std::ffi::c_char;
    fn sumi_image_preview(bytes: *const u8, length: usize) -> *mut std::ffi::c_char;
    fn sumi_free(pointer: *mut std::ffi::c_char);
}
fn native(bytes: &[u8], kind: &str) -> Res<String> {
    let kind = CString::new(kind).msg()?;
    unsafe {
        let pointer = sumi_read_document(bytes.as_ptr(), bytes.len(), kind.as_ptr());
        if pointer.is_null() {
            return Err("No readable text or image subjects were found in that file.".into());
        }
        let out = CStr::from_ptr(pointer).to_string_lossy().into_owned();
        sumi_free(pointer);
        Ok(out)
    }
}
type Archive = zip::ZipArchive<Cursor<Vec<u8>>>;
fn part(zip: &mut Archive, name: &str) -> Res<String> {
    let mut file = zip.by_name(name).msg()?;
    if file.size() > 4_000_000 {
        return Err("This document part is too large to read.".into());
    }
    let mut out = String::new();
    (&mut file).take(4_000_001).read_to_string(&mut out).msg()?;
    if out.len() > 4_000_000 {
        return Err("This document part is too large to read.".into());
    }
    Ok(out)
}
// Read only text, cells and rows. Never resolve external links, macros or formulas.
fn office_xml(xml: &str, strings: &[String], shared: bool) -> Res<Vec<String>> {
    let mut parser = Reader::from_str(xml);
    let mut rows = vec![];
    let mut row = String::new();
    let mut cell = String::new();
    let mut capture = false;
    let mut index = false;
    let mut next_col = 0;
    loop {
        match parser.read_event().msg()? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"t" | b"v" => capture = true,
                b"c" => {
                    cell.clear();
                    let column = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == b"r")
                        .and_then(|a| {
                            let mut letters = a
                                .value
                                .iter()
                                .copied()
                                .take_while(u8::is_ascii_alphabetic)
                                .peekable();
                            letters.peek()?;
                            letters
                                .try_fold(0usize, |n, b| {
                                    n.checked_mul(26)?
                                        .checked_add((b.to_ascii_uppercase() - b'A' + 1) as usize)
                                })
                                .map(|n| n - 1)
                        })
                        .unwrap_or(next_col);
                    if column > 16383 {
                        return Err("Invalid spreadsheet column.".into());
                    }
                    if next_col > 0 {
                        row.push('\t');
                    }
                    for _ in next_col..column {
                        row.push('\t');
                    }
                    next_col = column + 1;
                    index = e
                        .attributes()
                        .flatten()
                        .any(|a| a.key.as_ref() == b"t" && a.value.as_ref() == b"s");
                }
                _ => (),
            },
            Event::Text(e) if capture => cell.push_str(&e.decode().msg()?),
            Event::GeneralRef(e) if capture => {
                let name = e.decode().msg()?;
                if let Some(c) = e.resolve_char_ref().msg()? {
                    cell.push(c)
                } else if let Some(v) = quick_xml::escape::resolve_predefined_entity(&name) {
                    cell.push_str(v)
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"t" | b"v" => capture = false,
                b"c" => {
                    if index {
                        row.push_str(
                            cell.parse::<usize>()
                                .ok()
                                .and_then(|i| strings.get(i))
                                .map(String::as_str)
                                .unwrap_or(""),
                        )
                    } else {
                        row.push_str(&cell)
                    }
                    cell.clear();
                }
                b"p" | b"si" => {
                    rows.push(std::mem::take(&mut cell));
                }
                b"row" => {
                    rows.push(std::mem::take(&mut row));
                    next_col = 0;
                }
                _ => (),
            },
            Event::DocType(_) => return Err("Document type declarations are unsupported.".into()),
            Event::Eof => break,
            _ => (),
        }
        if rows.len() > if shared { 50000 } else { 12000 } {
            return Err("This document contains too many rows.".into());
        }
    }
    Ok(rows)
}
pub fn read(a: &Value) -> Res<Value> {
    let name = a["name"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() < 256
                && !s.contains(['/', '\\'])
                && !s.chars().any(char::is_control)
        })
        .ok_or("Invalid attachment name.")?;
    let encoded = a["data"]
        .as_str()
        .filter(|d| d.len() <= 11_184_812)
        .ok_or("Choose a file under 8 MB.")?;
    let bytes = STANDARD.decode(encoded).msg()?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Choose a file under 8 MB.".into());
    }
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let image = matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "heic" | "tiff"
    );
    let preview = if image {
        unsafe {
            let p = sumi_image_preview(bytes.as_ptr(), bytes.len());
            if p.is_null() {
                return Err("Could not open this image.".into());
            }
            let encoded = CStr::from_ptr(p).to_string_lossy().into_owned();
            sumi_free(p);
            Some(encoded)
        }
    } else {
        None
    };
    let text = match ext.as_str() {
        _ if image => native(&bytes, &ext).unwrap_or_else(|_| {
            "Image attached. Local text recognition found no readable words or reliable labels."
                .into()
        }),
        "txt" | "md" | "csv" | "tsv" => {
            String::from_utf8(bytes).map_err(|_| "Save the text file as UTF-8.")?
        }
        "pdf" | "doc" | "rtf" | "png" | "jpg" | "jpeg" | "webp" | "heic" | "tiff" => {
            native(&bytes, &ext)?
        }
        "docx" | "xlsx" => {
            let mut zip = Archive::new(Cursor::new(bytes)).msg()?;
            if zip.len() > 2048 {
                return Err("This document has too many parts.".into());
            }
            if ext == "docx" {
                office_xml(&part(&mut zip, "word/document.xml")?, &[], false)?.join("\n")
            } else {
                let strings = if zip.index_for_name("xl/sharedStrings.xml").is_some() {
                    office_xml(&part(&mut zip, "xl/sharedStrings.xml")?, &[], true)?
                } else {
                    vec![]
                };
                let mut sheets: Vec<_> = zip
                    .file_names()
                    .filter(|n| n.starts_with("xl/worksheets/sheet") && n.ends_with(".xml"))
                    .map(str::to_owned)
                    .collect();
                sheets.sort();
                let mut text = String::new();
                for sheet in sheets.iter().take(16) {
                    text.push_str(sheet);
                    text.push('\n');
                    text.push_str(
                        &office_xml(&part(&mut zip, sheet)?, &strings, false)?.join("\n"),
                    );
                    text.push('\n');
                    if text.len() > 65000 {
                        break;
                    }
                }
                text
            }
        }
        _ => return Err("Choose a PDF, document, image, spreadsheet or text file.".into()),
    };
    if text.trim().is_empty() {
        return Err("This file has no readable content.".into());
    }
    Ok(
        json!({"name":name,"text":text.chars().take(60000).collect::<String>(),"truncated":text.chars().count()>60000,"preview":preview}),
    )
}

pub fn context(files: &[Value], question: &str) -> Vec<Value> {
    let terms = crate::conversation::words(question);
    files.iter().map(|file|{
        let text=file["text"].as_str().unwrap_or("");
        let chars:Vec<char>=text.chars().collect();
        let mut chunks:Vec<_>=chars.chunks(700).enumerate().map(|(i,chunk)|{
            let body:String=chunk.iter().collect();let score=terms.intersection(&crate::conversation::words(&body)).count();(i,score,body)
        }).collect();
        chunks.sort_by_key(|(i,score,_)|(std::cmp::Reverse(*score),*i));
        chunks.truncate(2); chunks.sort_by_key(|v|v.0);
        json!({"name":file["name"],"excerpts":chunks.into_iter().map(|v|v.2).collect::<Vec<_>>()})
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn archived(parts: &[(&str, &str)]) -> String {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in parts {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        STANDARD.encode(zip.finish().unwrap().into_inner())
    }
    #[test]
    fn office_text_and_cells_remain_in_reading_order() {
        let doc=archived(&[("word/document.xml","<w:document xmlns:w='urn:word'><w:p><w:r><w:t>Cut &amp; </w:t></w:r><w:r><w:t>fold</w:t></w:r></w:p><w:p><w:t>Bind</w:t></w:p></w:document>")]);
        assert_eq!(
            read(&json!({"name":"notes.docx","data":doc})).unwrap()["text"],
            "Cut & fold\nBind"
        );
        let sheet = archived(&[
            (
                "xl/sharedStrings.xml",
                "<sst><si><t>Bookbinding</t></si></sst>",
            ),
            (
                "xl/worksheets/sheet1.xml",
                "<sheet><row><c r='A1' t='s'><v>0</v></c><c r='C1'><v>6</v></c></row></sheet>",
            ),
        ]);
        assert!(
            read(&json!({"name":"classes.xlsx","data":sheet})).unwrap()["text"]
                .as_str()
                .unwrap()
                .contains("Bookbinding\t\t6")
        );
        assert!(read(&json!({"name":"notes.docx","data":archived(&[("word/document.xml","<!DOCTYPE x><x/>")])})).is_err());
    }
    #[test]
    fn excerpt_selection_reaches_beyond_the_first_page() {
        let full = "Paper ".repeat(2500) + "The gallery code is MARIGOLD.";
        let answer = context(&[json!({"name":"notes.txt","text":full})], "gallery code");
        assert!(answer[0]["excerpts"].to_string().contains("MARIGOLD"));
    }
}

pub fn photos(files: &[Value]) -> Res<Vec<String>> {
    files
        .iter()
        .filter_map(|f| f["preview"].as_str())
        .map(|s| {
            if s.len() > 600000
                || !STANDARD
                    .decode(s)
                    .is_ok_and(|v| v.starts_with(&[0xff, 0xd8, 0xff]))
            {
                return Err("Invalid image preview.".into());
            }
            Ok(s.to_owned())
        })
        .take(4)
        .collect()
}

#[cfg(test)]
mod native_files {
    use super::*;
    #[test]
    fn reads_text_from_pdf_and_rtf_using_the_macos_readers() {
        let stream = "BT /F1 16 Tf 20 80 Td (Bookbinding on Sunday) Tj ET";
        let objects=["<< /Type /Catalog /Pages 2 0 R >>".to_owned(),"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 100] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".into(),"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),format!("<< /Length {} >>\nstream\n{}\nendstream",stream.len(),stream)];
        let mut doc = "%PDF-1.4\n".to_string();
        let mut offsets = vec![];
        for (i, item) in objects.iter().enumerate() {
            offsets.push(doc.len());
            doc += &format!("{} 0 obj\n{item}\nendobj\n", i + 1);
        }
        let xref = doc.len();
        doc += "xref\n0 6\n0000000000 65535 f \n";
        for o in offsets {
            doc += &format!("{o:010} 00000 n \n");
        }
        doc += &format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF");
        for (name, bytes) in [
            ("schedule.pdf", doc.as_bytes()),
            (
                "schedule.rtf",
                br"{\rtf1\ansi Bookbinding on Sunday}" as &[u8],
            ),
        ] {
            let parsed = read(&json!({"name":name,"data":STANDARD.encode(bytes)})).unwrap();
            assert!(parsed["text"]
                .as_str()
                .unwrap()
                .contains("Bookbinding on Sunday"));
        }
    }
}
