/**
 * The UltraBlur background field and its two renderers.
 *
 * The field is four corner colours, each fading out radially over a
 * near-black base. It is drawn by a WebGL2 fragment shader that computes
 * the whole field in floating point and adds a ±1 LSB triangular dither
 * before the 8-bit output, so the fade reaches the screen as a smooth
 * ramp. The CSS `radial-gradient()` rendering of the same field is kept
 * only as the fallback for when WebGL2 is unavailable or its context is
 * lost, because it bands on the WebKit ports:
 *
 * - WebKit/Cocoa (macOS, iOS) quantises each translucent gradient
 *   layer's colour ramp before CoreGraphics dithers the output, so every
 *   layer draws hard rings (~40 of them for a dark colour fading to
 *   transparent) and the four layers cross into a visible crosshatch.
 *   The output dither cannot remove steps that are already in the ramp.
 * - WebKitGTK (Skia and Cairo) paints gradients without any dither:
 *   plain 8-bit stair-steps per layer.
 * - Chromium dithers every gradient at raster time, which is close to
 *   smooth, but still one 8-bit dither per layer.
 *
 * A dark, near-neutral field spans only a handful of 8-bit levels across
 * the whole window, so any step in it is a hard contour; only dithering
 * the finished field once, at full precision, removes them.
 *
 * One description of the field (`FIELD_BASE`, `FALLOFF_STOPS`, `LAYERS`)
 * drives both the shader and the fallback's gradient list, so the two
 * cannot drift apart.
 */

export type Rgb = readonly [number, number, number];

export interface CornerRgb {
  topLeft: Rgb;
  topRight: Rgb;
  bottomLeft: Rgb;
  bottomRight: Rgb;
}

/** Colour under the four corner layers. */
export const FIELD_BASE: Rgb = [5, 5, 8];

/**
 * Corner fade as `[distance, coverage]` stops. Distance is the fraction
 * along the gradient ray of a CSS `ellipse farthest-corner` gradient
 * centred on the corner; coverage is how much of the corner colour sits
 * over what is beneath it.
 *
 * Each corner is fully faded by 80% of the farthest-corner distance. At
 * 100% every layer would span the whole surface, and because the layers
 * stack in paint order the top corners would wash over the bottom ones;
 * fading out before the opposite quadrant gives each corner equal
 * weight. The fade is eased through intermediate stops: a plain linear
 * `colour → transparent` fade has a slope kink at its end, which reads
 * as a hard arc across a near-monochrome background.
 */
export const FALLOFF_STOPS: ReadonlyArray<readonly [number, number]> = [
  [0, 1],
  [0.4, 0.55],
  [0.62, 0.18],
  [0.8, 0],
];

/**
 * Layers from back to front. Paint order is load-bearing: top-left
 * dominates wherever it is opaque and the others show through where it
 * has faded. The tone pass in `UltraBlurBackground` was tuned against
 * this order (and against the extraction-side chroma cap in
 * `blurArt.ts`); reordering changes every album's look.
 */
const LAYERS = [
  { corner: "bottomLeft", cssVar: "--ultrablur-bl", x: 0, y: 1 },
  { corner: "bottomRight", cssVar: "--ultrablur-br", x: 1, y: 1 },
  { corner: "topRight", cssVar: "--ultrablur-tr", x: 1, y: 0 },
  { corner: "topLeft", cssVar: "--ultrablur-tl", x: 0, y: 0 },
] as const;

/** Colour crossfade when the corners change (e.g. a new album). */
export const TRANSITION_MS = 800;

/**
 * CSS `ease-in-out`, i.e. `cubic-bezier(0.42, 0, 0.58, 1)`: solve the
 * curve's x(t) = `x` by bisection and return y(t).
 */
export function easeInOut(x: number): number {
  if (x <= 0) return 0;
  if (x >= 1) return 1;
  const bez = (t: number, p1: number, p2: number) =>
    3 * (1 - t) * (1 - t) * t * p1 + 3 * (1 - t) * t * t * p2 + t * t * t;
  let lo = 0;
  let hi = 1;
  let t = x;
  for (let i = 0; i < 24; i++) {
    t = (lo + hi) / 2;
    if (bez(t, 0.42, 0.58) < x) lo = t;
    else hi = t;
  }
  return bez(t, 0, 1);
}

// --- CSS fallback ---

/**
 * `background-image` for the CSS fallback: one `radial-gradient()` per
 * layer, listed front to back as CSS paints the first layer on top. The
 * colours are read from the `--ultrablur-*` custom properties, whose
 * registered `<color>` syntax (see `styles.css`) lets them transition.
 */
export function fallbackGradientCSS(): string {
  return [...LAYERS]
    .reverse()
    .map(({ cssVar, x, y }) => {
      const stops = FALLOFF_STOPS.map(([pos, coverage]) => {
        const colour =
          coverage >= 1
            ? `var(${cssVar})`
            : coverage <= 0
              ? "transparent"
              : `color-mix(in srgb, var(${cssVar}) ${Math.round(coverage * 100)}%, transparent)`;
        return `${colour} ${pos * 100}%`;
      });
      return `radial-gradient(ellipse farthest-corner at ${x * 100}% ${y * 100}%, ${stops.join(", ")})`;
    })
    .join(", ");
}

