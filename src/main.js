const { invoke } = window.__TAURI__.core;

// ── State ────────────────────────────────────────────────────────────────
let activeView = "chat";
let currentConversation = null;
let chatHistory = [];

// ── Navigation ───────────────────────────────────────────────────────────
const sidebar = document.getElementById("sidebar");
const mobileMenuBtn = document.getElementById("mobile-menu-btn");
const sidebarOverlay = document.getElementById("sidebar-overlay");

function closeMobileMenu() {
  if (sidebar) sidebar.classList.remove("open");
  if (sidebarOverlay) sidebarOverlay.classList.remove("open");
}

if (mobileMenuBtn && sidebar && sidebarOverlay) {
  mobileMenuBtn.addEventListener("click", () => {
    sidebar.classList.add("open");
    sidebarOverlay.classList.add("open");
  });
  sidebarOverlay.addEventListener("click", closeMobileMenu);
}

document.querySelectorAll(".nav-btn").forEach((btn) => {
  btn.addEventListener("click", (e) => {
    e.preventDefault();
    document.querySelectorAll(".nav-btn").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    activeView = btn.dataset.view;
    document.querySelectorAll(".view").forEach((v) => v.classList.remove("active"));
    const target = document.getElementById(`view-${activeView}`);
    if (target) target.classList.add("active");
    refreshView(activeView);
    closeMobileMenu();
  });
});

function refreshView(view) {
  if (view === "workspace") loadConversations();
  else if (view === "memory") loadMemory();
  else if (view === "smart-home") loadSmartHome();
  else if (view === "connect") loadConnect();
  else if (view === "attention") loadAttention();
  else if (view === "agent") refreshAgentStatus();
  else if (view === "settings") syncAllState();
  else if (view === "chat") loadConfig();
}

// ── Chat ────────────────────────────────────────────────────────────────
const chatForm = document.getElementById("chat-form");
const chatInput = document.getElementById("chat-input");
const chatLog = document.getElementById("chat-log");

function appendMessage(role, text) {
  const div = document.createElement("div");
  div.className = `msg ${role}`;
  const label = document.createElement("div");
  label.className = "msg-label";
  label.textContent = role === "user" ? "OPERATOR" : "RANVEER";
  const body = document.createElement("div");
  body.className = "msg-body";
  body.textContent = text;
  body.style.whiteSpace = "pre-wrap";
  div.appendChild(label);
  div.appendChild(body);
  chatLog.appendChild(div);
  chatLog.scrollTop = chatLog.scrollHeight;
  return body;
}

// ── Conversation history in the chat view ──────────────────────────────
function renderConversationDetail(detail) {
  chatLog.innerHTML = "";
  (detail.messages || []).forEach((m) => {
    if (m.role !== "system") appendMessage(m.role, m.content);
  });
}

async function loadConversation(id) {
  currentConversation = id;
  try {
    await invoke("store_activate_conversation", { conversationId: id });
    const detail = await invoke("store_get_conversation", { conversationId: id });
    renderConversationDetail(detail);
  } catch (_) {}
}

// Loads the active (or most recent) conversation so the user sees their full
// history as soon as the app opens.
async function initChatHistory() {
  try {
    const activeId = await invoke("store_active_conversation");
    if (activeId) {
      const detail = await invoke("store_get_conversation", { conversationId: activeId });
      currentConversation = activeId;
      renderConversationDetail(detail);
      return;
    }
  } catch (_) {}
  try {
    const convos = await invoke("store_list_conversations", { search: "" });
    if (convos.length) {
      await loadConversation(convos[0].id);
    }
  } catch (_) {}
}

chatForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const text = chatInput.value.trim();
  if (!text) return;
  chatInput.value = "";
  appendMessage("user", text);
  const thinking = appendMessage("assistant", "");
  const activity = document.createElement("div");
  activity.className = "chat-activity";
  chatLog.appendChild(activity);
  let unlisten = null;
  try {
    unlisten = await window.__TAURI__.event.listen("chat-stream", (event) => {
      const d = event.payload;
      if (!d) return;
      if (d.kind === "status") {
        activity.textContent = d.data;
      } else if (d.kind === "text") {
        thinking.textContent += d.data;
        chatLog.scrollTop = chatLog.scrollHeight;
      }
    });
    const reply = await invoke("chat_stream", { text });
    thinking.textContent = reply;
    if (!voiceRunning) {
      try { await invoke("speak", { text: reply }); } catch (_) {}
    }
  } catch (err) {
    thinking.textContent = `Error: ${err}`;
  } finally {
    if (unlisten) unlisten();
    activity.remove();
    loadConfig();
  }
});

async function newConversation() {
  chatLog.innerHTML = "";
  currentConversation = null;
  chatHistory = [];
  try {
    const id = await invoke("store_new_conversation", { title: "New Conversation" });
    currentConversation = id;
    await invoke("store_activate_conversation", { conversationId: id });
  } catch (_) {}
  document.querySelector('[data-view="chat"]').click();
}

document.getElementById("new-chat-btn").addEventListener("click", newConversation);

// ── Config indicator ────────────────────────────────────────────────────
async function loadConfig() {
  const el = document.getElementById("config-indicator");
  if (!el) return;
  try {
    const cfg = await invoke("config_summary");
    const model = cfg.ai_model || cfg.local_model || "";
    let stateTag = "offline";
    let isOk = false;

    if (cfg.ai_provider === "airllm") {
      if (cfg.airllm_loading) {
        stateTag = "loading model…";
        isOk = true;
      } else if (cfg.airllm_loaded) {
        stateTag = "ready";
        isOk = true;
      } else if (cfg.airllm_running) {
        stateTag = "ready";
        isOk = true;
      } else {
        stateTag = "offline";
        isOk = false;
      }
    } else if (cfg.ai_online) {
      stateTag = "online";
      isOk = true;
    }

    el.textContent = `AI: ${cfg.ai_engine}${model ? " · " + model : ""} (${stateTag}) · ${cfg.os}`;
    el.classList.toggle("ok", isOk);
  } catch (err) {
    el.textContent = `Error: ${err}`;
  }
}

// ── Workspace / conversations ───────────────────────────────────────────
const wsSearch = document.getElementById("ws-search");
wsSearch.addEventListener("input", debounce(loadConversations, 300));

async function loadConversations() {
  const list = document.getElementById("conversation-list");
  const convos = await invoke("store_list_conversations", { search: wsSearch.value });
  list.innerHTML = "";
  if (!convos.length) {
    list.innerHTML = '<p class="empty">No conversations yet.</p>';
    return;
  }
  for (const c of convos) {
    const card = document.createElement("div");
    card.className = "convo-card";
    const title = document.createElement("div");
    title.className = "convo-title";
    title.textContent = c.title;
    const meta = document.createElement("div");
    meta.className = "convo-meta";
    meta.textContent = new Date(c.updatedAt).toLocaleString() + (c.pinned ? " · Pinned" : "");
    card.appendChild(title);
    card.appendChild(meta);
    card.addEventListener("click", async () => {
      document.querySelectorAll(".convo-card").forEach((c) => c.classList.remove("active"));
      card.classList.add("active");
      currentConversation = c.id;
      await invoke("store_activate_conversation", { conversationId: c.id });
      const detail = await invoke("store_get_conversation", { conversationId: c.id });
      renderConversationDetail(detail);
      document.querySelector('[data-view="chat"]').click();
    });
    list.appendChild(card);
  }
}

