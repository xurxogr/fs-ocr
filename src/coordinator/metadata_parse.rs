//! Parsing of stockpile metadata text: client-language routing, the public-default name check, shard-name matching, and timestamp day/hour extraction.

use crate::enums::GameLanguage;

use super::pipeline::{TIME_MASK_CHINESE, TIME_MASK_CYRILLIC, TIME_MASK_LATIN};

/// Client UI language, inferred from the stockpile type (via the matching
/// [`TYPE_MASKS`] entry or type-template match).
///
/// Routes the timestamp decode mask to the right script and decides whether a
/// custom name is read via ocrs (Latin/Cyrillic) or the tesseract CLI (Chinese).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientLanguage {
    English,
    Chinese,
    Russian,
}

impl ClientLanguage {
    /// Collapse a fine-grained [`GameLanguage`] (from a type-template match) to
    /// the script routing used for the shard/timestamp block: the Latin locales
    /// all read as English, Russian as Cyrillic, Chinese as Han.
    pub(crate) fn from_game(lang: GameLanguage) -> Self {
        match lang {
            GameLanguage::Russian => ClientLanguage::Russian,
            GameLanguage::Chinese => ClientLanguage::Chinese,
            _ => ClientLanguage::English,
        }
    }

    /// The timestamp decode mask for this client's script.
    pub(crate) fn time_mask(self) -> &'static str {
        match self {
            ClientLanguage::English => TIME_MASK_LATIN,
            ClientLanguage::Russian => TIME_MASK_CYRILLIC,
            ClientLanguage::Chinese => TIME_MASK_CHINESE,
        }
    }
}

/// Known shard names. The shard-name crop is a single Latin word.
const KNOWN_SHARDS: [&str; 4] = ["ABLE", "CHARLIE", "LIVE", "Devbranch"];

/// Localized game labels for the default (non-custom) *public* stockpile name,
/// lowercased for comparison. A name region that reads as one of these is the
/// game's auto label for a public stockpile, not a user-chosen reserve name.
///
/// Latin scripts only (English/French share `public`): the recognizer's charset
/// no longer carries Chinese or Cyrillic, so it can never emit `公共` /
/// `Публичный` — those would be permanently-dead entries that falsely imply
/// support we've dropped.
const PUBLIC_DEFAULT_NAMES: &[&str] = &["public", "público", "öffentlich"];

/// Canonical label stored when a name matches a public default, regardless of
/// the client's language or the OCR noise that reached us.
pub(crate) const PUBLIC_CANONICAL_NAME: &str = "Public";

/// Whether an OCR'd name is the localized public default rather than a custom
/// reserve name.
///
/// Matched fuzzily so the geometric game font's `l`/`I` collision (e.g. `Public`
/// read as `PubIic`) and a dropped accent (`Público` → `Publico`) still resolve.
/// The `0.80` floor accepts ~one edit on the shortest entry (`public`, 6 chars:
/// one substitution scores `1 - 1/6 ≈ 0.83`) while staying tight enough that an
/// arbitrary custom name never collapses into the default.
pub(crate) fn is_public_default_name(name: &str) -> bool {
    const MIN_SIMILARITY: f64 = 0.80;

    let candidate = name.trim().to_lowercase();
    if candidate.is_empty() {
        return false;
    }
    PUBLIC_DEFAULT_NAMES
        .iter()
        .any(|&default| crate::text_utils::similarity(&candidate, default) >= MIN_SIMILARITY)
}

/// Match OCR'd shard text to the closest known shard name.
///
/// At low resolutions OCR garbles a character or two (e.g. "Devbranch" reads as
/// "Vevoranch" when the `D`'s stem blurs), so exact substring matching fails.
/// We instead pick the known shard with the highest character similarity and
/// accept it only above a confidence threshold — close enough to absorb a couple
/// of misread glyphs, strict enough to reject unrelated text.
pub(crate) fn match_shard_name(text: &str) -> Option<&'static str> {
    const MIN_SIMILARITY: f64 = 0.6;

    let candidate = text.trim().to_lowercase();
    if candidate.is_empty() {
        return None;
    }

    KNOWN_SHARDS
        .iter()
        .map(|&shard| {
            (
                shard,
                crate::text_utils::similarity(&candidate, &shard.to_lowercase()),
            )
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|&(_, similarity)| similarity >= MIN_SIMILARITY)
        .map(|(shard, _)| shard)
}

