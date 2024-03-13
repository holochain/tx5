import { unstable_dev } from 'wrangler';
import type { UnstableDevWorker } from 'wrangler';
import { describe, expect, assert, it, beforeAll, afterAll } from 'vitest';
import WebSocket from 'ws';

type RecvFunction = (msg: Array<string>) => void;

// helper class for client websocket testing
class C {
  ws: WebSocket;
  pend: Array<string>;
  res: RecvFunction | null;

  constructor(ws: WebSocket) {
    this.ws = ws;
    this.pend = [];
    this.res = null;
  }

  static connect(url: string): Promise<C> {
    return new Promise((res, _) => {
      const ws = new WebSocket(url);
      const out = new C(ws);
      ws.on('error', (err) => {
        console.error(`error: ${err}`);
        throw err;
      });
      ws.on('open', () => {
        res(out);
      });
      ws.on('message', (msg) => {
        msg = msg.toString('utf8');
        out.pend.push(msg);
        if (out.res) {
          const res = out.res;
          out.res = null;
          res(out.pend.splice(0));
        }
      });
    });
  }

  send(data: string) {
    this.ws.send(data);
  }

  recv(): Promise<Array<string>> {
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

  it('count', async () => {
    const res = await worker.fetch('http://' + addr + '/blabla/count');
    const txt = await res.text();
    assert(txt.startsWith('websocket count: '));
    expect(res.status).toBe(200);
  });

  it('websocket', async () => {
    const msgList = [];
    const wsA = await C.connect('ws://' + addr + '/nodeA/websocket');
    const wsB = await C.connect('ws://' + addr + '/nodeB/websocket');
    const recvPromiseA = wsA.recv();
    const recvPromiseB = wsB.recv();
    wsA.send(
      JSON.stringify({
        c: 'fwd',
        n: 'nodeB',
        d: 'hello',
      }),
    );
    const resA = JSON.parse((await recvPromiseA)[0]);
    expect(resA.c).toBe('resp');
    const resAD = JSON.parse(resA.d);
    expect(resAD).toStrictEqual({ status: 200, text: 'ok' });
    const resB = JSON.parse((await recvPromiseB)[0]);
    expect(resB).toStrictEqual({ c: 'fwd', s: 'nodeA', d: 'hello' });
  });
});
