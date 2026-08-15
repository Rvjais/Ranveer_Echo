"""AirLLM local model server for Ranveer Echo.

Serves an OpenAI-compatible chat/completions API backed by the AirLLM
library (repo cloned at ./airllm), so huge open models (70B on 4GB VRAM,
or CPU-only on machines without a GPU) can be used from the app.

Endpoints:
  GET  /health                    -> {"status", "model", "loaded", "loading", "device", "error"}
  GET  /v1/models                 -> OpenAI-style model list
  POST /v1/chat/completions       -> OpenAI-style chat completion (stream supported)

Console protocol (parsed by the Rust side):
  AIRLLM_SERVER_READY port=<p> device=<d> model=<m>
  AIRLLM_STATE  <json>
  AIRLLM_ERROR  <message>

Usage: venv\\Scripts\\python.exe airllm_server.py --port 8531 [--model HF_ID] [--shards-path PATH]
"""

import argparse
import json
import sys
import threading
import time
import traceback
def _log_to_file(e):
    import traceback
    with open('airllm_debug.log', 'a') as f:
        f.write('EXCEPTION:\n' + traceback.format_exc() + '\n')

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODEL = None          # the loaded AirLLM model (lazy)
TOKENIZER = None      # model.tokenizer
STATE = {
    "model": None,
    "loaded": False,
    "loading": False,
    "device": "cpu",
    "error": None,
}
MODEL_LOCK = threading.Lock()   # guards model loading
GEN_LOCK = threading.Lock()     # one generation at a time (memory)
ARGS = None


def log_state(**kw):
    STATE.update(kw)
    print("AIRLLM_STATE " + json.dumps(STATE), flush=True)


def log_error(msg):
    STATE["error"] = msg
    print("AIRLLM_ERROR " + str(msg)[:500], flush=True)


def _patch_accelerate_for_missing_submodules():
    """Patch accelerate's set_module_tensor_to_device to skip weights whose
    parent submodule doesn't exist in the model (e.g. k_norm / q_norm that
    Qwen3 checkpoints ship but transformers' Qwen2Attention doesn't build).
    Applied once; idempotent."""
    try:
        from accelerate.utils import modeling as accel_mod
        if getattr(accel_mod, '_ranveer_patched', False):
            return
        _orig = accel_mod.set_module_tensor_to_device

        def _safe_set(module, tensor_name, device, **kwargs):
            # Pre-check: walk the dotted path and bail if any intermediate
            # submodule is missing (returns None or doesn't exist).
            if "." in tensor_name:
                parts = tensor_name.split(".")
                m = module
                for part in parts[:-1]:
                    child = getattr(m, part, None)
                    if child is None:
                        # The model class simply doesn't have this sub-module;
                        # skip the weight silently (it wasn't needed).
                        return
                    m = child
                leaf = parts[-1]
                if leaf not in m._parameters and leaf not in m._buffers:
                    return
            return _orig(module, tensor_name, device, **kwargs)

        accel_mod.set_module_tensor_to_device = _safe_set
        accel_mod._ranveer_patched = True
    except Exception:
        pass   # If accelerate isn't installed the patch is irrelevant.


def load_model(model_id, force=False):
    global MODEL, TOKENIZER
    with MODEL_LOCK:
        if MODEL is not None and not force:
            return MODEL, TOKENIZER
        if force and MODEL is not None:
            MODEL = None
            TOKENIZER = None
        log_state(model=model_id, loading=True, error=None)
        try:
            import torch
            _patch_accelerate_for_missing_submodules()
            from airllm import AutoModel
            device = "cuda:0" if torch.cuda.is_available() else "cpu"
            kwargs = {
                "device": device,
                "max_seq_len": ARGS.max_seq_len,
            }
            if ARGS.shards_path:
                kwargs["layer_shards_saving_path"] = ARGS.shards_path
            if ARGS.compression in ("4bit", "8bit"):
                kwargs["compression"] = ARGS.compression
            MODEL = AutoModel.from_pretrained(model_id, **kwargs)
            TOKENIZER = MODEL.tokenizer
            log_state(model=model_id, loaded=True, loading=False, device=device, error=None)
            return MODEL, TOKENIZER
        except Exception as e:
            traceback.print_exc()
            log_state(model=model_id, loaded=False, loading=False, error=str(e))
            raise


