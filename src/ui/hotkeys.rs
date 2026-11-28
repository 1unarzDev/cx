//! Device letters (docs/design/concept-mission.md section 6).
//!
//! One namespace per device screen, by priority: its action keys (`c x a o n` start
//! agents and shells, `f` files, `e` links, `h j k l` move) are never device letters;
//! each device then takes the first free letter of its name, letter by letter (else
//! the first free letter of the alphabet; `g` last, so `gg` stays the picker's first
//! row); sessions get the letters left (`mission::slots`). The same letters open a
//! device from the picker and from every device's screen. A device keeps its letter
//! for as long as it stays in the list, so refreshes and new devices never move an
//! existing letter.
use std::collections::BTreeMap;

/// A device screen's action keys: never device letters.
pub(super) fn never(c: char) -> bool {
    matches!(c, 'e' | 'n' | 'f' | 'h' | 'j' | 'k' | 'l') || crate::model::is_agent_key(c)
}
/// Given out last: `gg` is the picker's first row while no device holds it.
const LATE: char = 'g';

/// Assign letters to `devices` (id, name), keeping every letter in `previous` whose device
/// is still listed. Devices past the 15 assignable letters get none.
pub(super) fn assign(
    devices: &[(&str, &str)],
    previous: &BTreeMap<String, char>,
) -> BTreeMap<String, char> {
    let mut out = BTreeMap::new();
    let mut taken: Vec<char> = Vec::new();
    for (id, _) in devices {
        if let Some(&c) = previous.get(*id) {
            if !taken.contains(&c) && !never(c) {
                out.insert(id.to_string(), c);
                taken.push(c);
            }
        }
    }
    let early = |c: char| c.is_ascii_lowercase() && !never(c) && c != LATE;
    for (id, name) in devices {
        if out.contains_key(*id) {
            continue;
        }
        let free = |c: &char, taken: &[char]| !taken.contains(c);
        // The name, letter by letter; then any early letter; then `g`.
        let pick = name
            .chars()
            .map(|c| c.to_ascii_lowercase())
            .find(|c| early(*c) && free(c, &taken))
            .or_else(|| ('a'..='z').find(|c| early(*c) && free(c, &taken)))
            .or_else(|| Some(LATE).filter(|c| free(c, &taken)));
        if let Some(c) = pick {
            out.insert(id.to_string(), c);
            taken.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(names: &[&str]) -> BTreeMap<String, char> {
        let list: Vec<(&str, &str)> = names.iter().map(|n| (*n, *n)).collect();
        assign(&list, &BTreeMap::new())
    }

    #[test]
    fn each_device_takes_the_first_free_letter_of_its_name() {
        // The user's fleet: `c` is claude, `r` is free now that retry is `R`.
        let m = run(&[
            "innovation",
            "tranquility",
            "blastoise-odroid",
            "creation",
            "blastoise-jetson",
        ]);
        assert_eq!(
            (m["innovation"], m["tranquility"], m["blastoise-odroid"]),
            ('i', 't', 'b')
        );
        assert_eq!((m["creation"], m["blastoise-jetson"]), ('r', 's'));
        // `l` moves, `a` is Antigravity's: laptop-guest is `p`.
        assert_eq!(run(&["laptop-guest"])["laptop-guest"], 'p');
        // A name of action keys only: the first free letter of the alphabet.
        assert_eq!(run(&["eco"])["eco"], 'b');
    }

    /// Two devices never share a letter, so an address reads without colour.
    #[test]
    fn device_letters_stay_unique_without_color() {
        let m = run(&["laptop", "lab-arm", "lab-arm-2", "field-rover"]);
        let mut letters: Vec<char> = m.values().copied().collect();
        letters.sort();
        letters.dedup();
        assert_eq!(letters.len(), 4, "{m:?}");
    }

    #[test]
    fn letters_stay_stable_when_devices_come_and_go() {
        let first = run(&["innovation", "isaac"]);
        assert_eq!((first["innovation"], first["isaac"]), ('i', 's'));
        // A new device listed first never takes an existing letter.
        let list = [
            ("ivy", "ivy"),
            ("innovation", "innovation"),
            ("isaac", "isaac"),
        ];
        let next = assign(&list, &first);
        assert_eq!((next["innovation"], next["isaac"]), ('i', 's'));
        assert_eq!(next["ivy"], 'v');
        // A removed device frees its letter; the rest keep theirs.
        let gone = assign(&[("isaac", "isaac")], &next);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone["isaac"], 's');
        // A letter now reserved is never kept from before (`f` and `h` were device
        // letters before they became files and motion on every device screen).
        let old = BTreeMap::from([("forge".to_string(), 'f'), ("hal".to_string(), 'h')]);
        let now = assign(&[("forge", "forge"), ("hal", "hal")], &old);
        assert_eq!((now["forge"], now["hal"]), ('r', 'b'));
    }

    #[test]
    fn g_only_last_and_never_an_action_key() {
        let names: Vec<String> = (0..26)
            .map(|i| format!("{}x", (b'a' + i) as char))
            .collect();
        let list: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), n.as_str())).collect();
        let m = assign(&list[..14], &BTreeMap::new());
        assert!(m.values().all(|c| !"ecxaonfhjklg".contains(*c)), "{m:?}");
        assert_eq!(assign(&list[..15], &BTreeMap::new())["ox"], 'g', "g last");
        for n in 1..=25 {
            let m = assign(&list[..n], &BTreeMap::new());
            assert!(m.values().all(|c| !never(*c)), "{n}: {m:?}");
            assert_eq!(m.len(), n.min(15), "15 assignable letters");
            let mut seen: Vec<char> = m.values().copied().collect();
            seen.sort();
            seen.dedup();
            assert_eq!(seen.len(), m.len(), "letters are unique");
        }
    }
}
