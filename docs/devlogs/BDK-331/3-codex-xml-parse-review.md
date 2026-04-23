# Analysis: MR 242 note 290024 (`host_xml_*` still reparses on new paths)

**Reviewed:** 2026-04-13\
**Branch:** `jku/BDK-331/regression-testing`\
**HEAD:** `d614d80ef06d42ea63e0c767b25ed1d418cc2fa8`\
**Note:** <https://gitlab.ii.zone/bos/bmc-main/-/merge_requests/242#note_290024>

## Reviewer note

František Boháček clarified that the original XML performance concern was not "repeated lookups of the same field", but
"multiple lookups of different fields from the same document".

The requested behavior was:

- parse the XML document once
- read several fields from that parsed document
- discard it when the widget no longer needs it

In other words, caching `(doc_id, path)` results is only a partial improvement. It does not address the main cost when a
widget reads several distinct fields from one XML payload.

## Current implementation at review time

The current branch still behaves as follows:

1. `host_xml_parse` validates the XML and stores the raw string in `HostState::xml_docs`. Reference:
   `bmc-wasm-runtime/src/runtime_wasmi.rs:1741-1759`, `bmc-wasm-runtime/src/host_api.rs:418`
2. `host_xml_get_str` and `host_xml_get_f64` build a cache key `(doc_id, path)`. Reference:
   `bmc-wasm-runtime/src/runtime_wasmi.rs:1773-1779`, `1805-1811`
3. On a cache miss, both functions call `roxmltree::Document::parse(xml_str)`. Reference:
   `bmc-wasm-runtime/src/runtime_wasmi.rs:1782-1787`, `1814-1819`
4. Only the result for that exact path is memoized in `xml_query_cache: HashMap<(u32, String), Option<String>>`.
   Reference: `bmc-wasm-runtime/src/host_api.rs:421`, `bmc-wasm-runtime/src/runtime_wasmi.rs:1792-1795`, `1824-1827`
5. `host_xml_free` removes the raw document and prunes cached path results for that `doc_id`. Reference:
   `bmc-wasm-runtime/src/runtime_wasmi.rs:1836-1840`

This was introduced by commit `71aff2a4` (`wasm: Cache XML query results per document #BDK-331`).

## Assessment

The note is correct and still applies on `HEAD`.

### What improved

The current cache avoids reparsing if the widget asks for the exact same path more than once.

Example:

- `host_xml_get_str(doc, "//title")`
- `host_xml_get_str(doc, "//title")`

The second lookup will hit `xml_query_cache`.

### What did not improve

Different paths still reparse the whole XML document on every first lookup.

Example:

- `host_xml_get_str(doc, "//title")`
- `host_xml_get_str(doc, "//pubDate")`
- `host_xml_get_f64(doc, "//ttl")`

Today that performs three independent `roxmltree::Document::parse(...)` calls, not one.

That means the implementation still has the behavior the reviewer was worried about: a widget that extracts several
fields from one feed keeps paying full parse cost for each new field.

## Why the current fix stopped short

The likely reason is Rust ownership and lifetimes.

`roxmltree::Document<'_>` borrows from the source XML string. The current host state stores owned XML strings in
`xml_docs`, so directly storing parsed `Document` values next to those strings would require a self-referential data
structure, which is awkward in safe Rust.

That makes path-result memoization an easy local patch, but it is not equivalent to caching the parsed document.

## Better fix direction

For the current host API, the cleanest fix is probably not to store `roxmltree::Document` at all.

The supported query language is deliberately small:

- `//local_name`
- `//local_name/@attr`

Because of that, `host_xml_parse` can parse once and build an owned per-document index, for example:

- `HashMap<String, String>` for first text value by local name
- `HashMap<(String, String), String>` for first attribute value by `(local_name, attr_name)`

Then:

- `host_xml_parse` does the expensive parse exactly once
- `host_xml_get_str` and `host_xml_get_f64` become pure lookups
- `host_xml_free` just drops the indexed document entry

This matches the reviewer intent and avoids self-referential storage.

## Testing gap

I did not find XML regression tests in `bmc-wasm-runtime` that would catch this distinction.

That matters because the missing test is specifically:

- one parsed document
- several distinct field lookups
- assertion that parsing happens once, not once per path

Without that coverage, the current `(doc_id, path)` cache looks plausible while still missing the original review
target.

## Update after implementation

The follow-up plan from this document is now implemented on `jku/BDK-331/regression-testing`.

Current XML behavior on the branch:

- `host_xml_parse` parses once and stores an owned per-document XML index
- `host_xml_get_str` and `host_xml_get_f64` read directly from that index
- `host_xml_free` drops the indexed document entry
- the old `(doc_id, path)` result cache and raw XML document storage are gone

That means the original review note was correct for the earlier branch state, but the issue it identified is now
resolved.

## Conclusion

The note should be resolved once the MR includes the three XML follow-up commits from this branch.

Recommended MR response:

- acknowledge that the original note was correct
- point to the new parse-once document index design
- mention that getters now read from the stored index and no longer reparse XML
