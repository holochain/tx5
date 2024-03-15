import { describe, expect, assert, it, beforeAll, afterAll } from 'vitest';
import { Msg } from './msg.ts';
import * as ed from '@noble/ed25519';
import { sha512 } from '@noble/hashes/sha512';
ed.etc.sha512Sync = (...m) => sha512(ed.etc.concatBytes(...m));

describe('Msg', () => {
  it('construct and verify', async () => {
    const priv1 = ed.utils.randomPrivateKey();
    const pub1 = ed.getPublicKey(priv1);
    const priv2 = ed.utils.randomPrivateKey();
    const pub2 = ed.getPublicKey(priv2);

    const msg = Msg.genMessageToSign(pub1, pub2, 42, new Uint8Array(8));

    const badSig = ed.sign(msg, priv2);
    expect(() => {
      Msg.fromPartsVerify(badSig, msg);
    }).toThrow();

    const sig = ed.sign(msg, priv1);
    const signedOth = Msg.fromPartsVerify(sig, msg);

    // shouldn't throw
    signedOth.verify();

    const encoded = signedOth.encoded();
    const signed = Msg.fromSignedVerify(encoded);

    // shouldn't throw
    signed.verify();

    expect(signed).instanceOf(Msg);
    expect(signed.len()).toEqual(142);
    expect(signed.srcPubKey()).toEqual(pub1);
    expect(signed.dstPubKey()).toEqual(pub2);
    expect(signed.nonce()).toEqual(42);
    expect(signed.message()).toEqual(new Uint8Array(8));
  });
});
