/** Whether the current display supports high dynamic range. */
export const isHDR: boolean =
  typeof window !== "undefined" &&
  window.matchMedia("(dynamic-range: high)").matches;
