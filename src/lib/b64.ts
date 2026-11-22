// The only base64 code in the UI. Works on raw bytes, so UTF-8 and binary are both safe.

const CHUNK = 0x8000;

export function bytesToB64(b: Uint8Array): string {
  let bin = '';
  for (let i = 0; i < b.length; i += CHUNK) {
    bin += String.fromCharCode(...b.subarray(i, i + CHUNK));
  }
  return btoa(bin);
}

export function b64ToBytes(s: string): Uint8Array {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
