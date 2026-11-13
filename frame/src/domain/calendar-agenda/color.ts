/** A calendar colour from the server, or the accent when it is not `#RRGGBB`. */
export const safeColor = (color: string | null): string =>
  color !== null && /^#[0-9a-f]{6}$/iu.test(color) ? color : "var(--color-accent)";
