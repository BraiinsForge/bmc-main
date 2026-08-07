# Scenes & Widgets

Scenes are the display pages a user configures for the Deck. Each scene contains one or more widgets, and the device
shows one scene at a time while allowing the user to move between scenes manually or through automatic cycling.

Widgets provide the content inside a scene. A fullscreen scene contains one `full` widget. A combined scene contains
multiple compatible widgets placed on the shared grid.

## User stories

### Build a set of display scenes

> As a user, I want to create several scenes so my Deck can show different information throughout the day.

- A user can create fullscreen scenes and combined scenes.
- Each scene has its own widget layout and widget-specific settings.
- Scenes persist across reboot.

### Configure widgets inside scenes

> As a user, I want each scene to contain the widgets I choose, configured for the information I care about.

- A fullscreen scene contains one widget that owns the whole display.
- A combined scene can contain multiple compatible widgets at once.
- Widget settings are configured per widget instance, so the same widget type can appear in different scenes with
  different settings.
- Updating widget settings applies to the scene without requiring the user to recreate it.

### Know when a scene is still loading

> As a user, I want clear feedback when a scene has no content to show yet, so a brief loading moment does not look like
> a broken device.

- Any scene — fullscreen or combined — whose widgets have not drawn their first frame yet shows the DECK logo with a
  "Loading scene…" caption beneath it.
- When no scene is configured at all, the device shows the logo alone.
- The loading placeholder moves with the scene during swipes, transitions, and previews, just like real content.
- An empty combined scene is not "loading": it shows its separator grid instead (see
  [Combined Scene](combined-scene.md)).

### Enable or disable scenes

> As a user, I want to keep a scene configured but temporarily remove it from the display rotation.

- Disabled scenes remain saved.
- Disabled scenes do not participate in scene cycling.
- Re-enabling a scene returns it to the configured scene order.

### Reorder scenes

> As a user, I want to choose the order in which scenes appear, so swipes and automatic cycling follow my preferred
> sequence.

- Scenes have a user-configured order.
- Reordering scenes changes the navigation and cycling order.
- If the currently displayed scene is still enabled and supported after a reorder, the device keeps showing that scene
  rather than jumping to a different scene.

## Scene cycling

Scene cycling is the way the device moves between enabled scenes. It uses the same configured scene order for manual
swipes and automatic rotation.

### Navigate scenes manually

> As a user, I want to swipe between scenes on the device, so I can choose what I see without opening the web UI.

- Swipe left or right moves through enabled scenes in the configured order.
- Scene transitions are animated so the outgoing and incoming scenes remain visually connected.
- Manual touch interaction pauses automatic scene cycling until the touch interaction finishes.

### Cycle scenes automatically

> As a user, I want the Deck to rotate between scenes automatically, so multiple scenes can be useful without manual
> interaction.

- Automatic scene cycling can be enabled or disabled.
- A global default duration controls how long each scene stays visible.
- A scene can override the default duration when it needs more or less time on screen.
- Automatic cycling follows the same configured scene order as manual navigation.
- Automatic cycling skips disabled or unsupported scenes.

### Choose the transition effect

> As a user, I want to pick how the display changes between scenes, so automatic cycling matches my taste — or gets out
> of my way entirely.

- The web UI's screen cycling settings offer three transition effects: Slide, Fade, and None.
- Slide moves the outgoing scene off one side of the display while the incoming scene follows it in.
- Fade cross-fades in place: the outgoing scene dissolves as the incoming scene appears over it.
- None switches scenes instantly with no animation, for users who find motion distracting.
- Whatever the effect, a scene change never shows a blank or half-drawn scene: the incoming scene is fully prepared
  before it appears.
- The chosen effect persists across reboot; the default is Slide.

## Constraints

- A fullscreen scene uses exactly one `full` widget.
- A combined scene uses compatible widgets in the supported grid sizes for the device.
- Scene cycling operates only on enabled scenes that are supported by the current hardware and installed widget
  manifests.
- If every configured scene is disabled or unsupported, the device cannot build a normal scene-cycling list from user
  configuration.
- The branding logo appears only on the Braiins Deck; other products show a plain dark background while a scene has no
  rendered content.
- The transition effect applies to automatic cycling; a manual swipe always slides, following the finger.

## Out of scope

- Widget-specific behavior and visual design are documented in individual widget stories.
- Combined scene layout details are covered in [Combined Scene](combined-scene.md).
- Touch gesture recognition details are covered in [Touch & Gesture Input](touch-and-gestures.md).
