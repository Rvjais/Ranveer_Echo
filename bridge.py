import argparse
import base64
import json
import os
import sys
import time
from pathlib import Path

import cv2
import numpy as np

BASE_DIR = Path(__file__).resolve().parent
FACES_DIR = BASE_DIR / "known_faces"
FACES_DIR.mkdir(exist_ok=True)
MODEL_PATH = FACES_DIR / "model.yml"
LABELS_PATH = FACES_DIR / "labels.json"

FACE_SIZE = (200, 200)
RECOGNITION_THRESHOLD = 90.0


def _face_detector():
    import mediapipe as mp

    return mp.solutions.face_detection.FaceDetection(
        model_selection=0, min_detection_confidence=0.5
    )


def _out(msg: str) -> None:
    print(msg, flush=True)


def _out_json(obj) -> None:
    _out(json.dumps(obj))


def _find_camera() -> int | None:
    for idx in range(0, 4):
        cap = cv2.VideoCapture(idx, cv2.CAP_DSHOW)
        ok, frame = cap.read()
        cap.release()
        if ok and frame is not None:
            return idx
    return None


def _open_camera():
    idx = _find_camera()
    if idx is None:
        return None
    cap = cv2.VideoCapture(idx, cv2.CAP_DSHOW)
    cap.set(cv2.CAP_PROP_FRAME_WIDTH, 640)
    cap.set(cv2.CAP_PROP_FRAME_HEIGHT, 480)
    return cap


def _crop_face(rgb, detector) -> np.ndarray | None:
    results = detector.process(rgb)
    if not results.detections:
        return None
    det = results.detections[0]
    bb = det.location_data.relative_bounding_box
    h, w, _ = rgb.shape
    x = max(0, int(bb.xmin * w))
    y = max(0, int(bb.ymin * h))
    bw = int(bb.width * w)
    bh = int(bb.height * h)
    x2 = min(w, x + bw)
    y2 = min(h, y + bh)
    if x2 <= x or y2 <= y:
        return None
    face_rgb = rgb[y:y2, x:x2]
    gray = cv2.cvtColor(face_rgb, cv2.COLOR_RGB2GRAY)
    if gray.size == 0:
        return None
    return cv2.resize(gray, FACE_SIZE)


def _load_labels() -> dict[str, int]:
    if LABELS_PATH.exists():
        try:
            return json.loads(LABELS_PATH.read_text(encoding="utf-8"))
        except Exception:
            pass
    return {}


def _save_labels(labels: dict[str, int]) -> None:
    LABELS_PATH.write_text(
        json.dumps(labels, indent=2, ensure_ascii=False), encoding="utf-8"
    )


def _load_recognizer():
    rec = cv2.face.LBPHFaceRecognizer_create()
    if MODEL_PATH.exists():
        rec.read(str(MODEL_PATH))
    return rec


def _save_recognizer(rec) -> None:
    rec.write(str(MODEL_PATH))


def cmd_gesture() -> None:
    try:
        import pyautogui
    except ImportError:
        _out("pyautogui is required for gesture control")
        return

    import mediapipe as mp

    pyautogui.FAILSAFE = False
    mp_hands = mp.solutions.hands
    hands = mp_hands.Hands(
        static_image_mode=False,
        max_num_hands=1,
        min_detection_confidence=0.6,
        min_tracking_confidence=0.6,
    )
    screen_w, screen_h = pyautogui.size()

    cap = _open_camera()
    if cap is None:
        _out("No camera found")
        return

    smooth_x, smooth_y = screen_w / 2, screen_h / 2
    prev_pinch = False
    _out("GESTURE READY")

    while True:
        ok, frame = cap.read()
        if not ok:
            time.sleep(0.05)
            continue
        frame = cv2.flip(frame, 1)
        rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
        results = hands.process(rgb)
        if results.multi_hand_landmarks:
            lm = results.multi_hand_landmarks[0].landmark
            index_tip = lm[8]
            thumb_tip = lm[4]
            dx = thumb_tip.x - index_tip.x
            dy = thumb_tip.y - index_tip.y
            pinch = (dx * dx + dy * dy) ** 0.5 < 0.05
            if pinch and not prev_pinch:
                pyautogui.click()
            prev_pinch = pinch

            target_x = int(index_tip.x * screen_w)
            target_y = int(index_tip.y * screen_h)
            smooth_x += (target_x - smooth_x) * 0.45
            smooth_y += (target_y - smooth_y) * 0.45
            pyautogui.moveTo(int(smooth_x), int(smooth_y), _pause=False)
        else:
            prev_pinch = False
        time.sleep(0.01)


def cmd_register(name: str) -> None:
    name = name.strip()
    if not name:
        _out_json({"ok": False, "error": "empty name"})
        return
    labels = _load_labels()
    if name not in labels:
        labels[name] = max(labels.values(), default=-1) + 1
        _save_labels(labels)

    rec = _load_recognizer()
    detector = _face_detector()
    cap = _open_camera()
    if cap is None:
        _out_json({"ok": False, "error": "no camera"})
        return

    samples = []
    last = time.time()
    while len(samples) < 10:
        ok, frame = cap.read()
        if not ok:
            continue
        rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
        face = _crop_face(rgb, detector)
        if face is not None:
            samples.append(face)
        now = time.time()
        if now - last < 0.2:
            continue
        last = now
        _out(f"captured {len(samples)}/10")
    cap.release()

    if len(samples) < 5:
        _out_json({"ok": False, "error": "not enough face samples"})
        return
    id_num = labels[name]
    rec.train(
        np.array(samples, dtype=np.uint8),
        np.array([id_num] * len(samples), dtype=np.int32),
    )
    _save_recognizer(rec)
    _out_json({"ok": True, "name": name})


