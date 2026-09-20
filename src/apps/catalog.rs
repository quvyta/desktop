//! What the launcher lists: entries by category, the ones to install, and search.

use std::cmp::Ordering;

use super::Category;
use super::entry::Entry;
use super::sources::Environment;

/// The loaded entries, each with whether its program is installed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Catalog {
    items: Vec<(Entry, bool)>,
}

/// The installed entries of one category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group<'a> {
    /// The category.
    pub category: Category,
    /// Its entries, by name.
    pub entries: Vec<&'a Entry>,
}

/// How well a search matched, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rank {
    /// The name starts with the query.
    NamePrefix,
    /// A word of the name starts with the query.
    NameWord,
    /// A word of a keyword, the command or the comment starts with the query.
    Word,
    /// The query is inside the name.
    NameInside,
    /// The query is inside a keyword, the command or the comment.
    Inside,
    /// The letters of the query appear in the name in order, with others between them.
    Scattered,
}

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit<'a> {
    /// The entry.
    pub entry: &'a Entry,
    /// Whether its program is installed; an entry that is not belongs to the Installable
    /// section.
    pub installed: bool,
    /// How well it matched.
    pub rank: Rank,
    /// The characters of the name in the searched language that matched, as character indices,
    /// ascending; empty when the match was elsewhere.
    pub name_matches: Vec<usize>,
}

impl Catalog {
    /// A catalog of `entries`, with `installed` saying whether each one's program is there.
    #[must_use]
    pub fn new(entries: Vec<Entry>, installed: impl Fn(&Entry) -> bool) -> Self {
        Self {
            items: entries
                .into_iter()
                .map(|entry| {
                    let found = installed(&entry);
                    (entry, found)
                })
                .collect(),
        }
    }

    /// A catalog of `entries` checked against the programs of `env`.
    #[must_use]
    pub fn with_environment(entries: Vec<Entry>, env: &Environment) -> Self {
        Self::new(entries, |entry| env.is_installed(entry))
    }

    /// The entry with this id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.items.iter().find(|(entry, _)| entry.id == id).map(|(entry, _)| entry)
    }

    /// Whether the entry with this id is there and installed.
    #[must_use]
    pub fn is_installed(&self, id: &str) -> bool {
        self.items.iter().any(|(entry, installed)| entry.id == id && *installed)
    }

    /// Every installed entry, by name in `language`: the launcher's All list.
    #[must_use]
    pub fn installed(&self, language: &str) -> Vec<&Entry> {
        self.sorted(language, true)
    }

    /// The installed entries by category, in the launcher's order. Empty categories are left
    /// out; the family has its own group.
    #[must_use]
    pub fn groups(&self, language: &str) -> Vec<Group<'_>> {
        let all = self.installed(language);
        Category::ALL
            .into_iter()
            .map(|category| Group {
                category,
                entries: all.iter().copied().filter(|entry| entry.category == category).collect(),
            })
            .filter(|group| !group.entries.is_empty())
            .collect()
    }

    /// The entries whose program is not installed, by name: the Installable section. Each says
    /// in [`Entry::install`] how to install it, when it knows.
    #[must_use]
    pub fn installable(&self, language: &str) -> Vec<&Entry> {
        self.sorted(language, false)
    }

    /// The entries that match `query`, best first.
    ///
    /// The query is looked for, ignoring case, in the name in `language` and in English, the
    /// comment, the keywords and the command. A name that starts with it comes first, then a
    /// word that starts with it, then text that holds it, then a name that holds its letters in
    /// order. Within a rank installed entries come first, so Enter opens something that runs.
    /// An empty query lists every entry.
    #[must_use]
    pub fn search(&self, query: &str, language: &str) -> Vec<Hit<'_>> {
        let query = fold(query.trim());
        let mut hits: Vec<(Hit<'_>, usize)> = self
            .items
            .iter()
            .filter_map(|(entry, installed)| {
                let (rank, name_matches, spread) =
                    if query.is_empty() { (Rank::NamePrefix, Vec::new(), 0) } else { rank(entry, &query, language)? };
                Some((Hit { entry, installed: *installed, rank, name_matches }, spread))
            })
            .collect();
        hits.sort_by(|(a, a_spread), (b, b_spread)| {
            a.rank
                .cmp(&b.rank)
                .then(b.installed.cmp(&a.installed))
                .then(a_spread.cmp(b_spread))
                .then_with(|| by_name(a.entry, b.entry, language))
        });
        hits.into_iter().map(|(hit, _)| hit).collect()
    }

    fn sorted(&self, language: &str, installed: bool) -> Vec<&Entry> {
        let mut entries: Vec<&Entry> =
            self.items.iter().filter(|(_, found)| *found == installed).map(|(entry, _)| entry).collect();
        entries.sort_by(|a, b| by_name(a, b, language));
        entries
    }
}

