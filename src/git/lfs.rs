//! Git LFS support: which paths use the LFS filter, and resolving pointer files.

use std::collections::HashSet;
use std::path::Path;

use super::cmd;

const POINTER_PREFIX: &[u8] = b"version https://git-lfs.github.com/spec/v1";

/// True for an LFS pointer file (what git stores in commits/index for LFS-tracked files).
pub fn is_pointer(bytes: &[u8]) -> bool {
    bytes.len() < 1024 && bytes.starts_with(POINTER_PREFIX)
}

/// Size of the real object, from a pointer's `size <n>` line.
pub fn pointer_size(bytes: &[u8]) -> Option<u64> {
    if !is_pointer(bytes) {
        return None;
    }
    String::from_utf8_lossy(bytes)
        .lines()
        .find_map(|l| l.strip_prefix("size ").and_then(|n| n.trim().parse().ok()))
}

/// Replaces a pointer with the real content (from the local LFS cache, or downloaded).
pub fn smudge(repo: &Path, path: &str, pointer: &[u8]) -> Result<Vec<u8>, String> {
    cmd::run_with_input_bytes(repo, &["lfs", "smudge", "--", path], pointer)
        .map_err(|e| if e.is_empty() { "git-lfs could not fetch the file".to_owned() } else { e })
}

/// The bytes a user expects for `path`: pointers are resolved through git-lfs when possible.
/// Returns the content and whether it still is an unresolved pointer.
pub fn resolve(repo: &Path, path: &str, bytes: Vec<u8>) -> (Vec<u8>, bool) {
    if !is_pointer(&bytes) {
        return (bytes, false);
    }
    match smudge(repo, path, &bytes) {
        Ok(real) => (real, false),
        Err(_) => (bytes, true),
    }
}

/// Paths (of `paths`) whose `filter` attribute is `lfs`. With `source`, attributes are read
/// from that commit's `.gitattributes` files instead of the working tree.
pub fn lfs_paths(repo: &Path, source: Option<&str>, paths: &[String]) -> HashSet<String> {
    if paths.is_empty() {
        return HashSet::new();
    }
    let mut input = Vec::new();
    for p in paths {
        input.extend_from_slice(p.as_bytes());
        input.push(0);
    }
    let source_arg = source.map(|s| format!("--source={s}"));
    let mut args = vec!["check-attr", "-z", "--stdin"];
    if let Some(s) = &source_arg {
        args.push(s);
    }
    args.push("filter");
    let out = cmd::run_with_input_bytes(repo, &args, &input)
        // `--source` needs git 2.40+; fall back to working-tree attributes.
        .or_else(|_| cmd::run_with_input_bytes(repo, &["check-attr", "-z", "--stdin", "filter"], &input))
        .unwrap_or_default();
    let out = String::from_utf8_lossy(&out);
    // Output: <path> NUL <attribute> NUL <value> NUL ...
    let fields: Vec<&str> = out.split('\0').collect();
    fields
        .chunks(3)
        .filter(|c| c.len() == 3 && c[2] == "lfs")
        .map(|c| c[0].to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const POINTER: &[u8] = b"version https://git-lfs.github.com/spec/v1\noid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 12345\n";

    #[test]
    fn detects_pointers() {
        assert!(is_pointer(POINTER));
        assert_eq!(pointer_size(POINTER), Some(12345));
        assert!(!is_pointer(b"\x89PNG\r\n"));
    }

    #[test]
    fn finds_lfs_paths_from_attributes() {
        let dir = std::env::temp_dir().join(format!("gitr-lfs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        cmd::run(&dir, &["init", "-q"]).unwrap();
        std::fs::write(dir.join(".gitattributes"), "*.psd filter=lfs diff=lfs merge=lfs -text\n").unwrap();
        let paths = vec!["art/cover.psd".to_owned(), "src/main.rs".to_owned()];
        let lfs = lfs_paths(&dir, None, &paths);
        assert!(lfs.contains("art/cover.psd"));
        assert!(!lfs.contains("src/main.rs"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
