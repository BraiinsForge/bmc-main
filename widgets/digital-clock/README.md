# Digital Clock Widget

A standalone digital clock widget for the Braiins Deck.

## Building

```bash
# Enter the nix development shell
nix develop

# Build the widget
cargo build -p bmc-widget-digital-clock
```

## Running Locally

### Standalone Mode (No IPC)

For local development and testing without the main Deck application:

```bash
cargo run -p bmc-widget-digital-clock --features standalone -- --standalone
```

This opens a window displaying the clock with default settings.

### With IPC

When running as part of the Deck application, the widget connects to the IPC socket specified by the `BMC_IPC_SOCKET`
environment variable:

```bash
BMC_IPC_SOCKET=/run/bmc/widgets/instance-id.sock cargo run -p bmc-widget-digital-clock
```

## Configuration

The widget accepts the following parameters via IPC `init` message:

| Parameter | Type | Default | Description | |-----------|------|---------|-------------| | `showSeconds` | boolean |
`true` | Display seconds on the clock | | `showTimezone` | boolean | `true` | Display timezone label | | `fontStyle` |
string | `"medium"` | Font weight: `"light"`, `"medium"`, or `"bold"` | | `timezone` | string | (none) | Override
timezone (e.g., `"America/New_York"`). If not set, uses global timezone setting |

## Supported Sizes

- `small` (317x238)
- `medium` (638x238)
- `large` (638x480)
- `full` (1280x480)

## Development

### Faster Compilation

Enable the `slint-embed-files` feature for faster compilation during development:

```bash
cargo build -p bmc-widget-digital-clock --features slint-embed-files,standalone
```

### Running Tests

```bash
cargo test -p bmc-widget-digital-clock
```
