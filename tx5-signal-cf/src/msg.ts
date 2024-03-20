import { ed } from './ed.ts';
import { err } from './err.ts';
import { toB64Url, fromB64Url } from './b64.ts';

interface Query {
  nodePubKey: Uint8Array;
  nonce: number;
}

export function encodeQuery(query: Query): string {
  const k = toB64Url(query.nodePubKey);
  return `k=${k}&n=${query.nonce}`;
}

export function encodeSignedQuery(
  signature: Uint8Array,
  query: string,
): string {
  const s = toB64Url(signature);
  return `${query}&s=${s}`;
}

export function verifySignedQuery(k: string, n: string, s: string): Query {
  const query = {
    nodePubKey: fromB64Url(k),
    nonce: parseFloat(n),
  };
  const enc = new TextEncoder().encode(encodeQuery(query));
  const sig = fromB64Url(s);
  if (!ed.verify(sig, enc, query.nodePubKey)) {
    throw err('invalid query signature', 400);
  }
  return query;
}

export function verifySignedHandshake(
  pk: Uint8Array,
  nonce: Uint8Array,
  sig: Uint8Array,
) {
  if (sig.byteLength !== 64) {
    throw err(`invalid signature length ${sig.byteLength}`, 400);
  }
  if (!ed.verify(sig, nonce, pk)) {
    throw err('invalid handshake signature', 400);
  }
}
