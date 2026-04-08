export function formatDuration(seconds: number): string {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}

export function formatCodec(codec: string | null, bitrate: number | null): string | null {
  if (!codec) return null;
  const lossless = ["flac", "alac", "wav", "aiff", "pcm"];
  if (lossless.includes(codec.toLowerCase())) return codec.toUpperCase();
  if (bitrate) return `${codec.toUpperCase()} ${bitrate}`;
  return codec.toUpperCase();
}