// ── Memory ──────────────────────────────────────────────────────────────
async function loadMemory() {
  const list = document.getElementById("memory-list");
  const mem = await invoke("memory_all");
  list.innerHTML = "";
  const catNames = {
    identity: "Identity",
    preferences: "Preferences",
    projects: "Projects",
    relationships: "Relationships",
    wishes: "Wishes / Plans",
    notes: "Notes",
  };
  for (const [cat, items] of Object.entries(mem)) {
    if (!items || !Object.keys(items).length) continue;
    const section = document.createElement("div");
    section.className = "memory-section";
    const h = document.createElement("h3");
    h.textContent = catNames[cat] || cat;
    section.appendChild(h);
    for (const [key, entry] of Object.entries(items)) {
      const row = document.createElement("div");
      row.className = "memory-row";
      const label = document.createElement("span");
      label.className = "memory-key";
      label.textContent = key;
      const val = document.createElement("span");
      val.className = "memory-value";
      val.textContent = entry.value || "";
      const rm = document.createElement("button");
      rm.className = "ghost-btn small";
      rm.textContent = "x";
      rm.addEventListener("click", async () => {
        await invoke("memory_forget", { category: cat, key });
        loadMemory();
      });
      row.appendChild(label);
      row.appendChild(val);
      row.appendChild(rm);
      section.appendChild(row);
    }
    list.appendChild(section);
  }
}

document.getElementById("memory-refresh").addEventListener("click", loadMemory);

// ── Smart Home ──────────────────────────────────────────────────────────
async function loadSmartHome() {
  const grid = document.getElementById("smart-home-platforms");
  const platforms = await invoke("smart_home_platforms");
  grid.innerHTML = "";
  (platforms || []).forEach((p) => {
    const card = document.createElement("div");
    card.className = "card";
    const h = document.createElement("h3");
    h.textContent = p.name;
    card.appendChild(h);
    const badge = document.createElement("span");
    badge.className = "status-pill";
    badge.textContent = p.coming_soon ? "Coming soon" : "Available";
    card.appendChild(badge);
    grid.appendChild(card);
  });
}

// ── Connect ─────────────────────────────────────────────────────────────
async function loadConnect() {
  const grid = document.getElementById("connect-devices");
  const info = await invoke("connect_list_devices");
  grid.innerHTML = "";
  (info.devices || []).forEach((d) => {
    const card = document.createElement("div");
    card.className = "card";
    const h = document.createElement("h3");
    h.textContent = d.name || "Device";
    card.appendChild(h);
    const badge = document.createElement("span");
    badge.className = "status-pill";
    badge.textContent = d.online ? "Online" : "Offline";
    badge.classList.toggle("ok", !!d.online);
    card.appendChild(badge);
    grid.appendChild(card);
  });
}

// ── Agent ───────────────────────────────────────────────────────────────
document.getElementById("agent-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const goal = document.getElementById("agent-goal").value.trim();
  if (!goal) return;
  const taskId = await invoke("agent_submit_task", { goal });
  document.getElementById("agent-goal").value = "";
  refreshAgentStatus();
  pollAgent(taskId);
});

async function pollAgent(taskId) {
  for (let i = 0; i < 120; i++) {
    const st = await invoke("agent_task_status", { taskId });
    if (!st) break;
    refreshAgentStatus();
    if (st.result || st.status === "failed" || st.status === "cancelled") break;
    await new Promise((r) => setTimeout(r, 2000));
  }
}

function refreshAgentStatus() {
  invoke("agent_tasks")
    .then((tasks) => {
      const el = document.getElementById("agent-status");
      el.innerHTML = "";
      tasks.forEach((t) => {
        let html = `<div class="agent-task"><strong>${t.task_id}</strong> · ${t.status}<br/><span style="color:var(--muted)">${t.goal}</span></div>`;
        if (t.result) {
          html += `<div class="agent-result">${t.result}</div>`;
        }
        el.innerHTML += html;
      });
    })
    .catch(() => {});
}

// ── Voice ────────────────────────────────────────────────────────────────
const voiceBtn = document.getElementById("voice-btn");
const voiceStatus = document.getElementById("voice-status");
const pushBtn = document.getElementById("push-to-talk");
const orb = document.getElementById("orb");
const orbLabel = document.getElementById("orb-label");
let voiceRunning = false;
let voicePoll = null;
let pushActive = false;
let pttGraceTimer = null;

