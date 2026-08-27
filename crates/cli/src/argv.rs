//! Argv normalisation for options whose values may start with `-`.
//!
//! triodion accepts block and timestamp ranges such as `-1000:latest`,
//! `-1000:-500` and `-7d:latest`. A bare parser reads those as flag clusters,
//! not values.
//!
//! The old `clap_cryo` fork solved this inside the parser with an
//! `is_number_range()` check. That fork pinned `anstream ^0.3`, which never
//! received the RUSTSEC-2024-0404 fix, so triodion is on upstream clap now.
//! Upstream offers two settings and neither is sufficient on its own:
//!
//! - `allow_negative_numbers` accepts a token only when the WHOLE token parses as a number. `-1000`
//!   passes; `-1000:latest` does not.
//! - `allow_hyphen_values` is greedy. With `num_args(1..)` it swallows the next real flag: `-b
//!   -1000:latest --align` puts `--align` into `blocks`.
//!
//! This module puts the check in front of the parser instead. It rewrites each
//! occurrence of a negative-tolerant option into attached form, one value per
//! token:
//!
//! ```text
//! -b -1000:latest 5000 --align   ->   --blocks=-1000:latest --blocks=5000 --align
//! ```
//!
//! Attached values are never re-read as flags, so the range survives and the
//! flag that follows it stays a flag. Everything else passes through byte for
//! byte, `--` included.
//!
//! # Parity with the parser
//!
//! Where this pass consumes values it must consume exactly what clap would.
//! clap's own multi-value consumption is greedy up to the next flag, so
//! `-b 1000 blocks` puts `blocks` in `blocks` and leaves the positional empty
//! — under the fork, under upstream, and here. Do not "fix" that asymmetry in
//! this module; it belongs to clap.

use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
};

/// A negative-tolerant option found at the head of a token, plus any short
/// flags clustered in front of it.
struct Occurrence<'a> {
    /// Shorts clustered before the value-taking one — `v` out of `-vb`. They
    /// take no values of their own, so they are re-emitted as their own token.
    leading: String,
    /// Canonical long name of the option the following values belong to.
    canonical: &'a str,
}

/// The spellings a command accepts, indexed for the two questions this pass
/// asks: is this token a negative-tolerant option, and is this character a
/// short flag at all?
struct Spellings {
    /// Long spelling (no `--`) of every negative-tolerant option, mapped to the
    /// canonical long name to emit for it.
    negative_longs: HashMap<String, String>,
    /// Short spelling of every negative-tolerant option, mapped the same way.
    negative_shorts: HashMap<char, String>,
    /// Short spelling of EVERY option, negative-tolerant or not. Used to tell a
    /// real cluster (`-vb`) from an attached value (`-bX`).
    all_shorts: HashSet<char>,
}

impl Spellings {
    /// Read the spellings off a built command, so this pass cannot drift out of
    /// sync with the `#[arg(..)]` attributes it serves.
    fn from_command(command: &clap::Command) -> Self {
        let mut negative_longs = HashMap::new();
        let mut negative_shorts = HashMap::new();
        let mut all_shorts = HashSet::new();

        for arg in command.get_arguments() {
            all_shorts.extend(arg.get_short());
            all_shorts.extend(arg.get_all_short_aliases().unwrap_or_default());

            if !arg.is_allow_negative_numbers_set() {
                continue;
            }
            // Attached form needs a long name to attach to. An option with only
            // a short spelling cannot be rewritten, so it is left alone.
            let Some(canonical) = arg.get_long() else { continue };

            for long in std::iter::once(canonical).chain(arg.get_all_aliases().unwrap_or_default())
            {
                negative_longs.insert(long.to_string(), canonical.to_string());
            }
            for short in
                arg.get_short().into_iter().chain(arg.get_all_short_aliases().unwrap_or_default())
            {
                negative_shorts.insert(short, canonical.to_string());
            }
        }

        Self { negative_longs, negative_shorts, all_shorts }
    }

    /// The occurrence `token` names, when it is a negative-tolerant option in
    /// its own right — `--blocks`, `-b`, or a cluster ending in one (`-vb`).
    ///
    /// Returns `None` for a token that already carries its value (`--blocks=X`,
    /// `-b=X`, `-bX`) and for a cluster whose leading characters are not short
    /// flags. Those are clap's to resolve, not this pass's.
    fn negative_option(&self, token: &str) -> Option<Occurrence<'_>> {
        if let Some(long) = token.strip_prefix("--") {
            let canonical = self.negative_longs.get(long)?;
            return Some(Occurrence { leading: String::new(), canonical });
        }

        let shorts = token.strip_prefix('-').filter(|shorts| !shorts.is_empty())?;

        // Only the LAST short in a cluster can take a value, so that is the one
        // whose values may need attaching. `-bX` and `-b=X` fail here because
        // their last character is not a value-taking short.
        let mut chars = shorts.chars();
        let last = chars.next_back()?;
        let canonical = self.negative_shorts.get(&last)?;

