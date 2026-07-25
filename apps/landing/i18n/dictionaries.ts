import type { Locale } from "./config";
import { en } from "./dictionaries/en";
import { zh } from "./dictionaries/zh";

export type Dictionary = typeof en;

const dictionaries: Record<Locale, Dictionary> = { en, zh };

export function getDictionary(locale: Locale): Dictionary {
  return dictionaries[locale];
}