const PTT_GRACE_MS = 8000; // keep listening after release so the utterance + reply finish

function setVoiceStatus(text, cls) {
  if (voiceStatus) {
    voiceStatus.textContent = text;
    voiceStatus.className = `status-pill ${cls || ""}`;
  }
}

function setOrbState(state) {
  // state: idle | listening | speaking
  orb.classList.toggle("listening", state === "listening");
  orb.classList.toggle("speaking", state === "speaking");
  if (state === "listening") orbLabel.textContent = "Listening…";
  else if (state === "speaking") orbLabel.textContent = "Speaking…";
  else orbLabel.textContent = voiceRunning || pushActive ? "Listening…" : "Tap to talk";
  orbLabel.classList.toggle("active", state !== "idle");
}

function updateVoiceStatus() {
  if (voiceRunning) {
    setVoiceStatus("Listening…", "listening");
    setOrbState("listening");
  } else {
    setVoiceStatus("Voice off");
    setOrbState("idle");
  }
}

async function startVoice(ptt) {
  try {
    await invoke("voice_start", { ptt: !!ptt });
    voiceRunning = true;
    voiceBtn.textContent = "Stop Voice";
    updateVoiceStatus();
    voicePoll = setInterval(async () => {
      try {
        const st = await invoke("voice_state");
        if (st.speaking) {
          setOrbState("speaking");
          setVoiceStatus("Speaking…", "speaking");
        } else if (voiceRunning || pushActive) {
          setOrbState("listening");
          setVoiceStatus("Listening…", "listening");
        } else {
          setOrbState("idle");
          setVoiceStatus("Voice off");
        }
      } catch (_) {}
    }, 800);
  } catch (err) {
    setVoiceStatus(`Voice error: ${err}`);
    setOrbState("idle");
  }
}

async function stopVoice() {
  if (pttGraceTimer) {
    clearTimeout(pttGraceTimer);
    pttGraceTimer = null;
  }
  try {
    await invoke("voice_stop");
  } catch (_) {}
  voiceRunning = false;
  if (voicePoll) clearInterval(voicePoll);
  voicePoll = null;
  voiceBtn.textContent = "Mic Voice";
  updateVoiceStatus();
}

voiceBtn.addEventListener("click", async () => {
  if (!voiceRunning) {
    await startVoice(false);
  } else {
    await stopVoice();
  }
});

// Push-to-talk: hold to talk, release to stop
function beginPushTalk() {
  if (voiceRunning) return; // already in continuous mode
  if (pttGraceTimer) {
    clearTimeout(pttGraceTimer);
    pttGraceTimer = null;
  }
  pushActive = true;
  pushBtn.classList.add("holding");
  pushBtn.textContent = "Release to stop";
  setOrbState("listening");
  startVoice(true);
}

function endPushTalk() {
  if (!pushActive) return;
  pushActive = false;
  pushBtn.classList.remove("holding");
  pushBtn.textContent = "Hold to Talk";
  // Don't kill the recognizer instantly — the utterance may still be
  // completing and the reply needs time to come back. Stop after a grace
  // period, or immediately if the session was never started.
  if (voiceRunning) {
    setOrbState("idle");
    pttGraceTimer = setTimeout(() => {
      pttGraceTimer = null;
      if (!pushActive) stopVoice();
    }, PTT_GRACE_MS);
  } else {
    setOrbState("idle");
  }
}

pushBtn.addEventListener("pointerdown", (e) => {
  e.preventDefault();
  beginPushTalk();
});
pushBtn.addEventListener("pointerup", endPushTalk);
pushBtn.addEventListener("pointerleave", () => {
  if (pushActive) endPushTalk();
});
pushBtn.addEventListener("pointercancel", () => {
  if (pushActive) endPushTalk();
});

// Tap the orb to toggle voice
orb.addEventListener("click", () => {
  if (voiceRunning || pushActive) {
    stopVoice();
  } else {
    beginPushTalk();
  }
});