/** Background colour for the CSS fallback. */
export function fallbackBaseCSS(): string {
  return `rgb(${FIELD_BASE.join(", ")})`;
}

/** `transition` for the CSS fallback's four colour properties. */
export function fallbackTransitionCSS(): string {
  return LAYERS.map(({ cssVar }) => `${cssVar} ${TRANSITION_MS}ms ease-in-out`).join(", ");
}

/** Maps each corner to its fallback custom property. */
export function fallbackColourVars(corners: CornerRgb): Record<string, string> {
  const out: Record<string, string> = {};
  for (const { corner, cssVar } of LAYERS) {
    const [r, g, b] = corners[corner];
    out[cssVar] = `rgb(${r}, ${g}, ${b})`;
  }
  return out;
}

// --- WebGL renderer ---

const glslFloat = (v: number) => v.toFixed(6);

/** GLSL body of the fade: piecewise-linear through `FALLOFF_STOPS`. */
function falloffGLSL(): string {
  const lines: string[] = [];
  for (let i = 1; i < FALLOFF_STOPS.length; i++) {
    const [p0, a0] = FALLOFF_STOPS[i - 1];
    const [p1, a1] = FALLOFF_STOPS[i];
    lines.push(
      `if (t < ${glslFloat(p1)}) return mix(${glslFloat(a0)}, ${glslFloat(a1)}, (t - ${glslFloat(p0)}) / ${glslFloat(p1 - p0)});`,
    );
  }
  lines.push(`return ${glslFloat(FALLOFF_STOPS[FALLOFF_STOPS.length - 1][1])};`);
  return lines.join("\n  ");
}

const VERTEX_SHADER = `#version 300 es
in vec2 aPos;
void main() { gl_Position = vec4(aPos, 0.0, 1.0); }`;

/*
 * Distance: for a gradient centred on a corner of a W×H box,
 * `ellipse farthest-corner` has radii √2·W and √2·H, so the ray fraction
 * at normalised position uv is |uv − corner| / √2.
 *
 * Colours are blended in gamma-encoded sRGB, as CSS gradients are, and
 * the result is written premultiplied with alpha = the overall opacity.
 * The compositor then adds (backdrop × (1 − alpha)), a per-region
 * constant, to an already-dithered image, so the dither survives even
 * an 8-bit compositor, where a separate CSS `opacity` pass would round
 * the dithered values again and bring back faint contours.
 *
 * Dither: triangular (TPDF) noise spanning ±1 LSB of the 8-bit output,
 * shared across channels so the grain carries no colour, from an
 * integer hash of the pixel position (static, so it never shimmers).
 */
const FRAGMENT_SHADER = `#version 300 es
precision highp float;
uniform vec2 uSize;
uniform vec3 uBase;
uniform vec3 uColors[${LAYERS.length}];
uniform float uOpacity;
out vec4 outColor;

float falloff(float t) {
  ${falloffGLSL()}
}

uint hash(uint x) {
  x ^= x >> 16; x *= 0x7feb352du;
  x ^= x >> 15; x *= 0x846ca68bu;
  x ^= x >> 16;
  return x;
}

float noise(uvec2 p, uint salt) {
  return float(hash(p.x ^ hash(p.y ^ hash(salt)))) * (1.0 / 4294967295.0);
}

void main() {
  vec2 uv = vec2(gl_FragCoord.x / uSize.x, 1.0 - gl_FragCoord.y / uSize.y);
  vec3 c = uBase;
${LAYERS.map(
  ({ x, y }, i) =>
    `  { float a = falloff(length(uv - vec2(${glslFloat(x)}, ${glslFloat(y)})) * 0.70710678); c = uColors[${i}] * a + c * (1.0 - a); }`,
).join("\n")}
  uvec2 p = uvec2(gl_FragCoord.xy);
  float dither = (noise(p, 1u) - noise(p, 2u)) / 255.0;
  outColor = vec4(clamp(c * uOpacity + dither, 0.0, uOpacity), uOpacity);
}`;

type Channels = [number, number, number];
type Field = Channels[];

function toField(corners: CornerRgb): Field {
  return LAYERS.map(({ corner }) => [...corners[corner]] as Channels);
}

function sameField(a: Field, b: Field): boolean {
  return a.every((c, i) => c[0] === b[i][0] && c[1] === b[i][1] && c[2] === b[i][2]);
}

/**
 * Draws the field into a canvas with WebGL2. Draws only when something
 * changes (colours, size, opacity) and during a colour transition, so an
 * idle background costs nothing. The canvas must be sized to exact
 * device pixels (`resize`): any resampling on the way to the screen
 * averages the dither away and the contours come back.
 */
export class UltraBlurRenderer {
  private readonly gl: WebGL2RenderingContext;
  private readonly program: WebGLProgram;
  private readonly buffer: WebGLBuffer;
  private readonly uSize: WebGLUniformLocation | null;
  private readonly uBase: WebGLUniformLocation | null;
  private readonly uColors: WebGLUniformLocation | null;
  private readonly uOpacity: WebGLUniformLocation | null;
  private readonly onContextLost: (e: Event) => void;

