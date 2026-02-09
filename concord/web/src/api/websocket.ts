import type { ClientCommand, ServerEvent } from './types';

type EventHandler = (event: ServerEvent) => void;
type StatusHandler = (connected: boolean) => void;

const RECONNECT_DELAYS = [1000, 2000, 4000, 8000, 15000];

export class WebSocketManager {
  private ws: WebSocket | null = null;
  private url: string;
  private onEvent: EventHandler;
  private onStatus: StatusHandler;
  private reconnectAttempt = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private intentionalClose = false;

  constructor(url: string, onEvent: EventHandler, onStatus: StatusHandler) {
    this.url = url;
    this.onEvent = onEvent;
    this.onStatus = onStatus;
  }

  connect() {
    this.intentionalClose = false;
    this.cleanup();

    const ws = new WebSocket(this.url);
    this.ws = ws;

    ws.onopen = () => {
      this.reconnectAttempt = 0;
      this.onStatus(true);
    };

    ws.onmessage = (e) => {
      try {
        const event: ServerEvent = JSON.parse(e.data);
        this.onEvent(event);
      } catch {
        console.warn('Invalid WebSocket message:', e.data);
      }
    };

    ws.onclose = () => {
      this.onStatus(false);
      if (!this.intentionalClose) {
        this.scheduleReconnect();
      }
    };

    ws.onerror = () => {
      // onclose will fire after this
    };
  }

  send(command: ClientCommand) {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(command));
    }
  }

  disconnect() {
    this.intentionalClose = true;
    this.cleanup();
  }

  private cleanup() {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.ws) {
      this.ws.onopen = null;
      this.ws.onmessage = null;
      this.ws.onclose = null;
      this.ws.onerror = null;
      if (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING) {
        this.ws.close();
      }
      this.ws = null;
    }
  }

  private scheduleReconnect() {
    const delay = RECONNECT_DELAYS[Math.min(this.reconnectAttempt, RECONNECT_DELAYS.length - 1)];
    this.reconnectAttempt++;
    console.log(`WebSocket reconnecting in ${delay}ms (attempt ${this.reconnectAttempt})`);
    this.reconnectTimer = setTimeout(() => this.connect(), delay);
  }
}
