/**
 * Ridgeline visualiser: a stack of horizontal lines rising from the
 * bottom of the window, each one a past moment of the spectrum drawn as
 * a mountain profile, bass on the left and treble on the right.
 *
 * The front (lowest) row is the live spectrum; every `ridgeRowMs` the
 * eased front row is copied into a history and the older rows step up
 * one slot, so the stack scrolls upward and fades as it ages. A row is
 * resampled from one point per band to several along a monotone cubic
 * before it is textured and stored, so a peak is a run of near-equal
 * points rather than one node with two lines meeting at it. Rows are
 * painted back to front, and each row erases the canvas under its own
 * line before stroking it, so a near peak hides the rows behind it. The
 * canvas is transparent over the backdrop, so the erase reveals the
 * backdrop rather than painting a background colour.
 *
 * Everything here is either a pure function of its arguments (the mix,
 * spread, smoothing, grain, window, resampling and row geometry, written
 * with explicit output buffers so they can be checked standalone) or a
 * plain canvas painter with no state of its own beyond scratch buffers;
 * `FocusVisualizer` owns the row buffers and the paint loop.
 */

/**
 * Average a stereo frame's bands into one 0..1 level per band, bass
 * first. `bands` holds `channels` runs of equal length; `out.length` must
 * equal that run length or the row is zeroed rather than read past its
 * end.
 */
export function mixBandsInto(bands: Uint8Array, channels: number, out: Float32Array): void {
  const n = out.length;
  if (channels < 1 || bands.length !== n * channels) {
    out.fill(0);
    return;
  }
  const scale = 1 / (255 * channels);
  for (let k = 0; k < n; k++) {
    let sum = 0;
    for (let c = 0; c < channels; c++) sum += bands[c * n + k];
    out[k] = sum * scale;
  }
}

/**
 * Blend each point with its two neighbours: `amount` is the total weight
 * given to the neighbours (0 copies `src` unchanged, 1 replaces every
 * point with the mean of its neighbours). The ends reuse their own value
 * for the missing neighbour. Takes the single-band spikes off the line
 * without flattening a real peak.
 */
export function smoothRow(src: Float32Array, out: Float32Array, amount: number): void {
  const n = src.length;
  const side = amount / 2;
  const self = 1 - amount;
  for (let i = 0; i < n; i++) {
    const l = src[i > 0 ? i - 1 : i];
    const r = src[i < n - 1 ? i + 1 : i];
    out[i] = self * src[i] + side * (l + r);
  }
}

// The bell for the last `sigma` and radius spread with, so a spread
// allocates nothing per frame.
let bell = new Float32Array(0);
let bellSigma = NaN;

/**
 * Widen every peak without lowering it: each point becomes the largest
 * of itself and its neighbours scaled by a bell of width `sigma` (in
 * points), so a level lifts the points beside it to a falling fraction
 * of itself. Taking the maximum rather than the sum keeps a plateau a
 * plateau and a peak at its own height; only its flanks grow. A single
 * band that clears the level curve alone then reads as a peak instead
 * of a one-point needle. `sigma` 0 copies `src` unchanged.
 */
export function spreadRow(src: Float32Array, out: Float32Array, sigma: number): void {
  const n = src.length;
  if (!(sigma > 0)) {
    out.set(src);
    return;
  }
  // Beyond three sigma the bell is under 1.2 % and changes nothing visible.
  const radius = Math.min(n - 1, Math.ceil(sigma * 3));
  if (sigma !== bellSigma || bell.length !== radius + 1) {
    bell = new Float32Array(radius + 1);
    for (let d = 0; d <= radius; d++) bell[d] = Math.exp(-(d * d) / (2 * sigma * sigma));
    bellSigma = sigma;
  }
  for (let i = 0; i < n; i++) {
    let best = src[i];
    for (let d = 1; d <= radius; d++) {
      const k = bell[d];
      if (i - d >= 0) {
        const v = src[i - d] * k;
        if (v > best) best = v;
      }
      if (i + d < n) {
        const v = src[i + d] * k;
        if (v > best) best = v;
      }
    }
    out[i] = best;
  }
}

/**
 * Texture a row: every point is scaled by `1 + amount * noise[i]`, with
 * `noise` in -1..1, and clamped at zero. Multiplicative, so a flat
 * stretch stays flat and only what rises gets grain in proportion to
 * its height. Meant for the resampled row, where the points are close
 * enough that a peak becomes a run of near-equal values rather than one
 * node. `amount` 0 copies `src` unchanged.
 */