  private shown: Field | null = null;
  private from: Field | null = null;
  private target: Field | null = null;
  private transitionStart = 0;
  private frame = 0;
  private opacity = 1;

  /** Returns null when WebGL2 is unavailable or the shader fails to build. */
  static create(canvas: HTMLCanvasElement, onLost: () => void): UltraBlurRenderer | null {
    const gl = canvas.getContext("webgl2", {
      alpha: true,
      premultipliedAlpha: true,
      antialias: false,
      depth: false,
      stencil: false,
      preserveDrawingBuffer: false,
      // Keeps dual-GPU Macs on the integrated GPU for a static background.
      powerPreference: "low-power",
    });
    if (!gl || gl.isContextLost()) return null;
    try {
      return new UltraBlurRenderer(canvas, gl, onLost);
    } catch {
      return null;
    }
  }

  private constructor(
    private readonly canvas: HTMLCanvasElement,
    gl: WebGL2RenderingContext,
    onLost: () => void,
  ) {
    this.gl = gl;
    const compile = (type: number, src: string) => {
      const s = gl.createShader(type);
      if (!s) throw new Error("createShader failed");
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
        const log = gl.getShaderInfoLog(s);
        gl.deleteShader(s);
        throw new Error(log ?? "shader compile failed");
      }
      return s;
    };
    const vs = compile(gl.VERTEX_SHADER, VERTEX_SHADER);
    const fs = compile(gl.FRAGMENT_SHADER, FRAGMENT_SHADER);
    const program = gl.createProgram();
    gl.attachShader(program, vs);
    gl.attachShader(program, fs);
    gl.linkProgram(program);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      const log = gl.getProgramInfoLog(program);
      gl.deleteProgram(program);
      throw new Error(log ?? "program link failed");
    }
    this.program = program;

    // One triangle covering the viewport.
    this.buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    const aPos = gl.getAttribLocation(program, "aPos");
    gl.enableVertexAttribArray(aPos);
    gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);

    this.uSize = gl.getUniformLocation(program, "uSize");
    this.uBase = gl.getUniformLocation(program, "uBase");
    this.uColors = gl.getUniformLocation(program, "uColors");
    this.uOpacity = gl.getUniformLocation(program, "uOpacity");

    this.onContextLost = () => {
      cancelAnimationFrame(this.frame);
      this.frame = 0;
      onLost();
    };
    canvas.addEventListener("webglcontextlost", this.onContextLost);
  }

  /** Sets the canvas backing store in device pixels and redraws. */
  resize(width: number, height: number): void {
    const w = Math.max(1, Math.round(width));
    const h = Math.max(1, Math.round(height));
    if (this.canvas.width === w && this.canvas.height === h) return;
    this.canvas.width = w;
    this.canvas.height = h;
    this.draw();
  }

  setOpacity(opacity: number): void {
    const o = Math.max(0, Math.min(1, opacity));
    if (o === this.opacity) return;
    this.opacity = o;
    this.draw();
  }

  /**
   * Sets the corner colours. The first call paints immediately; later
   * ones crossfade from whatever is on screen, so a change mid-transition
   * retargets smoothly instead of jumping.
   */
  setColors(corners: CornerRgb): void {
    const next = toField(corners);
    if (this.target && sameField(next, this.target)) return;
    this.target = next;
    if (!this.shown) {
      this.shown = next;
      this.draw();
      return;
    }
    this.from = this.shown.map((c) => [...c] as Channels);
    this.transitionStart = performance.now();
    if (!this.frame) this.frame = requestAnimationFrame(this.tick);
  }

  dispose(): void {
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.canvas.removeEventListener("webglcontextlost", this.onContextLost);
    const gl = this.gl;
    if (!gl.isContextLost()) {
      gl.deleteBuffer(this.buffer);
      gl.deleteProgram(this.program);
    }
  }

  private tick = (now: number): void => {
    this.frame = 0;
    if (!this.from || !this.target) return;
    const k = easeInOut((now - this.transitionStart) / TRANSITION_MS);
    const from = this.from;
    this.shown = this.target.map(
      (c, i) => c.map((v, ch) => from[i][ch] + (v - from[i][ch]) * k) as Channels,
    );
    this.draw();
    if (k < 1) this.frame = requestAnimationFrame(this.tick);
    else this.from = null;
  };

  private draw(): void {
    const gl = this.gl;
    if (!this.shown || gl.isContextLost()) return;
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.useProgram(this.program);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
    gl.uniform2f(this.uSize, this.canvas.width, this.canvas.height);
    gl.uniform3f(this.uBase, FIELD_BASE[0] / 255, FIELD_BASE[1] / 255, FIELD_BASE[2] / 255);
    gl.uniform3fv(
      this.uColors,
      this.shown.flat().map((v) => v / 255),
    );
    gl.uniform1f(this.uOpacity, this.opacity);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }
}
