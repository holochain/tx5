import { describe, expect, assert, it, beforeAll, afterAll } from 'vitest';
import { encodeQuery, encodeSignedQuery, verifySignedQuery } from './msg.ts';
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

      const s = ed.sign(new TextEncoder().encode(q), sk);

      const sq = new URL('none:?' + encodeSignedQuery(s, q));

      // @ts-expect-error // cf typescript bug //
      const ddk: string = sq.searchParams.get('k');
      // @ts-expect-error // cf typescript bug //
      const ddn: string = sq.searchParams.get('n');
      // @ts-expect-error // cf typescript bug //
      const dds: string = sq.searchParams.get('s');

      verifySignedQuery(ddk, ddn, dds);
    }
  });
});
