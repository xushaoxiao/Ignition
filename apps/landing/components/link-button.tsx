"use client";

import { Button } from "@heroui/react";
import type { ReactNode } from "react";

type Props = {
  href: string;
  children: ReactNode;
  variant?: "primary" | "secondary" | "tertiary" | "outline" | "ghost" | "danger";
  size?: "sm" | "md" | "lg";
  fullWidth?: boolean;
  className?: string;
};

// HeroUI v3 Button is action-first (onPress). This wraps it for navigation:
// in-page anchors scroll smoothly; external links open in a new tab.
export function LinkButton({ href, children, variant, size, fullWidth, className }: Props) {
  function handlePress() {
    if (href.startsWith("#")) {
      document.getElementById(href.slice(1))?.scrollIntoView({ behavior: "smooth" });
      return;
    }
    const external = /^https?:\/\//.test(href);
    window.open(href, external ? "_blank" : "_self", external ? "noopener,noreferrer" : undefined);
  }

  return (
    <Button
      onPress={handlePress}
      variant={variant}
      size={size}
      fullWidth={fullWidth}
      className={className}
    >
      {children}
    </Button>
  );
}
