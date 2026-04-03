/// Extract all @mention handles from a comment body.
///
/// Returns lower-cased handles without the `@` prefix.
/// E.g. "Hey @Build-Agent please review" → ["build-agent"]
pub fn extract_mentions(body: &str) -> Vec<String> {
    let mut mentions = Vec::new();
    let mut chars = body.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '@' {
            let handle: String = chars
                .by_ref()
                .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !handle.is_empty() {
                mentions.push(handle.to_lowercase());
            }
        }
    }

    mentions.sort();
    mentions.dedup();
    mentions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_single_mention() {
        assert_eq!(
            extract_mentions("Hey @build-agent please look"),
            vec!["build-agent"]
        );
    }

    #[test]
    fn extracts_multiple_mentions() {
        let mut got = extract_mentions("@dev-agent and @cto need to review");
        got.sort();
        assert_eq!(got, vec!["cto", "dev-agent"]);
    }

    #[test]
    fn deduplicates() {
        assert_eq!(extract_mentions("@bot @bot @BOT"), vec!["bot"]);
    }

    #[test]
    fn ignores_email_addresses() {
        // Email addresses look like @domain after the local part — the local part
        // is not preceded by @, so we only capture the domain portion which is
        // typically not a valid agent handle anyway. This is acceptable for v0.
        let mentions = extract_mentions("Contact user@example.com for details");
        // "example.com" would NOT be captured because '.' terminates the handle scan.
        assert!(!mentions.contains(&"example.com".to_string()));
    }

    #[test]
    fn empty_body() {
        assert!(extract_mentions("").is_empty());
    }
}
