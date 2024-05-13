import {
  common,
  createStarryNight,
} from "https://esm.sh/@wooorm/starry-night@3?bundle";
import { toDom } from "https://esm.sh/hast-util-to-dom@4?bundle";

const starryNight = await createStarryNight(common);
const prefix = "language-";

const nodes = Array.from(document.body.querySelectorAll("code"));

for (const node of nodes) {
  const className = Array.from(node.classList).find(function (d) {
    return d.startsWith(prefix);
  });
  if (!className) continue;
  const scope = starryNight.flagToScope(className.slice(prefix.length));
  if (!scope) continue;
  const tree = starryNight.highlight(node.textContent, scope);
  node.replaceChildren(toDom(tree, { fragment: true }));
}

class WSocket {
  constructor(endpoint) {
    this.endpoint = endpoint;
    this.ws = undefined;
    this.attempts = 0;
    this.autoReconnect = true;

    this._listeners = {
      message: [],
      reconnect: [],
    };
  }

  connect() {
    this.ws = new WebSocket(this.endpoint);
    this._attachListeners(this.ws);
  }

  close() {
    this.autoReconnect = false;
    this.ws?.close();
    this.ws = undefined;
  }

  addEventListener(event, listener) {
    this._listeners[event]?.push(listener);
  }

  _attachListeners(ws) {
    const onOpen = (_event) => {
      this.attempts = 0;
    };
    const onMessage = (event) => {
      this._listeners["message"].forEach((listener) => listener(event.data));
    };
    const onClose = (_event) => {
      ws.removeEventListener("open", onOpen);
      ws.removeEventListener("message", onMessage);
      ws.removeEventListener("close", onClose);

      if (this.autoReconnect) {
        setTimeout(() => {
          this.attempts = ++this.attempts > 10 ? 10 : this.attempts;
          this.ws.close();

          this._listeners["reconnect"].forEach((listener) =>
            listener(this.attempts),
          );

          this.connect();
        }, 1000 * this.attempts);
      }
    };

    ws.addEventListener("open", onOpen);
    ws.addEventListener("message", onMessage);
    ws.addEventListener("close", onClose);
  }
}

const webSocket = new WSocket(`ws://${location.host}/ws`);

webSocket.addEventListener("message", (message) => {
  console.log(message);
  if (
    message
      .toLowerCase()
      .includes(decodeURIComponent(window.location.pathname.toLowerCase()))
  ) {
    window.location.reload();
  }
});
webSocket.addEventListener("reconnect", (attempt) =>
  console.log("Trying to reconnect to websocket, current attempt: ", attempt),
);
webSocket.connect();
