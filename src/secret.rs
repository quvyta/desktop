//! Text that must not outlive its use, such as the password typed on the lock screen.

use std::fmt;

use zeroize::Zeroize;

/// A password held in one place, whose bytes are overwritten with zeros when it is cleared,
/// replaced or dropped, so the memory the allocator hands out next holds nothing of it.
///
/// It moves rather than copies: [`take`](Self::take) hands the text on and leaves this one empty,
/// and a `Secret` made [`from`](From::from) a `String` keeps that very buffer. A copy made with
/// `clone` is a `Secret` too and is overwritten in its turn. It never prints itself: `Debug` says
/// only that there is one.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// The text, to hand to whatever checks it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Hands the text on and leaves this one empty, without a copy.
    #[must_use]
    pub fn take(&mut self) -> Self {
        Self(std::mem::take(&mut self.0))
    }

    /// Overwrites the text and empties it.
    pub fn clear(&mut self) {
        drop(wipe(&mut self.0));
    }
}

impl From<String> for Secret {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.clear();
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// Empties `text` and overwrites every byte of its buffer with zeros, the spare room past the
/// text included: it may still hold a longer text of before. The buffer is handed back, so a test
/// can look at the very memory that held the text; it is freed when the caller drops it.
///
/// The zeros are written by `zeroize`, whose writes the compiler may not leave out as unused.
fn wipe(text: &mut String) -> Vec<u8> {
    let mut bytes = std::mem::take(text).into_bytes();
    // Within the capacity, so the buffer stays where it is.
    bytes.resize(bytes.capacity(), 0);
    bytes.as_mut_slice().zeroize();
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearing_overwrites_the_very_bytes_that_held_the_text() {
        let mut text = String::with_capacity(40);
        text.push_str("a longer password of before");
        text.truncate(6);
        let (at, capacity) = (text.as_ptr(), text.capacity());
        let bytes = wipe(&mut text);
        assert!(text.is_empty());
        assert_eq!(bytes.as_ptr(), at, "the same memory, not a copy of it");
        assert_eq!(bytes.len(), capacity, "the spare room past the text is covered too");
        assert!(bytes.iter().all(|&byte| byte == 0), "{:?}", String::from_utf8_lossy(&bytes));
    }

    #[test]
    fn taking_leaves_nothing_behind_and_printing_shows_nothing() {
        let mut secret = Secret::from("gizli".to_owned());
        let taken = secret.take();
        assert!(secret.is_empty());
        assert_eq!(taken.as_str(), "gizli");
        assert_eq!(format!("{taken:?}"), "Secret(..)");
        let mut cleared = taken.clone();
        cleared.clear();
        assert!(cleared.is_empty());
    }
}
