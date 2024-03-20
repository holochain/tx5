/**
 * This is currently a PoC, but working towards something production ready.
 *
 * What we need to do:
 *
 * - batching: clients should batch requests for a number of milliseconds,
 *   sending within say 5ms if no additional messages are received, but never
 *   more than 20ms after the first received message. one batch should not
 *   exceed some byte count... 4096?
 * - signing: nodes will be identified by an ed25519 pub key, messages
 *   will be individually signed, like:
 *   `["sig-base64","[\"fwd\",42,\"dest-base64\",\"msg-content\"]"]`
 * - websocket hello: to verify the client can sign with that pubkey
 *   must be received within some milliseconds of establishing the ws connection
 *   set a DO "alarm"?
 *   `["sig-base64","[\"hello\",0]"]`
 * - error if multiple websockets connected to one pubkey do
 * - ratelimit messages within a single do instance
 * - ratelimit messages (at larger rate) for an ip
 * - nonce/message id must be zero for the hello message
 * - nonce/message id must start at 1 for each peer communicated with
 *   and increment by exactly 1 for each additional message sent.
 *   we will need to maintain a map of these peer message ids that
 *   must be reset/cleared if the websocket is closed.
 */

import { RateLimit } from './rate-limit.ts';
import { verifySignedQuery, verifySignedHandshake } from './msg.ts';
import { err } from './err.ts';
import { toB64Url, fromB64Url } from './b64.ts';

export interface Env {
  SIGNAL: DurableObjectNamespace;
  RATE_LIMIT: DurableObjectNamespace;
}

async function ipRateLimit(env: Env, ip: string) {
  try {
    const ipId = env.RATE_LIMIT.idFromName(ip);
    const ipStub = env.RATE_LIMIT.get(ipId);
    const res = await ipStub.fetch(new Request(`http://do`));
    if (res.status !== 200) {
      throw err(`limit bad status ${res.status}`, 429);
    }
    const { limit } = (await res.json()) as { limit: number };
    if (limit > 0) {
      throw err(`limit ${limit}`, 429);
    }
  } catch (e) {
    throw err(`limit ${e}`, 429);
  }
}

function checkNonce(nonce: number) {
  const now = Date.now();
  if (nonce < now - 5000) {
    throw err('nonce older than 5 sec, update clock, or call /now', 400);
  }
  if (nonce > now + 5000) {
    throw err('nonce newer than 5 sec, update clock, or call /now', 400);
  }
}

export default {
  async fetch(
    request: Request,
    env: Env,
    ctx: ExecutionContext,
  ): Promise<Response> {
    try {
      const ip = request.headers.get('cf-connecting-ip') || 'no-ip';

      await ipRateLimit(env, ip);

      const method = request.method;
      const url = new URL(request.url);

      // @ts-expect-error // cf typescript bug //
      const path: string = url.pathname;

      // TODO - check headers for content-length / chunked encoding and reject?

      if (method !== 'GET') {
        throw err('expected GET', 400);
      }

      if (path === '/now') {
        return Response.json({ now: Date.now() });
      }

      if (path !== '/') {
        throw err('expected root path ("/")', 400);
      }

      // @ts-expect-error // cf typescript bug //
      const nodeId: string = url.searchParams.get('k');
      // @ts-expect-error // cf typescript bug //
      const n: string = url.searchParams.get('n');
      // @ts-expect-error // cf typescript bug //
      const s: string = url.searchParams.get('s');

      const q = verifySignedQuery(nodeId, n, s);
      checkNonce(q.nonce);

      // DO instanced by our nodeId
      const id = env.SIGNAL.idFromName(nodeId);
      const stub = env.SIGNAL.get(id);

      // just forward the full request / response
      return await stub.fetch(request);
    } catch (e: any) {
      return new Response(JSON.stringify({ err: e.toString() }), {
        status: e.status || 500,
      });
    }
  },
};

export class DoRateLimit implements DurableObject {
  state: DurableObjectState;
  env: Env;
  rl: RateLimit;

