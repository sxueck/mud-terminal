use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// Entries shown by [`list`]; one more on disk marks the listing as truncated.
const MAX_ENTRIES: usize = 500;
/// Bytes accepted by [`preview`]; one more read makes the file "oversized".
const MAX_PREVIEW: usize = 1 << 20;

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
}

#[derive(Debug, Clone)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub truncated: bool,
}

pub fn list(path: &Path) -> Result<Listing, String> {
    let directory = fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in directory {
        let entry = entry.map_err(|e| e.to_string())?;
        if entries.len() == MAX_ENTRIES {
            truncated = true;
            break;
        }
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path(),
            directory: file_type.is_dir(),
        });
    }
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Listing { entries, truncated })
}

pub fn preview(path: &Path) -> Result<String, String> {
    // Bound previews before allocating; full editable buffers arrive with the Phase 3 editor.
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Only regular text files can be previewed.".into());
    }
    let mut bytes = Vec::new();
    file.take((MAX_PREVIEW + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_PREVIEW {
        return Err("Preview limit is 1 MiB.".into());
    }
    if bytes.contains(&0) {
        return Err("Binary files cannot be previewed.".into());
    }
    String::from_utf8(bytes).map_err(|_| "This file is not UTF-8 text.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_real_tree_and_rejects_binary_and_oversized_previews() {
        let root = std::env::temp_dir().join(format!("mud-files-test-{}", std::process::id()));
        fs::create_dir_all(root.join("folder")).expect("directory");
        fs::write(root.join("a.txt"), "hello").expect("text");
        fs::write(root.join("b.bin"), [0, 1, 2]).expect("binary");
        fs::write(root.join("large"), vec![b'a'; MAX_PREVIEW + 1]).expect("large file");
        assert!(list(&root).expect("listing").entries[0].directory);
        assert_eq!(preview(&root.join("a.txt")).expect("preview"), "hello");
        assert!(preview(&root.join("b.bin")).is_err());
        assert!(preview(&root.join("large")).is_err());
        fs::remove_dir_all(root).expect("cleanup");
    }
}
