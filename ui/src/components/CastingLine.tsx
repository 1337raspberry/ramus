import { useCastStore } from "../stores/castStore";

/** "Playing on …" while casting; nothing otherwise. */
export default function CastingLine({ className }: { className: string }) {
  const player = useCastStore((s) => s.player);
  const lost = useCastStore((s) => s.link === "lost");
  if (!player) return null;
  return (
    <div className={className}>
      {lost ? `Reconnecting to ${player.name}…` : `Playing on ${player.name}`}
    </div>
  );
}