def render_prompt(messages):
    tok = TOKENIZER
    if tok is not None:
        try:
            return tok.apply_chat_template(messages, tokenize=False, add_generation_prompt=True)
        except Exception:
            pass
    parts = []
    for m in messages:
        role = m.get("role", "user")
        content = m.get("content", "")
        if role == "system":
            parts.append("System: " + content)
        elif role == "assistant":
            parts.append("Assistant: " + content)
        else:
            parts.append("User: " + content)
    parts.append("Assistant:")
    return "\n".join(parts)


def generate_reply(model_id, messages, max_new_tokens, temperature):
    if not model_id:
        raise RuntimeError("No model set. Enter a Hugging Face model id (e.g. Qwen/Qwen3-8B) in Settings and Start the server again.")
    model, tok = load_model(model_id)
    if model is None:
        raise RuntimeError("no model configured")
    prompt = render_prompt(messages)
    # Never let new tokens exceed the context window (max_seq_len//2 keeps
    # room for the prompt); generation longer than the window errors out.
    max_new = max(1, min(max_new_tokens, ARGS.max_seq_len // 2, 2048))
    input_ids = tok(
        prompt,
        return_tensors="pt",
        truncation=True,
        max_length=max(ARGS.max_seq_len - max_new, 1),
    )["input_ids"]
    input_len = input_ids.shape[1]
    gen_kwargs = dict(
        max_new_tokens=max_new,
        use_cache=True,
        return_dict_in_generate=True,
        eos_token_id=tok.eos_token_id,
        pad_token_id=tok.eos_token_id,
    )
    if temperature > 0.1:
        gen_kwargs.update(do_sample=True, temperature=min(temperature, 2.0))
    with GEN_LOCK:
        out = model.generate(input_ids, **gen_kwargs)
    new_tokens = out.sequences[0][input_len:]
    text = tok.decode(new_tokens, skip_special_tokens=True)
    return text.strip()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def log_message(self, *args):
        pass

    def _send_json(self, code, obj):
        body = json.dumps(obj).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/health":
            self._send_json(200, {"status": "ok", **STATE})
        elif path == "/v1/models":
            models = [STATE["model"]] if STATE["model"] else []
            self._send_json(200, {"object": "list", "data": [{"id": m} for m in models]})
        else:
            self._send_json(404, {"error": {"message": "not found"}})

    def do_POST(self):
        path = self.path.split("?")[0]
        if path == "/v1/models":
            try:
                # To completely avoid OSError 22 on Windows during rfile.read,
                # we'll also accept the model name in a custom header.
                model_header = self.headers.get("X-Model-Id", "").strip()
                if model_header:
                    model_id = model_header
                else:
                    # Fallback to body if header not present
                    length_str = self.headers.get("Content-Length")
                    if length_str:
                        length = int(length_str.split(",")[0].strip())
                        body_bytes = self.rfile.read(length)
                    else:
                        body_bytes = b'{}'
                    
                    if not body_bytes.strip():
                        body_bytes = b'{}'
                        
                    body = json.loads(body_bytes.decode("utf-8"))
                    model_id = (body.get("model") or "").strip()
                
                model_id = model_id or ARGS.model or ""
                if not model_id:
                    self._send_json(400, {"error": {"message": "model required"}})
                    return
                try:
                    load_model(model_id, force=True)
                except Exception as e:
                    traceback.print_exc()
                    log_error(e)
                    self._send_json(500, {"error": {"message": str(e)}})
                    return
                self._send_json(200, {"status": "ok", **STATE})
            except Exception as e:
                traceback.print_exc()
                self._send_json(400, {"error": {"message": f"bad request: {e}"}})
            return
        if path == "/v1/shutdown":
            self._send_json(200, {"status": "shutting down"})
            import os
            threading.Thread(target=lambda: (time.sleep(0.5), os._exit(0))).start()
            return
        if path != "/v1/chat/completions":
            self._send_json(404, {"error": {"message": "not found"}})
            return
        try:
            length_str = self.headers.get("Content-Length")
            if length_str:
                length = int(length_str.split(",")[0].strip())
                body = self.rfile.read(length).decode("utf-8")
            else:
                body = "{}"
                
            if not body.strip():
                body = "{}"
                
            data = json.loads(body)
        except json.JSONDecodeError as e:
            self._send_json(400, {"error": {"message": f"bad request: invalid JSON: {e}"}})
            return
        except OSError as e:
            self._send_json(400, {"error": {"message": f"bad request: could not read request body" + str(e)}})
            return
        except Exception as e:
            self._send_json(400, {"error": {"message": f"bad request: {e}"}})
            return

        messages = [m for m in data.get("messages", []) if isinstance(m, dict)]
        if not messages:
            self._send_json(400, {"error": {"message": "bad request: missing or invalid messages"}})
            return
        model_id = data.get("model") or ARGS.model or "Qwen/Qwen2.5-0.5B"
        max_new = int(data.get("max_tokens", 512) or 512)
        temperature = float(data.get("temperature", 0.7) or 0.7)
        stream = bool(data.get("stream", False))
        messages = [{k: v for k, v in m.items() if k in ("role", "content")} for m in messages]

        try:
            text = generate_reply(model_id, messages, max_new, temperature)
        except Exception as e:
            log_error(e)
            self._send_json(500, {"error": {"message": str(e)}})
            return

        if stream:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.end_headers()
            chunk = ""
            for ch in text:
                chunk += ch
                if len(chunk) >= 4:
                    self._sse({"choices": [{"delta": {"content": chunk}}]})
                    chunk = ""
            if chunk:
                self._sse({"choices": [{"delta": {"content": chunk}}]})
            self._sse({"choices": [{"delta": {}}]})
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
        else:
            self._send_json(200, {
                "id": "chatcmpl-airllm",
                "object": "chat.completion",
                "created": int(time.time()),
                "model": model_id,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": text},
                    "finish_reason": "stop",
                }],
                "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0},
            })

    def _sse(self, obj):
        self.wfile.write(b"data: " + json.dumps(obj).encode("utf-8") + b"\n\n")
        self.wfile.flush()


def main():
    global ARGS
    parser = argparse.ArgumentParser(description="AirLLM OpenAI-compatible server")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8531)
    parser.add_argument("--model", default=None, help="Hugging Face model id, e.g. Qwen/Qwen3-8B")
    parser.add_argument("--shards-path", default=None, help="where to cache the layer-sharded model")
    parser.add_argument("--compression", default=None, choices=[None, "4bit", "8bit"], help="block-wise quantization (needs bitsandbytes)")
    parser.add_argument("--max-seq-len", type=int, default=1024)
    ARGS = parser.parse_args()

    device = "cuda" if _cuda_available() else "cpu"
    log_state(model=ARGS.model, device=device)
    server = ThreadingHTTPServer((ARGS.host, ARGS.port), Handler)
    server.daemon_threads = True
    print(f"AIRLLM_SERVER_READY port={ARGS.port} device={device} model={ARGS.model}", flush=True)
    print(f"AirLLM server listening on http://{ARGS.host}:{ARGS.port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


def _cuda_available():
    try:
        import torch
        return torch.cuda.is_available()
    except Exception:
        return False


if __name__ == "__main__":
    main()