import type { SessionMeta } from "@/types";

export const getBaseName = (path?: string | null) => {
  if (!path) return "";
  const normalized = path.replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/);
  return parts[parts.length - 1] || normalized;
};

export const normalizeProjectDir = (path?: string | null) => {
  if (!path) return "Unknown";
  return path.replace(/^([A-Z]):/, (_, drive: string) => `${drive.toLowerCase()}:`);
};

export const formatSessionTitle = (session: SessionMeta) =>
  session.title || getBaseName(session.projectDir) || session.sessionId.slice(0, 8);

export const formatTimestamp = (value?: number) => {
  if (!value) return "—";
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
};