  constructor(state: DurableObjectState, env: Env) {
    this.state = state;
    this.env = env;
    this.rl = new RateLimit(5000);
  }

  async fetch(request: Request): Promise<Response> {
    return Response.json({ limit: this.rl.trackRequest(Date.now(), 1) });
  }
}

export class DoSignal implements DurableObject {
  state: DurableObjectState;
  env: Env;
  rl: RateLimit;

  constructor(state: DurableObjectState, env: Env) {
    this.state = state;
    this.env = env;
    this.rl = new RateLimit(5000);
  }

  async fetch(request: Request): Promise<Response> {
    return await this.state.blockConcurrencyWhile(async () => {
      try {
        const ip = request.headers.get('cf-connecting-ip') || 'no-ip';
        const url = new URL(request.url);
        // @ts-expect-error // cf typescript bug //
        const path: string = url.pathname;

        if (path === '/fwd') {
          const message = await request.arrayBuffer();
          for (const ws of this.state.getWebSockets()) {
            ws.send(message);
          }
          return new Response('ok');
        } else if (path === '/') {
          if (this.state.getWebSockets().length > 0) {
            throw err('websocket already connected', 400);
          }
          if (request.headers.get('Upgrade') !== 'websocket') {
            throw err('expected websocket', 426);
          }

          const [client, server] = Object.values(new WebSocketPair());

          this.state.acceptWebSocket(server);

          const nonce = new Uint8Array(32);
          crypto.getRandomValues(nonce);

          // @ts-expect-error // cf typescript bug //
          const nodeId = fromB64Url(url.searchParams.get('k'));

          server.serializeAttachment({
            nodeId,
            ip,
            nonce,
            valid: false,
          });

          server.send(nonce);

          return new Response(null, { status: 101, webSocket: client });
        } else {
          throw err('invalid path', 400);
        }
      } catch (e: any) {
        return new Response(JSON.stringify({ err: e.toString() }), {
          status: e.status || 500,
        });
      }
    });
  }

  // handle incoming websocket messages
  async webSocketMessage(ws: WebSocket, message: ArrayBuffer | string) {
    await this.state.blockConcurrencyWhile(async () => {
      try {
        const { nodeId, ip, nonce, valid } = ws.deserializeAttachment();
        if (!nodeId) {
          throw err('no associated nodeId');
        }
        if (!ip) {
          throw err('no associated ip');
        }

        await ipRateLimit(this.env, ip);

        if (this.rl.trackRequest(Date.now(), 20) > 0) {
          throw err('rate limit', 429);
        }

        // convert strings into binary
        let msg: Uint8Array;
        if (message instanceof ArrayBuffer) {
          msg = new Uint8Array(message);
        } else {
          const enc = new TextEncoder();
          msg = enc.encode(message);
        }

        if (!valid) {
          if (!nonce) {
            throw err('no associated nonce');
          }
          verifySignedHandshake(nodeId, nonce, msg);
          ws.serializeAttachment({
            nodeId,
            ip,
            nonce: true, // don't need to keep the actual nonce anymore
            valid: true,
          });
        } else {
          if (msg.byteLength < 32) {
            throw err('invalid destination', 400);
          }

          const dest = new Uint8Array(32);
          dest.set(msg.subarray(0, 32));
          msg.set(nodeId);

          const req = new Request('http://do/fwd', {
            method: 'POST',
            body: msg,
          });

          const id = this.env.SIGNAL.idFromName(toB64Url(dest));
          const stub = this.env.SIGNAL.get(id);

          // intentionally ignore errors here
          await stub.fetch(req);
        }
      } catch (e: any) {
        ws.close(4000 + (e.status || 500), e.toString());
      }
    });
  }

  // if the websocket is closed... close the websocket??
  async webSocketClose(
    ws: WebSocket,
    code: number,
    reason: string,
    wasClean: boolean,
  ) {
    ws.close(code, reason);
  }
}
