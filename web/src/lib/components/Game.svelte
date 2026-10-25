<script lang="ts">
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Connection } from '#lib/connection.svelte.js';
	import { GameState } from '#lib/game.svelte.js';
	import { playerName, playerToken, setPlayerName, socketUrl } from '#lib/identity.js';
	import type { ClientMsg, Move } from '#lib/protocol.js';
	import { onDestroy } from 'svelte';
	import { toast } from 'svelte-sonner';
	import Board from './Board.svelte';
	import GamePanel from './GamePanel.svelte';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

	interface Props {
		room: string;
	}

	let { room }: Props = $props();

	const token = playerToken();
	const game = new GameState();
	let name = $state(playerName());

	// Keyed on the room by the page, so this runs once per room.
	// svelte-ignore state_referenced_locally
	const connection = new Connection({
		url: socketUrl(room),
		onOpen: () => connection.send({ type: 'hello', token, name }),
		onMessage: (msg) => {
			if (msg.type === 'rejected') toast.error(msg.reason);
			if (!game.handle(msg)) connection.resync();
		}
	});
	onDestroy(() => connection.dispose());

	function send(msg: ClientMsg) {
		connection.send(msg);
	}

	function move(mv: Move) {
		send({ type: 'play', move_number: game.moveNumber, move: mv });
	}

	function play(vertex: number) {
		game.pending = vertex;
		game.rejection = null;
		move({ type: 'play', vertex });
	}

	function rename(next: string) {
		name = next.trim() || 'Anonymous';
		setPlayerName(name);
		send({ type: 'hello', token, name });
	}
</script>

<main class="mx-auto flex max-w-6xl flex-col gap-5 p-4 lg:flex-row lg:items-start lg:gap-8 lg:p-6">
	<div class="flex min-w-0 flex-1 justify-center">
		<Board
			size={game.size}
			board={game.board}
			lastMove={game.lastMove}
			pending={game.pending}
			dead={game.dead}
			territory={game.territory}
			scoring={game.scoring}
			myTurn={game.myTurn && game.bothSeated}
			myColor={game.you}
			onplay={play}
			ontoggledead={(vertex) => send({ type: 'toggle_dead', vertex })}
		/>
	</div>

	<GamePanel
		{game}
		{room}
		{name}
		status={connection.status}
		onclaim={(color) => send({ type: 'claim_seat', color })}
		onrelease={() => send({ type: 'release_seat' })}
		onpass={() => move({ type: 'pass' })}
		onresign={() => move({ type: 'resign' })}
		onaccept={() => send({ type: 'accept_score' })}
		onresume={() => send({ type: 'resume_play' })}
		onrename={rename}
	/>
</main>

{#if connection.status === 'desynchronized'}
	<div class="fixed inset-x-0 bottom-0 p-4">
		<Alert.Root variant="destructive" class="mx-auto max-w-md">
			<CircleAlertIcon />
			<Alert.Title>Lost contact with the server</Alert.Title>
			<Alert.Description>Reload the page to continue.</Alert.Description>
		</Alert.Root>
	</div>
{/if}