        let leading: String = chars.collect();
        leading
            .chars()
            .all(|c| self.all_shorts.contains(&c))
            .then_some(Occurrence { leading, canonical })
    }
}

/// Whether `token` is a value rather than a flag.
///
/// Three cases count as a value:
///
/// - No leading `-` at all.
/// - A lone `-`, which every CLI convention (and clap itself) reads as a value.
/// - A leading `-` followed by an ASCII digit. No short flag is a digit, so `-1000:latest`,
///   `-1000:-500` and `-7d:latest` cannot be anything else. This is the test the `clap_cryo` fork
///   applied inside the parser.
/// - A token that is not valid UTF-8. Every flag this CLI defines is ASCII, so such a token can
///   never be one of them; treating it as a value keeps it in the list it was written in rather
///   than silently demoting the rest of that list to positionals.
///
/// The digit test tracks the range grammar in `parse::blocks` and
/// `parse::timestamps`, both of which parse the characters after the leading
/// `-` as a number. It rejects the one exotic spelling those accept —
/// `-.5d:latest`, a leading decimal point with no integer part — which errors
/// rather than misparsing. Write `-0.5d:latest`.
fn is_value(token: &OsStr) -> bool {
    let Some(text) = token.to_str() else { return true };
    match text.strip_prefix('-') {
        None => true,
        Some(rest) => rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_digit()),
    }
}

