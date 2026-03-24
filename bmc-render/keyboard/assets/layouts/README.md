# Keyboard Layout Sources

Layout XML files in this directory are from the
[AnySoftKeyboard](https://github.com/AnySoftKeyboard/AnySoftKeyboard/tree/main/addons/languages) project, licensed under
**Apache 2.0**.

These files define the 3 letter rows of each keyboard layout. The bottom row (space, enter, symbols switch) is added by
our renderer at runtime.

## Files

| File            | Source path in AnySoftKeyboard                                          |
| --------------- | ----------------------------------------------------------------------- |
| `en_qwerty.xml` | `addons/languages/english/pack/src/main/res/xml/qwerty.xml`             |
| `de_qwertz.xml` | `addons/languages/german/pack/src/main/res/xml/de_qwertz.xml`           |
| `fr_azerty.xml` | `addons/languages/french/pack/src/main/res/xml/azerty.xml`              |
| `fi_qwerty.xml` | `addons/languages/finnish/pack/src/main/res/xml/finnish_qwerty.xml`     |
| `da_qwerty.xml` | `addons/languages/danish/pack/src/main/res/xml/qwerty.xml`              |
| `no_qwerty.xml` | `addons/languages/norwegian/pack/src/main/res/xml/norwegian_qwerty.xml` |

Popup keyboard XMLs (`*_popup_*.xml`) are referenced via `android:popupKeyboard` and resolved by `build.rs` at compile
time into inline popup characters.

## Format

AnySoftKeyboard uses Android XML keyboard format:

- `android:codes` — Unicode codepoint (integer or char) or special code (-1=shift, -5=delete)
- `android:keyLabel` — display label (optional, defaults to char from code)
- `android:keyWidth` — relative width as percentage (e.g. "15%p" for wider keys)
- `android:popupCharacters` — long-press alternatives
- `android:popupKeyboard` — reference to external popup keyboard XML (resolved by build.rs)
