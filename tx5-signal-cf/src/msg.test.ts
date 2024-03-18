import { describe, expect, assert, it, beforeAll, afterAll } from 'vitest';
import {
  encodeQuery,
  encodeSignedQuery,
  verifySignedQuery,
  encodeMessage,
  decodeMessage,
  encodeSignedMessage,
  verifySignedMessage,
} from './msg.ts';
import { ed } from './ed.ts';

describe('Msg', () => {
  it('query safe', async () => {
    for (let i = 0; i < 100; ++i) {
      const sk = ed.utils.randomPrivateKey();
      const pk = ed.getPublicKey(sk);

      const q = encodeQuery({
        nodePubKey: pk,
        nonce: Date.now(),
      });

      const s = ed.sign(new TextEncoder('utf8').encode(q), sk);

      const sq = new URL('none:?' + encodeSignedQuery(s, q));

      const ddk = sq.searchParams.get('k');
      const ddn = sq.searchParams.get('n');
      const dds = sq.searchParams.get('s');

      verifySignedQuery(ddk, ddn, dds);
    }
  });

  it('construct and verify', async () => {
    const priv1 = ed.utils.randomPrivateKey();
    const pub1 = ed.getPublicKey(priv1);
    const priv2 = ed.utils.randomPrivateKey();
    const pub2 = ed.getPublicKey(priv2);
    const nonce = Date.now();

    const msg = encodeMessage({
      srcPubKey: pub1,
      dstPubKey: pub2,
      nonce,
      message: new Uint8Array(8),
    });

    const encMsg = new TextEncoder('utf8').encode(msg);

    const badSig = ed.sign(encMsg, priv2);
    const badSigned = encodeSignedMessage({
      signature: badSig,
      message: msg,
    });
    expect(() => {
      verifySignedMessage(badSigned);
    }).toThrow();

    const sig = ed.sign(encMsg, priv1);
    const signed = encodeSignedMessage({
      signature: sig,
      message: msg,
    });

    const msgParsed = verifySignedMessage(signed);

    expect(msgParsed.srcPubKey).toEqual(pub1);
    expect(msgParsed.dstPubKey).toEqual(pub2);
    expect(msgParsed.nonce).toEqual(nonce);
    expect(msgParsed.message).toEqual(new Uint8Array(8));
  });
});
