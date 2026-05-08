# Manifest Widget Form: Live Validation and Number-Input Handling

Surface backend validation errors live (debounced) for both fullscreen and combined-scene widget edits, and stop
dropping unparseable number input on the floor.

Full spec: `docs/superpowers/specs/2026-05-08-widget-form-live-validation-design.md`.

## Summary

- Both edit flows already use `updateWidget` to apply changes; combined scenes already call it on every change. We
  extend the same pattern to fullscreen and stop swallowing errors in the catch block — backend already validates and
  reports per-field violations, no BE plumbing needed for that part.
- `makeNumberParamValue` stops mapping `NaN` to `nullValue`. Empty input still becomes null; valid numbers become
  `integerValue` / `doubleValue`; non-empty unparseable input is sent as `stringValue` so the backend's existing
  type-mismatch path fires.
- The form keeps a per-field raw-text map so the user's typed input is never overwritten on re-render.
- Backend violation strings are reworded for end-user readability (no error codes, no i18n in this pass).
