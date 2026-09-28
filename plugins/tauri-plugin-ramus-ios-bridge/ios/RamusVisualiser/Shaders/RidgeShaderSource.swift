// Ported from ramusTV `RamusTV/Visualiser/Ridge.metal`.

/// Metal source for the ridge: the erase and stroke pipelines. The layouts
/// of `RidgeUniforms`, `EraseSpan` and `StrokeSegment` must match
/// `RidgeUniforms`, `RidgeEraseSpan` and `RidgeStrokeSegment` in Swift
/// (`RidgeGeometryTests.testMemoryLayoutMatchesTheShaders`).
let ridgeShaderSource = #"""
#include <metal_stdlib>
using namespace metal;

// Shared with RidgeMetalRenderer.swift and RidgeGeometry.swift; the layouts
// must match RidgeUniforms, RidgeEraseSpan and RidgeStrokeSegment.

struct RidgeUniforms {
    float4 color;
    float2 viewport;
    float lineWidth;
    float padding;
};

struct EraseSpan {
    float2 a;
    float2 b;
    float x0;
    float x1;
    float top0;
    float top1;
    float floor;
};

struct StrokeSegment {
    float2 p0;
    float2 p1;
    float2 p2;
    float2 p3;
    float alpha;
    uint flags;
};

constant uint kOpenStart = 1;
constant uint kOpenEnd = 2;
/// Quad corners for two triangles: (0, 1, 2) and (2, 1, 3), where bit 0
/// picks the far end and bit 1 the lower (or outer) side.
constant uint kCorners[6] = {0, 1, 2, 2, 1, 3};

static float4 clipPosition(float2 p, float2 viewport) {
    return float4(p.x / viewport.x * 2 - 1, 1 - p.y / viewport.y * 2, 0, 1);
}

static float segmentDistance(float2 p, float2 a, float2 b) {
    float2 ab = b - a;
    float l2 = dot(ab, ab);
    float t = l2 > 0 ? clamp(dot(p - a, ab) / l2, 0.0, 1.0) : 0.0;
    return length(p - (a + t * ab));
}

// MARK: - Erase

struct EraseVarying {
    float4 position [[position]];
    float2 a [[flat]];
    float2 b [[flat]];
};

/// One trapezoid from the top edge down to the floor, its top raised far
/// enough for a pixel's width of antialiasing measured across the edge.
vertex EraseVarying ridgeEraseVertex(
    uint vid [[vertex_id]], uint iid [[instance_id]],
    const device EraseSpan *spans [[buffer(0)]],
    constant RidgeUniforms &u [[buffer(1)]])
{
    EraseSpan s = spans[iid];
    float2 d = s.b - s.a;
    float slope = d.x != 0 ? d.y / d.x : 0;
    float lift = sqrt(1 + slope * slope);
    uint c = kCorners[vid];
    float x = (c & 1) ? s.x1 : s.x0;
    float y = (c & 2) ? s.floor : ((c & 1) ? s.top1 : s.top0) - lift;
    EraseVarying out;
    out.position = clipPosition(float2(x, y), u.viewport);
    out.a = s.a;
    out.b = s.b;
    return out;
}

/// Coverage of the area below the line `a`-`b`: 0.5 on the line, reaching
/// 1 half a pixel below it. Blended as destination-out.
fragment float4 ridgeEraseFragment(EraseVarying in [[stage_in]]) {
    float2 d = in.b - in.a;
    float len = length(d);
    if (len == 0) {
        return float4(0, 0, 0, 1);
    }
    float2 n = float2(-d.y, d.x) / len;
    if (n.y < 0) {
        n = -n;
    }
    float below = dot(in.position.xy - in.a, n);
    return float4(0, 0, 0, clamp(0.5 + below, 0.0, 1.0));
}

// MARK: - Stroke

struct StrokeVarying {
    float4 position [[position]];
    float2 p0 [[flat]];
    float2 p1 [[flat]];
    float2 p2 [[flat]];
    float2 p3 [[flat]];
    float alpha [[flat]];
    uint flags [[flat]];
};

/// A quad around the segment `p1`-`p2`, reaching `lineWidth / 2 + 1` px to
/// each side and past each joined end, so it holds the segment's round join.
vertex StrokeVarying ridgeStrokeVertex(
    uint vid [[vertex_id]], uint iid [[instance_id]],
    const device StrokeSegment *segments [[buffer(0)]],
    constant RidgeUniforms &u [[buffer(1)]])
{
    StrokeSegment s = segments[iid];
    float2 d = s.p2 - s.p1;
    float len = length(d);
    float2 t = len > 0 ? d / len : float2(1, 0);
    float2 n = float2(-t.y, t.x);
    float r = u.lineWidth * 0.5 + 1;
    uint c = kCorners[vid];
    float2 end = (c & 1)
        ? s.p2 + t * ((s.flags & kOpenEnd) ? 0.0 : r)
        : s.p1 - t * ((s.flags & kOpenStart) ? 0.0 : r);
    StrokeVarying out;
    out.position = clipPosition(end + n * ((c & 2) ? r : -r), u.viewport);
    out.p0 = s.p0;
    out.p1 = s.p1;
    out.p2 = s.p2;
    out.p3 = s.p3;
    out.alpha = s.alpha;
    out.flags = s.flags;
    return out;
}

/// Box-filtered coverage of a line `lineWidth` px wide at the pixel's
/// distance from the segment. A pixel nearer a neighbouring segment belongs
/// to that segment (ties to the earlier one), so a row's joins are round
/// and no pixel is drawn twice. Nothing is drawn past an open end.
fragment float4 ridgeStrokeFragment(
    StrokeVarying in [[stage_in]],
    constant RidgeUniforms &u [[buffer(1)]])
{
    float2 p = in.position.xy;
    float2 ab = in.p2 - in.p1;
    float l2 = dot(ab, ab);
    float along = l2 > 0 ? dot(p - in.p1, ab) / l2 : 0.0;
    if ((in.flags & kOpenStart) && along < 0) {
        discard_fragment();
    }
    if ((in.flags & kOpenEnd) && along > 1) {
        discard_fragment();
    }
    float dist = segmentDistance(p, in.p1, in.p2);
    if (!(in.flags & kOpenStart) && segmentDistance(p, in.p0, in.p1) <= dist) {
        discard_fragment();
    }
    if (!(in.flags & kOpenEnd) && segmentDistance(p, in.p2, in.p3) < dist) {
        discard_fragment();
    }
    float half_width = u.lineWidth * 0.5;
    float coverage = clamp(min(dist + 0.5, half_width) - max(dist - 0.5, -half_width), 0.0, 1.0);
    float a = in.alpha * coverage;
    return float4(u.color.rgb * a, a);
}
"""#
