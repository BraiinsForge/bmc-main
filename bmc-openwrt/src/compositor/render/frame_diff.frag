#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform sampler2D previous;
uniform vec2 frame_size;

//_CELL_

// One fragment per CELL x CELL cell of the frame.
// Addressing texels through gl_FragCoord rather than v_coords
// keeps readback row r on frame rows CELL*r..CELL*r+CELL-1,
// whatever transform the renderer applies to the draw.
void main() {
    vec2 origin = floor(gl_FragCoord.xy) * float(CELL);
    float changed = 0.0;
    for (int y = 0; y < CELL; y++) {
        for (int x = 0; x < CELL; x++) {
            vec2 uv = (origin + vec2(float(x), float(y)) + 0.5) / frame_size;
            vec3 delta = abs(texture2D(tex, uv).rgb - texture2D(previous, uv).rgb);
            changed = max(changed, max(delta.r, max(delta.g, delta.b)));
        }
    }
    // Distinct RGB565 or 8-bit channel values differ by at least 1/255.
    gl_FragColor = vec4(step(0.5 / 255.0, changed));
}
