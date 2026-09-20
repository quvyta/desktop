//! Both parsers fed with many broken files: none may panic.
//!
//! The inputs come from a fixed seed, so a failure repeats on every run and every machine.

use std::path::Path;

use super::builtin::ENTRIES;
use super::entry::{Source, parse_entry};
use super::parse_desktop_file;

/// A small xorshift generator: enough to scatter mutations, with no dependency.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A number below `limit`, which must not be zero.
    fn below(&mut self, limit: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(limit).unwrap_or(u64::MAX)).unwrap_or(0)
    }

    fn byte(&mut self) -> u8 {
        self.next().to_le_bytes()[0]
    }
}

/// Bytes that tend to matter to the two formats.
const INTERESTING: &[u8] = b"=[]{}\"'\\%;#\n\r\t ,.~/0\xff\xc3\xe2";

const DESKTOP: &str = "[Desktop Entry]
Type=Application
Name=Htop
Name[tr]=Htop süreç görüntüleyici
Comment=Show System Processes
Exec=sh -c \"echo \\\\\"%% %f\\\\\"\" %U
TryExec=htop
Path=/srv
Terminal=true
Categories=ConsoleOnly;System;Monitor;
Keywords=system;process\\;task

[Desktop Action other]
Exec=other
";

const FULL_ENTRY: &str = r#"
name = { en = "Editor", tr = "Düzenleyici" }
comment = "Text"
command = ["vim", "-p"]
folder = "~/notes"
env = { EDITOR = "vim" }
category = "development"
keywords = ["text", "edit"]
single = true
close_on_exit = true
[window]
size = [100, 30]
maximized = true
[install]
qpac = "vim"
"#;

/// A variant of `seed`: bytes changed, inserted, removed, cut short or doubled.
fn mutate(random: &mut Random, seed: &[u8]) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    for _ in 0..=random.below(6) {
        let at = random.below(bytes.len() + 1);
        match random.below(7) {
            0 if at < bytes.len() => bytes[at] = random.byte(),
            1 if at < bytes.len() => bytes[at] ^= 1 << random.below(8),
            2 => bytes.insert(at, INTERESTING[random.below(INTERESTING.len())]),
            3 => bytes.insert(at, random.byte()),
            4 if at < bytes.len() => {
                bytes.remove(at);
            }
            5 => bytes.truncate(at),
            6 => {
                let end = (at + random.below(40)).min(bytes.len());
                let piece = bytes[at..end].to_vec();
                bytes.splice(at..at, piece);
            }
            _ => {}
        }
    }
    bytes
}

fn feed(bytes: &[u8]) {
    let _ = parse_entry("x", Path::new("x.toml"), bytes, Source::User, Some(Path::new("/home/ada")));
    let _ = parse_entry("x", Path::new("x.toml"), bytes, Source::Builtin, None);
    let _ = parse_desktop_file("x", Path::new("x.desktop"), bytes);
}

#[test]
fn mutated_files_never_panic() {
    let mut random = Random(0x9e37_79b9_7f4a_7c15);
    let mut seeds: Vec<&[u8]> = ENTRIES.iter().map(|(_, text)| text.as_bytes()).collect();
    seeds.push(DESKTOP.as_bytes());
    seeds.push(FULL_ENTRY.as_bytes());
    for round in 0..6000 {
        let seed = seeds[round % seeds.len()];
        feed(&mutate(&mut random, seed));
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut random = Random(0x2545_f491_4f6c_dd1d);
    for _ in 0..3000 {
        let length = random.below(200);
        let bytes: Vec<u8> = (0..length)
            .map(|_| if random.below(2) == 0 { INTERESTING[random.below(INTERESTING.len())] } else { random.byte() })
            .collect();
        feed(&bytes);
    }
}

#[test]
fn every_cut_of_a_valid_file_is_safe() {
    for seed in [DESKTOP.as_bytes(), FULL_ENTRY.as_bytes()] {
        for end in 0..=seed.len() {
            feed(&seed[..end]);
            feed(&seed[end..]);
        }
    }
}
