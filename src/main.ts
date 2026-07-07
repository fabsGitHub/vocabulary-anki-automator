import './styles.css';
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

const currentWindow = getCurrentWindow();

const i18nDictionary: Record<string, Record<string, string>> = {
  en: {
    lbl_front: "Front (Expression)",
    lbl_back: "Back (Translation)",
    lbl_example_source: "Context Example (Source)",
    lbl_example_target: "Context Example (Translation)",
    lbl_notes: "Automator System Notes",
    btn_dismiss: "Dismiss",
    btn_add: "Add to Anki",
    cfg_title: "Engine Configurations",
    cfg_subtitle: "Fine-tune automation behaviors and connection loops.",
    tab_translation: "Translation",
    tab_anki: "Anki Connect",
    sec_languages: "Translation Scope",
    lbl_source_lang: "Source Language",
    lbl_target_lang: "Target Language",
    sec_anki: "Anki Integration",
    btn_scan_anki: "Scan Local Anki",
    lbl_deck: "Deck Allocation Destination",
    lbl_model: "Target Card Model Type",
    btn_save: "Save Configuration",
    help_title: "How to Setup & Use Anki",
    help_step1_title: "1. Install AnkiConnect Add-on:",
    help_step1_desc: "Open Anki Desktop, navigate to Tools → Add-ons → Get Add-ons, insert code 2055492159 and restart.",
    help_step2_title: "2. Active Engine Handshake:",
    help_step2_desc: "Keep Anki running on your desktop. The automation system connects directly over secure background loop networks.",
    help_step3_title: "3. Trigger Processing Loop:",
    help_step3_desc: "Highlight text on any native app screen, execute your hotkey mapping, and watch structural validation auto-fill.",
    scanning: "Scanning local network environments...",
    scan_ok: "Anki link synchronized successfully!",
    saving: "Saving properties...",
    save_ok: "Configuration saved safely!",
    anki_fail: "Anki connection failed. Keeping simple backup input fields active."
  },
  de: {
    lbl_front: "Vorderseite (Begriff)",
    lbl_back: "Rückseite (Übersetzung)",
    lbl_example_source: "Kontext-Beispiel (Original)",
    lbl_example_target: "Kontext-Beispiel (Übersetzung)",
    lbl_notes: "Automator System-Notizen",
    btn_dismiss: "Schließen",
    btn_add: "Zu Anki hinzufügen",
    cfg_title: "Konfigurationen",
    cfg_subtitle: "Automationsverhalten und Verbindungsschleifen anpassen.",
    tab_translation: "Übersetzung",
    tab_anki: "Anki Verbindung",
    sec_languages: "Übersetzungs-Sprachpaar",
    lbl_source_lang: "Ausgangssprache",
    lbl_target_lang: "Zielsprache",
    sec_anki: "Anki Integration",
    btn_scan_anki: "Anki lokal scannen",
    lbl_deck: "Ziel-Stapel (Deck Allocation)",
    lbl_model: "Karten-Modelltyp (Model)",
    btn_save: "Konfiguration speichern",
    help_title: "Anki Einrichtung & Benutzung",
    help_step1_title: "1. AnkiConnect Add-on installieren:",
    help_step1_desc: "Öffnen Sie Anki, gehen Sie zu Extras → Erweiterungen → Erweit. herunterladen, Code 2055492159 eingeben und neu starten.",
    help_step2_title: "2. Anki im Hintergrund ausführen:",
    help_step2_desc: "Stellen Sie sicher, dass Anki geöffnet ist. Unser Automator kommuniziert direkt über lokale Schnittstellen.",
    help_step3_title: "3. Hotkey ausführen:",
    help_step3_desc: "Markieren Sie ein beliebiges Wort auf Ihrem Bildschirm und drücken Sie Ihren Hotkey, um die Karteikarte direkt zu befüllen.",
    scanning: "Scanne lokale Netzwerkumgebungen...",
    scan_ok: "Anki-Verbindung erfolgreich synchronisiert!",
    saving: "Speichere Einstellungen...",
    save_ok: "Einstellungen erfolgreich gesichert!",
    anki_fail: "Anki nicht gefunden. Manuelle Textfelder bleiben aktiv."
  }
};

let currentLanguage = "en";
let applicationHotkeys = "CommandOrControl+Shift+C";

function applyLocalization(langCode: string) {
  currentLanguage = langCode;
  const elements = document.querySelectorAll("[data-i18n]");
  elements.forEach((el) => {
    const key = el.getAttribute("data-i18n");
    if (key && i18nDictionary[langCode]?.[key]) {
      if (el.tagName === "INPUT" || el.tagName === "TEXTAREA") {
        (el as HTMLInputElement | HTMLTextAreaElement).placeholder = i18nDictionary[langCode][key];
      } else {
        el.textContent = i18nDictionary[langCode][key];
      }
    }
  });
}

