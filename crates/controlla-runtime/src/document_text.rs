//! Local OS-clipboard text readback for editors that paint their text.
//!
//! Canvas-backed rich editors (Google Docs is the reference case) never reflect
//! typed text in the DOM, so an `Input.insertText`/key-event transaction cannot
//! prove what the document now contains. The document text is still available
//! through the application's own copy path: select all, copy, and read the
//! plain-text flavor from the local clipboard.
//!
//! The clipboard belongs to the user, so this module treats it as borrowed
//! state: the current contents are captured before the copy, bounded to
//! `MAX_CLIPBOARD_BYTES`, restored afterwards, and never logged or returned in
//! a receipt. Only derived evidence (lengths, a hash, and window-match counts)
//! leaves the process.

use std::time::Duration;

/// A document larger than this is not copied: restoring a truncated clipboard
/// would silently damage the user's data.
pub const MAX_CLIPBOARD_BYTES: usize = 512 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

/// Borrowed clipboard state. `capture()` returns `None` when no local clipboard
/// tool is available or the saved contents exceed the safe bound, in which case
/// the caller must report verification as unavailable instead of copying.
pub struct Clipboard {
    saved: String,
}

impl Clipboard {
    /// Save the user's current plain-text clipboard. Returns `None` when the
    /// clipboard cannot be read or is too large to restore safely.
    pub async fn capture() -> Option<Self> {
        let saved = read_text().await?;
        if saved.len() > MAX_CLIPBOARD_BYTES {
            return None;
        }
        Some(Self { saved })
    }

    /// Read the clipboard after a copy this transaction performed.
    pub async fn read(&self) -> Option<String> {
        let text = read_text().await?;
        (text.len() <= MAX_CLIPBOARD_BYTES).then_some(text)
    }

    /// The contents that were present before the transaction copied anything.
    pub fn saved(&self) -> &str {
        &self.saved
    }

    /// Put the user's clipboard back. Returns false when the restore failed.
    pub async fn restore(&self) -> bool {
        write_text(&self.saved).await
    }
}

/// Normalize text for human-readable comparison: newlines, non-breaking
/// spaces, smart punctuation, and invisible bidi/zero-width marks vary between
/// what was typed and what an editor stores.
pub fn normalize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\u{00a0}' | '\u{202f}' => out.push(' '),
            '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' => out.push('\''),
            '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' => out.push('"'),
            '\u{2013}' | '\u{2014}' | '\u{2212}' => out.push('-'),
            '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}' => {}
            other => out.push(other),
        }
    }
    out
}

/// Canonical form used to prove insertion despite autocorrect: case, spacing,
/// and punctuation are editor policy, the letters and digits are the content.
pub fn canonical_alnum(text: &str) -> String {
    normalize_text(text)
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Up to three evenly spaced 40-character windows of the typed text. Three
/// windows make a lucky substring collision unlikely; one is enough when the
/// inserted text is short.
pub fn verification_windows(typed: &str) -> Vec<String> {
    const WINDOW: usize = 40;
    const MIN: usize = 16;
    let canonical: Vec<char> = canonical_alnum(typed).chars().collect();
    if canonical.len() < MIN {
        return Vec::new();
    }
    let width = WINDOW.min(canonical.len());
    // Spread the windows over the start positions that exist. Anchoring on
    // fractions of the whole string collapses to one window as soon as the text
    // is barely longer than a window, which silently weakened verification.
    let max_start = canonical.len().saturating_sub(width);
    let mut starts: Vec<usize> = Vec::new();
    for start in [0, max_start / 2, max_start] {
        if !starts.contains(&start) {
            starts.push(start);
        }
    }
    let mut windows: Vec<String> = Vec::new();
    for start in starts {
        let window: String = canonical[start..start + width].iter().collect();
        if !windows.contains(&window) {
            windows.push(window);
        }
    }
    windows
}

/// Evidence that typed text is present in a copied document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextMatch {
    pub matched_windows: usize,
    pub sampled_windows: usize,
    pub exact_contains: bool,
}

impl TextMatch {
    pub fn proven(&self) -> bool {
        self.sampled_windows > 0 && self.matched_windows == self.sampled_windows
    }
}

/// Compare typed text with the document text a copy produced.
pub fn verify_typed_text(typed: &str, document: &str) -> TextMatch {
    let windows = verification_windows(typed);
    let canonical_document = canonical_alnum(document);
    let matched_windows = windows
        .iter()
        .filter(|window| canonical_document.contains(window.as_str()))
        .count();
    let normalized_document = normalize_text(document);
    let normalized_typed = normalize_text(typed).trim().to_owned();
    let exact_contains = !normalized_typed.is_empty()
        && normalized_document.contains(normalized_typed.lines().next().unwrap_or("").trim());
    TextMatch {
        matched_windows,
        sampled_windows: windows.len(),
        exact_contains,
    }
}

