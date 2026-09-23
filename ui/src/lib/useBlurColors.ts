import { useMemo } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useSettingsStore } from "../stores/settingsStore";
import { DEFAULT_BLUR_COLORS, OLED_VOID_BLUR_COLORS } from "./accent";
import type { UltraBlurColors } from "./types";

/**
 * The colours for an `UltraBlurBackground`: the current album's art
 * corners, or the fixed sets the `defaultColours` and `oledVoid`
 * background styles substitute for them.
 */
export function useBlurColors(): UltraBlurColors {
  const albumColors = usePlaybackStore((s) => s.ultraBlurColors);
  const backgroundStyle = useSettingsStore((s) => s.backgroundStyle);
  return useMemo(() => {
    if (backgroundStyle === "defaultColours") return DEFAULT_BLUR_COLORS;
    if (backgroundStyle === "oledVoid") return OLED_VOID_BLUR_COLORS;
    return albumColors ?? DEFAULT_BLUR_COLORS;
  }, [albumColors, backgroundStyle]);
}
