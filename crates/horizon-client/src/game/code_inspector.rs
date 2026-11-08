//! Horizon's code inspector: look at an entity, press I, and a page with the
//! Rust that runs it opens in a window of its own (the system's default
//! program for `.html`).
//!
//! The sources are the ones this build was compiled from (`include_str!`), so
//! the page always matches the running game. It carries no prose, only the
//! entity's id, file paths and code, so nothing on it needs translating.
use super::Game;
use std::{fs, io, path::Path, process::Command};

/// One embedded source file: its path in the repository and its text.
pub(super) struct SourceFile {
    path: &'static str,
    text: &'static str,
}

macro_rules! source {
    ($path:literal) => {
        SourceFile {
            path: concat!("crates/horizon-client/src/", $path),
            text: include_str!(concat!("../", $path)),
        }
    };
}

/// How much of a file the page shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    /// All of it: the file is the entity's own.
    Whole,
    /// The places that name the entity, with their surroundings; the rest
    /// behind a button. A file that names it nowhere is left out, unless no
    /// file does, and then the page shows them whole.
    Hits,
}

/// What every mob runs through, shown after the mob's own files: the places
/// that name the species (its table entry, behaviour, animation, drops,
/// breeding and spawning).
static CORE: &[SourceFile] = &[
    source!("mobs/species.rs"),
    source!("mobs/ai.rs"),
    source!("mobs/animal_brain.rs"),
    source!("mobs/anim.rs"),
    source!("mobs/drops.rs"),
    source!("mobs/farm.rs"),
    source!("mobs/spawning.rs"),
    source!("mobs/spawn_rules.rs"),
];
static BOATS: SourceFile = source!("game/boats.rs");
static MINECARTS: SourceFile = source!("game/minecarts.rs");
static ARMOR_STANDS: SourceFile = source!("game/armor_stands.rs");
static PAINTINGS: SourceFile = source!("game/paintings.rs");
static FRAMES: SourceFile = source!("game/frames.rs");
static END_CRYSTAL: SourceFile = source!("end_crystal.rs");

/// Mobs with code of their own: the entity ids and the files that run them.
static FAMILIES: &[(&[&str], &[SourceFile])] = &[
    (&["allay"], &[source!("mobs/allay.rs")]),
    (&["armadillo"], &[source!("mobs/armadillo.rs")]),
    (&["axolotl"], &[source!("mobs/axolotl.rs")]),
    (&["bat"], &[source!("mobs/bat.rs")]),
    (&["bee"], &[source!("mobs/bee.rs")]),
    (&["breeze"], &[source!("mobs/breeze.rs")]),
    (&["camel"], &[source!("mobs/camel.rs")]),
    (&["creaking"], &[source!("mobs/creaking.rs")]),
    (&["dolphin"], &[source!("mobs/dolphin.rs")]),
    (&["enderman"], &[source!("mobs/enderman.rs")]),
    (&["fox"], &[source!("mobs/fox.rs")]),
    (&["frog"], &[source!("mobs/frog.rs")]),
    (&["goat"], &[source!("mobs/goat.rs")]),
    (&["happy_ghast"], &[source!("mobs/happy_ghast.rs")]),
    (&["llama", "trader_llama"], &[source!("mobs/llama.rs")]),
    (&["panda"], &[source!("mobs/panda.rs")]),
    (&["parrot"], &[source!("mobs/parrot.rs")]),
    (&["phantom"], &[source!("mobs/phantom.rs")]),
    (&["polar_bear"], &[source!("mobs/polar_bear.rs")]),
    (&["rabbit"], &[source!("mobs/rabbit.rs")]),
    (&["ravager"], &[source!("mobs/ravager.rs")]),
    (&["shulker"], &[source!("mobs/shulker.rs")]),
    (&["sniffer"], &[source!("mobs/sniffer.rs")]),
    (&["strider"], &[source!("mobs/strider.rs")]),
    (&["turtle"], &[source!("mobs/turtle.rs")]),
    (&["vex"], &[source!("mobs/vex.rs")]),
    (&["warden"], &[source!("mobs/warden.rs"), source!("mobs/warden_brain.rs")]),
    (&["witch"], &[source!("mobs/witch.rs")]),
    (
        &["horse", "donkey", "mule", "skeleton_horse", "zombie_horse"],
        &[source!("mobs/equine.rs")],
    ),
    (
        &["pillager", "vindicator", "evoker", "illusioner"],
        &[source!("mobs/illager.rs")],
    ),
    (
        &["zombie", "husk", "drowned", "skeleton", "stray", "bogged"],
        &[source!("mobs/undead.rs")],
    ),
    (
        &["villager"],
        &[
            source!("mobs/villager.rs"),
            source!("mobs/villager_brain.rs"),
            source!("game/villagers.rs"),
        ],
    ),
    (&["wandering_trader"], &[source!("mobs/trader.rs")]),
    (
        &["zombie_villager"],
        &[source!("mobs/zombie_villager.rs"), source!("mobs/undead.rs")],
    ),
    (
        &["piglin", "piglin_brute", "zombified_piglin"],
        &[source!("mobs/piglin.rs"), source!("game/piglins.rs")],
    ),
    (
        &["iron_golem", "snow_golem"],
        &[source!("mobs/golem.rs"), source!("game/golems.rs")],
    ),
    (&["copper_golem"], &[source!("mobs/copper_golem.rs")]),
    (&["guardian", "elder_guardian"], &[source!("mobs/guardian.rs")]),
    (&["wither"], &[source!("mobs/wither.rs"), source!("game/wither.rs")]),
    (&["ender_dragon"], &[source!("game/dragon.rs")]),
];