export function applyGrain(
  src: Float32Array,
  out: Float32Array,
  noise: Float32Array,
  amount: number,
): void {
  const n = src.length;
  if (!(amount > 0)) {
    out.set(src);
    return;
  }
  for (let i = 0; i < n; i++) {
    const v = src[i] * (1 + amount * noise[i]);
    out[i] = v > 0 ? v : 0;
  }
}

/** Fill `noise` with fresh uniform values in -1..1. */
export function rerollGrain(noise: Float32Array): void {
  for (let i = 0; i < noise.length; i++) noise[i] = Math.random() * 2 - 1;
}

/**
 * Per-point multiplier that brings both ends of a row down to its
 * baseline: a raised-cosine ramp over the first and last `taper` fraction
 * of the points (rounded to whole points) and 1 in between. `taper` 0
 * returns a flat window. Without it the lowest and highest bands would
 * end the line mid-air.
 */
export function edgeWindow(n: number, taper: number): Float32Array {
  const w = new Float32Array(n);
  const ramp = Math.round(Math.max(0, taper) * n);
  for (let i = 0; i < n; i++) {
    const fromEdge = Math.min(i, n - 1 - i);
    if (ramp <= 0 || fromEdge >= ramp) {
      w[i] = 1;
    } else {
      w[i] = 0.5 - 0.5 * Math.cos((Math.PI * fromEdge) / ramp);
    }
  }
  return w;
}

/**
 * Fixed-size ring of past rows, newest first. One backing buffer, rows
 * addressed through views made once, so pushing and reading allocate
 * nothing per frame.
 */
export class RidgeHistory {
  readonly rows: number;
  readonly width: number;
  private readonly views: Float32Array[];
  /** A row of silence, handed out for any slot the ring doesn't have. */
  private readonly silence: Float32Array;
  /** Slot the next push writes; the newest row is the slot before it. */
  private head = 0;

  constructor(rows: number, width: number) {
    this.rows = Math.max(1, rows);
    this.width = Math.max(0, width);
    const buf = new Float32Array(this.rows * this.width);
    this.views = Array.from({ length: this.rows }, (_, i) =>
      buf.subarray(i * this.width, (i + 1) * this.width),
    );
    this.silence = new Float32Array(this.width);
  }

  /**
   * Copy `row` in as the newest; the oldest row is dropped. A row wider
   * than the ring is cut to fit, a narrower one is padded with silence.
   */
  push(row: Float32Array): void {
    const slot = this.views[this.head];
    if (row.length === this.width) {
      slot.set(row);
    } else if (row.length > this.width) {
      slot.set(row.subarray(0, this.width));
    } else {
      slot.set(row);
      slot.fill(0, row.length);
    }
    this.head = (this.head + 1) % this.rows;
  }

  /**
   * Row `k` back from the newest (0 = newest). Silence until pushed, and
   * silence for any `k` the ring doesn't hold.
   */
  get(k: number): Float32Array {
    if (!(k >= 0 && k < this.rows)) return this.silence;
    const i = (((this.head - 1 - k) % this.rows) + this.rows) % this.rows;
    return this.views[i];
  }

  /** Every row back to silence. */
  clear(): void {
    for (const v of this.views) v.fill(0);
    this.head = 0;
  }
}

/** The subset of the visualiser parameters that lay the rows out. */
export interface RidgeLayout {
  /** Height of the stack (front baseline to back baseline) as a fraction of the window height. */
  ridgeHeight: number;
  /** Displacement of a full-scale point on the front row as a fraction of the window height. */
  ridgePeak: number;
  /** Peak scale of the back row relative to the front (1 = no perspective). */
  ridgeDepthScale: number;
  /** Gap between the front baseline and the bottom edge as a fraction of the window height. */
  ridgeBottom: number;
  /** Line alpha of the front row. */
  ridgeAlpha: number;
  /** Line alpha of the back row. */
  ridgeBackAlpha: number;
  /**
   * Shape of the alpha fade from the front row to the back: 1 is linear;
   * below 1 the rows hold their brightness further back and fall away
   * toward the back row, above 1 the fade falls faster near the front and
   * flattens toward the back.
   */
  ridgeFadeCurve: number;
}

export interface RidgeRowGeometry {
  /** Y of the row's resting line, in CSS px from the top. */
  baseline: number;
  /** Px a full-scale point rises above the baseline. */
  scale: number;
  /** Line alpha. */
  alpha: number;
}

