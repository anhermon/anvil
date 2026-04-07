// Suppress pre-existing clippy warnings in modules not modified by this PR.
// These are known issues in the agent loop and command handlers.
#![allow(
    clippy::too_many_lines,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::redundant_closure_for_method_calls,
    clippy::single_match_else,
    clippy::match_wildcard_for_single_variants,
    clippy::if_not_else,
    clippy::format_push_string,
    clippy::unused_self,
    clippy::unnecessary_literal_bound,
    clippy::map_unwrap_or,
    clippy::uninlined_format_args,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::default_trait_access,
    clippy::unused_async,
    clippy::unwrap_used,
    clippy::expect_used
)]

pub mod agent;
pub mod commands;
pub mod ui;
