---
name: frontend-style
description: Use when writing or modifying React/TSX/SCSS code under `frontend/` — adding a component, passing JSX children, deciding whether to inline a style or move it to a stylesheet, picking spacing values. Triggers on phrases like "add a component", "style this", "fix the layout", "passing children", "what's the design token for spacing", or when about to write `style={{ }}` with more than a one-off rule.
---

# Frontend style conventions (`frontend/`)

Two opinionated patterns that have come up in review: how to pass a single variable into JSX as children, and when to
move a rule from inline `style` to a co-located `.scss` (CSS module).

The frontend uses **React 19**, **Carbon Design System**, **CSS modules via SCSS**, and **Biome** for formatting/lint.
Biome handles the mechanical layer: indent, line width, quotes, semicolons, trailing commas, self-closing elements,
unused-template-literal, hook-dep exhaustiveness, etc. — anything `frontend/biome.json` lists with a level above `off`.
**Don't duplicate Biome-enforced rules here.** This skill is the taste layer Biome cannot reach: project-specific
conventions, architectural thresholds, and rules the project has explicitly disabled in Biome (notably `noChildrenProp`,
which the project disables on purpose so the `children` prop form is available — see below). The SCSS conventions below
are also genuinely outside Biome's reach: `frontend/biome.json` has `css.linter.enabled: false`.

If you find yourself about to add a rule here, check `frontend/biome.json` first. If Biome already enforces it, leave it
out — the linter is the source of truth.

## JSX — single-variable child as a prop, not a wrapper

When you pass a single variable as children to a component, write it as a `children` prop, not as a wrapped child:

```tsx
// don't:
<Tooltip label={t('foo')}>{labelText}</Tooltip>

// do:
<Tooltip children={labelText} label={t('foo')} />
```

The prop form reads as one more named prop on the component instead of pretending the JSX wrapping might mean something.
The wrapper form is reserved for *multi-child* JSX or for dynamic children expressions where the wrapping is
structurally significant:

```tsx
// fine — multiple children:
<Tooltip label={t('foo')}>
    <Icon />
    {labelText}
</Tooltip>

// fine — JSX is the value:
<Tooltip label={t('foo')}>
    {items.map((i) => <Row key={i.id} item={i} />)}
</Tooltip>
```

## Styling — when to move to `.scss`

The frontend convention is **co-located CSS modules**: every component that needs more than a one-off inline rule owns a
`Component.scss` next to its `Component.tsx`. The pattern is everywhere under `frontend/src/components/` and
`frontend/src/pages/` — follow it.

The threshold: **the moment a rule is beyond a one-off spacing tweak**, move it to `.scss`. In particular, any
flex+justify+padding combo, anything with a media query, anything with a `:hover`/`:focus`/`&[data-*]` selector, and
anything reusable across more than one element of the component.

### Boilerplate

```scss
// Foo.scss
@use '@/styles/carbon' as *;

.root {
    display: inline-flex;
    align-items: center;
    gap: mini-units(1);
    padding-inline: mini-units(2);
}
```

```tsx
// Foo.tsx
import css from './Foo.scss';

export const Foo = () => <div className={css.root}>...</div>;
```

Use `mini-units(N)` (from `@/styles/carbon`) for spacing — not raw px. It's the project's design-system spacing function
and respects the global rhythm. The convention is `.root` for the outermost class; sub-elements get descriptive names
(`.label`, `.icon`, `.actions`).

Stylesheets also pick up autoprefixer, theming via Carbon tokens, media queries (`@use '@/styles/var/custom-media'`),
and shared variables — none of which are available from inline `style`.

### When inline `style={{ }}` is fine

For purely spacing one-offs where the rule is genuinely only ever going to apply to this one element at this one site:

```tsx
<Spacer style={{ marginInlineStart: 'auto' }} />
<Row style={{ marginBlockStart: 'var(--cds-spacing-03)' }} />
```

`mini-units(N)` is a SCSS function — it does **not** work from inline `style`. For inline rules use literal CSS values
or a Carbon CSS custom property (`var(--cds-spacing-03)`). If you need `mini-units`, you're already past the threshold;
move the rule to a `.scss`.

The moment a second rule shows up on the same element, or the rule has to react to a state/media-query/theme, move to
the `.scss`. Don't grow an inline `style={{ }}` past one or two properties.

## Hard rules — never

- Never use `<Foo>{singleVar}</Foo>` when `<Foo children={singleVar} />` would do — single-variable JSX children read as
  if the wrapping is structurally meaningful when it isn't.
- Never put a flex+justify+padding combo in inline `style`. Move it to `.scss`.
- Never use raw `px` for spacing — use `mini-units(N)`.
- Never duplicate a Carbon design token in custom CSS — `@use '@/styles/carbon' as *;` and reuse.
