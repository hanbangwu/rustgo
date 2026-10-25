import { fetchRoom } from '#lib/identity.js';
import type { PageLoad } from './$types';

export const load: PageLoad = ({ params, fetch }) => ({
	id: params.room,
	room: fetchRoom(params.room, fetch)
});
