export function formatDuration(seconds: number): string {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}

const LOSSLESS = ["flac", "alac", "wav", "aiff", "aif", "pcm"];
const CBR_BITRATES = new Set([96, 128, 192, 256, 320]);

export interface CodecParts {
  label: string;
  detail: string | null;
}

export function formatCodecParts(codec: string | null, bitrate: number | null): CodecParts | null {
  if (!codec) return null;
  const upper = codec.toUpperCase();
  if (LOSSLESS.includes(codec.toLowerCase())) {
    return { label: upper, detail: null };
  }
  if (bitrate) {
    const tag = CBR_BITRATES.has(bitrate) ? "CBR" : "VBR";
    return { label: upper, detail: `${bitrate} ${tag}` };
  }
  return { label: upper, detail: null };
}

export function formatCodec(codec: string | null, bitrate: number | null): string | null {
  const parts = formatCodecParts(codec, bitrate);
  if (!parts) return null;
  if (parts.detail) return `${parts.label} ${parts.detail}`;
  return parts.label;
}

/// Byte-count display — "512 KB", "3.4 GB", "42 B". Uses binary prefixes
/// (1024-based) to match what users see in Finder / File Explorer.
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  const precision = value < 10 && unit > 0 ? 1 : 0;
  return `${value.toFixed(precision)} ${units[unit]}`;
}
