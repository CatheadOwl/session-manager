import React, { useMemo } from "react";
import { getProviderSvg, hasProviderIcon } from "@/icons";
import { getProviderMeta } from "@/icons/metadata";

interface ProviderIconProps {
  /** Provider id (lowercase, e.g. "claude", "gemini", "codex") */
  providerId: string;
  /** Display name used for fallback initials and title */
  name?: string;
  /** Override brand color */
  color?: string;
  /** Icon size in px (default 20) */
  size?: number;
  className?: string;
  /** Whether to show fallback initials when no SVG (default true) */
  showFallback?: boolean;
}

/**
 * Renders a provider brand icon.
 *
 * Priority:
 *   1. Inline SVG (via dangerouslySetInnerHTML) if a registered SVG exists
 *   2. Fallback: first letter of provider name
 */
export const ProviderIcon: React.FC<ProviderIconProps> = ({
  providerId,
  name,
  color,
  size = 20,
  className = "",
  showFallback = true,
}) => {
  const svg = useMemo(() => getProviderSvg(providerId), [providerId]);
  const hasIcon = useMemo(() => hasProviderIcon(providerId), [providerId]);

  const sizeStyle = useMemo(
    () => ({
      width: size,
      height: size,
      minWidth: size,
      fontSize: size,
      lineHeight: 1 as const,
    }),
    [size],
  );

  // Resolve effective color: prefer explicit `color` prop, then metadata default
  const resolvedColor = useMemo(() => {
    if (color) return color;
    const meta = getProviderMeta(providerId);
    return meta?.defaultColor ?? undefined;
  }, [providerId, color]);

  // 1. Inline SVG
  if (hasIcon && svg) {
    return (
      <span
        className={className}
        title={name ?? providerId}
        style={{
          ...sizeStyle,
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          color: resolvedColor ? `#${resolvedColor}` : undefined,
        }}
        dangerouslySetInnerHTML={{ __html: svg }}
      />
    );
  }

  // 2. Fallback initials
  if (showFallback) {
    const displayName = name ?? providerId;
    const initial = displayName.charAt(0).toUpperCase();
    const fallbackSize = Math.max(size * 0.55, 10);
    return (
      <span
        className={className}
        title={displayName}
        style={{
          ...sizeStyle,
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          borderRadius: "50%",
          fontWeight: 600,
          backgroundColor: resolvedColor ? `#${resolvedColor}22` : "var(--panel-strong)",
          color: resolvedColor ? `#${resolvedColor}` : "var(--muted)",
        }}
      >
        <span style={{ fontSize: fallbackSize, lineHeight: 1 }}>{initial}</span>
      </span>
    );
  }

  return null;
};
