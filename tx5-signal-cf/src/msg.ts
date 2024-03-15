import * as ed from '@noble/ed25519';

/*
 * [2 byte] u16_be msg len
 * [64 byte] ed25519 sig
 * [32 byte] src pk
 * [32 byte] dst pk
 * [4 byte] u32_be nonce
 * [N byte] msg
 */
export class Msg {
  signed: Uint8Array;

  constructor(signed: Uint8Array) {
    this.signed = signed;
  }

  static genMessageToSign(
    srcPubKey: Uint8Array,
    dstPubKey: Uint8Array,
    nonce: number,
    message: Uint8Array,
  ): Uint8Array {
    if (srcPubKey.byteLength !== 32) {
      throw new Error('invalid source pk length');
    }
    if (dstPubKey.byteLength !== 32) {
      throw new Error('invalid destination pk length');
    }
    if ((nonce | 0) !== nonce || nonce < 0 || nonce > 4294967295) {
      throw new Error('nonce must be an integer >= 0 && <= 4294967295');
    }
    if (message.byteLength > 4096 - 68 - 64 - 2) {
      throw new Error('msg too long');
    }
    const out = new Uint8Array(68 + message.byteLength);
    out.set(srcPubKey, 0);
    out.set(dstPubKey, 32);
    new DataView(out.buffer).setUint32(64, nonce, false);
    out.set(message, 68);
    return out;
  }

  static fromSignedVerify(signed: Uint8Array): Msg {
    const out = new Msg(signed);
    out.verify();

    return out;
  }

  static fromPartsVerify(signature: Uint8Array, message: Uint8Array): Msg {
    if (signature.byteLength !== 64) {
      throw new Error('invalid signature length');
    }
    if (message.byteLength < 68) {
      throw new Error('invalid message');
    }
    const len = signature.byteLength + message.byteLength + 2;
    if (len > 4096) {
      throw new Error('message too long');
    }

    const signed = new Uint8Array(len);
    new DataView(signed.buffer).setUint16(0, len, false);
    signed.set(signature, 2);
    signed.set(message, 66);

    return Msg.fromSignedVerify(signed);
  }

  verify() {
    if (
      !ed.verify(
        this.signed.subarray(2, 66),
        this.signed.subarray(66),
        this.signed.subarray(66, 98),
      )
    ) {
      throw new Error('invalid signature');
    }
  }

  len(): number {
    return new DataView(this.signed.buffer).getUint16(0, false);
  }

  srcPubKey(): Uint8Array {
    return this.signed.subarray(66, 98);
  }

  dstPubKey(): Uint8Array {
    return this.signed.subarray(98, 130);
  }

  nonce(): number {
    return new DataView(this.signed.buffer).getUint32(130, false);
  }

  message(): Uint8Array {
    return this.signed.subarray(134);
  }

  encoded(): Uint8Array {
    return this.signed.subarray(0);
  }
}
