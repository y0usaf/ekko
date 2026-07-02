//! The stock session-naming policy: the session's tilde-abbreviated working
//! directory plus a random adjective-noun pair, e.g. "~/Dev/ekko blue-lemur".
//! The directory half is display sugar (grouping keys off the real cwd);
//! the word pair is what keeps names distinct within a project.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use ekko_ext::{Extension, ExtensionHost, ExtensionManifest, NamerInput, SessionNamerSpec};

pub struct NamingExtension;

impl Extension for NamingExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.naming".into(),
            name: "session naming".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "name sessions '<tilde-cwd> <adjective>-<noun>'".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        host.register_session_namer(SessionNamerSpec {
            name: "cwd-petname".into(),
            generate: Arc::new(generate_name),
        })
    }
}

/// Produce `"<tilde-cwd> <adjective>-<noun>"`, cycling through word pairs
/// from a time-seeded start until one isn't taken. If every pair under this
/// directory is somehow taken, the last candidate is returned and the host's
/// uniquifier suffixes it.
pub fn generate_name(input: &NamerInput) -> String {
    let dir = tilde_abbrev(&input.cwd);
    let total = ADJECTIVES.len() * NOUNS.len();
    let mut index = seed() % total;
    let mut candidate = String::new();
    for _ in 0..total {
        candidate = format!(
            "{dir} {}-{}",
            ADJECTIVES[index % ADJECTIVES.len()],
            NOUNS[index / ADJECTIVES.len()]
        );
        if !input.taken.contains(&candidate) {
            return candidate;
        }
        index = (index + 1) % total;
    }
    candidate
}

/// Longest directory part we emit; beyond it the name would crowd out the
/// word pair (and risk the host's encoded-filename cap truncating it).
const MAX_DIR_LEN: usize = 40;

/// Abbreviate `$HOME` to `~`, falling back to the full path. Directories
/// too deep to read shrink to `…/{parent}/{leaf}`.
fn tilde_abbrev(path: &Path) -> String {
    let full = if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        if path == home {
            return "~".to_string();
        }
        match path.strip_prefix(&home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    } else {
        path.display().to_string()
    };
    if full.chars().count() <= MAX_DIR_LEN {
        return full;
    }
    let mut components = full.rsplit('/');
    let leaf = components.next().unwrap_or(&full);
    match components.next().filter(|parent| !parent.is_empty()) {
        Some(parent) => format!("…/{parent}/{leaf}"),
        None => format!("…{leaf}"),
    }
}

fn seed() -> usize {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0)
}

// Word lists borrowed from zellij (zellij-utils/src/sessions.rs, MIT).
const ADJECTIVES: &[&str] = &[
    "adamant",
    "adept",
    "adventurous",
    "arcadian",
    "auspicious",
    "awesome",
    "blossoming",
    "brave",
    "charming",
    "chatty",
    "circular",
    "considerate",
    "cubic",
    "curious",
    "delighted",
    "didactic",
    "diligent",
    "effulgent",
    "erudite",
    "excellent",
    "exquisite",
    "fabulous",
    "fascinating",
    "friendly",
    "glowing",
    "gracious",
    "gregarious",
    "hopeful",
    "implacable",
    "inventive",
    "joyous",
    "judicious",
    "jumping",
    "kind",
    "likable",
    "loyal",
    "lucky",
    "marvellous",
    "mellifluous",
    "nautical",
    "oblong",
    "outstanding",
    "polished",
    "polite",
    "profound",
    "quadratic",
    "quiet",
    "rectangular",
    "remarkable",
    "rusty",
    "sensible",
    "sincere",
    "sparkling",
    "splendid",
    "stellar",
    "tenacious",
    "tremendous",
    "triangular",
    "undulating",
    "unflappable",
    "unique",
    "verdant",
    "vitreous",
    "wise",
    "zippy",
];

const NOUNS: &[&str] = &[
    "aardvark",
    "accordion",
    "apple",
    "apricot",
    "bee",
    "brachiosaur",
    "cactus",
    "capsicum",
    "clarinet",
    "cowbell",
    "crab",
    "cuckoo",
    "cymbal",
    "diplodocus",
    "donkey",
    "drum",
    "duck",
    "echidna",
    "elephant",
    "foxglove",
    "galaxy",
    "glockenspiel",
    "goose",
    "hill",
    "horse",
    "iguanadon",
    "jellyfish",
    "kangaroo",
    "lake",
    "lemon",
    "lemur",
    "magpie",
    "megalodon",
    "mountain",
    "mouse",
    "muskrat",
    "newt",
    "oboe",
    "ocelot",
    "orange",
    "panda",
    "peach",
    "pepper",
    "petunia",
    "pheasant",
    "piano",
    "pigeon",
    "platypus",
    "quasar",
    "rhinoceros",
    "river",
    "rustacean",
    "salamander",
    "sitar",
    "stegosaurus",
    "tambourine",
    "tiger",
    "tomato",
    "triceratops",
    "ukulele",
    "viola",
    "weasel",
    "xylophone",
    "yak",
    "zebra",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn input(cwd: &str, taken: &[&str]) -> NamerInput {
        NamerInput {
            cwd: PathBuf::from(cwd),
            taken: taken.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn name_is_cwd_then_word_pair() {
        let name = generate_name(&input("/opt/proj", &[]));
        let (dir, pair) = name.rsplit_once(' ').expect("dir and pair");
        assert_eq!(dir, "/opt/proj");
        let (adjective, noun) = pair.split_once('-').expect("adjective-noun");
        assert!(
            ADJECTIVES.contains(&adjective),
            "unknown adjective '{adjective}'"
        );
        assert!(NOUNS.contains(&noun), "unknown noun '{noun}'");
    }

    #[test]
    fn avoids_taken_names() {
        // Take every pair except one; the namer must land on the free one.
        let free = "/p sensible-lemur".to_string();
        let all: Vec<String> = ADJECTIVES
            .iter()
            .flat_map(|a| NOUNS.iter().map(move |n| format!("/p {a}-{n}")))
            .filter(|name| name != &free)
            .collect();
        let taken: Vec<&str> = all.iter().map(String::as_str).collect();
        assert_eq!(generate_name(&input("/p", &taken)), free);
    }

    #[test]
    fn tilde_abbreviation_applies_to_home_prefix() {
        let home = std::env::var("HOME").unwrap_or_default();
        if home.is_empty() {
            return;
        }
        let name = generate_name(&input(&format!("{home}/Dev/ekko"), &[]));
        assert!(name.starts_with("~/Dev/ekko "), "got '{name}'");
        let at_home = generate_name(&input(&home, &[]));
        assert!(at_home.starts_with("~ "), "got '{at_home}'");
    }
}
