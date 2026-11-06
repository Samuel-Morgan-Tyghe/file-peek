use percent_encoding::percent_decode_str;
use pulldown_cmark::{Options, Parser, html};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use url::Url;

pub const MAX_TEXT_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

pub fn is_markdown(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "md" | "markdown" | "mdown" | "mkd" | "mdx"
    )
}

pub fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase()
}

pub fn known_text(path: &Path) -> bool {
    is_markdown(path)
        || matches!(
            extension(path).as_str(),
            "txt"
                | "text"
                | "json"
                | "yaml"
                | "yml"
                | "toml"
                | "csv"
                | "tsv"
                | "log"
                | "xml"
                | "html"
                | "htm"
                | "css"
                | "js"
                | "mjs"
                | "cjs"
                | "jsx"
                | "ts"
                | "tsx"
                | "swift"
                | "py"
                | "rb"
                | "sh"
                | "bash"
                | "zsh"
                | "sql"
                | "rs"
                | "go"
                | "c"
                | "h"
                | "cpp"
                | "java"
                | "ini"
                | "conf"
                | "env"
                | "gitignore"
        )
        || matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("Dockerfile" | "Makefile" | "LICENSE" | "README" | ".gitignore" | ".env")
        )
}

pub fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("Choose a file to preview, rather than a folder.".into());
    }
    let mut data = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|error| error.to_string())?;
    if data.len() as u64 > limit {
        return Err(format!(
            "This file exceeds the {} MB preview limit.",
            limit / 1024 / 1024
        ));
    }
    Ok(data)
}

pub fn decode_text(data: &[u8]) -> Result<String, String> {
    let text = if data.starts_with(&[0xff, 0xfe, 0, 0]) || data.starts_with(&[0, 0, 0xfe, 0xff]) {
        let little = data[0] == 0xff;
        if !(data.len() - 4).is_multiple_of(4) {
            return Err("Invalid UTF-32 text.".into());
        }
        data[4..]
            .chunks_exact(4)
            .map(|bytes| {
                let array = [bytes[0], bytes[1], bytes[2], bytes[3]];
                char::from_u32(if little {
                    u32::from_le_bytes(array)
                } else {
                    u32::from_be_bytes(array)
                })
                .ok_or_else(|| "Invalid UTF-32 text.".to_owned())
            })
            .collect::<Result<String, _>>()?
    } else if data.starts_with(&[0xff, 0xfe]) || data.starts_with(&[0xfe, 0xff]) {
        let little = data[0] == 0xff;
        if !(data.len() - 2).is_multiple_of(2) {
            return Err("Invalid UTF-16 text.".into());
        }
        let words: Vec<u16> = data[2..]
            .chunks_exact(2)
            .map(|bytes| {
                if little {
                    u16::from_le_bytes([bytes[0], bytes[1]])
                } else {
                    u16::from_be_bytes([bytes[0], bytes[1]])
                }
            })
            .collect();
        String::from_utf16(&words).map_err(|error| error.to_string())?
    } else {
        let bytes = data.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(data);
        String::from_utf8(bytes.to_vec())
            .map_err(|_| "This file isn't readable UTF-8, UTF-16 or UTF-32 text.".to_owned())?
    };
    if text.contains('\0') {
        return Err("This appears to be a binary file. Open it in its original app.".into());
    }
    Ok(text)
}

pub fn markdown_html(source: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    let mut output = String::new();
    html::push_html(&mut output, Parser::new_ext(source, options));
    output
}

pub fn local_asset(address: &str, directory: &Path) -> Option<PathBuf> {
    let url = Url::parse(address).ok()?;
    if url.scheme() != "peek-asset" || url.host_str() != Some("local") {
        return None;
    }
    let relative = percent_decode_str(url.path()).decode_utf8().ok()?;
    let root = directory.canonicalize().ok()?;
    let file = root
        .join(relative.trim_start_matches('/'))
        .canonicalize()
        .ok()?;
    (file.starts_with(&root) && file.is_file()).then_some(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn recognizes_markdown_and_source_files() {
        assert!(is_markdown(Path::new("README.MD")));
        assert!(known_text(Path::new("main.rs")));
        assert!(known_text(Path::new(".gitignore")));
        assert!(!known_text(Path::new("photo.png")));
    }

    #[test]
    fn decodes_unicode_encodings_and_rejects_binary() {
        assert_eq!(decode_text("café 🌍".as_bytes()).unwrap(), "café 🌍");
        assert_eq!(decode_text(&[0xff, 0xfe, 0x48, 0, 0x69, 0]).unwrap(), "Hi");
        assert_eq!(decode_text(&[0xfe, 0xff, 0, 0x48, 0, 0x69]).unwrap(), "Hi");
        assert_eq!(
            decode_text(&[0xff, 0xfe, 0, 0, 0x48, 0, 0, 0]).unwrap(),
            "H"
        );
        assert_eq!(
            decode_text(&[0, 0, 0xfe, 0xff, 0, 0, 0, 0x48]).unwrap(),
            "H"
        );
        assert_eq!(decode_text(&[0xef, 0xbb, 0xbf, b'H']).unwrap(), "H");
        for invalid in [
            &[0, 1, 2][..],
            &[0xff, 0xfe, 0][..],
            &[0xfe, 0xff, 0xd8, 0][..],
            &[0xff][..],
        ] {
            assert!(decode_text(invalid).is_err());
        }
    }

    #[test]
    fn markdown_preserves_tables_tasks_code_and_unicode() {
        let rendered = markdown_html(
            "# café 🌍\n\n**bold** and *italic*\n\n|A|B|\n|-|-|\n|1|2|\n\n- [x] Done\n\n```rust\nlet x = 1;\n```\n\n[Local](other.md)",
        );
        for expected in [
            "<h1>café 🌍</h1>",
            "<strong>bold</strong>",
            "<em>italic</em>",
            "<table>",
            "<td>1</td>",
            "checked=",
            "language-rust",
            "href=\"other.md\"",
        ] {
            assert!(
                rendered.contains(expected),
                "Missing {expected} in {rendered}"
            );
        }
    }

    #[test]
    fn enforces_size_and_image_directory_boundaries() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("file-peek-test-{nonce}"));
        let docs = base.join("docs");
        fs::create_dir_all(&docs).unwrap();
        let docs = docs.canonicalize().unwrap();
        fs::write(docs.join("image.png"), b"image").unwrap();
        fs::write(docs.join("literal%20name.png"), b"image").unwrap();
        fs::write(base.join("private.png"), b"private").unwrap();
        std::os::unix::fs::symlink(&base, docs.join("escape")).unwrap();
        assert_eq!(
            local_asset("peek-asset://local/image.png", &docs),
            Some(docs.join("image.png"))
        );
        assert_eq!(
            local_asset("peek-asset://local/literal%2520name.png", &docs),
            Some(docs.join("literal%20name.png"))
        );
        for invalid in [
            "https://example.com/image.png",
            "peek-asset://other/image.png",
            "peek-asset://local/%2e%2e/private.png",
            "peek-asset://local/escape/private.png",
            "peek-asset://local/%252e%252e/private.png",
        ] {
            assert!(local_asset(invalid, &docs).is_none(), "Accepted {invalid}");
        }
        assert!(read_limited(&docs.join("image.png"), 4).is_err());
        assert!(read_limited(&docs, 10).is_err());
        assert_eq!(read_limited(&docs.join("image.png"), 5).unwrap(), b"image");
        fs::remove_dir_all(base).unwrap();
    }
}
