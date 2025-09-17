# frontend

## TOOLS
 - [Yarn](https://yarnpkg.com)
 - [Volta](https://volta.sh)
 - [Biome](https://biomejs.dev)
 - [RSpack](https://rspack.dev)
 - [Storybook](https://storybook.js.org)
 - [Carbon Design System](https://carbondesignsystem.com)
 - [Jest](https://jestjs.io)
 - [buf](https://buf.build)

## Development

### `madplay`

If you want to have sound playback working, `madplay` is still used in backend and does not work for you (which is very likely),
you can use this shim script to forward the sound to `ffplay` instead

Just put in somewhere in your path and make it executable.

```bash
$ cat ~/.local/bin/madplay
#!/bin/bash

# Find the audio file (last non-option argument)
audio_file=""
for arg in "$@"; do
    if [[ ! "$arg" =~ ^- ]]; then
        audio_file="$arg"
    fi
done

echo -e "\e[33mRedirecting madplay to: ffplay -nodisp -autoexit \"$audio_file\"\e[0m"
exec ffplay -nodisp -autoexit "$audio_file"
```
