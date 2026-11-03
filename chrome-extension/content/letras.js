// Nivumi Lyrics Bridge - Content Script for Letras.mus.br
(() => {
  const NUSA_ENDPOINT = "http://127.0.0.1:48123/api/lyrics";

  // =========================================================================
  // 1. Anti-Adblock & Video Player Nuker (from letras-adblock-fix)
  // =========================================================================
  const AD_SELECTORS = [
    ".fc-ab-root",
    ".fc-dialog-overlay",
    ".fc-dialog-container",
    "[class*='fc-ab-']",
    ".player-media",
    "iframe[title='3rd party thumb']",
    ".contextualPlanModal",
    "[class*='contextualPlan']",
    "[class*='modal-assine']",
    "[class*='premium-banner']",
    ".cnt-ad",
    ".ad-box",
    ".pub-box",
    ".placeholder-ad"
  ];

  function nukeAntiAdblock() {
    AD_SELECTORS.forEach(sel => {
      try {
        const els = document.querySelectorAll(sel);
        els.forEach(el => el.remove());
      } catch {}
    });

    // Ensure scrolling is always enabled
    if (document.body) {
      document.body.style.setProperty("overflow", "auto", "important");
      document.body.style.setProperty("position", "static", "important");
    }
    if (document.documentElement) {
      document.documentElement.style.setProperty("overflow", "auto", "important");
      document.documentElement.style.setProperty("position", "static", "important");
    }
  }

  // Run immediate cleanup
  nukeAntiAdblock();
  setInterval(nukeAntiAdblock, 750);

  // MutationObserver for instant removal as elements get injected
  if (window.MutationObserver) {
    const observer = new MutationObserver(() => {
      nukeAntiAdblock();
    });
    const target = document.documentElement || document.body;
    if (target) {
      observer.observe(target, { childList: true, subtree: true });
    } else {
      document.addEventListener("DOMContentLoaded", () => {
        observer.observe(document.body, { childList: true, subtree: true });
      });
    }
  }

  // =========================================================================
  // 2. Lyrics & Translation Extractor
  // =========================================================================

  // Clean and extract readable lyric text from HTML element
  function extractLyricsFromContainer(container) {
    if (!container) return "";
    const clone = container.cloneNode(true);
    // Remove unwanted scripts, ads, video players, and annotations
    clone.querySelectorAll("script, style, iframe, .cnt-ad, .ad-box, .fc-ab-root, .player-media, .contextualPlanModal, [class*='contextualPlan'], button, .placeholder").forEach(el => el.remove());
    // Convert <br> to newline
    clone.querySelectorAll("br").forEach(br => br.replaceWith("\n"));
    // Verse spans are the line boundaries on current translation pages.
    const verses = clone.querySelectorAll(".verse");
    if (verses.length) {
      return Array.from(verses).map(verse => {
        verse.querySelectorAll(".romanization").forEach(el => el.remove());
        return verse.textContent.trim();
      }).filter(Boolean).join("\n");
    }
    // Process paragraphs
    const paragraphs = clone.querySelectorAll("p");
    if (paragraphs.length > 0) {
      return Array.from(paragraphs)
        .map(p => p.textContent.trim())
        .filter(t => {
          if (!t) return false;
          const low = t.toLowerCase();
          return !low.includes("pague uma vez") && !low.includes("ano inteiro") && !low.includes("assinar o letras");
        })
        .join("\n\n");
    }
    return (clone.textContent || "").trim();
  }

  // Scrape title and artist
  function getSongMeta() {
    let title = "";
    let artist = "";

    const titleEl = document.querySelector(".head-title, h1.head-title, .cnt-head_title, h1");
    if (titleEl) {
      title = titleEl.textContent.trim();
    }

    const artistEl = document.querySelector(".head-subtitle a, h2.head-subtitle a, .head-subtitle, h2 a, h2");
    if (artistEl) {
      artist = artistEl.textContent.trim();
    }

    return { title, artist };
  }

  // Find translation URL if on original page
  function getTranslationUrl() {
    const link = document.querySelector('a[href*="/traducao.html"], a.btn-traducao, a[data-type="traducao"]');
    if (link && link.href) {
      return link.href;
    }
    const path = window.location.pathname;
    if (!path.endsWith("/traducao.html") && path.length > 3) {
      return window.location.origin + path.replace(/\/$/, "") + "/traducao.html";
    }
    return null;
  }

  // Scrape both original and translation from DOM
  async function extractAllLyrics() {
    const { title, artist } = getSongMeta();
    if (!title) return null;

    let original = "";
    let translation = null;

    const isTranslationPage = window.location.pathname.endsWith("/traducao.html");

    if (isTranslationPage) {
      // Translation page has side-by-side or stacked lyric & translation
      const origEl = document.querySelector(".lyric-original, .cnt-letra:not(.cnt-trad), .cnt-letra");
      const transEl = document.querySelector(".lyric-translation, .lyric-traducao, .cnt-trad, .cnt-letra-trad");

      if (origEl) original = extractLyricsFromContainer(origEl);
      if (transEl) translation = extractLyricsFromContainer(transEl);
    } else {
      // Standard lyric page
      const origEl = document.querySelector(".lyric-original, .cnt-letra, .lyric");
      if (origEl) {
        original = extractLyricsFromContainer(origEl);
      }

      // Check if translation exists and fetch it seamlessly
      const transUrl = getTranslationUrl();
      if (transUrl) {
        try {
          const res = await fetch(transUrl);
          if (res.ok) {
            const html = await res.text();
            const parser = new DOMParser();
            const doc = parser.parseFromString(html, "text/html");
            const transEl = doc.querySelector(".lyric-translation, .lyric-traducao, .cnt-trad, .cnt-letra-trad");
            if (transEl) {
              const transText = extractLyricsFromContainer(transEl);
              if (transText && transText !== original) {
                translation = transText;
                // Use the original displayed alongside this translation for alignment.
                const pairedOriginal = doc.querySelector(".lyric-original, .cnt-letra:not(.cnt-trad)");
                if (pairedOriginal) {
                  const pairedText = extractLyricsFromContainer(pairedOriginal);
                  if (pairedText) original = pairedText;
                }
              }
            }
          }
        } catch {
          // Silent fallback if fetch fails
        }
      }
    }

    return {
      title,
      artist,
      original,
      translation,
      source: "letras.mus.br",
      url: window.location.href
    };
  }

  // Push lyrics to Nusa local server
  async function sendToNusa(lyricsData) {
    try {
      const response = await fetch(NUSA_ENDPOINT, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(lyricsData)
      });
      return response.ok;
    } catch {
      return false;
    }
  }

  // Inject floating Nusa button on Letras.mus.br page
  function injectFloatingButton() {
    if (document.getElementById("nusa-floating-bridge")) return;

    // Only inject if lyrics container exists
    const container = document.querySelector(".cnt-letra, .lyric-original, .head-title");
    if (!container) return;

    const btn = document.createElement("div");
    btn.id = "nusa-floating-bridge";
    btn.innerHTML = `
      <div class="nusa-badge-container">
        <div class="nusa-badge-icon">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M9 18V5l12-2v13" stroke="#38bdf8"/>
            <circle cx="6" cy="18" r="3" fill="#38bdf8"/>
            <circle cx="18" cy="16" r="3" fill="#2fd06b"/>
          </svg>
        </div>
        <div class="nusa-badge-content">
          <span class="nusa-badge-title">Enviar ao Nivumi</span>
          <span class="nusa-badge-sub">Original + Tradução PT-BR</span>
        </div>
        <div class="nusa-badge-status" id="nusa-btn-status"></div>
      </div>
    `;

    document.body.appendChild(btn);

    btn.addEventListener("click", async () => {
      const statusEl = document.getElementById("nusa-btn-status");
      statusEl.textContent = "Enviando…";
      btn.classList.add("loading");

      const data = await extractAllLyrics();
      if (!data || !data.original) {
        statusEl.textContent = "Erro ao extrair letra";
        setTimeout(() => { statusEl.textContent = ""; }, 2500);
        btn.classList.remove("loading");
        return;
      }

      const ok = await sendToNusa(data);
      btn.classList.remove("loading");
      if (ok) {
        btn.classList.add("success");
        statusEl.textContent = "✓ Sincronizado com o Notch!";
        setTimeout(() => {
          btn.classList.remove("success");
          statusEl.textContent = "";
        }, 3500);
      } else {
        btn.classList.add("error");
        statusEl.textContent = "Nivumi offline (inicie o app)";
        setTimeout(() => {
          btn.classList.remove("error");
          statusEl.textContent = "";
        }, 4000);
      }
    });
  }

  // Listen to messages from popup or background script
  chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
    if (msg.action === "getLyricsData") {
      extractAllLyrics().then(data => sendResponse({ success: !!data, data }));
      return true; // asynchronous response
    }
    if (msg.action === "sendToNusaDirect") {
      extractAllLyrics().then(async data => {
        if (!data) {
          sendResponse({ success: false, error: "Letra não encontrada na página." });
          return;
        }
        const ok = await sendToNusa(data);
        sendResponse({ success: ok, data });
      });
      return true;
    }
  });

  // Run on page load
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", injectFloatingButton);
  } else {
    injectFloatingButton();
  }
})();
