// Same confusable-free alphabet the backend uses for claim codes (no 0/O/1/I/L): the tracking id
// can end up typed or read off a QR in the field, so avoid characters that get misread.
const ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";

export function newTrackingId(len = 10): string {
  const bytes = crypto.getRandomValues(new Uint32Array(len));
  let out = "";
  for (const n of bytes) out += ALPHABET[n % ALPHABET.length];
  return out;
}