/// Rewrite `argv` so hyphen-leading range values reach clap as values.
pub(crate) fn normalize<I, T>(command: &clap::Command, argv: I) -> Vec<OsString>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let spellings = Spellings::from_command(command);
    let argv: Vec<OsString> = argv.into_iter().map(Into::into).collect();
    let mut out = Vec::with_capacity(argv.len());
    let mut rest = argv.into_iter().peekable();

    // argv[0] is the program name, never a flag.
    out.extend(rest.next());

    while let Some(token) = rest.next() {
        let Some(text) = token.to_str() else {
            out.push(token);
            continue;
        };

        // Everything after `--` is a value by definition.
        if text == "--" {
            out.push(token);
            out.extend(rest);
            return out;
        }

        let Some(occurrence) = spellings.negative_option(text) else {
            out.push(token);
            continue;
        };
        let canonical = occurrence.canonical.to_string();
        if !occurrence.leading.is_empty() {
            out.push(OsString::from(format!("-{}", occurrence.leading)));
        }

        // Consume this occurrence's values, attaching each one.
        let mut attached = 0;
        while rest.peek().is_some_and(|next| is_value(next)) {
            let value = rest.next().expect("peeked");
            let mut arg = OsString::from(format!("--{canonical}="));
            arg.push(value);
            out.push(arg);
            attached += 1;
        }

        // No values followed. Emit the option unchanged and let clap decide: an
        // error for `--blocks`, which needs one, and an empty list for
        // `--timestamps`, which does not.
        if attached == 0 {
            out.push(OsString::from(format!("--{canonical}")));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Args;
    use clap::{CommandFactory, Parser as _};

    /// Normalise a shell-style command line into the tokens clap will see.
    fn norm(command_line: &str) -> Vec<String> {
        let command = Args::command();
        normalize(&command, command_line.split_whitespace())
            .into_iter()
            .map(|arg| arg.into_string().expect("ascii"))
            .collect()
    }

    /// Parse a shell-style command line the way the binary does.
    fn parse(command_line: &str) -> Args {
        Args::try_parse_from_cli(command_line.split_whitespace()).expect("parses")
    }

    #[test]
    fn negative_range_survives_as_a_value() {
        assert_eq!(
            norm("triodion blocks -b -1000:latest"),
            ["triodion", "blocks", "--blocks=-1000:latest"]
        );
    }

    #[test]
    fn a_flag_after_a_negative_range_stays_a_flag() {
        assert_eq!(
            norm("triodion blocks -b -1000:latest --align"),
            ["triodion", "blocks", "--blocks=-1000:latest", "--align"]
        );
    }

    #[test]
    fn every_value_of_an_occurrence_is_attached() {
        assert_eq!(
            norm("triodion blocks -b -1000:-500 5000 6000 --hex"),
            [
                "triodion",
                "blocks",
                "--blocks=-1000:-500",
                "--blocks=5000",
                "--blocks=6000",
                "--hex"
            ]
        );
    }

    #[test]
    fn long_spelling_and_aliases_are_rewritten_too() {
        assert_eq!(
            norm("triodion blocks --timestamps -10:100"),
            ["triodion", "blocks", "--timestamps=-10:100"]
        );
    }

    #[test]
    fn already_attached_values_are_left_alone() {
        for attached in ["--blocks=-1000:latest", "-b=-1000:latest", "-b-1000:latest"] {
            let command_line = format!("triodion blocks {attached}");
            assert_eq!(norm(&command_line), ["triodion", "blocks", attached]);
        }
    }

    #[test]
    fn options_that_do_not_take_ranges_are_untouched() {
        assert_eq!(
            norm("triodion blocks --contract 0xdead -o out"),
            ["triodion", "blocks", "--contract", "0xdead", "-o", "out"]
        );
    }

    #[test]
    fn a_valueless_occurrence_is_canonicalised_for_clap_to_report() {
        assert_eq!(
            norm("triodion blocks -b --align"),
            ["triodion", "blocks", "--blocks", "--align"]
        );
    }

    #[test]
    fn everything_after_a_double_dash_is_verbatim() {
        assert_eq!(
            norm("triodion blocks -- -b -1000:latest"),
            ["triodion", "blocks", "--", "-b", "-1000:latest"]
        );
    }

    #[test]
    fn a_short_cluster_keeps_its_leading_flags() {
        assert_eq!(
            norm("triodion blocks -vb -1000:latest"),
            ["triodion", "blocks", "-v", "--blocks=-1000:latest"]
        );
    }

    #[test]
    fn a_cluster_of_unknown_shorts_is_left_for_clap_to_report() {
        // `z` is not a flag, so `-zb` is not a cluster and this pass must not
        // pretend it is.
        assert_eq!(norm("triodion blocks -zb 1000"), ["triodion", "blocks", "-zb", "1000"]);
    }

    #[test]
    fn a_lone_dash_is_a_value() {
        assert_eq!(
            norm("triodion blocks -b 1000 -"),
            ["triodion", "blocks", "--blocks=1000", "--blocks=-"]
        );
    }

    #[test]
    fn a_negative_range_reaches_the_blocks_field() {
        let args = parse("triodion blocks -b -1000:latest --align");
        assert_eq!(args.blocks.as_deref(), Some(&["-1000:latest".to_string()][..]));
        assert!(args.align);
        assert_eq!(args.datatype, ["blocks"]);
    }

    #[test]
    fn mixed_values_all_reach_the_blocks_field_in_order() {
        let args = parse("triodion blocks -b -1000:-500 5000 6000 --hex");
        assert_eq!(
            args.blocks.as_deref(),
            Some(&["-1000:-500".to_string(), "5000".to_string(), "6000".to_string()][..])
        );
        assert!(args.hex);
        // The trailing values must not leak into the positional.
        assert_eq!(args.datatype, ["blocks"]);
    }

    #[test]
    fn two_occurrences_of_the_option_accumulate() {
        let args = parse("triodion blocks -b -1000:latest -b 5000");
        assert_eq!(
            args.blocks.as_deref(),
            Some(&["-1000:latest".to_string(), "5000".to_string()][..])
        );
    }

    #[test]
    fn a_negative_timestamp_range_reaches_its_field() {
        let args = parse("triodion blocks --timestamps -7d:latest --dry");
        assert_eq!(args.timestamps.as_deref(), Some(&["-7d:latest".to_string()][..]));
        assert!(args.dry);
    }

    #[test]
    fn a_clustered_negative_range_reaches_both_fields() {
        let args = parse("triodion blocks -vb -1000:latest");
        assert!(args.verbose);
        assert_eq!(args.blocks.as_deref(), Some(&["-1000:latest".to_string()][..]));
    }

    #[test]
    fn a_valueless_timestamps_option_still_parses() {
        // `--timestamps` is `num_args(0..)`, so no values is legal and must not
        // become an error just because this pass rewrote the token.
        let args = parse("triodion blocks --timestamps --hex");
        assert_eq!(args.timestamps.as_deref(), Some(&[][..]));
        assert!(args.hex);
    }

    #[test]
    fn a_valueless_blocks_option_is_still_an_error() {
        let err = Args::try_parse_from_cli(["triodion", "blocks", "-b", "--align"])
            .expect_err("blocks requires a value");
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
    }

    /// The pass must be a no-op for any argv with no hyphen-leading values.
    /// This is the guard against it over- or under-consuming relative to clap:
    /// for these inputs, going through it must land every field exactly where
    /// clap's own parser would.
    #[test]
    fn normalisation_matches_clap_when_no_value_starts_with_a_hyphen() {
        let command_lines = [
            "triodion blocks",
            "triodion blocks -b 1000 2000",
            "triodion blocks --blocks 1000 --align --hex",
            // clap consumes `blocks` greedily into the option, leaving the
            // positional empty. This pass must reproduce that, not correct it.
            "triodion -b 1000 blocks",
            "triodion blocks -b 15M:+1000 -o /tmp --csv",
            "triodion logs --topic0 0xddf2 --contract 0xdead -l 20",
            "triodion blocks -- --align",
        ];
        for command_line in command_lines {
            let raw: Vec<&str> = command_line.split_whitespace().collect();
            let direct = Args::try_parse_from(&raw).expect("clap parses");
            let normalised = Args::try_parse_from_cli(&raw).expect("normalised parses");
            assert_eq!(
                serde_json::to_value(&direct).unwrap(),
                serde_json::to_value(&normalised).unwrap(),
                "diverged on: {command_line}"
            );
        }
    }
}
