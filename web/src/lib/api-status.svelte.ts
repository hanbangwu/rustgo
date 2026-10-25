/** Whether the igo server seems to be asleep. Free hosts can take a minute to wake it. */
class ServerStatus {
	slowRequests = $state(0);
	socketDown = $state(false);

	get waking(): boolean {
		return this.slowRequests > 0 || this.socketDown;
	}
}

export const server = new ServerStatus();
