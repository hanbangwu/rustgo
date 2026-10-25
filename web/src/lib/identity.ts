import type { Room, Settings } from './protocol.js';
import { server } from './api-status.svelte.js';

const TOKEN_KEY = 'igo:player-token';
const NAME_KEY = 'igo:player-name';

export function playerToken(): string {
	let token = localStorage.getItem(TOKEN_KEY);
	if (!token) {
		token = crypto.randomUUID();
		localStorage.setItem(TOKEN_KEY, token);
	}
	return token;
}

export function playerName(): string {
	let name = localStorage.getItem(NAME_KEY);
	if (!name) {
		name = randomName();
		localStorage.setItem(NAME_KEY, name);
	}
	return name;
}

export function setPlayerName(name: string): void {
	localStorage.setItem(NAME_KEY, name);
}

const ADJECTIVES = ['Quiet', 'Steady', 'Patient', 'Bold', 'Clever', 'Calm', 'Sharp', 'Gentle'];
const NOUNS = ['Crane', 'Turtle', 'Fox', 'Heron', 'Badger', 'Otter', 'Hare', 'Carp'];

function randomName(): string {
	const pick = <T>(xs: T[]): T => xs[Math.floor(Math.random() * xs.length)];
	return `${pick(ADJECTIVES)} ${pick(NOUNS)}`;
}

function apiUrl(path: string): URL {
	const base = import.meta.env.VITE_API_URL;
	return new URL(path, base ? base.replace(/\/*$/, '/') : window.location.origin);
}

export function socketUrl(room: string): string {
	const url = apiUrl(`api/socket/${room}`);
	url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
	return url.href;
}

export function sgfUrl(room: string): string {
	return apiUrl(`api/sgf/${room}`).href;
}

/** Starts waking the server early, so it is up by the time it is needed. */
export function wakeServer(): void {
	fetch(apiUrl('api/health'), { mode: 'no-cors' }).catch(() => {});
}

// How long a sleeping free server may take to come back, with some slack.
const WAKE_DEADLINE = 90 * 1000;
const SLOW_AFTER = 2 * 1000;
const MAX_RETRY_DELAY = 8 * 1000;

/**
 * Fetches from the API, retrying while the server is unreachable or its host
 * answers that it is not ready yet, as happens while a free server wakes up.
 */
async function request(path: string, init: RequestInit = {}, fetcher = fetch): Promise<Response> {
	const started = Date.now();
	let slow = false;
	const markSlow = () => {
		if (!slow) server.slowRequests++;
		slow = true;
	};
	const slowId = window.setTimeout(markSlow, SLOW_AFTER);
	try {
		for (let delay = 1000; ; delay = Math.min(delay * 2, MAX_RETRY_DELAY)) {
			let failure: unknown;
			try {
				const response = await fetcher(apiUrl(path), init);
				if (![502, 503, 504].includes(response.status)) return response;
				failure = new Error(response.statusText);
			} catch (e) {
				failure = e;
			}
			if (Date.now() - started + delay > WAKE_DEADLINE) throw failure;
			markSlow();
			await new Promise((resolve) => setTimeout(resolve, delay));
		}
	} finally {
		window.clearTimeout(slowId);
		if (slow) server.slowRequests--;
	}
}

export async function createRoom(settings: Settings, fetcher = fetch): Promise<Room> {
	const response = await request(
		'api/rooms',
		{
			method: 'POST',
			headers: { 'Content-Type': 'application/json' },
			body: JSON.stringify(settings)
		},
		fetcher
	);
	if (!response.ok) throw new Error((await response.text()) || response.statusText);
	return response.json();
}

/** The room's settings, or null if it does not exist (or has expired). */
export async function fetchRoom(id: string, fetcher = fetch): Promise<Room | null> {
	const response = await request(`api/rooms/${encodeURIComponent(id)}`, {}, fetcher);
	if (response.status === 404 || response.status === 400) return null;
	if (!response.ok) throw new Error(response.statusText);
	return response.json();
}
