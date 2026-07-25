import { type NextRequest, NextResponse } from "next/server";
import { defaultLocale, locales } from "./i18n/config";

function detectLocale(req: NextRequest): string {
  const header = req.headers.get("accept-language")?.toLowerCase() ?? "";
  const preferred = header.split(",").map((part) => part.split(";")[0]?.trim() ?? "");
  for (const tag of preferred) {
    if (tag.startsWith("zh")) return "zh";
    if (tag.startsWith("en")) return "en";
  }
  return defaultLocale;
}

export function proxy(req: NextRequest) {
  const { pathname } = req.nextUrl;

  const hasLocale = locales.some(
    (locale) => pathname === `/${locale}` || pathname.startsWith(`/${locale}/`),
  );
  if (hasLocale) return NextResponse.next();

  const locale = detectLocale(req);
  const url = req.nextUrl.clone();
  url.pathname = `/${locale}${pathname === "/" ? "" : pathname}`;
  return NextResponse.redirect(url);
}

export const config = {
  // Skip Next internals and any path that looks like a file (has an extension).
  matcher: ["/((?!_next/|.*\\.).*)"],
};
