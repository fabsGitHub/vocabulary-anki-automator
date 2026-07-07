<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

// --- State Engine Vectors ---
const windowLabel = ref('');
const currentTab = ref('lang');

// --- Interactive UX Status Trackers ---
const isAnkiOnline = ref(false);
const isPluginInstalledOnDisk = ref(false);
const isScanning = ref(false);
const isProvisioning = ref(false);
const isLaunchingAnki = ref(false);
const isSaving = ref(false);

const actionFeedback = ref('');
const feedbackType = ref<'info' | 'success' | 'error'>('info');

const discoveredDecks = ref<string[]>([]);
const discoveredModels = ref<string[]>([]);

// --- Configuration Form Alignment State ---
const config = ref({
  ui_lang: 'en',
  hotkeys: 'CommandOrControl+Shift+C', 
  deepl: { source_lang: 'EN', target_lang: 'DE' },
  anki: { deck_name: '', model_name: '' }
});

// --- Feedback Alerts Notification Engine ---
function showFeedback(msg: string, type: 'info' | 'success' | 'error' = 'info') {
  actionFeedback.value = msg;
  feedbackType.value = type;
  if (type !== 'info') {
    setTimeout(() => { actionFeedback.value = ''; }, 4000);
  }
}

// --- STEP 1: Automated Script Provisioning Add-on Injection ---
async function runSetupInstallation() {
  isProvisioning.value = true;
  showFeedback('Downloading and provisioning AnkiConnect...', 'info');
  try {
    const message = await invoke<string>('auto_provision_ankiconnect');
    isPluginInstalledOnDisk.value = true;
    showFeedback(message, 'success');
    await scanLocalAnki(true);
  } catch (err: any) {
    showFeedback(`Installation failed: ${err}`, 'error');
  } finally {
    isProvisioning.value = false;
  }
}

// --- STEP 2: Active Engine Handshake (App Core Startup) ---
async function bootAnkiApp() {
  isLaunchingAnki.value = true;
  showFeedback('Launching Anki Desktop process layers...', 'info');
  try {
    const res = await invoke<string>('launch_anki_desktop');
    showFeedback(res, 'success');
    
    let attempts = 0;
    const interval = setInterval(async () => {
      await scanLocalAnki(true);
      attempts++;
      if (isAnkiOnline.value || attempts > 10) {
        clearInterval(interval);
        isLaunchingAnki.value = false;
      }
    }, 1500);
  } catch (err: any) {
    showFeedback(`Auto-start failed: ${err}`, 'error');
    isLaunchingAnki.value = false;
  }
}

// --- STEP 3: Multi-Layer Diagnostics & Target Fetching ---
async function scanLocalAnki(isSilent = false) {
  if (!isSilent) {
    isScanning.value = true;
    showFeedback('Scanning local network port pathways...', 'info');
  }
  try {
    const [resDecks, resModels] = await Promise.all([
      fetch("http://127.0.0.1:8765", { method: "POST", body: JSON.stringify({ action: "deckNames", version: 6 }), signal: AbortSignal.timeout(1500) }),
      fetch("http://127.0.0.1:8765", { method: "POST", body: JSON.stringify({ action: "modelNames", version: 6 }), signal: AbortSignal.timeout(1500) })
    ]);
    
    const decksJson = await resDecks.json();
    const modelsJson = await resModels.json();

    if (decksJson.result && modelsJson.result) {
      discoveredDecks.value = decksJson.result;
      discoveredModels.value = modelsJson.result;
      
      if (!config.value.anki.deck_name && discoveredDecks.value.length > 0) {
        config.value.anki.deck_name = discoveredDecks.value[0];
      }
      if (!config.value.anki.model_name && discoveredModels.value.length > 0) {
        config.value.anki.model_name = discoveredModels.value[0];
      }

      isAnkiOnline.value = true;
      isPluginInstalledOnDisk.value = true;
      if (!isSilent) showFeedback('Anki Environment Link Synchronized!', 'success');
    }
  } catch (err) {
    isAnkiOnline.value = false;
    isPluginInstalledOnDisk.value = await invoke<boolean>('check_ankiconnect_installed');
    if (!isSilent) {
      if (!isPluginInstalledOnDisk.value) {
        showFeedback('Scan complete: AnkiConnect plugin is missing from local folders.', 'error');
      } else {
        showFeedback('Scan complete: Core plugin found, but Anki Desktop is closed.', 'error');
      }
    }
  } finally {
    if (!isSilent) isScanning.value = false;
  }
}

