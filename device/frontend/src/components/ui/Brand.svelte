<script lang="ts">
  let { small = false }: { small?: boolean } = $props();
</script>

<span
  class="font-semibold tracking-tight text-fg {small
    ? 'text-[15px]'
    : 'text-xl'}"
>
  Openstreamer
</span>
