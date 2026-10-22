"""WRITEpad narration worker: Kokoro TTS over a line-based JSON protocol.

    python kokoro_worker.py <model_dir>

On start, prints {"ready": true, "voices": [...]}. Then for each request line
{"text", "voice", "speed", "lexicon", "out"} it renders `text` to a WAV file
at `out` and prints {"ok": true, "seconds": n}, or {"error": "..."}.

Lexicon entries override pronunciation: the value is either respelled text
("WER-bose") or raw IPA between slashes ("/wˈɜɹbOs/").
"""

import json
import re
import sys
from pathlib import Path

# Keep stdout for protocol replies only; library chatter goes to stderr.
reply_stream = sys.stdout
sys.stdout = sys.stderr


def reply(obj: dict) -> None:
    reply_stream.write(json.dumps(obj) + "\n")
    reply_stream.flush()


try:
    import soundfile as sf
    from kokoro_onnx import Kokoro

    model_dir = Path(sys.argv[1])
    kokoro = Kokoro(str(model_dir / "kokoro-v1.0.onnx"), str(model_dir / "voices-v1.0.bin"))
except Exception as e:  # noqa: BLE001 - report any setup failure to the app
    reply({"error": f"Kokoro failed to load: {e}"})
    sys.exit(1)

LANG = "en-us"


def phonemes_for(text: str, lexicon: dict) -> str:
    """Phonemize text, splicing in lexicon pronunciations for whole words."""
    if not lexicon:
        return kokoro.tokenizer.phonemize(text, LANG)
    words = sorted(lexicon, key=len, reverse=True)
    pattern = re.compile(r"\b(" + "|".join(re.escape(w) for w in words) + r")\b", re.IGNORECASE)
    lookup = {w.lower(): v for w, v in lexicon.items()}
    parts = []
    pos = 0
    for m in pattern.finditer(text):
        if m.start() > pos:
            parts.append(kokoro.tokenizer.phonemize(text[pos : m.start()], LANG))
        say = lookup[m.group(0).lower()].strip()
        if len(say) > 2 and say.startswith("/") and say.endswith("/"):
            parts.append(say[1:-1])
        else:
            parts.append(kokoro.tokenizer.phonemize(say, LANG))
        pos = m.end()
    if pos < len(text):
        parts.append(kokoro.tokenizer.phonemize(text[pos:], LANG))
    return " ".join(p.strip() for p in parts if p.strip())


reply({"ready": True, "voices": sorted(kokoro.get_voices())})

for line in sys.stdin:
    try:
        req = json.loads(line)
        phonemes = phonemes_for(req["text"], req.get("lexicon") or {})
        audio, sr = kokoro.create(
            phonemes,
            voice=req.get("voice") or "af_heart",
            speed=float(req.get("speed") or 1.0),
            lang=LANG,
            is_phonemes=True,
        )
        sf.write(req["out"], audio, sr, subtype="PCM_16")
        reply({"ok": True, "seconds": round(len(audio) / sr, 3)})
    except Exception as e:  # noqa: BLE001 - one bad sentence shouldn't kill the worker
        reply({"error": str(e)})