/// Orders entries by name in `language` ignoring case, then by id so the order never depends on
/// the order files were read.
fn by_name(a: &Entry, b: &Entry, language: &str) -> Ordering {
    fold(a.name.get(language)).cmp(&fold(b.name.get(language))).then_with(|| a.id.cmp(&b.id))
}

/// Lower case, one character for one, so character positions stay those of the original. A
/// letter such as `İ`, whose lower case is two characters, keeps the first.
fn fold(text: &str) -> String {
    text.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

/// Where `query`, already folded, starts in folded `text`, as a character index: preferring a
/// word start. Returns the index and whether it is a word start.
fn find_in(text: &str, query: &str) -> Option<(usize, bool)> {
    let chars: Vec<char> = text.chars().collect();
    let wanted: Vec<char> = query.chars().collect();
    let starts: Vec<usize> = (0..chars.len()).filter(|start| chars[*start..].starts_with(&wanted)).collect();
    let word_start = |index: usize| index == 0 || !chars[index - 1].is_alphanumeric();
    starts
        .iter()
        .find(|start| word_start(**start))
        .map(|start| (*start, true))
        .or(starts.first().map(|start| (*start, false)))
}

/// The positions of the letters of `query` in `text`, in order, spaces in the query ignored,
/// taking the tightest run that starts earliest.
fn scattered(text: &str, query: &str) -> Option<Vec<usize>> {
    let chars: Vec<char> = text.chars().collect();
    let wanted: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
    let first = *wanted.first()?;
    let mut best: Option<Vec<usize>> = None;
    for start in (0..chars.len()).filter(|index| chars[*index] == first) {
        let mut positions = vec![start];
        for c in &wanted[1..] {
            let from = positions[positions.len() - 1] + 1;
            match (from..chars.len()).find(|index| chars[*index] == *c) {
                Some(index) => positions.push(index),
                None => return best,
            }
        }
        let spread = |run: &[usize]| run[run.len() - 1] - run[0];
        if best.as_ref().is_none_or(|kept| spread(&positions) < spread(kept)) {
            best = Some(positions);
        }
    }
    best
}

/// How `entry` matches `query`, the positions in its name, and how spread out a scattered match
/// is; `None` when it does not match.
fn rank(entry: &Entry, query: &str, language: &str) -> Option<(Rank, Vec<usize>, usize)> {
    let name = fold(entry.name.get(language));
    let length = query.chars().count();
    let run = |start: usize| (start..start + length).collect::<Vec<_>>();
    let english = fold(&entry.name.default);
    let names = [(name.as_str(), true), (english.as_str(), false)];

    let mut others: Vec<String> = entry.keywords.iter().map(|keyword| fold(keyword)).collect();
    if let Some(words) = match &entry.launch {
        super::Launch::Command(words) => Some(words),
        _ => None,
    } {
        others.extend(words.iter().map(|word| fold(word)));
        if let Some(program) = words.first().and_then(|program| program.rsplit('/').next()) {
            others.push(fold(program));
        }
    }
    if let Some(comment) = &entry.comment {
        others.push(fold(comment.get(language)));
        others.push(fold(&comment.default));
    }

    let mut best: Option<(Rank, Vec<usize>)> = None;
    let mut consider = |rank: Rank, positions: Vec<usize>| {
        if best.as_ref().is_none_or(|(kept, _)| rank < *kept) {
            best = Some((rank, positions));
        }
    };
    for (text, shown) in names {
        if let Some((start, word)) = find_in(text, query) {
            let rank = match (start, word) {
                (0, _) => Rank::NamePrefix,
                (_, true) => Rank::NameWord,
                (_, false) => Rank::NameInside,
            };
            consider(rank, if shown { run(start) } else { Vec::new() });
        }
    }
    for text in &others {
        if let Some((_, word)) = find_in(text, query) {
            consider(if word { Rank::Word } else { Rank::Inside }, Vec::new());
        }
    }
    if let Some((rank, positions)) = best {
        return Some((rank, positions, 0));
    }
    let positions = scattered(&name, query)?;
    let spread = positions[positions.len() - 1] - positions[0];
    Some((Rank::Scattered, positions, spread))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::apps::{Install, Launch, Localized, Screen, Source, WindowPrefs};

    fn entry(id: &str, name: Localized, launch: Launch, category: Category) -> Entry {
        Entry {
            id: id.to_owned(),
            name,
            comment: None,
            icon: None,
            launch,
            folder: None,
            env: Vec::new(),
            category,
            keywords: Vec::new(),
            single: false,
            close_on_exit: false,
            window: WindowPrefs::default(),
            install: Install::default(),
            try_exec: None,
            source: Source::User,
            file: None,
        }
    }

    fn command(id: &str, name: &str, category: Category) -> Entry {
        entry(id, Localized::plain(name), Launch::Command(vec![id.to_owned()]), category)
    }

    fn catalog() -> Catalog {
        let mut htop = command("htop", "htop", Category::System);
        htop.comment = Some(Localized {
            default: "Processes and system load".into(),
            translations: vec![("tr".into(), "Süreçler ve sistem yükü".into())],
        });
        let mut btop = command("btop", "btop", Category::System);
        btop.install.qpac = Some("btop".to_owned());
        let mut vim = command("vim", "Vim", Category::Development);
        vim.keywords = vec!["text".into(), "editor".into()];
        let settings = entry(
            "settings",
            Localized { default: "Settings".into(), translations: vec![("tr".into(), "Ayarlar".into())] },
            Launch::Screen(Screen::Settings),
            Category::System,
        );
        let mut qfocus = command("qfocus", "qfocus", Category::Family);
        qfocus.install.quvyta = Some("quvyta-focus".into());
        let entries = vec![
            htop,
            btop,
            vim,
            settings,
            qfocus,
            command("qcode", "qcode", Category::Family),
            command("thtml", "Thtml viewer", Category::Other),
            command("mc", "Midnight Commander", Category::Files),
            entry("logs", Localized::plain("Logs"), Launch::Open(PathBuf::from("/var/log")), Category::System),
        ];
        let missing = ["btop", "qfocus", "thtml"];
        Catalog::new(entries, |entry| !missing.contains(&entry.id.as_str()))
    }

    fn ids(entries: &[&Entry]) -> Vec<String> {
        entries.iter().map(|entry| entry.id.clone()).collect()
    }

    fn found(query: &str, language: &str) -> Vec<String> {
        catalog().search(query, language).iter().map(|hit| hit.entry.id.clone()).collect()
    }

    #[test]
    fn groups_follow_the_category_order_and_hold_installed_entries() {
        let catalog = catalog();
        let groups = catalog.groups("en");
        let shape: Vec<(Category, Vec<String>)> =
            groups.iter().map(|group| (group.category, ids(&group.entries))).collect();
        assert_eq!(
            shape,
            vec![
                (Category::System, vec!["htop".into(), "logs".into(), "settings".into()]),
                (Category::Development, vec!["vim".into()]),
                (Category::Files, vec!["mc".into()]),
                (Category::Family, vec!["qcode".into()]),
            ]
        );
    }

    #[test]
    fn names_sort_in_the_chosen_language() {
        let catalog = catalog();
        let system = |language| {
            catalog
                .groups(language)
                .into_iter()
                .find(|group| group.category == Category::System)
                .map(|g| ids(&g.entries))
        };
        assert_eq!(system("tr"), Some(vec!["settings".into(), "htop".into(), "logs".into()]));
    }

    #[test]
    fn missing_programs_go_to_installable_with_their_install_info() {
        let catalog = catalog();
        let installable = catalog.installable("en");
        assert_eq!(ids(&installable), vec!["btop", "qfocus", "thtml"]);
        assert_eq!(installable[0].install.qpac.as_deref(), Some("btop"));
        assert_eq!(installable[1].install.quvyta.as_deref(), Some("quvyta-focus"));
        assert!(!installable[2].install.is_known());
        assert!(!ids(&catalog.installed("en")).contains(&"btop".to_owned()));
        assert!(catalog.is_installed("htop") && !catalog.is_installed("btop") && !catalog.is_installed("gone"));
        assert_eq!(catalog.get("vim").map(|entry| entry.name.default.as_str()), Some("Vim"));
    }

    #[test]
    fn a_name_prefix_comes_first() {
        let hits = found("ht", "en");
        assert_eq!(hits[0], "htop");
        // `Midnight` and `thtml` hold `ht` inside; `thtml` is not installed.
        assert_eq!(hits, vec!["htop", "mc", "thtml"]);
        assert_eq!(catalog().search("ht", "en")[0].name_matches, vec![0, 1]);
    }

    #[test]
    fn word_starts_come_before_text_inside() {
        let catalog = catalog();
        let hits = catalog.search("comm", "en");
        assert_eq!(hits[0].entry.id, "mc");
        assert_eq!(hits[0].rank, Rank::NameWord);
        assert_eq!(hits[0].name_matches, vec![9, 10, 11, 12]);
        assert_eq!(found("edit", "en"), vec!["vim"]);
        assert_eq!(catalog.search("edit", "en")[0].rank, Rank::Word);
        // `Midnight Commander` holds the letters of `ditor` in order, so it follows last.
        assert_eq!(found("ditor", "en"), vec!["vim", "mc"]);
        assert_eq!(catalog.search("ditor", "en")[0].rank, Rank::Inside);
    }

    #[test]
    fn installed_entries_lead_within_a_rank() {
        let catalog = catalog();
        let hits = catalog.search("q", "en");
        assert_eq!(
            hits.iter().map(|hit| (hit.entry.id.as_str(), hit.installed)).collect::<Vec<_>>(),
            vec![("qcode", true), ("qfocus", false)]
        );
    }

    #[test]
    fn the_comment_and_the_localized_name_are_searched() {
        assert_eq!(found("process", "en"), vec!["htop"]);
        assert_eq!(found("süreç", "tr"), vec!["htop"]);
        assert_eq!(found("ayar", "tr"), vec!["settings"]);
        // The English name is found in every language.
        assert_eq!(found("sett", "tr"), vec!["settings"]);
        let catalog = catalog();
        // The positions are those of the name shown, so an English match in Turkish marks none.
        assert!(catalog.search("sett", "tr")[0].name_matches.is_empty());
    }

    #[test]
    fn case_is_ignored() {
        assert_eq!(found("VIM", "en"), vec!["vim"]);
        assert_eq!(found("midnight", "en"), vec!["mc"]);
    }

    #[test]
    fn scattered_letters_match_last() {
        let catalog = catalog();
        let hits = catalog.search("mdc", "en");
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].entry.id.as_str(), hits[0].rank), ("mc", Rank::Scattered));
        assert_eq!(hits[0].name_matches, vec![0, 2, 9]);
        assert!(catalog.search("zzz", "en").is_empty());
    }

    #[test]
    fn an_empty_query_lists_everything_installed_first() {
        let catalog = catalog();
        let hits = catalog.search("  ", "en");
        assert_eq!(hits.len(), 9);
        assert!(hits.iter().take(6).all(|hit| hit.installed));
        assert!(hits.iter().skip(6).all(|hit| !hit.installed));
    }

    #[test]
    fn folding_keeps_one_character_for_one() {
        assert_eq!(fold("İstanbul"), "istanbul");
        assert_eq!(fold("ÇAY").chars().count(), 3);
    }
}
