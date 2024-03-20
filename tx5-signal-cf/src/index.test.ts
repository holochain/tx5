import { unstable_dev } from 'wrangler';
import type { UnstableDevWorker } from 'wrangler';
import { describe, expect, assert, it, beforeAll, afterAll } from 'vitest';
import WebSocket from 'ws';
import { ed } from './ed.ts';
import { encodeQuery, encodeSignedQuery } from './msg.ts';

type RecvFunction = (msg: Array<Uint8Array>) => void;

// helper class for client websocket testing
class C {
  ws: WebSocket;
  sk: Uint8Array;
  pk: Uint8Array;
  pend: Array<Uint8Array>;
  res: RecvFunction | null;
  closing: boolean;

  constructor(sk: Uint8Array, pk: Uint8Array, ws: WebSocket) {
    this.sk = sk;
    this.pk = pk;
    this.ws = ws;
    this.pend = [];
    this.res = null;
    this.closing = false;
  }

  static connect(addr: string): Promise<C> {
    return new Promise((res, _) => {
      const sk = ed.utils.randomPrivateKey();
      const pk = ed.getPublicKey(sk);
      const q = encodeQuery({
        nodePubKey: pk,
        nonce: Date.now(),
      });
      const s = ed.sign(new TextEncoder().encode(q), sk);
      const sq = encodeSignedQuery(s, q);

      const ws = new WebSocket(`ws://${addr}/?${sq}`);
      const out = new C(sk, pk, ws);

      ws.on('error', (err) => {
        console.error(`error: ${err}`);
        throw err;
      });
      ws.on('open', async () => {
        const list = await out.recv();
        if (list.length !== 1) {
          throw new Error('invalid recv length');
        }
        const nonce = list[0];
        const handshake = ed.sign(nonce, out.sk);
        await out._send(handshake);
        res(out);
      });
      ws.on('close', (code, reason) => {
        if (!out.closing) {
          const v = `close code ${code} reason ${reason}`;
          console.error(v);
          throw new Error(v);
        }
      });
      ws.on('message', (msg) => {
        if (!(msg instanceof Uint8Array)) {
          throw new Error('invalid rcv message type');
        }
        out.pend.push(msg);
        if (out.res) {
          const res = out.res;
          out.res = null;
          res(out.pend.splice(0));
        }
      });
    });
  }

  close() {
    this.closing = true;
    this.ws.close();
  }

  pubKey(): Uint8Array {
    return this.pk;
  }

  _send(data: Uint8Array): Promise<null> {
    return new Promise((res, _) => {
      this.ws.send(data, {}, (e: any) => {
        if (e) {
          throw e;
        }
        res(null);
      });
    });
  }

  async send(peer: Uint8Array, data: string) {
    const enc = new TextEncoder().encode(data);
    const out = new Uint8Array(peer.byteLength + enc.byteLength);
    out.set(peer);
    out.set(enc, peer.byteLength);
    await this.ws.send(out);
  }

  recv(): Promise<Array<Uint8Array>> {
    if (this.pend.length) {
      return Promise.resolve(this.pend.splice(0));
    }
    return new Promise((res, _) => {
      this.res = res;
    });
  }
}

describe('Worker', () => {
  let worker: UnstableDevWorker;
  let addr: string;

  beforeAll(async () => {
    worker = await unstable_dev('src/index.ts');
    addr = worker.address + ':' + worker.port;
  });

  afterAll(async () => {
    await worker.stop();
  });

  it('now', async () => {
    const res = await worker.fetch('http://' + addr + '/now');
    const json = (await res.json()) as { now: number };
    expect(json.now).to.be.above(Date.now() - 5000);
    expect(json.now).to.be.below(Date.now() + 5000);
  });

  it('websocket', async () => {
    const wsA = await C.connect(addr);
    const wsB = await C.connect(addr);

    const recvPromiseA = wsA.recv();
    await wsB.send(wsA.pubKey(), 'test');
    const result = (await recvPromiseA)[0];

    wsA.close();
    wsB.close();

    expect(new Uint8Array(result).subarray(0, 32)).toEqual(wsB.pubKey());
    expect(new TextDecoder().decode(result.subarray(32))).toEqual('test');
  });
});