// Global keyboard shortcut: hold Space (in chat view) to talk
document.addEventListener("keydown", (e) => {
  if (e.code === "Space" && !e.repeat && !pushActive && !voiceRunning) {
    const chatView = document.getElementById("view-chat");
    const isTyping = document.activeElement && ["INPUT", "TEXTAREA"].includes(document.activeElement.tagName);
    if (chatView.classList.contains("active") && !isTyping) {
      e.preventDefault();
      beginPushTalk();
    }
  }
});

document.addEventListener("keyup", (e) => {
  if (e.code === "Space" && pushActive) {
    endPushTalk();
  }
});

// ── Settings / AI engine / API keys ─────────────────────────────────────
const savedFlash = document.getElementById("settings-saved");

function flashSaved(msg) {
  savedFlash.textContent = msg || "Saved ✓";
  savedFlash.style.opacity = 1;
  setTimeout(() => {
    savedFlash.style.opacity = 0;
  }, 2000);
}

const providerSelect = document.getElementById("ai-provider");
const modelInput = document.getElementById("ai-model-input");
const keyInput = document.getElementById("ai-key-input");
const testResult = document.getElementById("ai-test-result");

const KEY_FIELDS = {
  airllm: null,
  openai: "openai_api_key",
  deepseek: "deepseek_api_key",
  openrouter: "openrouter_api_key",
  gemini: "gemini_api_key",
  anthropic: "anthropic_api_key",
};

const MODEL_PLACEHOLDERS = {
  airllm: "e.g. Qwen/Qwen3-0.6B (Hugging Face id)",
  openai: "gpt-4o-mini",
  deepseek: "deepseek-chat",
  openrouter: "openai/gpt-4o-mini",
  gemini: "gemini-2.5-flash",
  anthropic: "claude-3-5-haiku-latest",
};

function updateKeyFieldVisibility() {
  const p = providerSelect.value;
  const isLocal = p === "airllm";
  keyInput.parentElement.style.opacity = isLocal ? 0.4 : 1;
  keyInput.placeholder = isLocal ? "(not used for local)" : "Paste your API key…";
  modelInput.placeholder = MODEL_PLACEHOLDERS[p] || "Model name";
}

async function loadKeyFields() {
  try {
    const cfg = await invoke("config_summary");
    const set = (id, text, ok) => {
      const el = document.getElementById(id);
      if (!el) return;
      el.textContent = text;
      el.classList.toggle("ok", !!ok);
    };
    set("ai-engine", cfg.ai_engine || "Local (AirLLM)", true);
    set("ai-model", cfg.ai_model || cfg.local_model || "…", true);

    let statusText = "offline";
    let isOk = false;
    if (cfg.ai_provider === "airllm") {
      if (cfg.airllm_loading) {
        statusText = "AirLLM loading model…";
        isOk = true;
      } else if (cfg.airllm_loaded || cfg.airllm_running) {
        statusText = "AirLLM ready";
        isOk = true;
      } else {
        statusText = "AirLLM offline";
        isOk = false;
      }
    } else if (cfg.ai_online) {
      statusText = `${cfg.ai_provider || "AI"} online`;
      isOk = true;
    } else {
      statusText = `${cfg.ai_provider || "AI"} offline`;
      isOk = false;
    }
    set("ai-status", statusText, isOk);

    if (providerSelect && cfg.ai_provider) providerSelect.value = cfg.ai_provider;
    if (modelInput) modelInput.value = cfg.ai_model || "";
    if (airllmModelSelect && cfg.ai_model) airllmModelSelect.value = cfg.ai_model;
    if (keyInput) keyInput.value = "";
    updateKeyFieldVisibility();
  } catch (err) {
    const el = document.getElementById("ai-status");
    if (el) el.textContent = `Error: ${err}`;
  }
}

if (providerSelect) {
  providerSelect.addEventListener("change", updateKeyFieldVisibility);
}

