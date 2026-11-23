import type { Room, Settings } from './protocol.js';

const TOKEN_KEY = 'rustgo:player-token';
const NAME_KEY = 'rustgo:player-name';

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

export async function createRoom(settings: Settings, fetcher = fetch): Promise<Room> {
	const response = await fetcher(apiUrl('api/rooms'), {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		body: JSON.stringify(settings)
	});
	if (!response.ok) throw new Error((await response.text()) || response.statusText);
	return response.json();
}

/** The room's settings, or null if it does not exist (or has expired). */
export async function fetchRoom(id: string, fetcher = fetch): Promise<Room | null> {
	const response = await fetcher(apiUrl(`api/rooms/${encodeURIComponent(id)}`));
	if (response.status === 404 || response.status === 400) return null;
	if (!response.ok) throw new Error(response.statusText);
	return response.json();
}
