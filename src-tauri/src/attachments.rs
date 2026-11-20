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
    loop {
        match parser.read_event().msg()? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"t" | b"v" => capture = true,
                b"c" => {
                    cell.clear();
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
                    if !row.is_empty() {
                        row.push('\t')
                    }
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
                b"row" => rows.push(std::mem::take(&mut row)),
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
    let text = match ext.as_str() {
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
        json!({"name":name,"text":text.chars().take(60000).collect::<String>(),"truncated":text.chars().count()>60000}),
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