// --- Form Submission Save Configuration Action ---
async function handleSaveConfig() {
  isSaving.value = true;
  showFeedback('Saving application configuration properties...', 'info');

  const payload = {
    ui_lang: config.value.ui_lang,
    hotkeys: config.value.hotkeys, 
    deepl: {
      api_key: "",
      source_lang: config.value.deepl.source_lang.toUpperCase(),
      target_lang: config.value.deepl.target_lang.toUpperCase(),
      max_chars: 100
    },
    anki: {
      connect_url: "http://127.0.0.1:8765",
      deck_name: config.value.anki.deck_name,
      model_name: config.value.anki.model_name
    }
  };

  try {
    // Verified apply_settings signature loop
    await invoke("apply_settings", { newCfg: payload });
    showFeedback('Configuration applied and live tracking triggered!', 'success');
  } catch (err: any) {
    showFeedback(`Configuration Error: ${err}`, 'error');
  } finally {
    isSaving.value = false;
  }
}

onMounted(async () => {
  const win = getCurrentWindow();
  windowLabel.value = win.label;
  
  try {
    const savedConfig = await invoke<any>("get_current_settings");
    if (savedConfig) {
      if (savedConfig.ui_lang) config.value.ui_lang = savedConfig.ui_lang;
      if (savedConfig.hotkeys) config.value.hotkeys = savedConfig.hotkeys;
      if (savedConfig.deepl) {
        config.value.deepl.source_lang = savedConfig.deepl.source_lang || 'EN';
        config.value.deepl.target_lang = savedConfig.deepl.target_lang || 'DE';
      }
      if (savedConfig.anki) {
        config.value.anki.deck_name = savedConfig.anki.deck_name || '';
        config.value.anki.model_name = savedConfig.anki.model_name || '';
      }
    }
  } catch (e) {
    console.error("Failed loading configuration maps:", e);
  }

  await scanLocalAnki(true);
});
</script>

