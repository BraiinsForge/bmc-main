// Copyright (C) 2026  Braiins Systems s.r.o.

//! Tag-namespace conventions shared between the host and SDK.
//!
//! All host-side asset registries store entries under string tags shaped
//! like `<segment>:<segment>:…`, with `:` as the segment delimiter.
//! Eviction by "prefix" matches at segment boundaries, not at arbitrary
//! character positions:
//!
//! - `tag_matches_prefix("foo", "foo")` → `true` (exact match)
//! - `tag_matches_prefix("foo:bar", "foo")` → `true` (child segment)
//! - `tag_matches_prefix("foobar", "foo")` → `false` (no delimiter)
//!
//! Without this rule, a `Slot::new("foo")` would silently evict a sibling
//! `Slot::new("foobar")` on every `set()`.

/// Segment delimiter for host-side asset tags.
pub const TAG_DELIMITER: char = ':';

/// Whether `tag` is `prefix` itself or a descendant segment of it.
///
/// Returns `false` when `prefix` is empty — an empty prefix would otherwise
/// match every tag and silently sweep the registry.
#[must_use]
pub fn tag_matches_prefix(tag: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return false;
    }
    let Some(tail) = tag.strip_prefix(prefix) else {
        return false;
    };
    tail.is_empty() || tail.starts_with(TAG_DELIMITER)
}

#[cfg(test)]
mod tests {
    use super::tag_matches_prefix;

    #[test]
    fn exact_match() {
        assert!(tag_matches_prefix("foo", "foo"));
    }

    #[test]
    fn child_segment_matches() {
        assert!(tag_matches_prefix("foo:bar", "foo"));
        assert!(tag_matches_prefix("foo:bar:baz", "foo"));
        assert!(tag_matches_prefix("foo:bar:baz", "foo:bar"));
    }

    #[test]
    fn sibling_with_shared_text_does_not_match() {
        assert!(!tag_matches_prefix("foobar", "foo"));
        assert!(!tag_matches_prefix("foo_thumbnail", "foo"));
        assert!(!tag_matches_prefix("foo-bar", "foo"));
    }

    #[test]
    fn unrelated_tag_does_not_match() {
        assert!(!tag_matches_prefix("bar", "foo"));
        assert!(!tag_matches_prefix("bar:foo", "foo"));
    }

    #[test]
    fn empty_prefix_matches_nothing() {
        assert!(!tag_matches_prefix("foo", ""));
        assert!(!tag_matches_prefix("", ""));
        assert!(!tag_matches_prefix("foo:bar", ""));
    }

    #[test]
    fn empty_tag_only_matches_empty_prefix_which_is_disabled() {
        // The empty-prefix guard short-circuits before the empty tag is
        // ever considered, so empty tags never match anything.
        assert!(!tag_matches_prefix("", "foo"));
        assert!(!tag_matches_prefix("", ""));
    }

    #[test]
    fn prefix_longer_than_tag_does_not_match() {
        assert!(!tag_matches_prefix("foo", "foo:bar"));
        assert!(!tag_matches_prefix("fo", "foo"));
    }

    #[test]
    fn trailing_delimiter_on_prefix_does_not_double_up() {
        // `"foo:"` is treated as the literal prefix; only `"foo::bar"` (the
        // empty child segment) would match it, not `"foo:bar"`.
        assert!(tag_matches_prefix("foo:", "foo:"));
        assert!(!tag_matches_prefix("foo:bar", "foo:"));
        assert!(tag_matches_prefix("foo::bar", "foo:"));
    }

    #[test]
    fn trailing_delimiter_on_tag_does_not_break_match() {
        // A tag ending in `:` (oddly shaped but technically legal) still
        // matches its parent prefix.
        assert!(tag_matches_prefix("foo:", "foo"));
        assert!(tag_matches_prefix("foo:bar:", "foo"));
        assert!(tag_matches_prefix("foo:bar:", "foo:bar"));
    }

    #[test]
    fn numeric_guest_id_namespacing_does_not_alias() {
        // Verifies the actual GuestId-prefix use case: instance 1's tags
        // must not be reached by a sweep targeting instance 11.
        assert!(tag_matches_prefix("1:album_art", "1"));
        assert!(!tag_matches_prefix("11:album_art", "1"));
        assert!(!tag_matches_prefix("1:album_art", "11"));
        assert!(tag_matches_prefix("11:album_art", "11"));
    }

    #[test]
    fn double_colon_inside_segment_is_just_text() {
        // The `include_*!` macros emit tags like `<crate>::<file_stem>`.
        // The host treats `<crate>::<file_stem>` as a single segment of an
        // outer `<guest_id>:<crate>::<file_stem>` tag — the `::` is content
        // inside one segment, not two delimiters.
        let tag = "1:bmc_widget_media_control::album_art";
        assert!(tag_matches_prefix(tag, "1"));
        assert!(tag_matches_prefix(
            tag,
            "1:bmc_widget_media_control::album_art"
        ));
        // `1:bmc_widget_media_control` is not a real segment boundary —
        // the next char is `:`, so this looks like a parent segment.
        assert!(tag_matches_prefix(tag, "1:bmc_widget_media_control"));
        // But `1:bmc_widget_media_control:` is the literal text up to the
        // first colon of the `::`, with an empty child segment after.
        // That's not a prefix of the tag.
        assert!(!tag_matches_prefix(tag, "1:bmc_widget_media_control:other"));
    }
}