document.getElementById("ai-save-btn").addEventListener("click", async () => {
  try {
    await invoke("config_set_ai_provider", { provider: providerSelect.value });
    if (modelInput.value.trim()) {
      if (providerSelect.value === "airllm") {
        await invoke("config_set_airllm_model", { model: modelInput.value.trim() });
      } else {
        await invoke("config_set_ai_model", { model: modelInput.value.trim() });
      }
    }
    if (keyInput.value.trim()) {
      const keyName = KEY_FIELDS[providerSelect.value];
      if (keyName) {
        await invoke("config_set_api_key", { name: keyName, value: keyInput.value.trim() });
      }
      keyInput.value = "";
    }
    if (providerSelect.value === "airllm" && airllmStartBtn) {
      airllmStartBtn.click();
    }
    flashSaved("Saved ✓");
    await syncAllState();
  } catch (err) {
    flashSaved(`Error: ${err}`);
  }
});

document.getElementById("ai-test-btn").addEventListener("click", async () => {
  testResult.textContent = "Testing…";
  testResult.classList.remove("ok");
  try {
    const res = await invoke("ai_test_connection");
    testResult.textContent = `${res.provider} · ${res.model} · ${res.latency_ms}ms · "${res.reply}"`;
    testResult.classList.add("ok");
    await syncAllState();
  } catch (err) {
    testResult.textContent = String(err);
  }
});

// ── AirLLM server control ───────────────────────────────────────────────
const airllmStatus = document.getElementById("airllm-status");

async function refreshAirllmStatus() {
  if (!airllmStatus) return;
  try {
    const s = await invoke("airllm_status");
    const bits = [];
    if (s.running) bits.push("RUNNING");
    else bits.push("OFFLINE");
    if (s.loading) bits.push("LOADING MODEL…");
    if (s.loaded) bits.push(`READY · ${s.model} · ${s.device}`);
    if (s.error) bits.push(`ERROR: ${s.error}`);
    airllmStatus.textContent = bits.join(" · ") || "Not running";
    airllmStatus.classList.toggle("ok", !!(s.running && !s.loading && !s.error));
  } catch (err) {
    airllmStatus.textContent = `Error: ${err}`;
  }
}

async function syncAllState() {
  await Promise.allSettled([
    loadConfig(),
    refreshAirllmStatus(),
    loadKeyFields(),
  ]);
}

const airllmStartBtn = document.getElementById("airllm-start-btn");
if (airllmStartBtn) {
  airllmStartBtn.addEventListener("click", async () => {
    airllmStatus.textContent = "Starting…";
    try {
      const model = modelInput.value.trim() || null;
      const res = await invoke("airllm_start", { model });
      if (res.started) {
        airllmStatus.textContent = "STARTED — waiting for model load…";
      } else {
        airllmStatus.textContent = "Already running";
      }
      setTimeout(syncAllState, 1500);
    } catch (err) {
      airllmStatus.textContent = `Error: ${err}`;
    }
  });
}

const airllmStopBtn = document.getElementById("airllm-stop-btn");
if (airllmStopBtn) {
  airllmStopBtn.addEventListener("click", async () => {
    try {
      await invoke("airllm_stop");
      airllmStatus.textContent = "Stopped";
      airllmStatus.classList.remove("ok");
      await syncAllState();
    } catch (err) {
      airllmStatus.textContent = `Error: ${err}`;
    }
  });
}

// ── AirLLM model picker + install ───────────────────────────────────────
const airllmModelSelect = document.getElementById("airllm-model-select");
if (airllmModelSelect) {
  airllmModelSelect.addEventListener("change", () => {
    if (modelInput) modelInput.value = airllmModelSelect.value;
  });
}

