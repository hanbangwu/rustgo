<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import { createRoom } from '#lib/identity.js';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

	interface Props {
		/** The room id the visitor asked for, if it does not exist. */
		missing?: string | null;
	}

	let { missing = null }: Props = $props();

	const SIZES = [19, 13, 9];
	const HANDICAPS = [0, 2, 3, 4, 5, 6, 7, 8, 9];

	let size = $state(19);
	let handicap = $state(0);
	let komi = $state(7.5);
	let komiEdited = $state(false);
	let busy = $state(false);
	let error = $state<string | null>(null);

	function pickHandicap(next: number) {
		handicap = next;
		if (!komiEdited) komi = next > 0 ? 0.5 : 7.5;
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		error = null;
		try {
			const room = await createRoom({ size, komi, handicap });
			await goto(resolve('/[room]', { room: room.id }));
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			busy = false;
		}
	}
</script>

<main class="mx-auto flex max-w-md flex-col gap-4 px-4 py-12">
	<h1 class="text-2xl font-semibold tracking-wide">igo</h1>

	{#if missing}
		<Alert.Root>
			<CircleAlertIcon />
			<Alert.Title>No game at /{missing}</Alert.Title>
			<Alert.Description>It may have expired after a day without play.</Alert.Description>
		</Alert.Root>
	{/if}

	<Card.Root>
		<Card.Header>
			<Card.Title>New game</Card.Title>
			<Card.Description>Chinese rules. Share the link with your opponent.</Card.Description>
		</Card.Header>
		<Card.Content>
			<form id="new-game" class="flex flex-col gap-5" onsubmit={submit}>
				<div class="flex flex-col gap-2">
					<Label>Board</Label>
					<ToggleGroup.Root
						type="single"
						variant="outline"
						value={String(size)}
						onValueChange={(v) => v && (size = Number(v))}
					>
						{#each SIZES as n (n)}
							<ToggleGroup.Item value={String(n)}>{n}×{n}</ToggleGroup.Item>
						{/each}
					</ToggleGroup.Root>
				</div>

				<div class="flex flex-col gap-2">
					<Label>Handicap</Label>
					<ToggleGroup.Root
						type="single"
						variant="outline"
						class="flex-wrap"
						value={String(handicap)}
						onValueChange={(v) => v && pickHandicap(Number(v))}
					>
						{#each HANDICAPS as n (n)}
							<ToggleGroup.Item value={String(n)}>{n === 0 ? 'None' : n}</ToggleGroup.Item>
						{/each}
					</ToggleGroup.Root>
				</div>

				<div class="flex flex-col gap-2">
					<Label for="komi">Komi</Label>
					<Input
						id="komi"
						type="number"
						step="0.5"
						min="-100"
						max="100"
						class="w-28"
						bind:value={komi}
						oninput={() => (komiEdited = true)}
					/>
				</div>

				{#if error}
					<p class="text-sm text-destructive" role="alert">{error}</p>
				{/if}
			</form>
		</Card.Content>
		<Card.Footer>
			<Button type="submit" form="new-game" disabled={busy}>
				{busy ? 'Creating…' : 'Create game'}
			</Button>
		</Card.Footer>
	</Card.Root>
</main>
