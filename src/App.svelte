<script lang="ts">
  import { onMount } from 'svelte';
  import { message } from '@tauri-apps/plugin-dialog';
  import Sidebar from './lib/components/Sidebar.svelte';
  import SetupCard from './lib/components/SetupCard.svelte';
  import ReplacePanel from './lib/components/ReplacePanel.svelte';
  import ResultsSection from './lib/components/ResultsSection.svelte';
  import StatusBar from './lib/components/StatusBar.svelte';
  import LogDrawer from './lib/components/LogDrawer.svelte';
  import { initListeners } from './lib/stores/app.svelte';

  onMount(() => {
    initListeners(async (result) => {
      if (result === 'finished') await message('Finished! :)', { title: 'NewGAN Manager' });
      else if (result === 'failed') await message('替换失败，请查看日志', { title: 'Error' });
    });
  });
</script>

<div class="app">
  <Sidebar />
  <main class="content">
    <SetupCard />
    <ReplacePanel />
    <ResultsSection />
  </main>
  <StatusBar />
</div>

<LogDrawer />