/// Extract day and hour from in-game timestamp text.
/// Expects format like "Day 1234, 2056 Hours" -> "1234, 20:56".
///
/// Every locale separates the day from the time with a comma — ASCII `,` for
/// Latin/Cyrillic clients, fullwidth `，` (U+FF0C) for Chinese — and it always
/// falls AFTER any thousands separator inside the day. So we split on the LAST
/// comma: digits to its left are the day, and the FIRST four digits to its right
/// are HHMM. Taking the first four (not the trailing four of the whole string)
/// means digits leaked from a misread trailing marker word — e.g. "Hours" read
/// as "Hour5" — can't shift the time window.
///
/// If OCR drops the separator entirely we fall back to "the last 4 digits are
/// HHMM, the rest is the day". Either way the result is rejected unless it parses
/// to a real clock time (HH 00-23, MM 00-59), so a misread digit yields no
/// timestamp rather than a confidently-wrong one.
pub(crate) fn extract_day_and_hour(text: &str) -> String {
    let (day, hhmm) = if let Some(sep) = text.rfind([',', '，']) {
        let day: String = text[..sep].chars().filter(|c| c.is_ascii_digit()).collect();
        let hhmm: String = text[sep..]
            .chars()
            .filter(|c| c.is_ascii_digit())
            .take(4)
            .collect();
        (day, hhmm)
    } else {
        // No separator: the last 4 digits are HHMM, the rest the day. Needs at
        // least the 4 time digits plus 1 day digit.
        let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() < 5 {
            return String::new();
        }
        let split = digits.len() - 4;
        (digits[..split].to_string(), digits[split..].to_string())
    };

    if day.is_empty() || hhmm.len() != 4 {
        return String::new();
    }

    // The in-game clock is HH 00-23, MM 00-59. A value outside that range means a
    // digit was misread, so reject the read instead of emitting a wrong time.
    let (hh, mm) = (&hhmm[..2], &hhmm[2..]);
    let in_range =
        hh.parse::<u32>().is_ok_and(|h| h < 24) && mm.parse::<u32>().is_ok_and(|m| m < 60);
    if !in_range {
        return String::new();
    }

    format!("{}, {}:{}", day, hh, mm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_shard_name_matches_exact_names() {
        assert_eq!(match_shard_name("ABLE"), Some("ABLE"));
        assert_eq!(match_shard_name("CHARLIE"), Some("CHARLIE"));
        assert_eq!(match_shard_name("LIVE"), Some("LIVE"));
        assert_eq!(match_shard_name("Devbranch"), Some("Devbranch"));
    }

    #[test]
    fn match_shard_name_tolerates_low_res_misreads() {
        // Observed eng OCR on a 1600x900 crop: the blurred `D` reads as `V`.
        assert_eq!(match_shard_name("Vevoranch"), Some("Devbranch"));
        assert_eq!(match_shard_name("DevBranch"), Some("Devbranch"));
    }

    #[test]
    fn match_shard_name_rejects_unrelated_and_empty() {
        assert_eq!(match_shard_name(""), None);
        assert_eq!(match_shard_name("   "), None);
        assert_eq!(match_shard_name("Public"), None);
    }

    #[test]
    fn public_default_matches_localized_labels() {
        // English/French, Portuguese, German defaults all read as public.
        assert!(is_public_default_name("Public"));
        assert!(is_public_default_name("público"));
        assert!(is_public_default_name("Öffentlich"));
    }

    #[test]
    fn public_default_tolerates_ocr_noise() {
        // The geometric-font l/I collision and a dropped accent must still match.
        assert!(is_public_default_name("PubIic")); // l read as capital I
        assert!(is_public_default_name("Publico")); // ó read without the accent
        assert!(is_public_default_name("  Public  ")); // surrounding whitespace
    }

    #[test]
    fn public_default_rejects_custom_and_empty_names() {
        assert!(!is_public_default_name("ABC DEF GH"));
        assert!(!is_public_default_name("ORCA-THR-C"));
        assert!(!is_public_default_name("Publish")); // 2 edits from "public"
        assert!(!is_public_default_name(""));
        assert!(!is_public_default_name("   "));
    }

    #[test]
    fn public_default_excludes_dropped_scripts() {
        // Chinese/Russian support is dropped, so the recognizer can never emit
        // these and they are deliberately absent from the dictionary.
        assert!(!is_public_default_name("公共"));
        assert!(!is_public_default_name("Публичный"));
    }

    #[test]
    fn extracts_day_and_hour_from_plain_text() {
        assert_eq!(extract_day_and_hour("Day 1234, 2056 Hours"), "1234, 20:56");
        assert_eq!(extract_day_and_hour("Day 702, 0304 Hours"), "702, 03:04");
    }

    #[test]
    fn extracts_day_and_hour_across_latin_locales() {
        // German/Portuguese use a period thousands separator, so the only comma
        // is the day/time split; French/English use a comma there. All read the
        // first 4 digits after the last comma as HHMM.
        assert_eq!(
            extract_day_and_hour("Tag 1.293, 1906 Stunden"),
            "1293, 19:06"
        );
        assert_eq!(
            extract_day_and_hour("Jour 1,293, 1906 Heures"),
            "1293, 19:06"
        );
        assert_eq!(extract_day_and_hour("Dia 1.293, 1906 Horas"), "1293, 19:06");
    }

    #[test]
    fn time_ignores_digits_leaked_from_a_misread_marker_word() {
        // Real misread: "Hours" recognized as "Hour5". Stripping all digits and
        // taking the trailing 4 used to yield "4181, 03:85"; splitting on the
        // comma and taking the FIRST 4 digits on the right reads it correctly.
        assert_eq!(extract_day_and_hour("Day 418, 1038 Hour5"), "418, 10:38");
    }

    #[test]
    fn extracts_day_and_hour_from_cjk_and_cyrillic_text() {
        // Real in-game formats: Chinese uses a fullwidth comma (`，`, U+FF0C)
        // and the 日/时/分 markers; the day carries a thousands separator. The
        // non-digit glyphs and separators are all stripped before parsing.
        assert_eq!(extract_day_and_hour("1,529日，08时51分"), "1529, 08:51");
        assert_eq!(
            extract_day_and_hour("1,529-й день, 08:51 часов"),
            "1529, 08:51"
        );
    }

    #[test]
    fn extracts_day_and_hour_when_separator_is_dropped() {
        // OCR sometimes drops the separator entirely (observed on a real
        // Chinese screenshot): "Day 1529, 0851" read as bare "15290851".
        assert_eq!(extract_day_and_hour("15290851"), "1529, 08:51");
        assert_eq!(extract_day_and_hour("7020304"), "702, 03:04");
    }

    #[test]
    fn day_is_unbounded_only_time_is_fixed_width() {
        // Whatever the day's digit count, the last 4 (no separator) are HH:MM and
        // the rest is the day — as long as the time is a valid clock value.
        assert_eq!(
            extract_day_and_hour("Day 12345, 2030 Hours"),
            "12345, 20:30"
        );
    }

    #[test]
    fn rejects_timestamp_noise() {
        assert_eq!(extract_day_and_hour(""), "");
        assert_eq!(extract_day_and_hour("0851"), ""); // 4 digits: can't tell day from time
    }

    #[test]
    fn rejects_impossible_clock_values() {
        // A misread digit that makes the time impossible (HH>23 or MM>59) is
        // rejected rather than emitted as a wrong-but-plausible timestamp.
        assert_eq!(extract_day_and_hour("Day 418, 1085 Hours"), ""); // MM 85
        assert_eq!(extract_day_and_hour("Day 418, 2538 Hours"), ""); // HH 25
        assert_eq!(extract_day_and_hour("123456789"), ""); // no comma -> 67:89, invalid
    }
}