def cmd_identify() -> None:
    labels = _load_labels()
    if not labels or not MODEL_PATH.exists():
        _out_json({"name": None, "known_faces": list(labels.keys())})
        return
    id_to_name = {v: k for k, v in labels.items()}
    rec = _load_recognizer()
    detector = _face_detector()
    cap = _open_camera()
    if cap is None:
        _out_json({"name": None, "known_faces": list(labels.keys()), "error": "no camera"})
        return
    best = None
    best_conf = float("inf")
    for _ in range(15):
        ok, frame = cap.read()
        if not ok:
            continue
        rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
        face = _crop_face(rgb, detector)
        if face is None:
            continue
        label, conf = rec.predict(face)
        if conf < best_conf:
            best_conf = conf
            best = label
    cap.release()
    if best is not None and best_conf < RECOGNITION_THRESHOLD:
        _out_json({"name": id_to_name.get(best), "confidence": round(float(best_conf), 1)})
    else:
        _out_json({"name": None, "confidence": round(float(best_conf), 1)})


def cmd_list() -> None:
    labels = _load_labels()
    _out_json({"names": list(labels.keys())})


def cmd_stream() -> None:
    """Live camera stream: emits one JSON line per frame with a JPEG frame
    (base64), current gesture state, and face identification. Also moves the
    cursor when a hand is present (pinch = click), like cmd_gesture."""
    import mediapipe as mp

    try:
        import pyautogui

        pyautogui.FAILSAFE = False
        screen_w, screen_h = pyautogui.size()
    except Exception:
        pyautogui = None
        screen_w, screen_h = 0, 0

    mp_hands = mp.solutions.hands
    hands = mp_hands.Hands(
        static_image_mode=False,
        max_num_hands=1,
        min_detection_confidence=0.6,
        min_tracking_confidence=0.6,
    )
    detector = _face_detector()
    labels = _load_labels()
    id_to_name = {v: k for k, v in labels.items()}
    rec = _load_recognizer() if MODEL_PATH.exists() else None

    cap = _open_camera()
    if cap is None:
        _out_json({"frame": None, "gesture": None, "face": None, "error": "no camera"})
        return

    smooth_x, smooth_y = screen_w / 2, screen_h / 2
    prev_pinch = False
    face_cache_name = None
    face_cache_conf = None
    frame_idx = 0

    while True:
        ok, frame = cap.read()
        if not ok:
            time.sleep(0.03)
            continue
        frame = cv2.flip(frame, 1)
        rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)

        gesture = None
        results = hands.process(rgb)
        if results.multi_hand_landmarks:
            lm = results.multi_hand_landmarks[0].landmark
            index_tip = lm[8]
            thumb_tip = lm[4]
            dx = thumb_tip.x - index_tip.x
            dy = thumb_tip.y - index_tip.y
            pinch = (dx * dx + dy * dy) ** 0.5 < 0.05
            gesture = "pinch" if pinch else "hand"
            if pinch and not prev_pinch and pyautogui is not None:
                pyautogui.click()
            prev_pinch = pinch
            if pyautogui is not None:
                target_x = int(index_tip.x * screen_w)
                target_y = int(index_tip.y * screen_h)
                smooth_x += (target_x - smooth_x) * 0.45
                smooth_y += (target_y - smooth_y) * 0.45
                pyautogui.moveTo(int(smooth_x), int(smooth_y), _pause=False)
        else:
            prev_pinch = False

        face = None
        conf = None
        if rec is not None and frame_idx % 10 == 0:
            crop = _crop_face(rgb, detector)
            if crop is not None:
                label, c = rec.predict(crop)
                if c < RECOGNITION_THRESHOLD:
                    face_cache_name = id_to_name.get(label)
                    face_cache_conf = round(float(c), 1)
                else:
                    face_cache_name = None
                    face_cache_conf = None
        face = face_cache_name
        conf = face_cache_conf
        frame_idx += 1

        ok_jpg, buf = cv2.imencode(".jpg", frame, [cv2.IMWRITE_JPEG_QUALITY, 55])
        b64 = base64.b64encode(buf.tobytes()).decode("ascii") if ok_jpg else ""
        _out_json(
            {
                "frame": b64,
                "gesture": gesture,
                "face": face,
                "confidence": conf,
            }
        )


def main() -> None:
    parser = argparse.ArgumentParser(description="Ranveer vision/gesture bridge")
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("gesture")
    sub.add_parser("identify")
    sub.add_parser("list")
    sub.add_parser("stream")

    p_reg = sub.add_parser("register")
    p_reg.add_argument("name")

    args = parser.parse_args()
    if args.command == "gesture":
        cmd_gesture()
    elif args.command == "identify":
        cmd_identify()
    elif args.command == "list":
        cmd_list()
    elif args.command == "stream":
        cmd_stream()
    elif args.command == "register":
        cmd_register(args.name)


if __name__ == "__main__":
    main()