/**
 * Where row `k` of `rows` sits in a window `h` px tall: the front row's
 * baseline is `ridgeBottom` up from the bottom edge, the back row's is
 * `ridgeHeight` above that, and rows are spaced evenly between. Peak
 * scale eases linearly from the front value to the back one, and alpha
 * along `(1 - depth) ^ ridgeFadeCurve`.
 */
export function ridgeRow(k: number, rows: number, h: number, p: RidgeLayout): RidgeRowGeometry {
  const depth = rows > 1 ? k / (rows - 1) : 0;
  const front = h * (1 - p.ridgeBottom);
  return {
    baseline: front - depth * h * p.ridgeHeight,
    scale: h * p.ridgePeak * (1 + (p.ridgeDepthScale - 1) * depth),
    alpha:
      p.ridgeBackAlpha + (p.ridgeAlpha - p.ridgeBackAlpha) * Math.pow(1 - depth, p.ridgeFadeCurve),
  };
}

// Secant scratch for the tangents, sized to the last row seen so a call
// allocates nothing per frame.
let secant = new Float32Array(0);

/**
 * Slope at every point for a curve through `y` that never overshoots
 * between neighbouring points (Fritsch–Carlson monotone cubic
 * interpolation). Interior slopes start as the mean of the two secants
 * and are set to zero at every local extremum and at both ends of a flat
 * segment, then limited so each segment stays monotone. A peak therefore
 * gets a level tangent at its top and a rounded foot below it instead
 * of two straight lines meeting at a corner. `y` and `out` are the same
 * length; slopes are in y units per point.
 */
export function monotoneTangents(y: Float32Array, out: Float32Array): void {
  const n = y.length;
  if (n === 0) return;
  if (n === 1) {
    out[0] = 0;
    return;
  }
  // Secant of each segment, then the first-guess slope at each point.
  if (secant.length !== n - 1) secant = new Float32Array(n - 1);
  const d = secant;
  for (let k = 0; k < n - 1; k++) d[k] = y[k + 1] - y[k];
  out[0] = d[0];
  out[n - 1] = d[n - 2];
  for (let k = 1; k < n - 1; k++) {
    out[k] = d[k - 1] * d[k] <= 0 ? 0 : (d[k - 1] + d[k]) / 2;
  }
  // Keep every segment monotone: a flat segment gets flat ends, and a
  // segment whose end slopes are too steep for its own secant has them
  // scaled back onto the circle of radius 3.
  for (let k = 0; k < n - 1; k++) {
    if (d[k] === 0) {
      out[k] = 0;
      out[k + 1] = 0;
      continue;
    }
    const a = out[k] / d[k];
    const b = out[k + 1] / d[k];
    const r2 = a * a + b * b;
    if (r2 > 9) {
      const t = 3 / Math.sqrt(r2);
      out[k] = t * a * d[k];
      out[k + 1] = t * b * d[k];
    }
  }
}

// Slope scratch for the resampler, sized to the last row seen.
let resampleSlope = new Float32Array(0);

/**
 * Resample a row to `k` points per band interval along the monotone
 * cubic through its points (`monotoneTangents`), so the curve's shape
 * survives as plain points: `out` holds `(src.length - 1) * k + 1`
 * values, every `k`-th one an original point, or is zeroed when its
 * length disagrees. `k` 1 copies `src`.
 */
export function resampleRow(src: Float32Array, out: Float32Array, k: number): void {
  const n = src.length;
  const steps = Math.max(1, Math.floor(k));
  if (n === 0 || out.length !== (n - 1) * steps + 1) {
    out.fill(0);
    return;
  }
  if (steps === 1) {
    out.set(src);
    return;
  }
  if (resampleSlope.length !== n) resampleSlope = new Float32Array(n);
  const m = resampleSlope;
  monotoneTangents(src, m);
  for (let i = 0; i < n - 1; i++) {
    const y0 = src[i];
    const y1 = src[i + 1];
    const m0 = m[i];
    const m1 = m[i + 1];
    const base = i * steps;
    out[base] = y0;
    for (let j = 1; j < steps; j++) {
      const t = j / steps;
      const t2 = t * t;
      const t3 = t2 * t;
      out[base + j] =
        (2 * t3 - 3 * t2 + 1) * y0 +
        (t3 - 2 * t2 + t) * m0 +
        (-2 * t3 + 3 * t2) * y1 +
        (t3 - t2) * m1;
    }
  }
  out[(n - 1) * steps] = src[n - 1];
}

