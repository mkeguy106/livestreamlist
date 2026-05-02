//! Spellcheck + autocorrect engine.
//!
//! Layout:
//! - `tokenize`  — pure tokenizer that classifies words / mentions / URLs /
//!   emote codes / all-caps shorthand. No external deps. Unit-testable.
//! - `personal`  — load/save the user's personal dictionary at
//!   `~/.config/livestreamlist/personal_dict.json`.
//! - `dict`      — enumerate installed hunspell dicts; bundled en_US fallback.
//! - (future)    — `SpellChecker` struct that wires these together against
//!   the hunspell crate. Lands in Task 5.

pub mod tokenize;