const airllmInstallBtn = document.getElementById("airllm-install-btn");
if (airllmInstallBtn) {
  airllmInstallBtn.addEventListener("click", async () => {
    const model = (modelInput.value || airllmModelSelect?.value || "").trim();
    if (!model) {
      flashSaved("Enter a Hugging Face model id first");
      return;
    }
    airllmInstallBtn.textContent = "INSTALLING…";
    airllmInstallBtn.disabled = true;
    airllmStatus.textContent = "INSTALLING — downloading + layer-sharding…";
    try {
      await invoke("config_set_airllm_model", { model });
      if (modelInput) modelInput.value = model;
      const res = await invoke("airllm_install", { model });
      if (res.ok) {
        airllmStatus.textContent = `READY · ${res.model} · ${res.device}`;
        airllmStatus.classList.add("ok");
        flashSaved(`Installed ✓ ${res.model}`);
      } else {
        airllmStatus.textContent = "Install finished with warnings";
      }
      await syncAllState();
    } catch (err) {
      airllmStatus.textContent = `Error: ${err}`;
      flashSaved(`Install error: ${err}`);
    }
    airllmInstallBtn.textContent = "INSTALL";
    airllmInstallBtn.disabled = false;
    setTimeout(syncAllState, 2000);
  });
}

// Real-time synchronization
syncAllState();
setInterval(syncAllState, 3000);

if (window.__TAURI__?.event) {
  window.__TAURI__.event.listen("airllm-state", () => {
    syncAllState();
  });
}

const keyToggleBtn = document.getElementById("key-toggle-btn");
if (keyToggleBtn) {
  keyToggleBtn.addEventListener("click", () => {
    keyInput.type = keyInput.type === "password" ? "text" : "password";
    keyToggleBtn.textContent = keyInput.type === "password" ? "SHOW" : "HIDE";
  });
}

// ── Dashboard ────────────────────────────────────────────────────────────
document.getElementById("open-dashboard").addEventListener("click", () => {
  window.open("http://localhost:8000", "_blank");
});

// ── Attention / Camera ──────────────────────────────────────────────────
async function loadAttention() {
  const el = document.getElementById("attention-state");
  try {
    const st = await invoke("attention_status");
    const mins = (st.idle_seconds / 60).toFixed(1);
    el.innerHTML = st.away
      ? `You've been away for ${mins} min. Active window: ${st.active_window || "—"}`
      : `Idle ${st.idle_seconds}s. Active window: ${st.active_window || "—"}`;
  } catch (err) {
    el.textContent = `Error: ${err}`;
  }
}

document.getElementById("attention-refresh").addEventListener("click", loadAttention);

// ── Live camera box (Chat page) ─────────────────────────────────────────
const cameraFeed = document.getElementById("camera-feed");
const cameraState = document.getElementById("camera-state");
const cameraGesture = document.getElementById("camera-gesture");
const cameraFace = document.getElementById("camera-face");

window.__TAURI__.event.listen("camera-frame", (event) => {
  const d = event.payload;
  if (!d || !d.frame) {
    if (cameraState) cameraState.textContent = "NO CAMERA";
    return;
  }
  if (cameraFeed && cameraFeed.src !== `data:image/jpeg;base64,${d.frame}`) {
    cameraFeed.src = `data:image/jpeg;base64,${d.frame}`;
    if (cameraState) cameraState.textContent = "LIVE";
  }
  if (cameraGesture) cameraGesture.textContent = d.gesture ? String(d.gesture).toUpperCase() : "—";
  if (cameraFace) {
    cameraFace.textContent = d.face ? `${d.face}${d.confidence ? ` (${d.confidence}%)` : ""}` : "—";
  }
});

async function startCamera() {
  try {
    const res = await invoke("camera_stream_start");
    if (cameraState) cameraState.textContent = "STARTING…";
  } catch (err) {
    if (cameraState) cameraState.textContent = "ERROR";
    console.error("camera_stream_start failed:", err);
  }
}

async function stopCamera() {
  try {
    await invoke("camera_stream_stop");
    if (cameraState) cameraState.textContent = "OFF";
    if (cameraFeed) cameraFeed.src = "";
  } catch (_) {}
}

