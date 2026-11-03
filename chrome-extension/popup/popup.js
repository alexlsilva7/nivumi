// Nivumi Lyrics Bridge - Popup Logic
document.addEventListener("DOMContentLoaded", () => {
  const statusEl = document.getElementById("connectionStatus");
  const statusText = document.getElementById("statusText");
  const mediaTitle = document.getElementById("mediaTitle");
  const mediaArtist = document.getElementById("mediaArtist");
  const nowPlayingActions = document.getElementById("nowPlayingActions");
  const searchCurrentBtn = document.getElementById("searchCurrentBtn");
  const searchLyricsifyBtn = document.getElementById("searchLyricsifyBtn");
  const searchForm = document.getElementById("searchForm");
  const searchInput = document.getElementById("searchInput");
  const searchLyricsifySubmit = document.getElementById("searchLyricsifySubmit");

  const songTitle = document.getElementById("songTitle");
  const songArtist = document.getElementById("songArtist");
  const tabPills = document.getElementById("tabPills");
  const tabOriginal = document.getElementById("tabOriginal");
  const tabTranslation = document.getElementById("tabTranslation");
  const tabDual = document.getElementById("tabDual");

  const emptyHint = document.getElementById("emptyHint");
  const lyricsText = document.getElementById("lyricsText");
  const actionFooter = document.getElementById("actionFooter");
  const sendBtn = document.getElementById("sendBtn");
  const sendBtnText = document.getElementById("sendBtnText");
  const copyBtn = document.getElementById("copyBtn");
  const copyBtnText = document.getElementById("copyBtnText");

  let currentLyricsData = null;
  let activeTabMode = "original";

  // Check connection to Nivumi Desktop
  async function checkNusa() {
    chrome.runtime.sendMessage({ action: "checkStatus" }, (response) => {
      if (response && response.online) {
        statusEl.className = "connection-status online";
        statusText.textContent = "Nivumi Conectado";
      } else {
        statusEl.className = "connection-status offline";
        statusText.textContent = "Nivumi Offline";
      }
    });
  }

  // Fetch current media playing on Windows
  async function loadCurrentMedia() {
    chrome.runtime.sendMessage({ action: "getCurrentMedia" }, (media) => {
      if (media && media.title && media.title.trim().length > 0) {
        mediaTitle.textContent = media.title;
        mediaArtist.textContent = media.artist || "Artista desconhecido";
        nowPlayingActions.style.display = "flex";

        searchCurrentBtn.onclick = () => {
          const query = `${media.artist} ${media.title}`.trim();
          chrome.runtime.sendMessage({ action: "openSearchTab", query });
        };

        searchLyricsifyBtn.onclick = () => {
          const query = `${media.artist} ${media.title}`.trim();
          chrome.runtime.sendMessage({ action: "openLyricsifySearchTab", query });
        };
      } else {
        mediaTitle.textContent = "Nenhuma música detectada";
        mediaArtist.textContent = "Toque uma música no Spotify, YouTube, etc.";
        nowPlayingActions.style.display = "none";
      }
    });
  }

  // Handle Search Input (Letras)
  searchForm.addEventListener("submit", (e) => {
    e.preventDefault();
    const query = searchInput.value.trim();
    if (query) {
      chrome.runtime.sendMessage({ action: "openSearchTab", query });
    }
  });

  // Handle Search Input (Lyricsify LRC)
  if (searchLyricsifySubmit) {
    searchLyricsifySubmit.addEventListener("click", (e) => {
      e.preventDefault();
      const query = searchInput.value.trim();
      if (query) {
        chrome.runtime.sendMessage({ action: "openLyricsifySearchTab", query });
      }
    });
  }

  // Render Lyrics Preview depending on active mode
  function renderLyricsPreview() {
    if (!currentLyricsData) return;

    if (activeTabMode === "original") {
      lyricsText.innerHTML = "";
      lyricsText.textContent = currentLyricsData.original || "(Sem letra original)";
      lyricsText.style.display = "block";
    } else if (activeTabMode === "translation") {
      lyricsText.innerHTML = "";
      lyricsText.textContent = currentLyricsData.translation || "(Sem tradução disponível)";
      lyricsText.style.display = "block";
    } else if (activeTabMode === "dual") {
      lyricsText.innerHTML = "";
      const grid = document.createElement("div");
      grid.className = "lyrics-dual-grid";

      const colOrig = document.createElement("div");
      colOrig.innerHTML = `<div class="dual-col-title">Original (com tempos)</div><pre style="white-space: pre-wrap; font-family: inherit;">${currentLyricsData.original || ""}</pre>`;

      const colTrans = document.createElement("div");
      colTrans.innerHTML = `<div class="dual-col-title">Tradução PT-BR</div><pre style="white-space: pre-wrap; font-family: inherit;">${currentLyricsData.translation || "(Sem tradução)"}</pre>`;

      grid.appendChild(colOrig);
      grid.appendChild(colTrans);
      lyricsText.appendChild(grid);
      lyricsText.style.display = "block";
    }
  }

  function setTab(mode) {
    activeTabMode = mode;
    [tabOriginal, tabTranslation, tabDual].forEach(t => t.classList.remove("active"));
    if (mode === "original") tabOriginal.classList.add("active");
    if (mode === "translation") tabTranslation.classList.add("active");
    if (mode === "dual") tabDual.classList.add("active");
    renderLyricsPreview();
  }

  tabOriginal.addEventListener("click", () => setTab("original"));
  tabTranslation.addEventListener("click", () => setTab("translation"));
  tabDual.addEventListener("click", () => setTab("dual"));

  // Check if active Chrome tab is on Letras.mus.br OR Lyricsify.com
  chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
    if (!tabs || tabs.length === 0) return;
    const tab = tabs[0];
    if (!tab.url) return;

    if (tab.url.includes("letras.mus.br")) {
      chrome.tabs.sendMessage(tab.id, { action: "getLyricsData" }, (res) => {
        if (chrome.runtime.lastError || !res || !res.success || !res.data) return;

        currentLyricsData = res.data;
        songTitle.textContent = currentLyricsData.title;
        songArtist.textContent = currentLyricsData.artist;

        emptyHint.style.display = "none";
        tabPills.style.display = "flex";
        actionFooter.style.display = "flex";

        if (!currentLyricsData.translation) {
          tabTranslation.style.display = "none";
          tabDual.style.display = "none";
        } else {
          tabTranslation.style.display = "block";
          tabDual.style.display = "block";
        }

        setTab("original");
      });
    } else if (tab.url.includes("lyricsify.com")) {
      chrome.tabs.sendMessage(tab.id, { action: "getLyricsifyData" }, (res) => {
        if (chrome.runtime.lastError || !res || !res.success || !res.data) return;

        currentLyricsData = res.data;
        songTitle.textContent = currentLyricsData.title || "Lyricsify Track";
        songArtist.textContent = currentLyricsData.artist || "Letra Sincronizada (LRC)";

        emptyHint.style.display = "none";
        tabPills.style.display = "flex";
        actionFooter.style.display = "flex";

        tabTranslation.style.display = "none";
        tabDual.style.display = "none";
        tabOriginal.textContent = "LRC Tempos";

        setTab("original");
      });
    }
  });

  // Send to Nivumi button
  sendBtn.addEventListener("click", async () => {
    if (!currentLyricsData) return;
    sendBtnText.textContent = "Enviando…";
    sendBtn.disabled = true;

    chrome.runtime.sendMessage({
      action: "pushLyrics",
      payload: currentLyricsData
    }, (res) => {
      sendBtn.disabled = false;
      if (res && res.success) {
        sendBtnText.textContent = "✓ Sincronizado!";
        sendBtn.style.background = "#2fd06b";
        setTimeout(() => {
          sendBtnText.textContent = "Enviar ao Notch";
        }, 3000);
      } else {
        sendBtnText.textContent = "⚠️ Falha (Nivumi offline?)";
        sendBtn.style.background = "#ef4444";
        setTimeout(() => {
          sendBtnText.textContent = "Enviar ao Notch";
          sendBtn.style.background = "";
        }, 3000);
      }
    });
  });

  // Copy lyrics button
  copyBtn.addEventListener("click", () => {
    if (!currentLyricsData) return;
    let textToCopy = "";
    if (activeTabMode === "translation") {
      textToCopy = currentLyricsData.translation || currentLyricsData.original;
    } else if (activeTabMode === "dual") {
      textToCopy = `=== ${currentLyricsData.title} - ${currentLyricsData.artist} ===\n\n[ORIGINAL]\n${currentLyricsData.original}\n\n[TRADUÇÃO PT-BR]\n${currentLyricsData.translation || ""}`;
    } else {
      textToCopy = currentLyricsData.original;
    }

    navigator.clipboard.writeText(textToCopy).then(() => {
      copyBtnText.textContent = "✓ Copiado!";
      setTimeout(() => {
        copyBtnText.textContent = "Copiar";
      }, 2500);
    });
  });

  // Initial runs
  checkNusa();
  loadCurrentMedia();
});
