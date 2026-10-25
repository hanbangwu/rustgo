<script lang="ts">
	import Game from '#lib/components/Game.svelte';
	import NewGame from '#lib/components/NewGame.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

{#await data.room then room}
	{#if room}
		{#key data.id}
			<Game room={data.id} />
		{/key}
	{:else}
		<NewGame missing={data.id} />
	{/if}
{:catch}
	<main class="mx-auto max-w-md px-4 py-12">
		<Alert.Root variant="destructive">
			<CircleAlertIcon />
			<Alert.Title>Could not reach the server</Alert.Title>
			<Alert.Description>Reload the page to try again.</Alert.Description>
		</Alert.Root>
	</main>
{/await}
