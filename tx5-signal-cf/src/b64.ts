import { fromUint8Array, toUint8Array } from 'js-base64';

export function toB64Url(s: Uint8Array): string {
  return fromUint8Array(s)
    .replace(/\=/g, '')
    .replace(/\+/g, '-')
    .replace(/\//g, '_');
}

export function fromB64Url(s: string): Uint8Array {
  return toUint8Array(s.replace(/\-/g, '+').replace(/\_/g, '/'));
}
