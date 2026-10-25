<script lang="ts">
	import './layout.css';
	import favicon from '#lib/assets/favicon.svg';
	import WakingNotice from '#lib/components/WakingNotice.svelte';
	import { Toaster } from '#lib/components/ui/sonner/index.js';
	import { wakeServer } from '#lib/identity.js';
	import { server } from '#lib/api-status.svelte.js';
	import { ModeWatcher } from 'mode-watcher';
	import { onMount } from 'svelte';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	onMount(wakeServer);
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
	<title>igo</title>
</svelte:head>

<ModeWatcher />
<Toaster position="bottom-center" />
{#if server.waking}
	<WakingNotice />
{/if}

{@render children()}
