# RANVEER // H.U.D. — Jarvis-Style Desktop AI Assistant 🤖

> Rust-native Tauri v2 desktop assistant with H.U.D. UI, multi-LLM reasoning, voice wake-word, face + hand-gesture vision, memory, smart-home and computer control.

![Tauri](https://img.shields.io/badge/Tauri-v2-blue)
![Rust](https://img.shields.io/badge/Rust-Tokio-orange)
![Frontend](https://img.shields.io/badge/Frontend-Vanilla_JS-yellow)
![Python](https://img.shields.io/badge/Vision-Python_OpenCV-green)
![LLM](https://img.shields.io/badge/AI-Ollama_OpenAI_Gemini-purple)

## ✨ What It Does

`ranveer_echo` (`com.veer.ranveer`, 1100x720) is a Jarvis-style assistant with H.U.D. interface.

## 🎯 Features in Detail — What it does & How it helps you

| Feature | What it does | How it helps you |
|---------|--------------|------------------|
| 💬 **Streaming AI chat (ReAct tool-loop)** | Chat streams token-by-token, runs up to 3 tool steps (search → open → answer), 180s timeout, SAPI voice read-out, history saved. | Feels instant, not frozen. Complex tasks like “weather + open maps” finish in one prompt. |
| 🎙️ **Voice control + Wake-word** | Windows STT, SAPI TTS with barge-in (interrupt), wake-word `Ranveer`, Hold-SPACE push-to-talk, mic picker + wave visualizer + dB meter. | Hands-free while cooking/coding. Barge-in = no waiting for long speech. |
| 👁️ **Face ID + Hand gestures** | LBPH face register/identify (`known_faces/model.yml`), live camera feed, MediaPipe hand-cursor + pinch-click, camera description via vision LLM. | Auto-login by face, touchless click for presentations / dirty hands, “what do you see?” for blind assistance. |
| 🧠 **Long-term memory + Workspace** | Auto-extracts facts (identity/preferences/projects/relationships), SQLite + `long_term.json`, pin/rename/search/export to `Desktop\ranveer_*.json`. | Remembers birthday, project, wifi password — no repeat prompts. Export = backup/portfolio. |
| 🖥️ **Full computer control** | Open apps, type/press keys, click/screenshot, volume/brightness/lock, browser control, create/read/list/find files, reminders. | “Open VS Code, set 40% volume, screenshot” in one line — accessibility + speed. |
| 📄 **File processor (Office/PDF)** | Reads/writes `docx/xlsx/pptx/pdf` via `openpyxl/python-pptx/reportlab`, disk info + Desktop/Downloads shortcuts. | Auto-generate report/invoice PPT without opening Office. |
| 🏠 **Smart-home (Kasa) + Connect relay** | Controls Kasa plugs/bulbs, tiny_http relay on `:8000` + `OPEN RELAY` button + `ranveer_connect`. | “Turn off bedroom light” from PC. Phone can ping PC remotely. |
| 🤖 **Agent DEPLOY queue** | Goal → planner splits into polled tasks → executor runs → cancel anytime. | “DEPLOY research + make slides” runs in background while you work. |
| 🌐 **Web + YouTube + Weather tools** | `duckduckgo-search`, `beautifulsoup4`, `playwright`, YouTube lookup, weather_report. | Real answers with sources, not hallucinated — plus video + forecast inline. |

## 🛠️ Tech Stack

- **Shell:** Tauri 2.x (`frontendDist=../src`, CSP disabled), `@tauri-apps/cli`
- **Frontend:** Vanilla HTML/CSS/JS — `src/index.html` (9 views), `src/main.js` (~1349 lines), `src/styles.css` (HUD/scanlines)
- **Backend Rust:** `tokio, reqwest 0.12, rusqlite, serde, chrono, uuid, regex, base64, tiny_http`
- **Python Bridge (`bridge.py`):** `opencv-contrib (LBPH), mediapipe (Face+Hands), pyautogui, numpy` — JSON-lines + base64 JPEG subprocess
- **Python AI/OS:** `google-genai, pillow, requests, python-kasa, beautifulsoup4, duckduckgo-search, playwright, pyautogui, pygetwindow, mss, psutil, comtypes/pycaw, openpyxl, python-pptx, reportlab, discord.py, fastapi, edge-tts`
- **AI Engine:** Multi-provider — `ollama (default qwen3-vl:2b) | openai | deepseek | openrouter | gemini | anthropic`. Prompt: `core/prompt_rust.txt`
- **Storage:** `config/workspace_store.sqlite3 + app_settings.json + api_keys.json`, `memory/long_term.json`, `known_faces/model.yml`

## 📁 Project Structure

```
ranveer_echo/
├── src/ index.html, main.js, styles.css, assets/
├── src-tauri/ tauri.conf.json, Cargo.toml
│   └── src/ lib.rs, orchestrator.rs, main.rs
│       ├── ai/ mod.rs, ollama.rs, or_client.rs, llm.rs
│       ├── actions/ open_app, web_search, weather, shell, desktop, file_processor
│       ├── agent/ planner, executor, task_queue
│       └── voice.rs, vision.rs, gesture.rs, memory.rs, workspace_store.rs, dashboard.rs
├── bridge.py, download_model.py
├── core/prompt_rust.txt
├── config/ app_settings.json, api_keys.json
├── memory/, known_faces/
└── requirements.txt, package.json
```

## 🚀 Getting Started

**Prerequisites:** Rust stable, Node 18+, Python 3.10/11, Ollama

```powershell
# 1. Python deps
.\venv\Scripts\python -m pip install -r requirements.txt
# optional local HF model:
pip install "airllm==3.1.0" "transformers<5" torch
python download_model.py  # Qwen2.5-7B-Instruct

# 2. Config - set Ollama host + API keys (or via Settings UI, stays local)
# config/app_settings.json -> provider=ollama, model=qwen3-vl:2b, host=http://localhost:11434
ollama pull qwen3-vl:2b

# 3. Dev
npm install
npx tauri dev

# 4. Build
npx tauri build
```

Rust backend auto-spawns `bridge.py` + dashboard relay on `:8000`.

## 🎯 Usage Examples

- `who am i` → face identify
- `register my face` → save face model
- `open YouTube and play...` → open_app + browser_control
- `remind me in 10 minutes...` → reminder tool
- `DEPLOY build my report` → agent planner queue

## 👤 Author

**Ranveer Jaiswal** — H.U.D. experiment in Tauri + Rust + Python vision.
