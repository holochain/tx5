import { ed } from './ed.ts';
import { fromUint8Array, toUint8Array } from 'js-base64';

function toUrlSafe(s: Uint8Array): string {
  return fromUint8Array(s)
    .replace(/\=/g, '')
    .replace(/\+/g, '-')
    .replace(/\//g, '_');
}

function fromUrlSafe(s: string): Uint8Array {
  return toUint8Array(s.replace(/\-/g, '+').replace(/\_/g, '/'));
}

interface Query {
  nodePubKey: Uint8Array;
  nonce: number;
}

export function encodeQuery(query: Query): string {
  const k = toUrlSafe(query.nodePubKey);
  return `k=${k}&n=${query.nonce}`;
}

export function encodeSignedQuery(
  signature: Uint8Array,
  query: string,
): string {
  const s = toUrlSafe(signature);
  return `${query}&s=${s}`;
}

export function verifySignedQuery(k: string, n: string, s: string): Query {
  const query = {
    nodePubKey: fromUrlSafe(k),
    nonce: parseFloat(n),
  };
  const enc = new TextEncoder('utf8').encode(encodeQuery(query));
  const sig = fromUrlSafe(s);
  if (!ed.verify(sig, enc, query.nodePubKey)) {
    throw new Error('invalid signature');
  }
  return query;
}

interface Message {
  srcPubKey: Uint8Array;
  dstPubKey: Uint8Array;
  nonce: number;
  message: Uint8Array;
}

export function encodeMessage(message: Message): string {
  return JSON.stringify({
    s: fromUint8Array(message.srcPubKey),
    d: fromUint8Array(message.dstPubKey),
    n: message.nonce,
    m: fromUint8Array(message.message),
  });
}

export function decodeMessage(message: string): Message {
  const tmp = JSON.parse(message);
  const out = {};
  if ('s' in tmp) {
    out.srcPubKey = toUint8Array(tmp.s);
  } else {
    throw new Error('srcPubKey required field "s"');
  }
  if ('d' in tmp) {
    out.dstPubKey = toUint8Array(tmp.d);
  } else {
    throw new Error('dstPubKey required field "d"');
  }
  if ('n' in tmp) {
    out.nonce = tmp.n;
  } else {
    throw new Error('nonce required field "n"');
  }
  if ('m' in tmp) {
    out.message = toUint8Array(tmp.m);
  } else {
    throw new Error('message required field "m"');
  }
  return out;
}

interface SignedMessage {
  signature: Uint8Array;
  message: string;
}

export function encodeSignedMessage(signed: SignedMessage): string {
  return JSON.stringify({
    s: fromUint8Array(signed.signature),
    m: signed.message,
  });
}

export function verifySignedMessage(signed: string): Message {
  const tmp = JSON.parse(signed);

  let signature;
  if ('s' in tmp) {
    signature = toUint8Array(tmp.s);
  } else {
    throw new Error('signature required field "s"');
  }

  let msg;
  if ('m' in tmp) {
    msg = decodeMessage(tmp.m);
  } else {
    throw new Error('message required field "m"');
  }

  const toVerify = new TextEncoder('utf8').encode(tmp.m);
  if (!ed.verify(signature, toVerify, msg.srcPubKey)) {
    throw new Error('invalid signature');
  }

  return msg;
}
