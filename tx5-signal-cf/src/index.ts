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
import { verifySignedQuery, verifySignedMessage } from './msg.ts';
import { fromUint8Array } from 'js-base64';

export interface Env {
  SIGNAL: DurableObjectNamespace;
  IP_RATE_LIMIT: DurableObjectNamespace;
}

async function ipRateLimit(env: Env, ip: string) {
  try {
    const ipId = env.RATE_LIMIT.idFromName(ip);
    const ipStub = env.RATE_LIMIT.get(ipId);
    const res = await ipStub.fetch(new Request(`http://do`));
    const { limit } = await res.json();
    if (limit > 0) {
      throw new Error(`limit ${limit}`);
    }
  } catch (err) {
    throw new Error(`limit ${err}`);
  }
}

function checkNonce(last: number, nonce: number) {
  if (nonce < last) {
    throw new Error('received earlier nonce, replay?');
  }
  if (nonce < Date.now() - 5000) {
    throw new Error('nonce older than 5 sec, update clock?');
  }
}

function sleep(ms: number): Promise<null> {
  return new Promise((res, _) => {
    setTimeout(ms, () => {
      res(null);
    });
  });
}

export default {
  async fetch(
    request: Request,
    env: Env,
    ctx: ExecutionContext,
  ): Promise<Response> {
    let ip = request.headers.get('cf-connecting-ip') || 'no-ip';

    try {
      await ipRateLimit(env, ip);
    } catch (err) {
      console.error('limit', err);
      return new Response(err, { status: 429 });
    }

    const method = request.method;
    const url = new URL(request.url);
    const path = url.pathname;
    const body = await request.arrayBuffer();

    if (method !== 'GET') {
      return new Response('expected GET', { status: 400 });
    }

    if (path !== '/') {
      return new Response('expected root path ("/")', { status: 400 });
    }

    if (body.byteLength > 0) {
      return new Response('invalid body content', { status: 400 });
    }

    let nodeId = '';

    try {
      const k = url.searchParams.get('k');
      const n = url.searchParams.get('n');
      const s = url.searchParams.get('s');
      const q = verifySignedQuery(k, n, s);
      checkNonce(0, q.nonce);
      nodeId = fromUint8Array(q.nodePubKey);
    } catch (e) {
      console.error(e);
      return new Response(`invalid signature ${e}`, { status: 400 });
    }

    // DO instanced by our nodeId
    const id = env.SIGNAL.idFromName(nodeId);
    const stub = env.SIGNAL.get(id);

    // just forward the full request / response
    return await stub.fetch(request);
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

  constructor(state: DurableObjectState, env: Env) {
    this.state = state;
    this.env = env;
  }

  async fetch(request: Request): Promise<Response> {
    try {
      if (request.headers.get('Upgrade') !== 'websocket') {
        return new Response('Expected Websocket', { status: 426 });
      }

      const [client, server] = Object.values(new WebSocketPair());

      this.state.acceptWebSocket(server);

      return new Response(null, { status: 101, webSocket: client });

      /*
      // switch based of the api command
      if (api === 'websocket') {
        // we are a websocket

        // expect an upgrade
        if (request.headers.get('Upgrade') !== 'websocket') {
          return new Response('Expected Websocket', { status: 426 });
        }

        // get the websocket pair
        const [client, server] = Object.values(new WebSocketPair());

        // this makes this a hibernating websocket
        // so we don't use up a bunch of cpu time...
        // the signal protocol is going to be very sparse
        this.state.acceptWebSocket(server);

        // notify the client of the successful websocket upgrade
        return new Response(null, { status: 101, webSocket: client });
      } else if (api === 'fwd') {
        // a different durable object has forwarded a message to us
        // we need to pass it back down to our connected websocket(s)

        const data = JSON.parse(await request.text());

        // the normal use-case would be for a client to only be connected
        // to their DO websocket server once... but if they are muliply
        // connected, just forward the message to all of them
        for (const ws of this.state.getWebSockets()) {
          ws.send(JSON.stringify({ c: 'fwd', s: data.s, d: data.d }));
        }

        // let the other DO know we succeeded
        return new Response('ok');
      } else {
        throw new Error(`bad api: ${api}`);
      }
      */
    } catch (e) {
      return new Response(JSON.stringify({ c: 'err', e: e.toString() }), {
        status: 500,
      });
    }
  }

  // handle incoming websocket messages
  async webSocketMessage(ws: WebSocket, message: ArrayBuffer | string) {
    try {
      // convert binary messages into javascript strings
      if (message instanceof ArrayBuffer) {
        const dec = new TextDecoder('utf-8');
        message = dec.decode(message);
      }

      const m = JSON.parse(message);

      const c = m.c || '[unknown]';

      // switch on the command in the json message
      if (c === 'fwd') {
        // the client has asked us to forward a message to a different peer

        // get the DO that corresponds to that peer
        const id = this.env.SIGNAL.idFromName(m.n);
        const stub = this.env.SIGNAL.get(id);

        // make up the request so we can forward the message
        const req = new Request(`http://do/${m.n}/fwd`, {
          method: 'POST',
          body: JSON.stringify({ c: 'fwd', s: this.ident, d: m.d }),
        });

        // actually forward the message
        const resp = await stub.fetch(req);

        // parse the response
        const out = {
          status: resp.status,
          text: await resp.text(),
        };

        // return the response to the node that requested the forward
        ws.send(JSON.stringify({ c: 'resp', d: JSON.stringify(out) }));
      } else {
        throw new Error(`invalid c: ${c}`);
      }
    } catch (e) {
      ws.send(JSON.stringify({ c: 'err', e: e.toString() }));
    }
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
