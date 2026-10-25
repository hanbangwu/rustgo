import { fetchRoom } from '#lib/identity.js';
import type { PageLoad } from './$types';

// Not awaited, so the page can show that the server is waking up instead of a blank screen.
export const load: PageLoad = ({ params, fetch }) => ({
	id: params.room,
	room: fetchRoom(params.room, fetch)
});
