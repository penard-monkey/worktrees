/** Release-owned routing, never repository/project settings or user identifiers. */
export type FeedbackConfig = { key: string; apiUrl: string };

export function feedbackConfig(key: unknown, apiUrl: unknown, development: boolean): FeedbackConfig | null {
  if (typeof key !== "string" || !/^pk_[A-Za-z0-9_-]{5,77}$/.test(key) || key.length > 80) return null;
  if (typeof apiUrl !== "string" || apiUrl !== apiUrl.trim()) return null;
  try {
    const url = new URL(apiUrl);
    const loopback = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
    if (url.protocol !== "https:" && !(development && loopback && url.protocol === "http:")) return null;
    if (url.username || url.password || url.search || url.hash || url.pathname !== "/") return null;
    return { key, apiUrl: url.origin };
  } catch {
    return null;
  }
}
