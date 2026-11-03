// Nivumi Lyrics Bridge - Background Service Worker
const NUSA_BASE = "http://127.0.0.1:48123";

// Context Menu Setup for YouTube & web quick search
chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: "pesquisar-letras",
      title: "Buscar letra no Letras.mus.br: '%s'",
      contexts: ["selection"]
    });
    chrome.contextMenus.create({
      id: "pesquisar-lyricsify",
      title: "Buscar letra com tempos no Lyricsify: '%s'",
      contexts: ["selection"]
    });
  });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const rawText = (info.selectionText || "").trim();
  if (!rawText) return;

  const targetUrl = info.menuItemId === "pesquisar-lyricsify"
    ? buildLyricsifySearchUrl(rawText)
    : buildLetrasSearchUrl(rawText);

  try {
    if (chrome.scripting && tab && tab.id) {
      await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        func: (url) => {
          window.open(url, "_blank", "noopener,noreferrer");
        },
        args: [targetUrl]
      });
    } else {
      chrome.tabs.create({ url: targetUrl });
    }
  } catch {
    chrome.tabs.create({ url: targetUrl });
  }
});

// Check health of local Nusa server
async function checkNusaStatus() {
  try {
    const res = await fetch(`${NUSA_BASE}/api/status`, { method: "GET" });
    if (res.ok) {
      const data = await res.json();
      return { online: true, data };
    }
  } catch {
    // Offline
  }
  return { online: false };
}

// Get current song playing in Windows / Nusa
async function getCurrentMedia() {
  try {
    const res = await fetch(`${NUSA_BASE}/api/current`, { method: "GET" });
    if (res.ok) {
      return await res.json();
    }
  } catch {
    // Error or offline
  }
  return null;
}

// Push lyrics payload to Nusa
async function pushLyricsToNusa(payload) {
  try {
    const res = await fetch(`${NUSA_BASE}/api/lyrics`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload)
    });
    return res.ok;
  } catch {
    return false;
  }
}

// Search Letras.mus.br
function buildLetrasSearchUrl(query) {
  const q = encodeURIComponent(query.trim());
  return `https://www.letras.mus.br/?q=${q}#gsc.tab=0&gsc.q=${q}`;
}

// Search Lyricsify.com
function buildLyricsifySearchUrl(query) {
  const q = encodeURIComponent(query.trim());
  return `https://www.lyricsify.com/search?q=${q}`;
}

// Message handler
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg.action === "checkStatus") {
    checkNusaStatus().then(sendResponse);
    return true;
  }
  if (msg.action === "getCurrentMedia") {
    getCurrentMedia().then(sendResponse);
    return true;
  }
  if (msg.action === "pushLyrics") {
    pushLyricsToNusa(msg.payload).then(ok => sendResponse({ success: ok }));
    return true;
  }
  if (msg.action === "openSearchTab") {
    const url = buildLetrasSearchUrl(msg.query);
    chrome.tabs.create({ url });
    sendResponse({ success: true, url });
    return true;
  }
  if (msg.action === "openLyricsifySearchTab") {
    const url = buildLyricsifySearchUrl(msg.query);
    chrome.tabs.create({ url });
    sendResponse({ success: true, url });
    return true;
  }
});
