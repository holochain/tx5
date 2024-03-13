export interface Env {
  MY_DURABLE_OBJECT: DurableObjectNamespace;
}

// The first path element is the nodeId (ident)
// The second path element is the api command (api)
function parsePath(url: string) {
  const pathParts = new URL(url).pathname.split('/');
  const ident = pathParts[1] || '';
  const api = pathParts[2] || '';
  return { ident, api };
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    const { ident, api } = parsePath(request.url);

    // DO instanced by our nodeId (ident)
    const id = env.MY_DURABLE_OBJECT.idFromName(ident);
    const stub = env.MY_DURABLE_OBJECT.get(id);

    // just forward the full request / response
    return await stub.fetch(request);
  },
};

export class MyDurableObject implements DurableObject {
  state: DurableObjectState;
  env: Env;
  ident: string;

  constructor(state: DurableObjectState, env: Env) {
    this.state = state;
    this.env = env;
    this.ident = '';
  }

  async fetch(request: Request): Promise<Response> {
    try {
      const { ident, api } = parsePath(request.url);

      // store our identity... would be nice if this was in the constructor,
      // but we are instantiated by this always so this is equivalent.
      this.ident = ident;

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
      } else if (api === 'count') {
        // this is just a debugging api, TODO - delete this

        const count = this.state.getWebSockets().length;
        return new Response(`websocket count: ${count}`);
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
    } catch (e) {
      return new Response(JSON.stringify({ c: 'err', e: e.toString() }), { status: 500 });
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
        const id = this.env.MY_DURABLE_OBJECT.idFromName(m.n);
        const stub = this.env.MY_DURABLE_OBJECT.get(id);

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
  async webSocketClose(ws: WebSocket, code: number, reason: string, wasClean: boolean) {
    ws.close(code, reason);
  }
}
