"""Serve Supertonic 3 (Supertone's on-device TTS, ONNX) as an OpenAI-style speech server, to compare voices.

`maru tts-eval --candidate "Supertonic 3=http://127.0.0.1:18096,voice=F1"` then puts it on the listening page
beside the configured voice. This is for comparing, not a voice Maru installs or uses by default.

  pip install supertonic            (the model, OpenRAIL-M licence, is downloaded on first start)
  python tools/supertonic_server.py [--port 18096] [--threads N]

Routes: `GET /health` (also says how long the model took to load), `GET /v1/models`, `GET /v1/audio/voices`,
`POST /v1/audio/speech` with `{"input": "...", "voice": "F1", "speed": 1.0, "language": "Korean"}` -> WAV.
"""
import argparse
import io
import json
import os
import tempfile
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODEL = "supertonic-3"
# The names the package ships styles for; the server answers 400 for one it cannot load.
VOICES = ["F1", "F2", "F3", "F4", "F5", "M1", "M2", "M3", "M4", "M5"]
LANGUAGES = {"korean": "ko", "english": "en", "japanese": "ja", "spanish": "es"}


def language_code(name):
    """`Korean` (what Maru sends) or a code such as `ko`; Korean when nothing is said."""
    name = (name or "ko").strip().lower()
    return LANGUAGES.get(name, name[:2] or "ko")


def speech_speed(asked):
    """Maru's 1.0 is "normal": the package's own default pace."""
    return 1.05 * float(asked or 1.0)


class Voice:
    def __init__(self, threads=None):
        from supertonic import TTS  # imported here: the tests need no model

        started = time.perf_counter()
        self.tts = TTS(model=MODEL, auto_download=True, intra_op_num_threads=threads)
        self.styles = {}
        # One short sentence, so the first real request does not pay for the first run.
        self.speak("안녕하세요.", "F1", 1.0, "ko")
        self.load_seconds = time.perf_counter() - started

    def speak(self, text, voice, speed, lang):
        if voice not in self.styles:
            self.styles[voice] = self.tts.get_voice_style(voice_name=voice)
        wav, _ = self.tts.synthesize(text, voice_style=self.styles[voice], speed=speech_speed(speed), lang=lang)
        # The package writes the file itself (it knows its sample rate).
        fd, path = tempfile.mkstemp(suffix=".wav")
        os.close(fd)
        try:
            self.tts.save_audio(wav, path)
            return open(path, "rb").read()
        finally:
            os.remove(path)


def handler(voice):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def reply(self, status, body, kind="application/json"):
            data = body if isinstance(body, bytes) else json.dumps(body, ensure_ascii=False).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            if self.path == "/health":
                self.reply(200, {"status": "ok", "model": MODEL, "load_seconds": round(voice.load_seconds, 2)})
            elif self.path == "/v1/models":
                self.reply(200, {"data": [{"id": MODEL}]})
            elif self.path == "/v1/audio/voices":
                self.reply(200, {"voices": VOICES})
            else:
                self.reply(404, {"error": {"message": "no such route"}})

        def do_POST(self):
            if self.path != "/v1/audio/speech":
                return self.reply(404, {"error": {"message": "no such route"}})
            try:
                request = json.load(io.BytesIO(self.rfile.read(int(self.headers.get("Content-Length", 0)))))
                text = (request.get("input") or "").strip()
                if not text:
                    return self.reply(400, {"error": {"message": "nothing to say: `input` is empty"}})
                audio = voice.speak(text, request.get("voice") or "F1", request.get("speed"), language_code(request.get("language")))
            except Exception as error:  # a bad voice name, a language the model lacks…
                return self.reply(400, {"error": {"message": f"{type(error).__name__}: {error}"}})
            self.reply(200, audio, "audio/wav")

    return Handler


def main():
    args = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    args.add_argument("--port", type=int, default=18096)
    args.add_argument("--threads", type=int, default=None, help="CPU threads for ONNX Runtime (default: its own choice)")
    args = args.parse_args()
    voice = Voice(args.threads)
    print(f"Supertonic 3 loaded in {voice.load_seconds:.1f} s; listening on http://127.0.0.1:{args.port} (process {os.getpid()})", flush=True)
    ThreadingHTTPServer(("127.0.0.1", args.port), handler(voice)).serve_forever()


if __name__ == "__main__":
    main()
