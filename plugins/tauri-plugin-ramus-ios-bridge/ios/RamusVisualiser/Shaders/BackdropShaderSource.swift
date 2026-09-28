// Ported from ramusTV `RamusTV/Backdrop/Backdrop.metal`.

/// Metal source for the backdrop field, its dither and the present pass.
/// `BackdropUniforms` must match `BackdropUniforms` in `BackdropField.swift`
/// (`BackdropFieldTests.testUniformsLayoutMatchesTheShader`).
let backdropShaderSource = #"""
#include <metal_stdlib>
using namespace metal;

// The backdrop field (ramus ui/src/lib/ultraBlurField.ts, FRAGMENT_SHADER)
// in Metal. BackdropUniforms must match BackdropUniforms in
// BackdropField.swift, and backdropFalloff must match
// BackdropField.falloffStops: BackdropRendererTests compares every pixel with
// BackdropField's CPU reference.

struct BackdropUniforms {
    // Corner colours in paint order, 0...1.
    float4 bottomLeft;
    float4 bottomRight;
    float4 topRight;
    float4 topLeft;
    // rgb: the field's base colour; a: the page colour under the dimmed field.
    float4 base;
    // The pass's target size in pixels.
    float2 viewport;
    float opacity;
    float strength;
};

struct BackdropVarying {
    float4 position [[position]];
};

/// One triangle covering the viewport.
vertex BackdropVarying backdropVertex(uint vid [[vertex_id]]) {
    BackdropVarying out;
    out.position = float4(vid == 1 ? 3.0 : -1.0, vid == 2 ? 3.0 : -1.0, 0.0, 1.0);
    return out;
}

/// Coverage of a corner's colour at ray fraction t: piecewise linear
/// through (0, 1), (0.4, 0.55), (0.62, 0.18) and (0.8, 0).
static float backdropFalloff(float t) {
    if (t < 0.4) return mix(1.0, 0.55, t / 0.4);
    if (t < 0.62) return mix(0.55, 0.18, (t - 0.4) / 0.22);
    if (t < 0.8) return mix(0.18, 0.0, (t - 0.62) / 0.18);
    return 0.0;
}

static float3 backdropLayer(float3 under, float3 color, float2 uv, float2 corner) {
    float a = backdropFalloff(length(uv - corner) * 0.70710678);
    return color * a + under * (1.0 - a);
}

static uint backdropHash(uint x) {
    x ^= x >> 16; x *= 0x7feb352du;
    x ^= x >> 15; x *= 0x846ca68bu;
    x ^= x >> 16;
    return x;
}

static float backdropNoise(uint2 p, uint salt) {
    return float(backdropHash(p.x ^ backdropHash(p.y ^ backdropHash(salt)))) * (1.0 / 4294967295.0);
}

/// The field at one sample of the grid, computed in float, dimmed to
/// `opacity` over the page colour and scaled by `strength`. Sample (i, j)
/// sits at uv (i, j) / (n - 1) for an n-sample axis, so the outer samples
/// lie on the edges of the screen.
fragment float4 backdropFieldFragment(BackdropVarying in [[stage_in]],
                                      constant BackdropUniforms &u [[buffer(0)]]) {
    float2 uv = (in.position.xy - 0.5) / (u.viewport - 1.0);
    float3 c = u.base.rgb;
    c = backdropLayer(c, u.bottomLeft.rgb, uv, float2(0.0, 1.0));
    c = backdropLayer(c, u.bottomRight.rgb, uv, float2(1.0, 1.0));
    c = backdropLayer(c, u.topRight.rgb, uv, float2(1.0, 0.0));
    c = backdropLayer(c, u.topLeft.rgb, uv, float2(0.0, 0.0));
    return float4((c * u.opacity + u.base.a * (1.0 - u.opacity)) * u.strength, 1.0);
}

/// A triangular dither spanning ±1 LSB of the 8-bit output, in LSBs: the
/// difference of two uniform noises from an integer hash of the pixel
/// position. It is the same in every channel, so the grain carries no
/// colour, and static, so it never shimmers.
fragment float4 backdropDitherFragment(BackdropVarying in [[stage_in]]) {
    uint2 p = uint2(in.position.xy);
    return float4(backdropNoise(p, 1u) - backdropNoise(p, 2u), 0.0, 0.0, 1.0);
}

/// The field sampled from the grid with bilinear filtering, plus the dither
/// once before the 8-bit store. Only dithering the finished field removes
/// the contours a dark, near-neutral field would otherwise show in 8 bits.
fragment float4 backdropFragment(BackdropVarying in [[stage_in]],
                                 constant BackdropUniforms &u [[buffer(0)]],
                                 texture2d<float> grid [[texture(0)]],
                                 texture2d<float, access::read> dither [[texture(1)]]) {
    constexpr sampler bilinear(filter::linear, address::clamp_to_edge);
    float2 n = float2(grid.get_width(), grid.get_height());
    float2 uv = in.position.xy / u.viewport;
    float3 shown = grid.sample(bilinear, (uv * (n - 1.0) + 0.5) / n).rgb;
    float d = u.strength > 0.0 ? dither.read(uint2(in.position.xy)).r / 255.0 : 0.0;
    return float4(clamp(shown + d, 0.0, 1.0), 1.0);
}
"""#
