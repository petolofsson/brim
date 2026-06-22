use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn short_id(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() < 8 {
        return value.to_string();
    }
    let prefix: String = chars.iter().take(4).collect();
    let suffix: String = chars
        .iter()
        .rev()
        .take(4)
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}\u{2026}{suffix}")
}

/// Read at most this many bytes from the file tail (CODERULES r2-3).
/// Covers ~100 recent turns even for verbose transcripts.
const TAIL_CAP_BYTES: u64 = 256 * 1024;

/// Read the tail of a file up to TAIL_CAP_BYTES, returning only complete lines.
pub fn read_tail(path: &Path) -> anyhow::Result<String> {
    let mut file = fs::File::open(path)?;
    let file_len = file.metadata()?.len();
    let start = file_len.saturating_sub(TAIL_CAP_BYTES);

    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.take(TAIL_CAP_BYTES).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf).into_owned();

    if start > 0 {
        // Discard the partial first line produced by mid-file seek.
        return Ok(text
            .find('\n')
            .map(|i| text[i + 1..].to_string())
            .unwrap_or_default());
    }
    Ok(text)
}