let cameraMinimized = false;
const cameraMinimizeBtn = document.getElementById("camera-minimize-btn");
const cameraBox = document.getElementById("camera-box");
if (cameraMinimizeBtn) {
  cameraMinimizeBtn.addEventListener("click", () => {
    cameraMinimized = !cameraMinimized;
    cameraBox.classList.toggle("minimized", cameraMinimized);
    cameraMinimizeBtn.textContent = cameraMinimized ? "⛁" : "⛁";
  });
}

// ── Gesture control ────────────────────────────────────────────────────
let gesturesRunning = false;
const gestureBtn = document.getElementById("gesture-btn");
const gestureState = document.getElementById("gesture-state");
if (gestureBtn) {
  gestureBtn.addEventListener("click", async () => {
    if (!gesturesRunning) {
      try {
        await invoke("gesture_start");
        gesturesRunning = true;
        gestureBtn.textContent = "STOP GESTURES";
        gestureState.textContent = "RUNNING — hand steers cursor, pinch = click";
      } catch (err) {
        gestureState.textContent = `Error: ${err}`;
      }
    } else {
      await invoke("gesture_stop");
      gesturesRunning = false;
      gestureBtn.textContent = "START GESTURES";
      gestureState.textContent = "IDLE";
    }
  });
}

// ── Face recognition ───────────────────────────────────────────────────
const faceResult = document.getElementById("face-result");
document.getElementById("face-register").addEventListener("click", async () => {
  const name = document.getElementById("face-name").value.trim();
  if (!name) {
    faceResult.textContent = "Enter a name first.";
    return;
  }
  faceResult.textContent = `Registering ${name}… look at the camera`;
  try {
    const res = await invoke("face_register", { name });
    faceResult.textContent = res.ok
      ? `Registered "${name}". You can now use WHO AM I?`
      : `Registration failed: ${res.error || "unknown"}`;
  } catch (err) {
    faceResult.textContent = `Error: ${err}`;
  }
});

document.getElementById("face-identify").addEventListener("click", async () => {
  faceResult.textContent = "Looking…";
  try {
    const res = await invoke("face_identify");
    faceResult.textContent = res.name
      ? `Hello, ${res.name} (confidence ${res.confidence})`
      : "No known face detected.";
  } catch (err) {
    faceResult.textContent = `Error: ${err}`;
  }
});

document.getElementById("face-refresh").addEventListener("click", async () => {
  try {
    const res = await invoke("face_list");
    const names = res.names || [];
    faceResult.textContent = names.length
      ? `Known faces: ${names.join(", ")}`
      : "No faces registered yet.";
  } catch (err) {
    faceResult.textContent = `Error: ${err}`;
  }
});

// ── Utils ───────────────────────────────────────────────────────────────
function debounce(fn, ms) {
  let t;
  return (...args) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...args), ms);
  };
}

// ── Live speech-to-text (voice transcript) ─────────────────────────────
const liveTranscriptText = document.getElementById("live-transcript-text");
const liveTranscriptPanel = document.getElementById("live-transcript");
let liveTranscriptTimer = null;

function showLiveTranscript(role, text) {
  if (!liveTranscriptText) return;
  liveTranscriptText.textContent = text;
  liveTranscriptPanel.classList.toggle("speaking", role === "user");
  if (liveTranscriptTimer) clearTimeout(liveTranscriptTimer);
  liveTranscriptTimer = setTimeout(() => {
    liveTranscriptText.textContent = "Awaiting speech input…";
    liveTranscriptPanel.classList.remove("speaking");
  }, 2500);
}

window.__TAURI__.event.listen("voice-transcript", (event) => {
  const d = event.payload;
  if (!d || !d.text) return;
  if (d.role === "user") {
    appendMessage("user", d.text);
  } else if (d.role === "assistant") {
    appendMessage("assistant", d.text);
  }
  showLiveTranscript(d.role, d.text);
});

// Auto-start the live camera feed on the main page (no clicking needed).
startCamera();

loadConfig();
loadKeyFields();
initChatHistory();
