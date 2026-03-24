# Future Improvements

## Performance

- Declarative host-side animations to replace keyframe dependency (see [design doc](declarative-animations.md))
- GPU rendering with shaders (wgpu?) to replace tiny-skia software rasterizer

## API Enhancements

- More draw primitives (circles, lines, arcs)
- Scroll momentum / inertial scrolling for modals
- Theming support

## Binary Compatibility

- SDK version detection in host runtime
- Binary format compatibility check at widget load time
- Mismatch warning when recompile is needed

## QA

- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) for dependency auditing
