//! The single write path for every record kind.
//!
//! CLI, MCP, GUI and importers all funnel through here so that id
//! generation, path resolution, validation and the refuse-to-overwrite rule
//! exist in exactly one place.

use grain_id::GrainId;

/// Mint a time-ordered record id, for records whose id is their filename.
///
/// `GrainId::now_unix()` has decisecond resolution, so records made in the
/// same tenth of a second collide; the create functions re-mint rather than
/// overwrite. Time ordering is what makes a `{year}/{MM}/` listing readable.
pub fn new_id() -> String {
    GrainId::now_unix().to_string()
}

/// Mint a random record id, for records whose id is *not* their filename.
///
/// Accounts use this. Their id does no ordering work — `opened_at` carries
/// the meaningful date — and a chart of accounts is typically created in one
/// sitting, where time-ordered ids would all share a long leading prefix
/// exactly when there are the most to tell apart at a CLI prompt.
pub fn new_random_id() -> String {
    GrainId::random().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn both_generators_produce_seven_chars() {
        assert_eq!(new_id().chars().count(), 7);
        assert_eq!(new_random_id().chars().count(), 7);
    }

    #[test]
    fn random_ids_vary_in_their_leading_character() {
        // The whole reason accounts use random(): a burst of ids must not
        // share a prefix. 200 draws over a 32-char alphabet hitting only one
        // leading character would be about 1e-300 -- so this is a real signal,
        // not a flaky threshold.
        let leads: HashSet<char> = (0..200)
            .filter_map(|_| new_random_id().chars().next())
            .collect();
        assert!(leads.len() > 1, "random ids all began with the same character");
    }
}
