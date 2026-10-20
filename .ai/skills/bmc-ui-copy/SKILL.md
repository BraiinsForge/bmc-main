---
name: bmc-ui-copy
description: Use when adding, changing or moving any user-facing text — on the device, in the web app, or in a data crate such as a schema or a wire protocol. Covers where a message is declared (`format_message!` or `formatMessage` at the use site, a lazy `Message` where there is no formatter), what the translator's description must say, the ICU MessageFormat traps the gates refuse, how values and numbers enter a message, and the extract → translate → verify round trip. Triggers on "add a label", "reword", "translate", "new message", "this string is English", "`default_message`", "`defaultMessage`", or any literal a person will read on the device or the web.
---

# User-facing copy

Every sentence a person reads is an ICU MessageFormat message in English, declared in code, extracted into a translator
file and compiled into a catalog at build time. English is the source; there is no `en` catalog. The rules below are
what the gates enforce and what they cannot.

## Where a message is declared

**A Rust crate that holds a formatter words its copy inline**, with `bmc_intl::format_message!` where the text is used:

```rust
let title = format_message!(
    intl,
    default_message: "Connect to {ssid}",
    description: "Setup screen title; ssid is the Wi-Fi network's name",
    values: { ssid: network.ssid() },
);
```

The pattern, its description and its values sit together. Always give values in the `values: { name: value }` form: only
that form checks the placeholders against the English at compile time, and `no-unchecked-message-values` refuses a
`values: some_map` that would skip the check. A `format_message!` with one caller is written at that site; a function
exists only for shared words or for an enum mapping (one sentence per variant). The `no-message-descriptor` rule refuses
a hoisted `message_descriptor!`, and `no-format-message-in-macro` refuses one nested in another macro (`vec![…]`,
`assert!(…)`), which the extractor cannot see into: bind it to a local first.

Hold the formatter, never build one per render: `bmc_intl::Localized` keeps one for the surface's language and says when
it changes; code answering many callers takes `bmc_intl::shared_formatter`.

**A crate with no formatter — a schema, a wire protocol — declares its copy as a lazy message** with
`bmc_intl_message::message_descriptor!`, next to the data it names, and the edge that knows the language words it with
`bmc_intl::format_lazy`:

```rust
// in the schema crate, no formatter anywhere near
pub name: Message = message_descriptor!(
    default_message: "Braiins Pool",
    description: "Credential type name; the product name",
);

// at the edge, which knows the language
name: format_lazy(intl, &t.name),
```

A lazy message carries **no arguments**: values can only be checked where they exist, so text that needs values is a
`format_message!` where the values are, taking the lazy label as one of them. `Message` is neither `Display` nor
`Into<String>`; it cannot become a string without a language. Adopting one means adding the crate to the
`no-message-descriptor` rule's `ignores`.

**The web** calls `formatMessage({ defaultMessage, description }, values)` or `<FormattedMessage>` where the text is
rendered. `no-define-messages-ts` refuses a `defineMessages` table. Copy that belongs to data (a step list, a table of
options) stays beside the data as a function taking `formatMessage`; keep the parameter named `formatMessage`, the call
the extractor collects.

## The description

It is for the translator, who sees the English and nothing else. Say where the text appears, what it is (a title, a
button, a one-line status), what any placeholder holds and in what form (`ago is a relative time such as 7m ago`), and
when a word is a product name. Short words that translate differently by context ("Off", "None", "Time") need one. Two
messages with the same English and description are one entry. The id is a hash of the English and the description, so
changing either changes the id; `intl extract` recovers the translation by identical English when exactly one prior
entry has it.

## Values and numbers

Numbers, dates and durations are formatted with the device's preference-aware helpers first and enter the message as
values; the ICU `{n, number}` and `{d, date}` styles are not used. The Rust runtime prints a plain integer argument
ungrouped (`{millis}` with `1234` is `1234`), and so does the web for a simple argument.

## ICU traps the gates refuse

- `#` in a plural or selectordinal branch is the count through the locale's number formatter, grouped ("1,234"). Write
  `{count}`. `no-hash-in-plural` (Rust and web) and `intl verify` over translations.
- An ASCII apostrophe before `{` quotes the argument out of the message: `'{state}'` renders the braces. Reword or use a
  typographic quote. `no-quoted-argument`.
- A newline, tab or doubled space in a default message is collapsed before hashing. A deliberate line break is an
  argument the translation places: `{br}` with `"\n"`. `no-whitespace-run-in-message`.
- A missing or extra argument in a `format_message!` fails to compile; in a translation, `intl verify` refuses it.
- Broken ICU in a translator file fails `intl verify` naming the file and the entry; a host one fails `bmc-intl`'s build
  too.

## The round trip

1. Declare or change the message in code.
2. `just intl::extract [host|web]` rewrites every `lang/<key>.json` of the surface: new entries with
   `"translation": null`, gone entries dropped, `source` and `description` refreshed. Ids of untouched messages survive.
   A language is added by adding an empty `lang/<key>.json` — `bmc-intl/core/lang/` for the host, `frontend/lang/` for
   the web — keyed as ICU4X minimises the locale: `zh` for Simplified Chinese, never `zh-Hans`.
3. Fill the `null`s. Reuse the terms the files already use; leave product names as they are; match the register of
   neighbouring entries. One you cannot fill stays `null`: it shows in English and fails nothing.
   `just intl::untranslated` lists every one, and the merge request's `intl-untranslated` job lists those it adds.
4. `just intl::verify` — every surface must pass. It runs in `just validate` and CI.
5. Never write a test that asserts one of your own translations; assert the English default only. Language coverage is
   the tooling's job.

## What a review looks for

- Copy declared where it is used, or lazily where the data is — never mapped at an edge.
- A description a translator could act on without opening the code.
- Values formatted by the device's preference before they enter the pattern, and no `#`; a count through
  `{n, plural, …}`, never `{n} days`; a value with its unit as one message, `{amount} BTC`.
- A reason or caption crossing an API is a whole sentence, a message where it is made, never a fragment the other end
  frames (`Restart: {caption}`); a label formatted outside render does not follow a language change.
- Every new entry translated, or left `null` on purpose; `intl verify` green, and the ast-grep scan clean.
