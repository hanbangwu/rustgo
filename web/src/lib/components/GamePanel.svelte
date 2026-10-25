<script lang="ts">
	import { resolve } from '$app/paths';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import type { ConnectionStatus } from '#lib/connection.svelte.js';
	import type { GameState } from '#lib/game.svelte.js';
	import { sgfUrl } from '#lib/identity.js';
	import { blackTotal, describeResult, whiteTotal, type Color } from '#lib/protocol.js';
	import { cn } from '#lib/utils.js';
	import CheckIcon from '@lucide/svelte/icons/check';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import DownloadIcon from '@lucide/svelte/icons/download';
	import PlusIcon from '@lucide/svelte/icons/plus';

	interface Props {
		game: GameState;
		room: string;
		status: ConnectionStatus;
		name: string;
		onclaim: (color: Color) => void;
		onrelease: () => void;
		onpass: () => void;
		onresign: () => void;
		onaccept: () => void;
		onresume: () => void;
		onrename: (name: string) => void;
	}

	let {
		game,
		room,
		status,
		name,
		onclaim,
		onrelease,
		onpass,
		onresign,
		onaccept,
		onresume,
		onrename
	}: Props = $props();

	let copied = $state(false);

	const shareUrl = $derived(new URL(resolve('/[room]', { room }), window.location.origin).href);
	const canRelease = $derived(game.you !== null && game.moveNumber === 0);
	const iAccepted = $derived(game.you !== null && game.accepted.includes(game.you));

	async function copyLink() {
		try {
			await navigator.clipboard.writeText(shareUrl);
			copied = true;
			setTimeout(() => (copied = false), 1500);
		} catch {
			copied = false;
		}
	}

	function occupant(color: Color): string | null {
		return color === 'black' ? game.seats.black : game.seats.white;
	}
</script>