export interface RidgePaint extends RidgeLayout {
  /** Width of the ridge as a fraction of the window width, centred. */
  ridgeSpan: number;
  /** Stroke width in CSS px. */
  ridgeLineWidth: number;
}

// Per-row scratch for the screen y of each point, sized to the last row
// painted so a paint allocates nothing per row.
let rowY = new Float32Array(0);

// Stroke colour of every row for the row count, alphas and colour last
// painted with; rebuilt only when one of those changes, so a paint builds
// no strings.
let strokeStyles: string[] = [];
let strokeStylesFor = "";

/**
 * Paint `rows` rows back to front. `row(k)` returns row `k`'s levels
 * (0 = front), `edge` the per-point edge multiplier (same length), and
 * `rgb` the stroke colour's channels. Each row first erases the canvas
 * between its line and its baseline (`destination-out`, which on this
 * transparent canvas exposes the backdrop) so the rows behind it are
 * hidden where it rises, then strokes its line as straight segments;
 * the rows are already resampled finely enough for that to read as a
 * curve. The erase reaches the line's centre, not its outer edge, so up
 * to half the stroke width of a row behind can show above a row in
 * front; at hairline widths that is under a pixel.
 */
export function drawRidgeline(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  rows: number,
  row: (k: number) => Float32Array,
  edge: Float32Array,
  rgb: string,
  p: RidgePaint,
): void {
  const n = edge.length;
  if (n < 2 || rows < 1) return;
  const fieldW = w * p.ridgeSpan;
  const fieldX = (w - fieldW) / 2;
  const step = fieldW / (n - 1);
  if (rowY.length !== n) rowY = new Float32Array(n);
  const ys = rowY;
  // Device pixels per CSS px, read off the context's own transform, so
  // each baseline can be put on the same sub-pixel phase: the stroke's
  // upper edge on a device-pixel boundary. A row at rest is a straight
  // rule, and without this each rule lands at its own fraction of a
  // pixel and the anti-aliasing spreads it over one bright row or two
  // dim ones at random, which at a device pixel ratio of 1 reads as
  // alternating fat and thin lines up the stack.
  const scale = ctx.getTransform().a || 1;
  const halfStroke = (p.ridgeLineWidth * scale) / 2;
  const snap = (y: number) => (Math.round(y * scale - halfStroke) + halfStroke) / scale;
  const styleKey = `${rows}|${p.ridgeAlpha}|${p.ridgeBackAlpha}|${p.ridgeFadeCurve}|${rgb}`;
  if (styleKey !== strokeStylesFor) {
    strokeStyles = Array.from(
      { length: rows },
      (_, k) => `rgba(${rgb}, ${ridgeRow(k, rows, h, p).alpha})`,
    );
    strokeStylesFor = styleKey;
  }
  ctx.lineWidth = p.ridgeLineWidth;
  ctx.lineJoin = "round";
  // Butt caps, deliberately: CoreGraphics strokes a long path in runs of
  // about 128 segments and caps each run, so round caps overlap at every
  // run boundary and a translucent line shows a brighter dot there, at
  // the same x on every row. Butt-capped runs abut exactly.
  ctx.lineCap = "butt";
  // Any opaque fill erases fully under `destination-out`.
  ctx.fillStyle = "#000";
  for (let k = rows - 1; k >= 0; k--) {
    const g = ridgeRow(k, rows, h, p);
    const baseline = snap(g.baseline);
    const values = row(k);
    for (let i = 0; i < n; i++) ys[i] = baseline - values[i] * edge[i] * g.scale;
    const line = new Path2D();
    line.moveTo(fieldX, ys[0]);
    for (let i = 1; i < n; i++) line.lineTo(fieldX + i * step, ys[i]);
    // Erase under the line, down to this row's baseline, so anything
    // painted behind it disappears where the line rises. The fill is a
    // copy of the line closed along the baseline; the line itself stays
    // open so the baseline is never stroked.
    const under = new Path2D(line);
    under.lineTo(fieldX + fieldW, baseline);
    under.lineTo(fieldX, baseline);
    under.closePath();
    ctx.globalCompositeOperation = "destination-out";
    ctx.fill(under);
    ctx.globalCompositeOperation = "source-over";
    ctx.strokeStyle = strokeStyles[k];
    ctx.stroke(line);
  }
}
