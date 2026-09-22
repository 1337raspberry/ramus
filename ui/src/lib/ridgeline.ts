/**
 * Ridgeline visualiser: a stack of horizontal lines rising from the
 * bottom of the window, each one a past moment of the spectrum drawn as
 * a mountain profile, bass on the left and treble on the right.
 *
 * The front (lowest) row is the live spectrum; every `ridgeRowMs` the
 * eased front row is copied into a history and the older rows step up
 * one slot, so the stack scrolls upward and fades as it ages. Rows are
 * painted back to front, and each row erases the canvas under its own
 * line before stroking it, so a near peak hides the rows behind it. The
 * canvas is transparent over the backdrop, so the erase reveals the
 * backdrop rather than painting a background colour.
 *
 * Everything here is either pure (the mix, smoothing, window, history and
 * row geometry, which have standalone checks) or a plain canvas painter
 * with no state of its own; `FocusVisualizer` owns the buffers and the
 * paint loop.
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
  /** Slot the next push writes; the newest row is the slot before it. */
  private head = 0;

  constructor(rows: number, width: number) {
    this.rows = Math.max(1, rows);
    this.width = Math.max(0, width);
    const buf = new Float32Array(this.rows * this.width);
    this.views = Array.from({ length: this.rows }, (_, i) =>
      buf.subarray(i * this.width, (i + 1) * this.width),
    );
  }

  /** Copy `row` in as the newest; the oldest row is dropped. */
  push(row: Float32Array): void {
    this.views[this.head].set(row.subarray(0, this.width));
    this.head = (this.head + 1) % this.rows;
  }

  /** Row `k` back from the newest (0 = newest). Silence until pushed. */
  get(k: number): Float32Array {
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
 * scale and alpha ease linearly from the front values to the back ones.
 */
export function ridgeRow(k: number, rows: number, h: number, p: RidgeLayout): RidgeRowGeometry {
  const depth = rows > 1 ? k / (rows - 1) : 0;
  const front = h * (1 - p.ridgeBottom);
  return {
    baseline: front - depth * h * p.ridgeHeight,
    scale: h * p.ridgePeak * (1 + (p.ridgeDepthScale - 1) * depth),
    alpha: p.ridgeAlpha + (p.ridgeBackAlpha - p.ridgeAlpha) * depth,
  };
}

export interface RidgePaint extends RidgeLayout {
  /** Width of the ridge as a fraction of the window width, centred. */
  ridgeSpan: number;
  /** Stroke width in CSS px. */
  ridgeLineWidth: number;
}

/**
 * Paint `rows` rows back to front. `row(k)` returns row `k`'s levels
 * (0 = front), `window` the per-point edge multiplier (same length), and
 * `rgb` the stroke colour's channels. Each row first erases the canvas
 * between its line and its baseline (`destination-out`, which on this
 * transparent canvas exposes the backdrop) so the rows behind it are
 * hidden where it rises, then strokes its line.
 */
export function drawRidgeline(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  rows: number,
  row: (k: number) => Float32Array,
  window: Float32Array,
  rgb: string,
  p: RidgePaint,
): void {
  const n = window.length;
  if (n < 2 || rows < 1) return;
  const fieldW = w * p.ridgeSpan;
  const fieldX = (w - fieldW) / 2;
  const step = fieldW / (n - 1);
  ctx.lineWidth = p.ridgeLineWidth;
  ctx.lineJoin = "round";
  ctx.lineCap = "round";
  for (let k = rows - 1; k >= 0; k--) {
    const g = ridgeRow(k, rows, h, p);
    const values = row(k);
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const x = fieldX + i * step;
      const y = g.baseline - values[i] * window[i] * g.scale;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    // Erase under the line, down to this row's baseline, so anything
    // painted behind it disappears where the line rises.
    ctx.save();
    ctx.lineTo(fieldX + fieldW, g.baseline);
    ctx.lineTo(fieldX, g.baseline);
    ctx.closePath();
    ctx.globalCompositeOperation = "destination-out";
    ctx.fillStyle = "#000";
    ctx.fill();
    ctx.restore();
    // The stroke path again, open this time: closing it would draw the
    // baseline as a visible line.
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const x = fieldX + i * step;
      const y = g.baseline - values[i] * window[i] * g.scale;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = `rgba(${rgb}, ${g.alpha})`;
    ctx.stroke();
  }
}
