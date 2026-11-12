//! Device letters for the hub (docs/design/concept-mission.md section 6).
//!
//! Hub keys are non-letters plus `e` (link) and `j k` (motion), so every other letter can
//! be a device. `h j k l` go last, only past 21 devices, so a vim hand never opens a
//! device by accident; `e` is never assigned. A device keeps its letter for as long as it
//! stays in the list, so refreshes and new devices never move an existing letter.
use std::collections::BTreeMap;

const NEVER: [char; 1] = ['e'];
const LATE: [char; 4] = ['h', 'j', 'k', 'l'];
/// Session slots inside a device: a device whose initial is taken prefers other letters.
const SLOTS: [char; 4] = ['a', 's', 'd', 'g'];

/// Assign letters to `devices` (id, name), keeping every letter in `previous` whose device
/// is still listed. Devices past the 25 assignable letters get none.
pub(super) fn assign(
    devices: &[(&str, &str)],
    previous: &BTreeMap<String, char>,
) -> BTreeMap<String, char> {
    let mut out = BTreeMap::new();
    let mut taken: Vec<char> = Vec::new();
    for (id, _) in devices {
        if let Some(&c) = previous.get(*id) {
            if !taken.contains(&c) {
                out.insert(id.to_string(), c);
                taken.push(c);
            }
        }
    }
    let early = |c: char| c.is_ascii_lowercase() && !NEVER.contains(&c) && !LATE.contains(&c);
    for (id, name) in devices {
        if out.contains_key(*id) {
            continue;
        }
        let letters: Vec<char> = name.chars().map(|c| c.to_ascii_lowercase()).collect();
        let free = |c: &char, taken: &[char]| !taken.contains(c);
        // The initial, then the name's other letters that are not slots, then any early
        // letter, then the late ones (the name's first).
        let pick = letters
            .first()
            .copied()
            .filter(|c| early(*c) && free(c, &taken))
            .or_else(|| {
                letters
                    .iter()
                    .copied()
                    .find(|c| early(*c) && !SLOTS.contains(c) && free(c, &taken))
            })
            .or_else(|| ('a'..='z').find(|c| early(*c) && free(c, &taken)))
            .or_else(|| {
                letters
                    .iter()
                    .copied()
                    .find(|c| LATE.contains(c) && free(c, &taken))
            })
            .or_else(|| LATE.iter().copied().find(|c| free(c, &taken)));
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
    fn initials_first_reserved_initials_take_a_later_non_slot_letter() {
        let m = run(&["tranquility", "innovation", "laptop-guest", "eve", "nas"]);
        assert_eq!(m["tranquility"], 't');
        assert_eq!(m["innovation"], 'i');
        // `l` is late and `a` a slot: laptop-guest is `p`, as in the design.
        assert_eq!(m["laptop-guest"], 'p');
        // `e` is never a device letter.
        assert_eq!(m["eve"], 'v');
        assert_eq!(m["nas"], 'n');
    }

    /// Two devices never share a letter, so an address reads without colour (the old
    /// two-letter chips repeated `[la]` for laptop and lab-arm).
    #[test]
    fn device_initials_stay_unique_without_color() {
        let m = run(&["laptop", "lab-arm", "lab-arm-2", "field-rover"]);
        let mut letters: Vec<char> = m.values().copied().collect();
        letters.sort();
        letters.dedup();
        assert_eq!(letters.len(), 4, "{m:?}");
    }

    #[test]
    fn letters_stay_stable_when_devices_come_and_go() {
        let first = run(&["innovation", "isaac"]);
        assert_eq!((first["innovation"], first["isaac"]), ('i', 'c'));
        // A new device listed first never takes an existing letter.
        let list = [
            ("ivy", "ivy"),
            ("innovation", "innovation"),
            ("isaac", "isaac"),
        ];
        let next = assign(&list, &first);
        assert_eq!((next["innovation"], next["isaac"]), ('i', 'c'));
        assert_eq!(next["ivy"], 'v');
        // A removed device frees its letter; the rest keep theirs.
        let gone = assign(&[("isaac", "isaac")], &next);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone["isaac"], 'c');
    }

    #[test]
    fn hjkl_only_past_21_devices_and_never_e() {
        let names: Vec<String> = (0..26)
            .map(|i| format!("{}x", (b'a' + i) as char))
            .collect();
        let list: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), n.as_str())).collect();
        let m = assign(&list[..21], &BTreeMap::new());
        assert!(m.values().all(|c| !"ehjkl".contains(*c)), "{m:?}");
        assert_eq!(m.len(), 21);
        let m = assign(&list, &BTreeMap::new());
        assert!(m.values().all(|c| *c != 'e'));
        assert_eq!(m.len(), 25, "25 assignable letters");
        let mut seen: Vec<char> = m.values().copied().collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 25, "letters are unique");
    }
}