window.addEventListener("DOMContentLoaded", async () => {
  const windowLabel = currentWindow.label;

  try {
    const currentConfig = await invoke<any>("get_current_settings");
    if (currentConfig) {
      if (currentConfig.ui_lang) currentLanguage = currentConfig.ui_lang;
      if (currentConfig.hotkeys) applicationHotkeys = currentConfig.hotkeys;
    }
  } catch (_) {}
  
  applyLocalization(currentLanguage);

  if (windowLabel === "preview") {
    initializePreviewView();
  } else {
    initializeSettingsView();
  }
});

async function initializePreviewView() {
  document.getElementById("preview-view")?.classList.remove("hidden");

  const frontInput = document.getElementById("preview-front") as HTMLInputElement;
  const backInput = document.getElementById("preview-back") as HTMLInputElement;
  const exampleSource = document.getElementById("preview-example-source") as HTMLTextAreaElement;
  const exampleTarget = document.getElementById("preview-example-target") as HTMLTextAreaElement;
  const notesInput = document.getElementById("preview-notes") as HTMLInputElement;
  const notesContainer = document.getElementById("notes-container") as HTMLElement;
  const btnSubmit = document.getElementById("btn-submit") as HTMLButtonElement;
  const btnCancel = document.getElementById("btn-cancel") as HTMLButtonElement;

  await listen<any>("translation-payload", (event) => {
    const { front, back, example, notes } = event.payload;
    frontInput.value = front || "";
    backInput.value = back || "";
    
    // ✨ Split the single multi-line text block back out for the DOM fields
    if (example) {
      const parts = example.split("\n");
      exampleSource.value = parts[0] || "";
      exampleTarget.value = parts[1] || "";
    } else {
      exampleSource.value = "";
      exampleTarget.value = "";
    }
    
    if (notes && notes.trim() !== "") {
      notesContainer.style.display = "flex";
      notesInput.value = notes;
    } else {
      notesContainer.style.display = "none";
    }
    btnSubmit.disabled = false;
  });

  btnCancel.addEventListener("click", () => currentWindow.hide());

  btnSubmit.addEventListener("click", async () => {
    btnSubmit.disabled = true;
    try {
      // Recombine the separate inputs with a newline delimiter to match structural expectations
      const combinedExample = `${exampleSource.value}\n${exampleTarget.value}`;
      await invoke("execute_card_generation", {
        front: frontInput.value,
        back: backInput.value,
        example: combinedExample,
        note: notesInput.value
      });
      currentWindow.hide();
    } catch (err) {
      alert(`Anki error: ${err}`);
      btnSubmit.disabled = false;
    }
  });
}