<template>
  <main class="view app-fade-in">
    <header class="mac-header">
      <div class="header-titles">
        <h2>System Engine Setup</h2>
        <p>Zero-overhead capture pipeline orchestration parameters.</p>
      </div>
      <div class="controls-cluster">
        <span :class="['status-badge', isAnkiOnline ? 'online' : 'offline']">
          ● Engine Link: {{ isAnkiOnline ? 'Active' : 'Offline' }}
        </span>
      </div>
    </header>

    <nav class="segmented-bar">
      <button type="button" :class="['segment-btn', { active: currentTab === 'lang' }]" @click="currentTab = 'lang'">Capture & Language</button>
      <button type="button" :class="['segment-btn', { active: currentTab === 'anki' }]" @click="currentTab = 'anki'">Anki Core Connection</button>
    </nav>

    <form @submit.prevent="handleSaveConfig" class="config-container">
      
      <div v-if="currentTab === 'lang'" class="pane-layout animate-slide">
        <div class="grid-row">
          <div class="field-group">
            <label>Source Language</label>
            <select v-model="config.deepl.source_lang" class="mac-select">
              <option value="EN">English</option>
              <option value="DE">German</option>
              <option value="FR">French</option>
              <option value="ES">Spanish</option>
            </select>
          </div>
          <div class="field-group">
            <label>Target Language</label>
            <select v-model="config.deepl.target_lang" class="mac-select">
              <option value="DE">German</option>
              <option value="EN">English</option>
              <option value="FR">French</option>
              <option value="ES">Spanish</option>
            </select>
          </div>
        </div>

        <div class="field-group" style="margin-top: 14px;">
          <label>Global Activation Hotkey Trigger</label>
          <input type="text" v-model="config.hotkeys" placeholder="e.g. CommandOrControl+Shift+C" class="mac-input" />
          <span class="input-hint">Highlight text inside any desktop application screen and execute this mapping to auto-fill.</span>
        </div>
      </div>

      <div v-if="currentTab === 'anki'" class="pane-layout animate-slide">
        
        <div v-if="!isPluginInstalledOnDisk" class="onboarding-notice">
          <div class="notice-icon">⚡</div>
          <h4>1. Install AnkiConnect Add-on</h4>
          <p>The system needs to provision secure automated background loop network configurations straight to your local directory structures.</p>
          <button type="button" @click="runSetupInstallation" :disabled="isProvisioning" class="btn btn-primary btn-wide" style="padding: 10px 16px;">
            {{ isProvisioning ? 'Injecting Script Extenders...' : 'Run Automated One-Click Setup' }}
          </button>
        </div>

        <div v-else-if="!isAnkiOnline" class="onboarding-notice warning">
          <div class="notice-icon">📡</div>
          <h4>2. Active Engine Handshake</h4>
          <p>Your background bridge utility is completely installed! Please launch your Anki Desktop application to activate the sync loop network.</p>
          
          <div class="action-row-center" style="display: flex; gap: 12px; width: 100%; max-width: 440px; margin: 16px auto 0 auto;">
            <button type="button" @click="bootAnkiApp" :disabled="isLaunchingAnki" class="btn btn-primary" style="flex: 1; padding: 10px 16px;">
              {{ isLaunchingAnki ? 'Launching Core App...' : 'Auto-Launch Anki App' }}
            </button>
            <button type="button" @click="scanLocalAnki(false)" :disabled="isScanning" class="btn btn-secondary" style="flex: 1; padding: 10px 16px;">
              {{ isScanning ? 'Polling...' : '🔄 Rescan Connection' }}
            </button>
          </div>
        </div>

        <div v-else class="form-fields animate-slide">
          <div class="pane-title-row" style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 14px;">
            <h3 class="pane-title" style="margin: 0;">3. Target Selection Configuration</h3>
            <button type="button" @click="scanLocalAnki(false)" :disabled="isScanning" class="btn btn-secondary" style="padding: 10px 16px; min-width: 160px;">
              {{ isScanning ? 'Refreshing...' : '🔄 Re-Verify Targets' }}
            </button>
          </div>
          
          <div class="grid-row">
            <div class="field-group">
              <label>Target Flashcard Deck</label>
              <select v-model="config.anki.deck_name" class="mac-select">
                <option v-for="deck in discoveredDecks" :key="deck" :value="deck">{{ deck }}</option>
              </select>
            </div>

            <div class="field-group">
              <label>Target Card Model Type</label>
              <select v-model="config.anki.model_name" class="mac-select">
                <option v-for="model in discoveredModels" :key="model" :value="model">{{ model }}</option>
              </select>
            </div>
          </div>
          
          <div class="success-notice-card">
            <span>✓ Operational loop verification online. Ready to catch clipboard sequences!</span>
          </div>
        </div>

      </div>

      <div class="footer-action-bar">
        <div v-if="actionFeedback" :class="['interactive-toast', feedbackType]">
          {{ actionFeedback }}
        </div>
        <button type="submit" :disabled="isSaving" class="btn btn-primary position-right" style="padding: 10px 20px;">
          {{ isSaving ? 'Applying Changes...' : 'Save Configuration' }}
        </button>
      </div>
    </form>
  </main>
</template>