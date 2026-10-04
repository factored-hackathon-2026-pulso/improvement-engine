"""Run the shim: python -m roleplay_llm --queue DIR [--port 8640] [--hold-s 55] [--replay-only]."""
import argparse

from .shim import HOLD_S, Shim, serve


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--queue", required=True)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8640)
    ap.add_argument("--hold-s", type=float, default=HOLD_S)
    ap.add_argument("--replay-only", action="store_true")
    ap.add_argument("--api-key", default="dummy")
    a = ap.parse_args()
    server = serve(Shim(a.queue, hold_s=a.hold_s, replay_only=a.replay_only), host=a.host, port=a.port,
                   api_key=a.api_key)
    server.serve_forever()


if __name__ == "__main__":
    main()
