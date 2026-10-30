//! Images a provider generates during a turn, such as Codex's image tool.
//! They are saved as PNG files in the data folder, as the Tauri app does, and
//! an `image` row names the file.
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

pub const ROLE: &str = "image";
/// The folder under the data folder, shared with the Tauri app.
pub const DIR: &str = "generated-images";
const MAX_BYTES: usize = 25 * 1024 * 1024;
const MAX_DATA_BYTES: usize = MAX_BYTES * 4 / 3 + 4;

/// An `image` row's text, in the Tauri app's `image` block shape.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    pub path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
}

fn safe_name(name: &str) -> String {
    let leaf = Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("attachment");
    let cleaned: String = leaf
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(['.', '-']);
    if trimmed.is_empty() {
        "attachment".into()
    } else {
        trimmed.chars().take(80).collect()
    }
}

/// Decodes base64 PNG `data` and writes it to a new file in `data_dir`.
pub fn save(data_dir: &Path, data: &str, name: &str, alt: Option<String>) -> Result<Image, String> {
    let too_large = || {
        format!(
            "Generated image is too large (maximum {} MB).",
            MAX_BYTES / 1024 / 1024
        )
    };
    if data.len() > MAX_DATA_BYTES {
        return Err(too_large());
    }
    let encoded: String = data
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .map_err(|_| "Generated image data is not valid base64.".to_owned())?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(too_large());
    }
    if !bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Err("Generated image data is not a PNG image.".into());
    }
    let dir = data_dir.join(DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = safe_name(name);
    let destination = dir.join(format!("{}-{name}.png", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|e| format!("{}: {e}", destination.display()))?;
    if let Err(error) = file.write_all(&bytes) {
        let _ = std::fs::remove_file(&destination);
        return Err(format!("{}: {error}", destination.display()));
    }
    Ok(Image {
        path: destination.to_string_lossy().into_owned(),
        name,
        mime_type: "image/png".into(),
        size: bytes.len() as u64,
        alt: alt
            .map(|alt| alt.trim().to_owned())
            .filter(|alt| !alt.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];

    #[test]
    fn a_png_is_saved_once_under_the_data_folder_with_a_safe_name() {
        let dir = tempfile::tempdir().unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(PNG);
        // Codex may wrap long base64 lines.
        let wrapped = format!("{}\n{}", &data[..8], &data[8..]);
        let first = save(
            dir.path(),
            &wrapped,
            "../../etc/a cat?.png",
            Some("  A cat  ".into()),
        )
        .unwrap();
        let second = save(dir.path(), &data, "", None).unwrap();
        assert_ne!(first.path, second.path, "each image gets its own file");
        for image in [&first, &second] {
            let path = Path::new(&image.path);
            assert_eq!(path.parent().unwrap(), dir.path().join(DIR));
            assert_eq!(std::fs::read(path).unwrap(), PNG);
            assert_eq!(image.size, PNG.len() as u64);
            assert_eq!(image.mime_type, "image/png");
        }
        assert_eq!(first.name, "a-cat-.png");
        assert!(first.path.ends_with("-a-cat-.png.png"));
        assert_eq!(first.alt.as_deref(), Some("A cat"));
        assert_eq!(second.name, "attachment");
        assert_eq!(second.alt, None);
    }

    #[test]
    fn only_bounded_base64_pngs_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        assert_eq!(
            save(dir.path(), "not base64!", "x", None).unwrap_err(),
            "Generated image data is not valid base64."
        );
        assert_eq!(
            save(dir.path(), &encode(b"GIF89a"), "x", None).unwrap_err(),
            "Generated image data is not a PNG image."
        );
        assert!(save(dir.path(), "", "x", None)
            .unwrap_err()
            .contains("too large"));
        let huge = "A".repeat(MAX_DATA_BYTES + 1);
        assert!(save(dir.path(), &huge, "x", None)
            .unwrap_err()
            .contains("too large"));
        assert!(
            !dir.path().join(DIR).exists(),
            "nothing is written for a refusal"
        );
    }

    #[test]
    fn rows_use_the_tauri_image_block_shape() {
        let image = Image {
            path: "/data/generated-images/1-cat.png".into(),
            name: "cat".into(),
            mime_type: "image/png".into(),
            size: 12,
            alt: None,
        };
        let value = serde_json::to_value(&image).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"path":"/data/generated-images/1-cat.png","name":"cat","mimeType":"image/png","size":12})
        );
        let sparse: Image =
            serde_json::from_value(serde_json::json!({"path":"/x.png","future":1})).unwrap();
        assert_eq!(sparse.path, "/x.png");
    }
}