async function initializeSettingsView() {
  document.getElementById("settings-view")?.classList.remove("hidden");

  const form = document.getElementById("settings-form") as HTMLFormElement;
  const statusMsg = document.getElementById("status-msg") as HTMLElement;
  const langSwitcher = document.getElementById("ui-lang-switcher") as HTMLSelectElement;
  
  langSwitcher.value = currentLanguage;
  langSwitcher.addEventListener("change", (e) => {
    applyLocalization((e.target as HTMLSelectElement).value);
  });

  document.getElementById("help-trigger")?.addEventListener("click", () => 
    document.getElementById("help-modal")?.classList.remove("hidden")
  );
  document.getElementById("help-close")?.addEventListener("click", () => 
    document.getElementById("help-modal")?.classList.add("hidden")
  );

  const tabLinks = document.querySelectorAll(".tab-lnk");
  tabLinks.forEach(btn => {
    btn.addEventListener("click", () => {
      tabLinks.forEach(t => t.classList.remove("active"));
      document.querySelectorAll(".tab-content").forEach(c => c.classList.remove("active-content"));
      
      btn.classList.add("active");
      const target = btn.getAttribute("data-target");
      if (target) document.getElementById(target)?.classList.add("active-content");
    });
  });

  try {
    const currentConfig = await invoke<any>("get_current_settings");
    if (currentConfig) {
      (document.getElementById("cfg-deepl-source") as HTMLSelectElement).value = currentConfig.deepl.source_lang || "EN";
      (document.getElementById("cfg-deepl-target") as HTMLSelectElement).value = currentConfig.deepl.target_lang || "DE";
      (document.getElementById("cfg-anki-deck") as HTMLInputElement).value = currentConfig.anki.deck_name || "";
      (document.getElementById("cfg-anki-model") as HTMLInputElement).value = currentConfig.anki.model_name || "";
      if (currentConfig.hotkeys) applicationHotkeys = currentConfig.hotkeys;

      const hotkeyField = document.getElementById("cfg-hotkeys") as HTMLInputElement;
      if (hotkeyField) hotkeyField.value = applicationHotkeys;
      
      attemptAnkiAutodiscovery(currentConfig.anki.deck_name, currentConfig.anki.model_name, false);
    }
  } catch (err) {
    console.error(err);
  }

  document.getElementById("btn-sync-anki")?.addEventListener("click", () => {
    const dVal = (document.getElementById("cfg-anki-deck") as HTMLInputElement).value;
    const mVal = (document.getElementById("cfg-anki-model") as HTMLInputElement).value;
    attemptAnkiAutodiscovery(dVal, mVal, true);
  });

  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    statusMsg.className = "status-text";
    statusMsg.textContent = i18nDictionary[currentLanguage].scanning;

    const deckInput = document.getElementById("cfg-anki-deck") as HTMLInputElement;
    const deckSelect = document.getElementById("cfg-anki-deck-select") as HTMLSelectElement;
    const modelInput = document.getElementById("cfg-anki-model") as HTMLInputElement;
    const modelSelect = document.getElementById("cfg-anki-model-select") as HTMLSelectElement;

    const finalDeck = !deckSelect.classList.contains("hidden") ? deckSelect.value : deckInput.value;
    const finalModel = !modelSelect.classList.contains("hidden") ? modelSelect.value : modelInput.value;

    const hotkeyField = document.getElementById("cfg-hotkeys") as HTMLInputElement;
    if (hotkeyField) applicationHotkeys = hotkeyField.value;

    const targetPayload = {
      ui_lang: currentLanguage,
      hotkeys: applicationHotkeys, 
      deepl: {
        api_key: "", 
        source_lang: (document.getElementById("cfg-deepl-source") as HTMLSelectElement).value,
        target_lang: (document.getElementById("cfg-deepl-target") as HTMLSelectElement).value,
        max_chars: 100
      },
      anki: {
        connect_url: "http://127.0.0.1:8765",
        deck_name: finalDeck,
        model_name: finalModel
      }
    };

    try {
      // Corrected payload mapping format
      await invoke("apply_settings", { newCfg: targetPayload });
      statusMsg.className = "status-text success";
      statusMsg.textContent = i18nDictionary[currentLanguage].save_ok;
      setTimeout(() => statusMsg.textContent = "", 3000);
    } catch (err: any) {
      statusMsg.className = "status-text error";
      statusMsg.textContent = err.toString();
    }
  });
}

async function attemptAnkiAutodiscovery(activeDeck: string, activeModel: string, isManualClick = false) {
  const dSelect = document.getElementById("cfg-anki-deck-select") as HTMLSelectElement;
  const dInput = document.getElementById("cfg-anki-deck") as HTMLInputElement;
  const mSelect = document.getElementById("cfg-anki-model-select") as HTMLSelectElement;
  const mInput = document.getElementById("cfg-anki-model") as HTMLInputElement;
  const statusMsg = document.getElementById("status-msg");

  if (isManualClick && statusMsg) {
    statusMsg.className = "status-text";
    statusMsg.textContent = i18nDictionary[currentLanguage].scanning;
  }

  try {
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), 3000);

    const [resDecks, resModels] = await Promise.all([
      fetch("http://127.0.0.1:8765", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "deckNames", version: 6 }),
        signal: controller.signal
      }),
      fetch("http://127.0.0.1:8765", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "modelNames", version: 6 }),
        signal: controller.signal
      })
    ]);
    
    const jsonDecks = await resDecks.json();
    const jsonModels = await resModels.json();
    clearTimeout(timeoutId);

    if (jsonDecks.result && jsonModels.result) {
      dSelect.innerHTML = jsonDecks.result.map((d: string) => `<option value="${d}">${d}</option>`).join("");
      dSelect.classList.remove("hidden");
      dInput.classList.add("hidden");
      if(jsonDecks.result.includes(activeDeck)) dSelect.value = activeDeck;

      mSelect.innerHTML = jsonModels.result.map((m: string) => `<option value="${m}">${m}</option>`).join("");
      mSelect.classList.remove("hidden");
      mInput.classList.add("hidden");
      if(jsonModels.result.includes(activeModel)) mSelect.value = activeModel;

      if (isManualClick && statusMsg) {
        statusMsg.className = "status-text success";
        statusMsg.textContent = i18nDictionary[currentLanguage].scan_ok;
        setTimeout(() => statusMsg.textContent = "", 3000);
      }
    }
  } catch (err) {
    dSelect.classList.add("hidden");
    dInput.classList.remove("hidden");
    mSelect.classList.add("hidden");
    mInput.classList.remove("hidden");
    
    if (isManualClick && statusMsg) {
      statusMsg.className = "status-text error";
      statusMsg.textContent = i18nDictionary[currentLanguage].anki_fail;
    }
  }
}