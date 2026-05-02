# Combined Scene

The combined scene lets the user build a single display scene from multiple compatible widgets. Instead of dedicating
the whole screen to one widget, the device composes several widgets together and renders them as one scene.

## User stories

### Compose multiple widgets on one scene

> As a user, I want to combine several widgets on a single screen so I can see more than one kind of information at the
> same time.

- A combined scene can contain multiple widgets at once.
- Each widget occupies only part of the scene instead of assuming fullscreen ownership.
- The device renders the configured widgets together as a single composed scene.

### Start from an empty combined scene

> As a user, I want to create a combined scene first and then add only the widgets I actually want in it.

- Creating a combined scene starts with an empty scene.
- The user can add widgets to the scene one by one.
- Removing a widget removes only that widget; it does not delete the whole scene.

### Use a shared widget size model

> As a user, I want widgets in a combined scene to follow a consistent sizing model so different widgets can coexist
> predictably.

- Combined-scene widgets use the size classes `small`, `medium`, and `large`. Fullscreen widgets (`full`) are reserved
  for fullscreen scenes.
- Each widget declares which of those sizes it supports.
- The system only offers sizes that the selected widget actually supports.

### Keep widget-specific configuration

> As a user, I want each widget in a combined scene to keep its own settings so I can configure the content without
> losing the combined layout.

- Each widget in the scene is configured individually.
- Combined-scene support does not replace widget-specific settings; it adds the ability to render the widget inside a
  shared scene.
- Editing one widget does not require recreating the scene or changing the other widgets in it.

## Constraints

- Combined scene is a separate scene type from fullscreen scene.
- A widget may appear in a combined scene only if it implements the combined scene contract.
- A compatible widget must render correctly within its assigned viewport for each supported size.
