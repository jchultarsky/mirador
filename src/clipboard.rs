//! Putting text on the clipboard, by asking the terminal to do it.
//!
//! OSC 52 is an escape sequence: the program writes the text, base64-encoded,
//! and the terminal decides whether to honour it. That is the whole mechanism.
//! No dependency, no platform call, no window-system connection — the same
//! answer this project gave to playing a sound, where the chime is a raw `\x07`
//! rather than an audio library.
//!
//! **It is best-effort, and the honesty about that matters more than the code.**
//! The sequence is write-only: nothing comes back, so mirador cannot know
//! whether the clipboard was actually set. Many terminals disable OSC 52 by
//! default because a program that can write the clipboard can also overwrite
//! whatever was on it, and tmux needs `set-clipboard on` to pass it through.
//!
//! So a caller must not report "copied". It can report that it asked, which is
//! true either way, and let the reader discover the rest by pasting.

use std::io::Write;

/// Ask the terminal to put `text` on the system clipboard.
///
/// Errors only when the write itself fails. A terminal that ignores the
/// sequence is indistinguishable from one that honoured it.
pub fn copy(text: &str) -> std::io::Result<()> {
    // `c` is the system clipboard. The alternative, `p`, is the X primary
    // selection, which is a different thing on one platform and nothing at all
    // on the others.
    let payload = format!("\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let mut out = std::io::stdout();
    out.write_all(payload.as_bytes())?;
    out.flush()
}

/// Standard base64 with padding, which is what OSC 52 wants.
///
/// Hand-rolled rather than pulled in: this is the only base64 in the program,
/// and a dependency for twenty lines is the trade `quick-xml` was chosen over
/// `rss` to avoid.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        // Pad the group to three bytes, remembering how many were real: the
        // padding decides how many `=` go on the end.
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let triple = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let indices = [
            (triple >> 18) & 0x3f,
            (triple >> 12) & 0x3f,
            (triple >> 6) & 0x3f,
            triple & 0x3f,
        ];
        // `chunk.len()` real bytes produce `len + 1` meaningful characters.
        for (position, index) in indices.iter().enumerate() {
            if position <= chunk.len() {
                out.push(ALPHABET[*index as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RFC 4648 vectors, which exist precisely because the padding cases
    /// are where a hand-rolled encoder goes wrong.
    #[test]
    fn base64_matches_the_published_test_vectors() {
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), want, "encoding {input:?}");
        }
    }

    /// A URL is what this exists for, and it has to survive byte for byte.
    #[test]
    fn a_real_link_round_trips_through_the_encoder() {
        let link = "https://arstechnica.com/science/2026/07/if-a-quantum-computer/";
        let encoded = base64(link.as_bytes());
        assert_eq!(
            decode(&encoded),
            link.as_bytes(),
            "the link did not survive encoding"
        );
    }

    /// Non-ASCII goes through as UTF-8 bytes rather than characters. A link can
    /// carry them, and a title certainly can if this is ever reused.
    #[test]
    fn multi_byte_text_encodes_as_bytes() {
        for text in ["日本語", "café", "🌞", "a🌞b"] {
            assert_eq!(decode(&base64(text.as_bytes())), text.as_bytes(), "{text}");
        }
    }

    /// The sequence is what the terminal actually parses, so its shape is part
    /// of the contract: introducer, selection, payload, terminator.
    #[test]
    fn the_escape_sequence_is_shaped_the_way_terminals_expect() {
        let payload = format!("\x1b]52;c;{}\x07", base64(b"hi"));
        assert!(payload.starts_with("\x1b]52;c;"), "OSC 52 introducer");
        assert!(payload.ends_with('\x07'), "BEL terminator");
        assert_eq!(&payload[7..payload.len() - 1], "aGk=");
    }

    /// Decode, for the tests only. Not `pub`: nothing in mirador reads a
    /// clipboard, and an unused decoder is a thing to keep working for nobody.
    fn decode(text: &str) -> Vec<u8> {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut bits = Vec::new();
        for c in text.bytes().filter(|c| *c != b'=') {
            let index = ALPHABET
                .iter()
                .position(|a| *a == c)
                .expect("valid base64 alphabet");
            for shift in (0..6).rev() {
                bits.push((index >> shift) & 1);
            }
        }
        bits.as_chunks::<8>()
            .0
            .iter()
            .map(|byte| byte.iter().fold(0u8, |acc, bit| (acc << 1) | *bit as u8))
            .collect()
    }
}