/// The files that run the entity type `entity` (`minecraft:bat`, `oak_boat`),
/// the most specific first. A mob's own files (if it has any) come whole;
/// after them the places in the shared mob code that name it.
pub(super) fn files_for(entity: &str) -> Vec<(&'static SourceFile, Scope)> {
    let id = entity.strip_prefix("minecraft:").unwrap_or(entity);
    if id.ends_with("boat") || id.ends_with("raft") {
        return vec![(&BOATS, Scope::Hits)];
    }
    if id.contains("minecart") {
        return vec![(&MINECARTS, Scope::Hits)];
    }
    match id {
        "armor_stand" => return vec![(&ARMOR_STANDS, Scope::Hits)],
        "painting" => return vec![(&PAINTINGS, Scope::Hits)],
        "item_frame" | "glow_item_frame" => return vec![(&FRAMES, Scope::Hits)],
        "end_crystal" => return vec![(&END_CRYSTAL, Scope::Hits)],
        _ => {}
    }
    let own = FAMILIES
        .iter()
        .filter(|(ids, _)| ids.contains(&id))
        .flat_map(|(_, files)| files.iter())
        .map(|file| (file, Scope::Whole));
    let core = CORE.iter().map(|file| (file, Scope::Hits));
    own.chain(core).collect()
}

