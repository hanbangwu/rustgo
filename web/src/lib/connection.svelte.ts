import type { ClientMsg, ServerMsg } from './protocol.js';

export type ConnectionStatus = 'connecting' | 'connected' | 'disconnected' | 'desynchronized';

export type ConnectionOptions = {
	url: string;
	onMessage: (msg: ServerMsg) => void;
	onOpen: () => void;
	reconnectInterval?: number;
};

// Keeps idle connections open through proxies; the server answers without waking the room.
const HEARTBEAT_INTERVAL = 30 * 1000;
const MAX_RECONNECT_DELAY = 10 * 1000;

export class Connection {
	status = $state<ConnectionStatus>('connecting');

	#options: ConnectionOptions;
	#interval: number;
	#ws?: WebSocket;
	#connecting = false;
	#attempts = 0;
	#recentFailures = 0;
	#retryId?: number;
	#heartbeatId: number;
	#resetFailuresId: number;
	#disposed = false;

	constructor(options: ConnectionOptions) {
		this.#options = options;
		this.#interval = options.reconnectInterval ?? 1000;

		this.#tryConnect();
		this.#heartbeatId = window.setInterval(
			() => this.send({ type: 'heartbeat' }),
			HEARTBEAT_INTERVAL
		);
		this.#resetFailuresId = window.setInterval(
			() => (this.#recentFailures = 0),
			15 * this.#interval
		);
	}

	send(msg: ClientMsg): boolean {
		if (this.#ws?.readyState !== WebSocket.OPEN) return false;
		this.#ws.send(JSON.stringify(msg));
		return true;
	}

	dispose(): void {
		this.#disposed = true;
		window.clearTimeout(this.#retryId);
		window.clearInterval(this.#heartbeatId);
		window.clearInterval(this.#resetFailuresId);
		const ws = this.#ws;
		this.#ws = undefined;
		ws?.close();
	}

	#tryConnect(): void {
		if (this.#disposed || this.#connecting || this.#ws) return;
		this.#connecting = true;

		const ws = new WebSocket(this.#options.url);

		ws.onopen = () => {
			this.#connecting = false;
			this.#ws = ws;
			this.#attempts = 0;
			this.status = 'connected';
			this.#options.onOpen();
		};

		ws.onclose = () => {
			if (this.#ws === ws) {
				this.#ws = undefined;
				this.status = 'disconnected';
				if (++this.#recentFailures >= 5) {
					this.dispose();
					this.status = 'desynchronized';
					return;
				}
			} else {
				this.#connecting = false;
				this.status = 'disconnected';
			}
			this.#scheduleRetry();
		};

		ws.onerror = () => {};

		ws.onmessage = ({ data }) => {
			if (typeof data !== 'string') return;
			this.#options.onMessage(JSON.parse(data) as ServerMsg);
		};
	}

	/** Backs off while the server is unreachable. */
	#scheduleRetry(): void {
		if (this.#disposed) return;
		window.clearTimeout(this.#retryId);
		const delay = Math.min(this.#interval * 2 ** this.#attempts, MAX_RECONNECT_DELAY);
		this.#attempts++;
		this.#retryId = window.setTimeout(() => this.#tryConnect(), delay);
	}

	resync(): void {
		const ws = this.#ws;
		this.#ws = undefined;
		ws?.close();
	}
}
