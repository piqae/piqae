<script lang="ts">
  let { compact = false }: { compact?: boolean } = $props();
</script>

<span class="logo-mark" aria-hidden="true">
  <span></span><span></span><span></span>
</span>
{#if !compact}<span class="wordmark">Spool</span>{/if}

<style>
  .logo-mark {
    width: 27px;
    height: 27px;
    position: relative;
    display: inline-grid;
    place-items: center;
    overflow: hidden;
    border-radius: 8px;
    background: #17161b;
  }
  .logo-mark span {
    position: absolute;
    width: 13px;
    height: 4px;
    border: 1.5px solid white;
    border-radius: 999px;
  }
  .logo-mark span:nth-child(1) { transform: translateY(-5px); }
  .logo-mark span:nth-child(2) { border-color: #9f8afb; }
  .logo-mark span:nth-child(3) { transform: translateY(5px); }
  .wordmark {
    font-size: 17px;
    font-weight: 680;
    letter-spacing: -0.035em;
  }
</style>