/// `iron_golem` as the enum variant `IronGolem`.
fn camel(id: &str) -> String {
    id.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect()
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `needle` stands in `line` as a whole word (`Zombie` is not in
/// `ZombieVillager`).
fn contains_word(line: &str, needle: &str) -> bool {
    !needle.is_empty()
        && line.match_indices(needle).any(|(at, _)| {
            let before = line[..at].chars().next_back();
            let after = line[at + needle.len()..].chars().next();
            !before.is_some_and(word_char) && !after.is_some_and(word_char)
        })
}

fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "else", "enum", "false", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
    "use", "where", "while",
];

/// One line as HTML: keywords and the trailing `//` comment get a class.
fn highlight(line: &str, out: &mut String) {
    let (code, comment) = line.split_at(line.find("//").unwrap_or(line.len()));
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        if KEYWORDS.contains(&word.as_str()) {
            out.push_str("<b>");
            out.push_str(word);
            out.push_str("</b>");
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in code.chars() {
        if word_char(c) {
            word.push(c);
        } else {
            flush(&mut word, out);
            escape(c.encode_utf8(&mut [0; 4]), out);
        }
    }
    flush(&mut word, out);
    if !comment.is_empty() {
        out.push_str("<i>");
        escape(comment, out);
        out.push_str("</i>");
    }
}

/// Lines of a file shown before and after each line that names the entity.
const BEFORE: usize = 3;
const AFTER: usize = 14;

/// The page for `entity`: each file with numbered lines, the lines that name
/// the entity marked and the first of them scrolled into view. A file of
/// [`Scope::Hits`] shows only those lines and their surroundings; the button
/// shows the rest.
pub(super) fn render_html(entity: &str, files: &[(&SourceFile, Scope)]) -> String {
    let id = entity.strip_prefix("minecraft:").unwrap_or(entity);
    let variant = camel(id);
    // Per file: its lines and which of them name the entity. The variant
    // (`Species::IronGolem`) first; the plain id only when the file never
    // spells the variant.
    let prepared: Vec<(&SourceFile, Scope, Vec<&str>, Vec<bool>)> = files
        .iter()
        .map(|&(file, scope)| {
            let lines: Vec<&str> = file.text.lines().collect();
            let by_variant = lines.iter().any(|l| contains_word(l, &variant));
            let needle = if by_variant { variant.as_str() } else { id };
            let hit = lines.iter().map(|l| contains_word(l, needle)).collect();
            (file, scope, lines, hit)
        })
        .collect();
    let any_hits = prepared.iter().any(|f| f.3.iter().any(|&h| h));
    let mut out = String::from("<!doctype html><meta charset=\"utf-8\"><title>");
    escape(id, &mut out);
    out.push_str(
        "</title><style>\
         body{background:#1e2327;color:#d8dee4;font:13px/1.45 ui-monospace,Consolas,monospace;margin:16px}\
         h1,h2{font-size:15px;margin:18px 0 6px}h2{color:#8fb7ff}\
         pre{margin:0;overflow-x:auto}\
         .l{display:block;white-space:pre}\
         .l::before{content:attr(data-n);display:inline-block;width:5ch;margin-right:2ch;\
         text-align:right;color:#6b7680}\
         .hit{background:#3b3a1e}\
         .x{display:none}body.all .x{display:block}\
         .gap{display:block;color:#6b7680;margin:6px 0}body.all .gap{display:none}\
         button{position:fixed;top:12px;right:16px;background:#2b3238;color:#d8dee4;\
         border:1px solid #4a545d;border-radius:4px;padding:2px 10px;cursor:pointer}\
         b{color:#f0795a;font-weight:normal}i{color:#7d8a96;font-style:normal}\
         </style><button onclick=\"document.body.classList.toggle('all')\">&hellip;</button><h1>",
    );
    escape(id, &mut out);
    out.push_str("</h1>");
    let mut anchored = false;
    for (file, scope, lines, hit) in &prepared {
        let n = lines.len();
        let named = hit.iter().any(|&h| h);
        let shown: Vec<bool> = if *scope == Scope::Whole || (!named && !any_hits) {
            vec![true; n]
        } else if named {
            let mut shown = vec![false; n];
            for (i, &is_hit) in hit.iter().enumerate() {
                if is_hit {
                    for s in &mut shown[i.saturating_sub(BEFORE)..(i + AFTER + 1).min(n)] {
                        *s = true;
                    }
                }
            }
            shown
        } else {
            // Another file names it, this one never does: nothing to show.
            continue;
        };
        out.push_str("<h2>");
        escape(file.path, &mut out);
        out.push_str("</h2><pre>");
        for (i, line) in lines.iter().enumerate() {
            if shown[i] && i > 0 && !shown[i - 1] {
                out.push_str("<span class=\"gap\">&hellip;</span>");
            }
            let first = hit[i] && !anchored;
            anchored |= hit[i];
            out.push_str("<span class=\"l");
            if hit[i] {
                out.push_str(" hit");
            }
            if !shown[i] {
                out.push_str(" x");
            }
            out.push('"');
            if first {
                out.push_str(" id=\"first\"");
            }
            out.push_str(&format!(" data-n=\"{}\">", i + 1));
            highlight(line, &mut out);
            out.push_str("</span>");
        }
        out.push_str("</pre>");
    }
    out.push_str(
        "<script>var e=document.getElementById('first');\
         if(e){e.scrollIntoView({block:'center'});}</script>",
    );
    out
}

/// Opens `path` with the system's default program, in a window of its own.
fn open_window(path: &Path) -> io::Result<()> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut child = Command::new(program).arg(path).spawn()?;
    // Reaped on a thread, so the launcher program never lingers as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Where the page of `entity` is written: a folder of this process in the
/// system's temporary directory.
fn page_path(entity: &str) -> io::Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("horizon-code-inspector-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let name: String = entity
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    Ok(dir.join(format!("{name}.html")))
}

impl Game<'_> {
    /// The I key: the Rust behind the entity in the crosshair (the type
    /// `debug_lines.rs` finds for the debug overlay), in a window of its own.
    /// Nothing happens when the crosshair is not on an entity.
    pub(super) fn inspect_crosshair_entity(&mut self) {
        let Some(entity) = self.crosshair_entity_type() else {
            return;
        };
        let html = render_html(&entity, &files_for(&entity));
        let opened = page_path(&entity)
            .and_then(|path| fs::write(&path, html).map(|()| path))
            .and_then(|path| open_window(&path));
        if let Err(e) = opened {
            log::warn!("Horizon: code inspector: {entity}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(entity: &str) -> Vec<(&'static str, Scope)> {
        files_for(entity)
            .iter()
            .map(|(f, scope)| (f.path, *scope))
            .collect()
    }

    const SRC: &str = "crates/horizon-client/src/";

    #[test]
    fn a_mob_gets_its_own_files_whole_then_the_shared_code_by_hits() {
        let bat = paths("minecraft:bat");
        assert_eq!(bat[0], (format!("{SRC}mobs/bat.rs").as_str(), Scope::Whole));
        assert!(bat[1..].iter().all(|(_, scope)| *scope == Scope::Hits));
        assert!(bat.iter().any(|(p, _)| p.ends_with("mobs/species.rs")));
        // No code of its own: the shared files alone, all by hits.
        let cow = paths("minecraft:cow");
        assert!(cow.iter().all(|(_, scope)| *scope == Scope::Hits));
        assert!(cow.iter().any(|(p, _)| p.ends_with("mobs/anim.rs")));
    }

    #[test]
    fn objects_get_their_file_and_no_mob_code() {
        assert_eq!(
            paths("minecraft:bamboo_chest_raft"),
            [(format!("{SRC}game/boats.rs").as_str(), Scope::Hits)]
        );
        assert_eq!(
            paths("minecraft:chest_minecart"),
            [(format!("{SRC}game/minecarts.rs").as_str(), Scope::Hits)]
        );
        assert_eq!(
            paths("minecraft:armor_stand"),
            [(format!("{SRC}game/armor_stands.rs").as_str(), Scope::Hits)]
        );
    }

    #[test]
    fn variants_are_whole_words() {
        assert_eq!(camel("iron_golem"), "IronGolem");
        assert!(contains_word("Species::Zombie => 1,", "Zombie"));
        assert!(!contains_word("Species::ZombieVillager => 1,", "Zombie"));
        assert!(!contains_word("my_zombie_count", "zombie"));
    }

    #[test]
    fn the_page_marks_the_entity_and_escapes_code() {
        let file = SourceFile {
            path: "demo.rs",
            text: "fn a() {}\nSpecies::Bat => x < y && z, // Bat\nSpecies::Bee => 1,\n",
        };
        let html = render_html("minecraft:bat", &[(&file, Scope::Whole)]);
        assert!(html.contains("<title>bat</title>"));
        assert_eq!(html.matches("class=\"l hit\"").count(), 1);
        assert!(html.contains("id=\"first\" data-n=\"2\""));
        assert!(html.contains("x &lt; y &amp;&amp; z"));
        assert!(html.contains("<b>fn</b>"));
        assert!(html.contains("<i>// Bat</i>"));
    }

    #[test]
    fn a_hits_file_shows_the_surroundings_and_hides_the_rest() {
        let text: String = (1..=60)
            .map(|n| {
                if n == 30 {
                    "Species::Hen => 1,\n".to_owned()
                } else {
                    format!("let line{n} = {n};\n")
                }
            })
            .collect();
        let hit_file = SourceFile {
            path: "shared.rs",
            text: Box::leak(text.into_boxed_str()),
        };
        let other = SourceFile {
            path: "other.rs",
            text: "let nothing = 1;\n",
        };
        let html = render_html(
            "minecraft:hen",
            &[(&hit_file, Scope::Hits), (&other, Scope::Hits)],
        );
        // Lines 27..=44 are the hit with 3 before and 14 after.
        assert!(html.contains("class=\"l\" data-n=\"27\""));
        assert!(html.contains("class=\"l\" data-n=\"44\""));
        assert!(html.contains("class=\"l x\" data-n=\"26\""));
        assert!(html.contains("class=\"l x\" data-n=\"45\""));
        assert_eq!(html.matches("class=\"gap\"").count(), 1);
        // Another file names it and this one never does: left out.
        assert!(!html.contains("other.rs"));
        // Nobody names it: every file shows whole.
        let none = render_html("minecraft:hen", &[(&other, Scope::Hits)]);
        assert!(none.contains("other.rs") && !none.contains("class=\"l x\""));
    }
}