/// Stable non-cryptographic fingerprint for correlating two receipts without
/// exposing document contents.
pub fn text_hash(text: &str) -> String {
    let mut hash: u32 = 0x811c9dc5;
    for byte in text.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

async fn read_text() -> Option<String> {
    for (program, args) in read_candidates() {
        if let Some(text) = run(program, &args, None).await {
            return Some(text);
        }
    }
    None
}

async fn write_text(text: &str) -> bool {
    for (program, args) in write_candidates() {
        if run(program, &args, Some(text)).await.is_some() {
            return true;
        }
    }
    false
}

async fn run(program: &str, args: &[&str], stdin: Option<&str>) -> Option<String> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().ok()?;
    if let Some(input) = stdin {
        use tokio::io::AsyncWriteExt;
        let mut pipe = child.stdin.take()?;
        let bytes = input.as_bytes().to_vec();
        tokio::spawn(async move {
            let _ = pipe.write_all(&bytes).await;
            let _ = pipe.shutdown().await;
        });
    }
    let output = tokio::time::timeout(COMMAND_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    if output.stdout.len() > MAX_CLIPBOARD_BYTES {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "macos")]
fn read_candidates() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![("pbpaste", Vec::new())]
}

#[cfg(target_os = "macos")]
fn write_candidates() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![("pbcopy", Vec::new())]
}

#[cfg(target_os = "linux")]
fn read_candidates() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        ("wl-paste", vec!["--no-newline"]),
        ("xclip", vec!["-selection", "clipboard", "-o"]),
        ("xsel", vec!["--clipboard", "--output"]),
    ]
}

#[cfg(target_os = "linux")]
fn write_candidates() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        ("wl-copy", Vec::new()),
        ("xclip", vec!["-selection", "clipboard", "-i"]),
        ("xsel", vec!["--clipboard", "--input"]),
    ]
}

#[cfg(target_os = "windows")]
fn read_candidates() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![(
        "powershell",
        vec![
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; Get-Clipboard -Raw",
        ],
    )]
}

#[cfg(target_os = "windows")]
fn write_candidates() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![(
        "powershell",
        vec![
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Set-Clipboard -Value ([Console]::In.ReadToEnd())",
        ],
    )]
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn read_candidates() -> Vec<(&'static str, Vec<&'static str>)> {
    Vec::new()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn write_candidates() -> Vec<(&'static str, Vec<&'static str>)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_is_editor_tolerant() {
        assert_eq!(normalize_text("a\r\nb\u{00a0}c\u{200b}"), "a\nb c");
        assert_eq!(
            normalize_text("\u{201c}hi\u{201d} \u{2014} ok"),
            "\"hi\" - ok"
        );
        assert_eq!(canonical_alnum("Hello, World!"), "helloworld");
        assert_eq!(canonical_alnum("CAFÉ"), "café");
    }

    #[test]
    fn windows_are_evenly_sampled_and_bounded() {
        let typed = "Controlla typed this into a fresh Google Doc. Each character arrived as a trusted key event, paced like a person typing.";
        let windows = verification_windows(typed);
        let canonical = canonical_alnum(typed);
        assert_eq!(windows.len(), 3);
        assert!(windows.iter().all(|window| window.len() == 40));
        assert!(
            windows
                .iter()
                .all(|window| canonical.contains(window.as_str()))
        );
        assert!(verification_windows("too short").is_empty());
    }

    #[test]
    fn autocorrect_punctuation_and_caps_still_match() {
        let typed = "General partner co-founder; it is \"necessary\" -- and it's 2026.";
        let document = "General Partner co-founder; it is “necessary” — and it’s 2026.";
        let result = verify_typed_text(typed, document);
        assert!(result.proven(), "{result:?}");
        assert_eq!(result.sampled_windows, 3);
    }

    #[test]
    fn absent_text_is_not_proven() {
        let typed = "Controlla typed this into a fresh Google Doc, and every character was paced.";
        let document = "An unrelated pre-existing paragraph with nothing in common.";
        let result = verify_typed_text(typed, document);
        assert!(!result.proven());
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(text_hash("abc"), text_hash("abc"));
        assert_ne!(text_hash("abc"), text_hash("abd"));
    }
}
