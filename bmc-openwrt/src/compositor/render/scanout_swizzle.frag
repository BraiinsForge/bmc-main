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

varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

#if defined(COLOR_ADJUSTMENT)
uniform float shadow_floor;
uniform vec4 gain_curve;

vec3 adjust_color(vec3 rgb) {
    // The rise follows luma, so dark tinted fills stay near their design color;
    // the fall follows the peak, so the gain stops before any channel clips.
    // Flooring both keeps black off a division by zero, which GLSL ES leaves unspecified.
    vec2 luma_peak = vec2(dot(rgb, vec3(0.2126, 0.7152, 0.0722)), max(rgb.r, max(rgb.g, rgb.b)));
    vec2 inverse = 1.0 / max(vec2(shadow_floor), luma_peak);
    // One vec2 multiply-add: the two segments as scalars measured ~0.6 ms slower on the GC400.
    vec2 gains = gain_curve.xz + gain_curve.yw * inverse;
    return rgb * min(gains.x, gains.y);
}
#endif

void main() {
    vec4 color = texture2D(tex, v_coords);

#if defined(COLOR_ADJUSTMENT)
    color.rgb = adjust_color(color.rgb);
#endif

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    // RG16 has no alpha and the pass always draws at alpha 1.0; applying smithay's alpha
    // uniform measured about 1 ms slower per frame on the BMM101 GC400.
    gl_FragColor = vec4(color.bgr, 1.0);
}
