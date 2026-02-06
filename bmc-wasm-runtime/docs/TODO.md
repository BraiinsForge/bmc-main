# Future Improvements

## Performance

- Node-level animations/transitions (apply to layout nodes, not just draw commands)
- Explicit `phase_offset` for time-synced staggered animations
- GPU rendering with shaders (wgpu?) to replace tiny-skia software rasterizer

## API Enhancements

- More draw primitives (circles, lines, arcs)
- Scroll momentum / inertial scrolling for modals
- Theming support

## Binary Compatibility

- SDK version detection in host runtime
- Binary format compatibility check at widget load time
- Mismatch warning when recompile is needed

## Build Guardrails

- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) to ban heavy crates (serde, regex, etc.) from widget builds — needs adding to nix flake first
