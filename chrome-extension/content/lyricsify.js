// Nivumi Lyrics Bridge - Content Script for Lyricsify.com
(() => {
  const NUSA_ENDPOINT = "http://127.0.0.1:48123/api/lyrics";

  // =========================================================================
  // 1. LRC Extraction Logic
  // =========================================================================

  // Extract song metadata (Title, Artist)
  function getSongMetadata() {
    let title = "";
    let artist = "";

    // Check headings
    const h1 = document.querySelector("h1");
    if (h1) {
      const text = h1.textContent.trim();
      if (text.includes(" - ")) {
        const parts = text.split(" - ");
        artist = parts[0].trim();
        title = parts.slice(1).join(" - ").replace(/\s+(?:LRC\s+)?Lyrics(?:\s+LRC)?\s*$/i, "").trim();
      } else {
        title = text;
      }
    }

    const h2 = document.querySelector("h2, .artist, .song-artist");
    if (h2 && !artist) {
      artist = h2.textContent.replace(/^by\s+/i, "").trim();
    }

    // Fallback: parse page title
    if (!title || !artist) {
      const docTitle = document.title || "";
      // e.g. "Tame Impala - The Less I Know The Better Lyrics | Lyricsify"
      const clean = docTitle.replace(/\|.*$/i, "").replace(/Lyrics.*$/i, "").trim();
      if (clean.includes(" - ")) {
        const parts = clean.split(" - ");
        if (!artist) artist = parts[0].trim();
        if (!title) title = parts.slice(1).join(" - ").trim();
      }
    }

    return { title, artist };
  }

  // DOMParser documents and detached clones do not expose reliable innerText.
  function readLrcText(element) {
    if (typeof element.value === "string") return element.value;
    const clone = element.cloneNode(true);
    clone.querySelectorAll("script, style").forEach(el => el.remove());
    clone.querySelectorAll("br").forEach(br => br.replaceWith("\n"));
    return clone.textContent || "";
  }

  // Extract raw timestamped LRC content from the page
  function extractLrcContent() {
    // 1. Direct container IDs or classes
    const candidates = [
      "#lyrics_details",
      ".lyrics-body",
      "#lyrics_content",
      ".lyrics_content",
      "#lrc",
      ".lrc",
      "textarea#lrc",
      "pre.lrc",
      ".lyrics-container",
      "#lyrics",
      ".lyrics_box"
    ];

    for (const sel of candidates) {
      const el = document.querySelector(sel);
      if (el) {
        const val = readLrcText(el);
        if (hasLrcTimestamps(val)) {
          return cleanLrcText(val);
        }
      }
    }

    // 2. Scan paragraphs or lines on the page for timestamps
    const paragraphs = document.querySelectorAll("p, div.line, .verse");
    const matchedLines = [];
    paragraphs.forEach(p => {
      const text = readLrcText(p).trim();
      if (hasLrcTimestamps(text)) {
        matchedLines.push(text);
      }
    });

    if (matchedLines.length >= 3) {
      return matchedLines.join("\n");
    }

    // 3. Fallback: check whole body text for lines starting with [mm:ss
    const allLines = (document.body.innerText || "").split("\n");
    const timestampLines = allLines.filter(l => hasLrcTimestamps(l.trim()));
    if (timestampLines.length >= 5) {
      return timestampLines.join("\n");
    }

    return null;
  }

  function hasLrcTimestamps(text) {
    return /\[\d+:[0-5]\d(?:\.\d+)?\]/.test(text);
  }

  function cleanLrcText(raw) {
    return raw
      .replace(/<br\s*[\/]?>/gi, "\n")
      .split("\n")
      .map(l => l.trim())
      .filter(l => /^\[(?:\d+:[0-5]\d(?:\.\d+)?|[a-z]+:[^\]]*)\]/i.test(l))
      .join("\n");
  }

  // =========================================================================
  // 2. Local Nusa Bridge HTTP Client
  // =========================================================================
  async function sendLrcToNusa(lrcPayload) {
    try {
      const response = await fetch(NUSA_ENDPOINT, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(lrcPayload)
      });
      return response.ok;
    } catch {
      return false;
    }
  }

  // =========================================================================
  // 3. Floating Button on Song Page
  // =========================================================================
  function injectFloatingLrcBridge() {
    if (document.getElementById("nusa-floating-lyricsify")) return;

    const lrc = extractLrcContent();
    if (!lrc) return; // Only inject if LRC timestamps are available on this page

    const { title, artist } = getSongMetadata();

    const btn = document.createElement("div");
    btn.id = "nusa-floating-lyricsify";
    btn.innerHTML = `
      <div class="nusa-lyricsify-badge">
        <div class="nusa-lyricsify-icon">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="12" cy="12" r="10" stroke="#38bdf8"/>
            <polyline points="12 6 12 12 16 14" stroke="#38bdf8"/>
          </svg>
        </div>
        <div class="nusa-lyricsify-info">
          <span class="nusa-lyricsify-title">Enviar LRC ao Nivumi</span>
          <span class="nusa-lyricsify-sub">${artist ? artist + " · " : ""}Tempos Sincronizados</span>
        </div>
        <div class="nusa-lyricsify-status" id="nusa-lyricsify-status"></div>
      </div>
    `;

    document.body.appendChild(btn);

    btn.addEventListener("click", async () => {
      const statusEl = document.getElementById("nusa-lyricsify-status");
      statusEl.textContent = "Sincronizando…";
      btn.classList.add("loading");

      const currentLrc = extractLrcContent() || lrc;
      const meta = getSongMetadata();

      const payload = {
        title: meta.title || title || "Unknown Song",
        artist: meta.artist || artist || "",
        original: currentLrc,
        translation: null,
        source: "lyricsify.com"
      };

      const ok = await sendLrcToNusa(payload);
      btn.classList.remove("loading");
      if (ok) {
        btn.classList.add("success");
        statusEl.textContent = "✓ Sincronizado no Notch!";
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

  // =========================================================================
  // 4. Search Results Quick Sync Buttons
  // =========================================================================
  function enhanceSearchResults() {
    if (!window.location.pathname.includes("/search")) return;

    const trackLinks = document.querySelectorAll("a[href*='/lyrics/'], a[href*='/lyric/']");
    trackLinks.forEach(link => {
      if (link.dataset.nusaEnhanced) return;
      link.dataset.nusaEnhanced = "true";

      const syncPill = document.createElement("button");
      syncPill.className = "nusa-search-sync-btn";
      syncPill.innerHTML = `<span>⏱️ Nivumi Sync</span>`;
      syncPill.title = "Enviar letra sincronizada diretamente ao Nivumi";

      syncPill.addEventListener("click", async (e) => {
        e.preventDefault();
        e.stopPropagation();

        syncPill.disabled = true;
        syncPill.innerHTML = `<span>Baixando…</span>`;

        try {
          const res = await fetch(link.href);
          if (!res.ok) throw new Error("Fetch failed");
          const html = await res.text();
          const parser = new DOMParser();
          const doc = parser.parseFromString(html, "text/html");

          // Extract LRC from parsed document
          let docLrc = "";
          const targetEl = doc.querySelector("#lyrics_details, .lyrics-body, #lyrics_content, .lyrics_content, textarea, pre, #lrc, .lrc");
          if (targetEl) {
            docLrc = readLrcText(targetEl);
          }
          if (!hasLrcTimestamps(docLrc)) {
            const lines = (readLrcText(doc.body) || "").split("\n").filter(l => hasLrcTimestamps(l.trim()));
            if (lines.length >= 3) {
              docLrc = lines.join("\n");
            }
          }

          if (!docLrc || !hasLrcTimestamps(docLrc)) {
            syncPill.innerHTML = `<span>LRC não encontrado</span>`;
            setTimeout(() => {
              syncPill.disabled = false;
              syncPill.innerHTML = `<span>⏱️ Nivumi Sync</span>`;
            }, 3000);
            return;
          }

          // Metadata
          const h1 = doc.querySelector("h1");
          let songName = link.textContent.trim();
          let artistName = "";
          if (h1 && h1.textContent.includes(" - ")) {
            const parts = h1.textContent.split(" - ");
            artistName = parts[0].trim();
            songName = parts.slice(1).join(" - ").replace(/\s+(?:LRC\s+)?Lyrics(?:\s+LRC)?\s*$/i, "").trim();
          }

          const payload = {
            title: songName,
            artist: artistName,
            original: cleanLrcText(docLrc),
            translation: null,
            source: "lyricsify.com"
          };

          const sent = await sendLrcToNusa(payload);
          if (sent) {
            syncPill.classList.add("sent");
            syncPill.innerHTML = `<span>✓ Sincronizado!</span>`;
            setTimeout(() => {
              syncPill.disabled = false;
              syncPill.classList.remove("sent");
              syncPill.innerHTML = `<span>⏱️ Nivumi Sync</span>`;
            }, 4000);
          } else {
            syncPill.innerHTML = `<span>Nivumi offline</span>`;
            setTimeout(() => {
              syncPill.disabled = false;
              syncPill.innerHTML = `<span>⏱️ Nivumi Sync</span>`;
            }, 3000);
          }
        } catch {
          syncPill.innerHTML = `<span>Erro ao obter</span>`;
          setTimeout(() => {
            syncPill.disabled = false;
            syncPill.innerHTML = `<span>⏱️ Nivumi Sync</span>`;
          }, 3000);
        }
      });

      // Insert adjacent to link
      if (link.parentNode) {
        link.parentNode.insertBefore(syncPill, link.nextSibling);
      }
    });
  }

  // =========================================================================
  // 5. Lifecycle Initialization
  // =========================================================================
  function init() {
    injectFloatingLrcBridge();
    enhanceSearchResults();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }

  // Periodically check for dynamic injections or client-side navigation
  setInterval(() => {
    injectFloatingLrcBridge();
    enhanceSearchResults();
  }, 1500);

  // Message listener for popup
  chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
    if (msg.action === "getLyricsifyData") {
      const meta = getSongMetadata();
      const lrc = extractLrcContent();
      sendResponse({
        success: !!lrc,
        data: {
          title: meta.title,
          artist: meta.artist,
          original: lrc,
          translation: null,
          source: "lyricsify.com",
          url: window.location.href
        }
      });
      return true;
    }
  });
})();
