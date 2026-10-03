<script lang="ts">
  // The UG monogram: a U drawn as a shield with a G inside.
  let { size = 28 }: { size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="15 13 70 77" aria-hidden="true">
  <path d="M27 20 V47 C27 64 37 76 50 83 C63 76 73 64 73 47 V20" fill="none" stroke="var(--ug-fg, #f3eeff)" stroke-width="7" stroke-linecap="round" stroke-linejoin="round" />
  <path d="M59.5 38.5 A12 12 0 1 0 62 47 H52" fill="none" stroke="var(--ug-accent, #b794ff)" stroke-width="5" stroke-linecap="round" stroke-linejoin="round" />
</svg>