<aside class="flex w-full flex-col gap-4 lg:w-80 lg:shrink-0">
	<header class="flex items-center justify-between gap-2">
		<a href={resolve('/')} class="text-xl font-semibold tracking-wide">igo</a>
		<div class="flex items-center gap-2">
			<Badge variant={status === 'connected' ? 'secondary' : 'outline'}>
				{status}{#if game.connections > 0}&nbsp;· {game.connections} here{/if}
			</Badge>
			<Button href={resolve('/')} variant="ghost" size="icon-sm" aria-label="New game">
				<PlusIcon />
			</Button>
		</div>
	</header>

	<Card.Root size="sm">
		<Card.Content class="flex flex-col gap-2">
			<Label for="share">Invite your opponent</Label>
			<div class="flex gap-2">
				<Input id="share" readonly value={shareUrl} onclick={(e) => e.currentTarget.select()} />
				<Button variant="outline" size="icon" onclick={copyLink} aria-label="Copy link">
					{#if copied}<CheckIcon />{:else}<CopyIcon />{/if}
				</Button>
			</div>
		</Card.Content>
	</Card.Root>

	<Card.Root size="sm">
		<Card.Content class="flex flex-col gap-2">
			{#each ['black', 'white'] as const as color (color)}
				<div
					class={cn(
						'flex items-center gap-3 rounded-lg border px-3 py-2',
						game.you === color && 'border-ring',
						game.playing && game.toPlay === color && 'bg-muted'
					)}
				>
					<span
						class={cn(
							'size-4 shrink-0 rounded-full',
							color === 'black' ? 'bg-neutral-900' : 'border border-neutral-400 bg-stone-50'
						)}
					></span>
					<div class="flex min-w-0 flex-1 flex-col">
						<span class="truncate text-sm font-medium">{occupant(color) ?? 'Empty seat'}</span>
						<span class="text-xs text-muted-foreground">
							{game.captures[color === 'black' ? 0 : 1]} captured
							{#if color === 'black' && game.handicap > 0}· {game.handicap} handicap{/if}
							{#if color === 'white'}· {game.komi} komi{/if}
						</span>
					</div>
					{#if occupant(color) === null && game.you === null}
						<Button size="sm" variant="outline" onclick={() => onclaim(color)}>Sit</Button>
					{:else if game.you === color && canRelease}
						<Button size="sm" variant="ghost" onclick={onrelease}>Leave</Button>
					{/if}
				</div>
			{/each}

			<p class="text-sm text-muted-foreground">
				{#if game.you === null}
					You are watching. Take a seat to play.
				{:else if !game.bothSeated}
					Waiting for an opponent — send them the link.
				{:else if game.playing}
					{game.myTurn ? 'Your move.' : 'Waiting for your opponent.'}
				{/if}
			</p>
		</Card.Content>
		{#if game.playing && game.bothSeated && game.you !== null}
			<Card.Footer class="gap-2">
				<Button class="flex-1" variant="outline" onclick={onpass} disabled={!game.myTurn}>
					Pass
				</Button>
				<AlertDialog.Root>
					<AlertDialog.Trigger class={cn(buttonVariants({ variant: 'destructive' }), 'flex-1')}>
						Resign
					</AlertDialog.Trigger>
					<AlertDialog.Content>
						<AlertDialog.Header>
							<AlertDialog.Title>Resign this game?</AlertDialog.Title>
							<AlertDialog.Description>
								Your opponent wins immediately. This cannot be undone.
							</AlertDialog.Description>
						</AlertDialog.Header>
						<AlertDialog.Footer>
							<AlertDialog.Cancel>Keep playing</AlertDialog.Cancel>
							<AlertDialog.Action onclick={onresign}>Resign</AlertDialog.Action>
						</AlertDialog.Footer>
					</AlertDialog.Content>
				</AlertDialog.Root>
			</Card.Footer>
		{/if}
	</Card.Root>

	{#if game.scoring}
		<Card.Root size="sm">
			<Card.Header>
				<Card.Title>Scoring</Card.Title>
				<Card.Description>Click a group to mark it dead. Both players must agree.</Card.Description>
			</Card.Header>
			<Card.Content class="flex flex-col gap-3">
				{#if game.score}
					<Table.Root>
						<Table.Body>
							<Table.Row>
								<Table.Head>Black</Table.Head>
								<Table.Cell class="text-muted-foreground">
									{game.score.black_stones} + {game.score.black_territory}
								</Table.Cell>
								<Table.Cell class="text-right font-medium tabular-nums">
									{blackTotal(game.score)}
								</Table.Cell>
							</Table.Row>
							<Table.Row>
								<Table.Head>White</Table.Head>
								<Table.Cell class="text-muted-foreground">
									{game.score.white_stones} + {game.score.white_territory} + {game.score.komi}
								</Table.Cell>
								<Table.Cell class="text-right font-medium tabular-nums">
									{whiteTotal(game.score)}
								</Table.Cell>
							</Table.Row>
						</Table.Body>
					</Table.Root>
				{/if}
				{#if game.accepted.length > 0}
					<p class="text-sm text-muted-foreground">Accepted: {game.accepted.join(', ')}</p>
				{/if}
			</Card.Content>
			{#if game.you !== null}
				<Card.Footer class="gap-2">
					<Button class="flex-1" onclick={onaccept} disabled={iAccepted}>
						{iAccepted ? 'Waiting for opponent' : 'Accept'}
					</Button>
					<Button class="flex-1" variant="outline" onclick={onresume}>Resume play</Button>
				</Card.Footer>
			{/if}
		</Card.Root>
	{/if}

	{#if game.phase.state === 'finished'}
		<Card.Root size="sm">
			<Card.Header>
				<Card.Title>{describeResult(game.phase.result)}</Card.Title>
			</Card.Header>
			<Card.Footer>
				<Button href={sgfUrl(room)} download="{room}.sgf" variant="outline">
					<DownloadIcon /> Download game record
				</Button>
			</Card.Footer>
		</Card.Root>
	{/if}

	<div class="flex flex-col gap-2">
		<Label for="name">Your name</Label>
		<Input
			id="name"
			value={name}
			maxlength={40}
			onchange={(e) => onrename(e.currentTarget.value)}
		/>
	</div>
</aside>
