<script lang="ts">
	import * as Alert from '#lib/components/ui/alert/index.js';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
</script>

<div class="pointer-events-none fixed inset-x-0 top-0 z-50 p-4" role="status">
	<Alert.Root class="pointer-events-auto mx-auto max-w-md">
		<LoaderCircleIcon class="animate-spin" />
		<Alert.Title>Waking up the server…</Alert.Title>
		<Alert.Description
			>It sleeps when nobody is playing and can take a minute to start.</Alert.Description
		>
	</Alert.Root>
</div>
